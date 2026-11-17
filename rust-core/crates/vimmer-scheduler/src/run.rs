use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};

use tokio::{sync::Semaphore, task::JoinSet};
use vimmer_dsl::{validate, ValidationError, VideoTimeline};
use vimmer_media::{normalize_image, AudioProbe, MediaError, Speech};

use crate::control::{Progress, RenderContext};
use crate::narrate::{narrate_scenes, TtsConfig};
use crate::plan::{chunk_props, plan_chunks, Chunk, DEFAULT_CHUNK_MS};
use crate::wordsubs::{add_word_subtitles_with, PageWords, WordSubtitleConfig};

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
    #[error("music file not found: {0}")]
    MusicMissing(String),
    #[error("{0}")]
    MusicPick(String),
    #[error("mixing audio failed: {0}")]
    Mix(String),
    #[error("{what}: {message}")]
    Video { what: String, message: String },
    /// Narration cannot be made (Kokoro is needed but missing or failing).
    #[error("{0}")]
    Narration(String),
    #[error("voice-over clip `{clip}`: file not found: {path}")]
    VoiceoverMissing { clip: String, path: String },
    #[error("brand {what}: file not found: {path}")]
    BrandAssetMissing { what: String, path: String },
    #[error("scene `{scene}` sound effect {index}: file not found: {path}")]
    SfxMissing {
        scene: String,
        index: usize,
        path: String,
    },
    #[error("scene `{scene}` layer `{layer}`: file not found: {path}")]
    LayerAssetMissing {
        scene: String,
        layer: String,
        path: String,
    },
}

/// Replaces `music.pick` by the chosen library track, noting which one and why.
pub fn resolve_music(
    timeline: &mut VideoTimeline,
    public_dir: &Path,
    notes: &mut Vec<String>,
    warnings: &mut Vec<String>,
) -> Result<(), SchedulerError> {
    if timeline.music.as_ref().is_none_or(|m| m.pick.is_none()) {
        return Ok(());
    }
    let pick_error = |e: vimmer_dsl::LibraryError| SchedulerError::MusicPick(e.to_string());
    let (library, missing) = vimmer_dsl::MusicLibrary::load(public_dir).map_err(pick_error)?;
    if !missing.is_empty() {
        warnings.push(format!(
            "music library: file missing for {}; picking among the others",
            missing.join(", ")
        ));
    }
    if let Some(choice) = vimmer_dsl::resolve_music_pick(timeline, &library).map_err(pick_error)? {
        notes.push(format!(
            "music: {} ({}) - {}",
            choice.track.source(),
            choice.track.title,
            choice.reason
        ));
    }
    Ok(())
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
    /// Extra folders an asset path may be found in (a Studio project, the timeline's own
    /// folder). Files found there are staged into the public dir (see `localize.rs`). Empty:
    /// every path is relative to `public_dir`, as it always was.
    pub asset_dirs: Vec<PathBuf>,
    /// `true`: a file in `asset_dirs` wins over the same path in `public_dir` (a Studio
    /// project). `false`: `asset_dirs` only supply paths that `public_dir` does not have.
    pub asset_dirs_first: bool,
    pub chunk_ms: u64,
    pub preprocess_concurrency: usize,
    pub render_concurrency: usize,
    pub ffmpeg: PathBuf,
    /// PATH for child processes. Set this when the parent's PATH is minimal (e.g. a GUI app).
    pub child_path: Option<String>,
    /// Word-by-word subtitle timing for synthesized narration (on by default).
    pub word_subtitles: WordSubtitleConfig,
    /// Narration engine (`say` unless chosen here or by `$VIMMER_TTS`).
    pub tts: TtsConfig,
    /// The narrator avatar: show ready clips (default), require them, off, or generate/status
    /// only (see `avatar.rs`).
    pub avatar: crate::avatar::AvatarConfig,
}

impl SchedulerConfig {
    pub fn new(work_dir: PathBuf, output: PathBuf, remotion_dir: PathBuf) -> Self {
        let public_dir = remotion_dir.join("public");
        Self {
            work_dir,
            output,
            remotion_dir,
            public_dir,
            asset_dirs: Vec::new(),
            asset_dirs_first: false,
            chunk_ms: DEFAULT_CHUNK_MS,
            preprocess_concurrency: 4,
            render_concurrency: 2,
            ffmpeg: PathBuf::from("ffmpeg"),
            child_path: None,
            word_subtitles: WordSubtitleConfig::default(),
            tts: TtsConfig::default(),
            avatar: crate::avatar::AvatarConfig::default(),
        }
    }
}

