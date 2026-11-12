use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};

use tokio::{sync::Semaphore, task::JoinSet};
use vimmer_dsl::{validate, ValidationError, VideoTimeline};
use vimmer_media::{normalize_image, AudioProbe, MediaError, Speech};

use crate::control::{Progress, RenderContext};
use crate::narrate::narrate_scenes;
use crate::plan::{chunk_timeline, plan_chunks, Chunk, DEFAULT_CHUNK_MS};

#[derive(Debug, thiserror::Error)]
pub enum SchedulerError {
    #[error("timeline is invalid: {}", join(.0))]
    Invalid(Vec<ValidationError>),
    #[error(transparent)]
    Media(#[from] MediaError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("render of chunk {chunk} failed: {stderr}")]
    RenderFailed { chunk: usize, stderr: String },
    #[error("ffmpeg concat failed: {0}")]
    Concat(String),
    #[error("background task panicked: {0}")]
    Join(String),
    #[error("render cancelled")]
    Cancelled,
}

fn join(errors: &[ValidationError]) -> String {
    errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Scratch space for chunk props, chunk renders and the concat list.
    pub work_dir: PathBuf,
    /// Final MP4 path.
    pub output: PathBuf,
    /// Remotion project root (contains `package.json` and `src/index.ts`).
    pub remotion_dir: PathBuf,
    /// Remotion public dir. Image and audio sources are relative to it; normalized
    /// images are written to `<public_dir>/generated/`.
    pub public_dir: PathBuf,
    pub chunk_ms: u64,
    pub preprocess_concurrency: usize,
    pub render_concurrency: usize,
    pub ffmpeg: PathBuf,
    /// PATH for child processes. Set this when the parent's PATH is minimal (e.g. a GUI app).
    pub child_path: Option<String>,
}

impl SchedulerConfig {
    pub fn new(work_dir: PathBuf, output: PathBuf, remotion_dir: PathBuf) -> Self {
        let public_dir = remotion_dir.join("public");
        Self {
            work_dir,
            output,
            remotion_dir,
            public_dir,
            chunk_ms: DEFAULT_CHUNK_MS,
            preprocess_concurrency: 4,
            render_concurrency: 2,
            ffmpeg: PathBuf::from("ffmpeg"),
            child_path: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunReport {
    pub output: PathBuf,
    pub chunks: usize,
    pub normalized_images: usize,
    pub warnings: Vec<String>,
}

/// Renders a whole timeline to one MP4, without progress reporting or cancellation.
pub async fn render_timeline(
    timeline: VideoTimeline,
    cfg: SchedulerConfig,
    probe: Arc<dyn AudioProbe>,
    speech: Arc<dyn Speech>,
) -> Result<RunReport, SchedulerError> {
    render_timeline_with(timeline, cfg, probe, speech, RenderContext::silent()).await
}

/// Renders a whole timeline, reporting progress and honouring cancellation through `ctx`.
pub async fn render_timeline_with(
    mut timeline: VideoTimeline,
    cfg: SchedulerConfig,
    probe: Arc<dyn AudioProbe>,
    speech: Arc<dyn Speech>,
    ctx: RenderContext,
) -> Result<RunReport, SchedulerError> {
    ctx.emit(Progress::Stage {
        name: "Checking timeline".into(),
    });
    validate(&timeline).map_err(SchedulerError::Invalid)?;
    tokio::fs::create_dir_all(&cfg.work_dir).await?;
    let mut warnings = Vec::new();
    ctx.check()?;

    // Narration can stretch scenes, so the timeline is validated again afterwards.
    let narrated = narrate_scenes(
        &mut timeline,
        &cfg,
        speech,
        probe.clone(),
        &ctx,
        &mut warnings,
    )
    .await?;
    if narrated > 0 {
        validate(&timeline).map_err(SchedulerError::Invalid)?;
    }

    ctx.emit(Progress::Stage {
        name: "Preparing images".into(),
    });
    let normalized_images = preprocess_images(&mut timeline, &cfg, &ctx).await?;

    ctx.emit(Progress::Stage {
        name: "Checking audio".into(),
    });
    check_audio(&timeline, &cfg, probe, &mut warnings, &ctx).await?;

    let chunks = plan_chunks(&timeline, cfg.chunk_ms);
    ctx.emit(Progress::Stage {
        name: "Rendering video".into(),
    });
    let rendered = render_chunks(&timeline, &chunks, &cfg, &ctx).await?;

    ctx.emit(Progress::Stage {
        name: "Joining segments".into(),
    });
    concat(&rendered, &cfg, &ctx).await?;

    ctx.emit(Progress::Done {
        output: cfg.output.display().to_string(),
    });
    Ok(RunReport {
        output: cfg.output.clone(),
        chunks: chunks.len(),
        normalized_images,
        warnings,
    })
}

/// Runs an external command in its own process group, so cancelling can kill the whole tree.
async fn run_process(
    mut cmd: tokio::process::Command,
    cfg: &SchedulerConfig,
    ctx: &RenderContext,
) -> Result<std::process::Output, SchedulerError> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    if let Some(path) = &cfg.child_path {
        cmd.env("PATH", path);
    }
    let child = cmd.spawn()?;
    let pid = child.id().unwrap_or(0);
    ctx.cancel.track(pid);
    let output = child.wait_with_output().await;
    ctx.cancel.untrack(pid);
    let output = output?;
    ctx.check()?;
    Ok(output)
}

/// Normalizes every local image to the output frame in parallel. Rewrites each
/// scene's `image` to the public-relative path of the normalized copy.
async fn preprocess_images(
    timeline: &mut VideoTimeline,
    cfg: &SchedulerConfig,
    ctx: &RenderContext,
) -> Result<usize, SchedulerError> {
    let gen_dir = cfg.public_dir.join("generated");
    tokio::fs::create_dir_all(&gen_dir).await?;

    let mut unique: Vec<String> = Vec::new();
    for scene in &timeline.scenes {
        if let Some(img) = &scene.image {
            if !is_url(img) && !unique.contains(img) {
                unique.push(img.clone());
            }
        }
    }

    let total = unique.len();
    let semaphore = Arc::new(Semaphore::new(cfg.preprocess_concurrency.max(1)));
    let mut tasks = JoinSet::new();
    let mut outputs: Vec<(String, String)> = Vec::new();
    for (i, rel) in unique.iter().enumerate() {
        let file = format!("{i:03}-{}.png", sanitize(rel));
        let public_rel = format!("generated/{file}");
        let src = cfg.public_dir.join(rel);
        let dst = gen_dir.join(&file);
        let target = timeline.resolution;
        let permit = semaphore.clone();
        outputs.push((rel.clone(), public_rel));
        tasks.spawn(async move {
            let _permit = permit.acquire_owned().await.expect("semaphore open");
            tokio::task::spawn_blocking(move || normalize_image(&src, &dst, target))
                .await
                .map_err(|e| SchedulerError::Join(e.to_string()))?
                .map_err(SchedulerError::from)
        });
    }
    let mut done = 0;
    ctx.emit(Progress::Images { done, total });
    while let Some(res) = tasks.join_next().await {
        res.map_err(|e| SchedulerError::Join(e.to_string()))??;
        done += 1;
        ctx.emit(Progress::Images { done, total });
        ctx.check()?;
    }

    for scene in &mut timeline.scenes {
        if let Some(img) = &scene.image {
            if let Some((_, public_rel)) = outputs.iter().find(|(orig, _)| orig == img) {
                scene.image = Some(public_rel.clone());
            }
        }
    }
    Ok(outputs.len())
}

/// Probes audio durations and warns when a clip runs past the end of its scene.
async fn check_audio(
    timeline: &VideoTimeline,
    cfg: &SchedulerConfig,
    probe: Arc<dyn AudioProbe>,
    warnings: &mut Vec<String>,
    ctx: &RenderContext,
) -> Result<(), SchedulerError> {
    for scene in &timeline.scenes {
        let Some(audio) = &scene.audio else { continue };
        let path = cfg.public_dir.join(&audio.source);
        let probe = probe.clone();
        let duration = tokio::task::spawn_blocking(move || probe.duration_ms(&path))
            .await
            .map_err(|e| SchedulerError::Join(e.to_string()))??;
        let available = scene.duration_ms.saturating_sub(audio.offset_ms);
        if duration > available {
            warnings.push(format!(
                "scene `{}`: audio `{}` is {duration} ms but only {available} ms fit before the scene ends; it will be cut",
                scene.id, audio.source
            ));
        }
        ctx.check()?;
    }
    Ok(())
}

/// Renders each chunk through the Remotion CLI, at most `render_concurrency` at a time.
async fn render_chunks(
    timeline: &VideoTimeline,
    chunks: &[Chunk],
    cfg: &SchedulerConfig,
    ctx: &RenderContext,
) -> Result<Vec<PathBuf>, SchedulerError> {
    let chunk_dir = cfg.work_dir.join("chunks");
    tokio::fs::create_dir_all(&chunk_dir).await?;

    let total = chunks.len();
    let semaphore = Arc::new(Semaphore::new(cfg.render_concurrency.max(1)));
    let mut tasks = JoinSet::new();
    let mut outputs = Vec::with_capacity(chunks.len());

    for chunk in chunks {
        let sub = chunk_timeline(timeline, chunk);
        let props = chunk_dir.join(format!("chunk-{:03}.json", chunk.index));
        let mp4 = chunk_dir.join(format!("chunk-{:03}.mp4", chunk.index));
        tokio::fs::write(
            &props,
            serde_json::to_vec_pretty(&sub).expect("serializable"),
        )
        .await?;
        outputs.push(mp4.clone());

        let permit = semaphore.clone();
        let cfg = cfg.clone();
        let ctx = ctx.clone();
        let index = chunk.index;
        tasks.spawn(async move {
            let _permit = permit.acquire_owned().await.expect("semaphore open");
            ctx.check()?;
            run_remotion(&cfg, &props, &mp4, &ctx)
                .await
                .map_err(|stderr| SchedulerError::RenderFailed {
                    chunk: index,
                    stderr,
                })
        });
    }
    let mut done = 0;
    ctx.emit(Progress::Chunks { done, total });
    while let Some(res) = tasks.join_next().await {
        res.map_err(|e| SchedulerError::Join(e.to_string()))??;
        done += 1;
        ctx.emit(Progress::Chunks { done, total });
    }
    Ok(outputs)
}

/// Returns `Err(stderr)` when Remotion fails; cancellation surfaces as `SchedulerError::Cancelled`
/// through the caller's `ctx.check()`.
async fn run_remotion(
    cfg: &SchedulerConfig,
    props: &Path,
    out: &Path,
    ctx: &RenderContext,
) -> Result<(), String> {
    let mut cmd = tokio::process::Command::new("npx");
    cmd.current_dir(&cfg.remotion_dir)
        .args(["remotion", "render", "src/index.ts", "Timeline"])
        .arg(out)
        .arg(format!("--props={}", props.display()))
        .arg("--log=error");
    let output = run_process(cmd, cfg, ctx)
        .await
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

/// Joins rendered chunks with ffmpeg's concat demuxer without re-encoding.
async fn concat(
    parts: &[PathBuf],
    cfg: &SchedulerConfig,
    ctx: &RenderContext,
) -> Result<(), SchedulerError> {
    let list = cfg.work_dir.join("concat.txt");
    let body: String = parts
        .iter()
        .map(|p| {
            format!(
                "file '{}'\n",
                p.canonicalize().unwrap_or_else(|_| p.clone()).display()
            )
        })
        .collect();
    tokio::fs::write(&list, body).await?;

    if let Some(parent) = cfg.output.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut cmd = tokio::process::Command::new(&cfg.ffmpeg);
    cmd.args([
        "-y",
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "concat",
        "-safe",
        "0",
        "-i",
    ])
    .arg(&list)
    .args(["-c", "copy"])
    .arg(&cfg.output);
    let output = run_process(cmd, cfg, ctx).await?;
    if output.status.success() {
        Ok(())
    } else {
        Err(SchedulerError::Concat(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ))
    }
}

fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

fn sanitize(rel: &str) -> String {
    rel.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}