impl SchedulerConfig {
    /// The same config with `work_dir`, `output`, `remotion_dir` and `public_dir` made absolute
    /// against the current directory (without touching the filesystem).
    pub fn absolutized(mut self) -> Result<Self, SchedulerError> {
        for path in [
            &mut self.work_dir,
            &mut self.output,
            &mut self.remotion_dir,
            &mut self.public_dir,
        ] {
            *path = std::path::absolute(&*path)?;
        }
        for dir in &mut self.asset_dirs {
            *dir = std::path::absolute(&*dir)?;
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunReport {
    pub output: PathBuf,
    pub chunks: usize,
    pub normalized_images: usize,
    /// Video clip files (pictures and sounds) encoded by this render; cached ones are not counted.
    pub encoded_videos: usize,
    pub warnings: Vec<String>,
    /// What the render did that is worth a line in the log (beat sync results).
    pub notes: Vec<String>,
    /// The narrator avatar's clips (empty without an avatar).
    pub avatar: Vec<crate::avatar::RunStatus>,
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
    // Remotion runs inside `remotion_dir`, so a relative props or output path handed to it
    // would resolve against the wrong directory. Anchor every path at our own cwd first.
    let mut cfg = cfg.absolutized()?;
    // Which scene audio files are recordings is read from their paths, before staging renames
    // them (word subtitles time recordings like narration).
    cfg.word_subtitles.recorded_scenes = crate::wordsubs::recorded_scenes(&timeline);
    ctx.emit(Progress::Stage {
        name: "Checking timeline".into(),
    });
    // Template scenes become ordinary layers first; everything below sees the expanded timeline.
    if let Err(errors) = vimmer_dsl::expand_templates(&mut timeline) {
        return Err(SchedulerError::Invalid(
            errors
                .into_iter()
                .map(|e| ValidationError::BadTemplate {
                    scene: e.scene,
                    message: e.message,
                })
                .collect(),
        ));
    }
    validate(&timeline).map_err(SchedulerError::Invalid)?;
    // Before staging rewrites paths: the same signature `vimmer check` computes.
    let avatar_signature = vimmer_dsl::avatar_plan_signature(&timeline);
    tokio::fs::create_dir_all(&cfg.work_dir).await?;
    let mut warnings = Vec::new();
    let mut notes = Vec::new();
    ctx.check()?;

    // Files of a project (or next to the timeline) are staged into the public dir and their
    // paths rewritten; from here on every asset path is relative to `public_dir`. Without
    // `asset_dirs`, or without a file found there, this changes nothing.
    crate::localize::stage_assets(&mut timeline, &cfg, &mut notes)?;

    // A music pick becomes a file (and, with beat sync, the track's beat grid) before anything
    // reads the music.
    resolve_music(&mut timeline, &cfg.public_dir, &mut notes, &mut warnings)?;

    if let Some(music) = &timeline.music {
        let path = cfg.public_dir.join(music.source_path());
        if !path.is_file() {
            return Err(SchedulerError::MusicMissing(path.display().to_string()));
        }
    }

    check_brand_assets(&timeline, &cfg.public_dir)?;

    // Layer images and icons are drawn as they are (not normalized), so only their presence is
    // checked here; a missing file would otherwise fail inside the renderer.
    for scene in &timeline.scenes {
        for layer in &scene.layers {
            if let Some(src) = layer.content.asset() {
                let path = cfg.public_dir.join(src);
                if !path.is_file() {
                    return Err(SchedulerError::LayerAssetMissing {
                        scene: scene.id.clone(),
                        layer: layer.id.clone(),
                        path: path.display().to_string(),
                    });
                }
            }
        }
    }

    crate::video::check_scene_videos(&timeline, &cfg)?;
    check_sfx_files(&timeline, &cfg.public_dir)?;
    check_voiceover_files(&timeline, &cfg.public_dir)?;

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

    // Scene lengths are final now: move the cuts onto the music's beats before anything reads
    // scene start times (word subtitles, clip fitting, audio placement, chunks).
    if let Some(report) =
        crate::beatsync::apply_beat_sync(&mut timeline, &cfg, probe.clone(), &ctx, &mut warnings)
            .await?
    {
        notes.push(report.summary(timeline.fps));
        notes.extend(report.lines(timeline.fps));
    }

    // Recognize the narration again to time subtitles word by word. Without an engine this
    // only warns, and subtitles stay as they were.
    let plain_subtitles = timeline.subtitles.clone();
    let mut page_words = PageWords::new();
    let timed = add_word_subtitles_with(
        &mut timeline,
        &cfg,
        probe.clone(),
        &ctx,
        &mut warnings,
        &mut notes,
        &mut page_words,
    )
    .await?;
    if timed > 0 {
        // A word-timing bug must never fail the render: fall back to the plain subtitles.
        if let Err(errors) = validate(&timeline) {
            warnings.push(format!(
                "word-by-word subtitles dropped, they did not validate: {}",
                join(&errors)
            ));
            timeline.subtitles = plain_subtitles;
        }
    }

    // Paged scene texts: when each page starts, from the final scene lengths and the words.
    crate::pages::add_text_pages(&mut timeline, &page_words, &cfg, probe.clone(), &mut notes)
        .await?;

    ctx.emit(Progress::Stage {
        name: "Preparing images".into(),
    });
    let normalized_images = preprocess_images(&mut timeline, &cfg, &ctx).await?;
    // After narration, which can lengthen scenes: clips are fitted to their final length.
    let (encoded_videos, clip_sounds) =
        crate::video::preprocess_videos(&mut timeline, &cfg, &ctx, &mut warnings).await?;

    ctx.emit(Progress::Stage {
        name: "Checking audio".into(),
    });
    let mut placements = crate::audio::place_scene_audio(&timeline);
    let mut clip_ms = check_audio(
        &timeline,
        &placements,
        &cfg,
        probe.clone(),
        &mut warnings,
        &ctx,
    )
    .await?;
    // Video clip sounds go after the scene audio, so a timeline without them mixes as before.
    let (sound_placements, sound_ms) = crate::audio::place_clip_sounds(&timeline, &clip_sounds);
    placements.extend(sound_placements);
    clip_ms.extend(sound_ms);
    // The voice-over track next: it ducks the music like narration. Without one, nothing changes.
    let voiceover_ms = probe_voiceover(&timeline, &cfg, probe.clone(), &mut warnings, &ctx).await?;
    let (voiceover, voiceover_audible) = crate::audio::place_voiceover(&timeline, &voiceover_ms);
    // The narrator avatar follows the speech: narration and scene audio, and the voice-over.
    // Without an avatar this does nothing.
    let speech: Vec<_> = crate::audio::place_scene_audio(&timeline)
        .into_iter()
        .chain(voiceover.iter().cloned())
        .collect();
    let avatar = crate::avatar::apply(
        &mut timeline,
        &speech,
        avatar_signature.as_deref(),
        &cfg,
        &ctx,
        &mut warnings,
    )
    .await?;
    if cfg.avatar.mode.stops_early() {
        return Ok(RunReport {
            output: cfg.output.clone(),
            chunks: 0,
            normalized_images,
            encoded_videos,
            warnings,
            notes,
            avatar,
        });
    }
    placements.extend(voiceover);
    clip_ms.extend(voiceover_audible);
    // Sound effects last; only those with `duck_music` count toward ducking.
    let sfx = crate::audio::place_sfx(&timeline);
    let has_sfx = !sfx.is_empty();
    for (p, ducks) in sfx {
        let ms = if ducks {
            let path = cfg.public_dir.join(&p.source);
            let probe = probe.clone();
            tokio::task::spawn_blocking(move || probe.duration_ms(&path))
                .await
                .map_err(|e| SchedulerError::Join(e.to_string()))??
        } else {
            0
        };
        placements.push(p);
        clip_ms.push(ms);
    }

    let chunks = plan_chunks(&timeline, cfg.chunk_ms);
    ctx.emit(Progress::Stage {
        name: "Rendering video".into(),
    });
    let rendered = render_chunks(&timeline, &chunks, &cfg, &ctx).await?;

    ctx.emit(Progress::Stage {
        name: "Joining segments".into(),
    });
    // Chunks are rendered without audio; the whole soundtrack is mixed once, from the timeline,
    // so the audio never drifts at a seam (see audio.rs).
    let joined = cfg.work_dir.join("joined.mp4");
    concat(&rendered, &joined, &cfg, &ctx).await?;
    if let Some(parent) = cfg.output.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    ctx.emit(Progress::Stage {
        name: "Mixing audio".into(),
    });
    let mixed = crate::audio::mux_soundtrack(
        &timeline,
        &placements,
        &clip_ms,
        has_sfx,
        &joined,
        &cfg.output,
        &cfg,
        &ctx,
    )
    .await?;
    if !mixed {
        tokio::fs::copy(&joined, &cfg.output).await?;
    }
    let _ = tokio::fs::remove_file(&joined).await;

    ctx.emit(Progress::Done {
        output: cfg.output.display().to_string(),
    });
    Ok(RunReport {
        output: cfg.output.clone(),
        chunks: chunks.len(),
        normalized_images,
        encoded_videos,
        warnings,
        notes,
        avatar,
    })
}

/// Brand fonts and the logo are loaded by the renderer, so a missing file would only fail there
/// (a font silently falls back): check them up front, like layer images.
pub(crate) fn check_brand_assets(
    timeline: &VideoTimeline,
    public_dir: &Path,
) -> Result<(), SchedulerError> {
    let Some(brand) = &timeline.brand else {
        return Ok(());
    };
    for (what, src) in brand.assets() {
        let path = public_dir.join(src);
        if !path.is_file() {
            return Err(SchedulerError::BrandAssetMissing {
                what: what.to_string(),
                path: path.display().to_string(),
            });
        }
    }
    Ok(())
}

/// Sound effect files are mixed after the render, so a missing one is reported before it.
pub(crate) fn check_sfx_files(
    timeline: &VideoTimeline,
    public_dir: &Path,
) -> Result<(), SchedulerError> {
    for scene in &timeline.scenes {
        for (index, fx) in scene.sfx.iter().enumerate() {
            let path = public_dir.join(&fx.src);
            if !path.is_file() {
                return Err(SchedulerError::SfxMissing {
                    scene: scene.id.clone(),
                    index,
                    path: path.display().to_string(),
                });
            }
        }
    }
    Ok(())
}

/// Runs an external command in its own process group, so cancelling can kill the whole tree.
pub(crate) async fn run_process(
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

/// Every voice-over clip's file must exist (the mix would otherwise fail inside ffmpeg).
pub(crate) fn check_voiceover_files(
    timeline: &VideoTimeline,
    public_dir: &std::path::Path,
) -> Result<(), SchedulerError> {
    for clip in &timeline.voiceover {
        let path = public_dir.join(&clip.source);
        if !path.is_file() {
            return Err(SchedulerError::VoiceoverMissing {
                clip: clip.id.clone(),
                path: path.display().to_string(),
            });
        }
    }
    Ok(())
}

/// Probes every voice-over file (lengths in clip order) and warns about clips that overlap or
/// run past the end of the video (they are cut there).
pub(crate) async fn probe_voiceover(
    timeline: &VideoTimeline,
    cfg: &SchedulerConfig,
    probe: Arc<dyn AudioProbe>,
    warnings: &mut Vec<String>,
    ctx: &RenderContext,
) -> Result<Vec<u64>, SchedulerError> {
    let mut lengths = Vec::with_capacity(timeline.voiceover.len());
    for clip in &timeline.voiceover {
        let path = cfg.public_dir.join(&clip.source);
        let probe = probe.clone();
        let ms = tokio::task::spawn_blocking(move || probe.duration_ms(&path))
            .await
            .map_err(|e| SchedulerError::Join(e.to_string()))??;
        lengths.push(ms);
        ctx.check()?;
    }
    warnings.extend(voiceover_warnings(timeline, &lengths));
    Ok(lengths)
}

/// Render warnings for the voice-over track, given each clip's file length.
pub(crate) fn voiceover_warnings(timeline: &VideoTimeline, lengths: &[u64]) -> Vec<String> {
    let clips = &timeline.voiceover;
    let total = timeline.total_duration_ms();
    let mut out = Vec::new();
    for (a, b) in vimmer_dsl::overlapping_clips(clips, lengths) {
        out.push(format!(
            "voice-over clips `{}` and `{}` overlap; both play at once",
            clips[a].id, clips[b].id
        ));
    }
    for (clip, &ms) in clips.iter().zip(lengths) {
        let end = clip.end_ms(ms);
        if clip.start_ms < total && end > total {
            out.push(format!(
                "voice-over clip `{}` runs to {end} ms, past the end of the video ({total} ms); the last {} ms are cut",
                clip.id,
                end - total
            ));
        }
    }
    out
}

/// Probes the length of every placed clip and warns when one runs past the end of its scene.
/// Returns the lengths in the order of `placements`.
async fn check_audio(
    timeline: &VideoTimeline,
    placements: &[crate::audio::Placement],
    cfg: &SchedulerConfig,
    probe: Arc<dyn AudioProbe>,
    warnings: &mut Vec<String>,
    ctx: &RenderContext,
) -> Result<Vec<u64>, SchedulerError> {
    let mut durations = Vec::with_capacity(placements.len());
    for (index, scene) in timeline.scenes.iter().enumerate() {
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
        if placements.iter().any(|p| p.scene == index) {
            durations.push(duration);
        }
        ctx.check()?;
    }
    Ok(durations)
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
        let sub = chunk_props(timeline, chunk);
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
        // The soundtrack is mixed once for the whole timeline (audio.rs), never per chunk.
        .arg("--muted")
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

/// Joins rendered (silent) chunks into `out` with ffmpeg's concat demuxer without re-encoding.
async fn concat(
    parts: &[PathBuf],
    out: &Path,
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
    .args(["-c", "copy", "-an"])
    .arg(out);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voiceover_files_and_warnings() {
        let dir = std::env::temp_dir().join(format!("vimmer-vo-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("rec")).unwrap();
        std::fs::write(dir.join("rec/a.wav"), b"wav").unwrap();
        let t: VideoTimeline = serde_json::from_str(
            r#"{"fps":30,"resolution":{"width":1920,"height":1080},"scenes":[{"id":"a","duration_ms":3000,"text":"a"}],
            "voiceover":[{"id":"one","source":"rec/a.wav","start_ms":0},{"id":"two","source":"rec/b.wav","start_ms":1500}]}"#,
        )
        .unwrap();
        let err = check_voiceover_files(&t, &dir).unwrap_err().to_string();
        assert!(err.starts_with("voice-over clip `two`: file not found:"), "{err}");
        assert_eq!(
            voiceover_warnings(&t, &[2000, 2000]),
            vec![
                "voice-over clips `one` and `two` overlap; both play at once".to_string(),
                "voice-over clip `two` runs to 3500 ms, past the end of the video (3000 ms); the last 500 ms are cut".to_string(),
            ]
        );
        assert!(voiceover_warnings(&t, &[1500, 1500]).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_brand_files_are_named() {
        let dir = std::env::temp_dir().join(format!("vimmer-brand-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("assets/logo.png"), b"png").unwrap();
        let mut t: VideoTimeline = serde_json::from_str(
            r#"{"fps":30,"resolution":{"width":1920,"height":1080},"scenes":[{"id":"a","duration_ms":1000,"text":"a"}],
            "brand":{"fonts":{"heading":{"family":"Demo","src":"assets/demo.woff2"}},"logo":{"src":"assets/logo.png"}}}"#,
        )
        .unwrap();
        let err = check_brand_assets(&t, &dir).unwrap_err().to_string();
        assert!(
            err.starts_with("brand heading font: file not found:"),
            "{err}"
        );
        assert!(err.ends_with("assets/demo.woff2"), "{err}");
        t.brand.as_mut().unwrap().fonts.heading = None;
        check_brand_assets(&t, &dir).unwrap();
        t.brand = None;
        check_brand_assets(&t, &dir).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn relative_paths_are_anchored_at_the_current_dir() {
        let cfg = SchedulerConfig::new(
            PathBuf::from("target/vimmer-work"),
            PathBuf::from("out.mp4"),
            PathBuf::from("../remotion-renderer"),
        )
        .absolutized()
        .unwrap();
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(cfg.work_dir, cwd.join("target/vimmer-work"));
        assert_eq!(cfg.output, cwd.join("out.mp4"));
        assert!(cfg.remotion_dir.is_absolute());
        assert!(cfg.public_dir.is_absolute());
        assert!(cfg.public_dir.ends_with("remotion-renderer/public"));
        let abs = SchedulerConfig::new(
            PathBuf::from("/w"),
            PathBuf::from("/o.mp4"),
            PathBuf::from("/r"),
        )
        .absolutized()
        .unwrap();
        assert_eq!(abs.work_dir, PathBuf::from("/w"));
    }
}
