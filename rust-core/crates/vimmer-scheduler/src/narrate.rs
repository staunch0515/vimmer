//! Narration step: synthesizes each scene's spoken text (its spoken line, else its text), attaches it as audio and stretches the scene to fit.
//!
//! Two engines: macOS `say` (the default) and Kokoro, a local neural TTS run through a Python
//! helper in one batch per render. When Kokoro is chosen but cannot run, all narration falls back
//! to `say` with one warning, so a render never fails because of it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::Arc;
use std::time::Duration;

use vimmer_dsl::{AudioMapping, VideoTimeline};
use vimmer_media::{
    batch_timeout, default_voice_for, looks_like_kokoro_voice, resolve_kokoro_voice, AudioProbe,
    KokoroItem, KokoroLang, KokoroSpeech, Limits, MediaError, Speech, TtsEngine, ENV_TTS,
};

use crate::control::{Progress, RenderContext};
use crate::run::{SchedulerConfig, SchedulerError};

/// Pause after speech before a scene may end, so a sentence is not clipped.
pub(crate) const TAIL_MS: u64 = 300;

/// Which narration engine to use, and where Kokoro lives.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TtsConfig {
    /// `None`: `$VIMMER_TTS` (`say` or `kokoro`), else `say`.
    pub engine: Option<TtsEngine>,
    /// Kokoro interpreter. `None`: `$VIMMER_KOKORO_PYTHON`, else the default venv.
    pub kokoro_python: Option<PathBuf>,
    /// Kokoro model directory. `None`: `$VIMMER_KOKORO_MODELS`, else the default directory.
    pub kokoro_models: Option<PathBuf>,
    /// Time budget for the Kokoro batch. `None`: derived from the amount of text.
    pub kokoro_timeout: Option<Duration>,
}

impl TtsConfig {
    /// The engine to use: the explicit choice, else `$VIMMER_TTS`, else `say`. An unknown
    /// `$VIMMER_TTS` value adds a warning and means `say`.
    pub fn resolve_engine(&self, warnings: &mut Vec<String>) -> TtsEngine {
        if let Some(engine) = self.engine {
            return engine;
        }
        engine_from_env(std::env::var(ENV_TTS).ok().as_deref(), warnings)
    }
}

/// The engine named by a `$VIMMER_TTS` value; unset or empty means `say`.
fn engine_from_env(value: Option<&str>, warnings: &mut Vec<String>) -> TtsEngine {
    match value {
        Some(v) if !v.trim().is_empty() => TtsEngine::parse(v).unwrap_or_else(|| {
            warnings.push(format!(
                "{ENV_TTS}=`{v}` is not a narration engine (say or kokoro), using say"
            ));
            TtsEngine::Say
        }),
        _ => TtsEngine::Say,
    }
}

/// A scene to narrate.
struct Job {
    index: usize,
    text: String,
    /// The voice the scene asks for, if any.
    voice: Option<String>,
}

/// A narration file ready for a scene.
struct Narrated {
    index: usize,
    public_rel: String,
    path: PathBuf,
    /// The file was already there: nothing was synthesized for it.
    cached: bool,
}

/// Synthesizes narration for every scene that asks for it and has no audio of its own.
/// Results are cached (say: by voice and text; Kokoro: by model, voice, language, speed and
/// text), so an unchanged scene is not synthesized again. Returns the number of scenes narrated
/// and appends a warning for each scene stretched.
pub async fn narrate_scenes(
    timeline: &mut VideoTimeline,
    cfg: &SchedulerConfig,
    speech: Arc<dyn Speech>,
    probe: Arc<dyn AudioProbe>,
    ctx: &RenderContext,
    warnings: &mut Vec<String>,
) -> Result<usize, SchedulerError> {
    let tts_dir = cfg.public_dir.join("generated").join("tts");
    tokio::fs::create_dir_all(&tts_dir).await?;

    let jobs = collect_jobs(timeline, None);
    if jobs.is_empty() {
        return Ok(0);
    }
    let (files, _) = synthesize(timeline, &jobs, cfg, speech, &tts_dir, ctx, warnings).await?;

    let mut narrated = 0;
    for file in files {
        let duration = measure(&probe, file.path).await?;
        ctx.check()?;
        let scene = &mut timeline.scenes[file.index];
        scene.audio = Some(AudioMapping {
            source: file.public_rel,
            offset_ms: 0,
        });
        let needed = duration + TAIL_MS;
        if needed > scene.duration_ms {
            warnings.push(format!(
                "scene `{}` was extended from {} ms to {needed} ms to fit its narration",
                scene.id, scene.duration_ms
            ));
            scene.duration_ms = needed;
        }
        narrated += 1;
    }
    Ok(narrated)
}

/// The scenes a render narrates: narration on, no audio of their own, something to say.
/// `only`: just the scenes with these ids (a preview of some scenes).
fn collect_jobs(timeline: &VideoTimeline, only: Option<&[String]>) -> Vec<Job> {
    let mut jobs = Vec::new();
    for (index, scene) in timeline.scenes.iter().enumerate() {
        if only.is_some_and(|ids| !ids.contains(&scene.id)) {
            continue;
        }
        let Some(narration) = &scene.narration else {
            continue;
        };
        if scene.audio.is_some() {
            continue;
        }
        let Some(text) = scene.spoken_text().map(String::from) else {
            continue;
        };
        jobs.push(Job {
            index,
            text,
            voice: narration.voice.clone().filter(|v| !v.trim().is_empty()),
        });
    }
    jobs
}

/// The narration files for `jobs`, synthesizing the ones not in the cache, with the engine
/// that made them: Kokoro when chosen and it works, else `say` (with one warning).
async fn synthesize(
    timeline: &VideoTimeline,
    jobs: &[Job],
    cfg: &SchedulerConfig,
    speech: Arc<dyn Speech>,
    tts_dir: &Path,
    ctx: &RenderContext,
    warnings: &mut Vec<String>,
) -> Result<(Vec<Narrated>, TtsEngine), SchedulerError> {
    if cfg.tts.resolve_engine(warnings) == TtsEngine::Kokoro {
        match narrate_kokoro(timeline, jobs, cfg, tts_dir, ctx).await {
            Ok((narrated, notes)) => {
                warnings.extend(notes);
                return Ok((narrated, TtsEngine::Kokoro));
            }
            Err(SchedulerError::Cancelled | SchedulerError::Media(MediaError::Cancelled)) => {
                return Err(SchedulerError::Cancelled)
            }
            Err(e) => warnings.push(format!(
                "Kokoro narration failed, all narration uses `say` instead: {e}"
            )),
        }
    }
    let files = narrate_say(timeline, jobs, speech, tts_dir, ctx, warnings).await?;
    Ok((files, TtsEngine::Say))
}

/// How many narration previews may synthesize at the same time (per process). More wait their
/// turn, so clicking "listen" on many scenes cannot start dozens of `say` or Kokoro processes.
pub const PREVIEW_SLOTS: usize = 2;

fn preview_slots() -> &'static tokio::sync::Semaphore {
    static SLOTS: std::sync::OnceLock<tokio::sync::Semaphore> = std::sync::OnceLock::new();
    SLOTS.get_or_init(|| tokio::sync::Semaphore::new(PREVIEW_SLOTS))
}

/// One scene's narration, exactly as a render would make it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PreviewClip {
    pub scene_id: String,
    /// Position of the scene in the timeline.
    pub index: usize,
    /// What is spoken (the spoken line, else the scene text).
    pub text: String,
    /// The voice the scene asks for (`None`: by language).
    pub voice: Option<String>,
    /// The cached file, relative to the public dir: the same file a render attaches.
    pub source: String,
    /// Length of the speech.
    pub speech_ms: u64,
    /// The scene's length in the timeline.
    pub scene_ms: u64,
    /// The scene's length in a render: stretched to the speech plus a short pause when longer.
    pub render_ms: u64,
    /// The file came from the cache (nothing was synthesized).
    pub cached: bool,
}

/// What [`preview_narration`] made.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct NarrationPreview {
    /// The engine that spoke: Kokoro, or `say` when Kokoro was not chosen or could not run.
    pub engine: &'static str,
    pub clips: Vec<PreviewClip>,
    /// The render's warnings for these scenes (a Kokoro fallback, a voice replaced).
    pub warnings: Vec<String>,
}

/// Synthesizes the narration of `timeline` (only the scenes `only` names, when given) with the
/// same engine choice, voices, file names and cache as a render, without changing the
/// timeline: a clip made here is reused by the render, and one a render made is reused here.
/// At most [`PREVIEW_SLOTS`] previews synthesize at once; the others wait.
pub async fn preview_narration(
    timeline: &VideoTimeline,
    only: Option<&[String]>,
    cfg: &SchedulerConfig,
    speech: Arc<dyn Speech>,
    probe: Arc<dyn AudioProbe>,
    ctx: &RenderContext,
) -> Result<NarrationPreview, String> {
    let jobs = collect_jobs(timeline, only);
    if jobs.is_empty() {
        return Err(match only {
            Some([id]) => format!(
                "Scene `{id}` has nothing to narrate (narration is off, it has its own audio, or it has no text)."
            ),
            _ => "No scene has narration to listen to.".to_string(),
        });
    }
    let _slot = preview_slots().acquire().await.map_err(|e| e.to_string())?;
    preview_jobs(timeline, &jobs, cfg, speech, probe, ctx)
        .await
        .map_err(|e| preview_error(&e))
}

async fn preview_jobs(
    timeline: &VideoTimeline,
    jobs: &[Job],
    cfg: &SchedulerConfig,
    speech: Arc<dyn Speech>,
    probe: Arc<dyn AudioProbe>,
    ctx: &RenderContext,
) -> Result<NarrationPreview, SchedulerError> {
    ctx.check()?;
    let tts_dir = cfg.public_dir.join("generated").join("tts");
    tokio::fs::create_dir_all(&tts_dir).await?;
    let mut warnings = Vec::new();
    let (files, engine) =
        synthesize(timeline, jobs, cfg, speech, &tts_dir, ctx, &mut warnings).await?;
    let mut clips = Vec::with_capacity(files.len());
    for (file, job) in files.into_iter().zip(jobs) {
        let speech_ms = measure(&probe, file.path).await?;
        let scene = &timeline.scenes[file.index];
        clips.push(PreviewClip {
            scene_id: scene.id.clone(),
            index: file.index,
            text: job.text.clone(),
            voice: job.voice.clone(),
            source: file.public_rel,
            speech_ms,
            scene_ms: scene.duration_ms,
            render_ms: scene.duration_ms.max(speech_ms + TAIL_MS),
            cached: file.cached,
        });
    }
    Ok(NarrationPreview {
        engine: engine.as_str(),
        clips,
        warnings,
    })
}

/// A preview failure in words for the editor: what went wrong and what to do.
pub fn preview_error(e: &SchedulerError) -> String {
    let text = e.to_string();
    let lower = text.to_lowercase();
    if matches!(e, SchedulerError::Cancelled) {
        "The preview was stopped.".into()
    } else if lower.contains("voice") && lower.contains("not found")
        || lower.contains("invalid voice")
    {
        format!("This voice is not installed for macOS `say` (System Settings → Accessibility → Spoken Content → System voice → Manage Voices), or pick another voice. ({text})")
    } else if matches!(e, SchedulerError::Media(MediaError::Io(io)) if io.kind() == std::io::ErrorKind::NotFound)
    {
        format!(
            "A speech tool was not found: macOS `say` and `ffmpeg` must be on this Mac. ({text})"
        )
    } else {
        format!("Narration could not be made: {text}")
    }
}

/// `say`, one scene at a time, cached by voice and text exactly as before Kokoro existed.
async fn narrate_say(
    timeline: &VideoTimeline,
    jobs: &[Job],
    speech: Arc<dyn Speech>,
    tts_dir: &Path,
    ctx: &RenderContext,
    warnings: &mut Vec<String>,
) -> Result<Vec<Narrated>, SchedulerError> {
    let mut files = Vec::with_capacity(jobs.len());
    for job in jobs {
        let scene = &timeline.scenes[job.index];
        let voice = match &job.voice {
            // A Kokoro voice name would make `say` fail; it cannot speak it.
            Some(v) if looks_like_kokoro_voice(v.trim()) => {
                let fallback = default_voice_for(&job.text);
                warnings.push(format!(
                    "scene `{}`: voice `{v}` is a Kokoro voice, `say` uses {fallback} instead",
                    scene.id
                ));
                fallback.to_string()
            }
            Some(v) => v.clone(),
            None => default_voice_for(&job.text).to_string(),
        };
        let file = format!(
            "{}-{:016x}.wav",
            sanitize(&scene.id),
            fnv1a(&format!("{voice}\n{}", job.text))
        );
        let dst = tts_dir.join(&file);

        ctx.emit(Progress::Stage {
            name: format!("Narrating {}", scene.id),
        });
        let cached = dst.exists();
        if !cached {
            let (speech, text, dst) = (speech.clone(), job.text.clone(), dst.clone());
            tokio::task::spawn_blocking(move || synthesize_whole(&*speech, &text, &voice, &dst))
                .await
                .map_err(|e| SchedulerError::Join(e.to_string()))??;
        }
        files.push(Narrated {
            index: job.index,
            public_rel: format!("generated/tts/{file}"),
            path: dst,
            cached,
        });
    }
    Ok(files)
}

/// Kokoro: every clip not in the cache, synthesized in one helper run. Returns the files and
/// the warnings to show, which are only shown when Kokoro is actually used.
async fn narrate_kokoro(
    timeline: &VideoTimeline,
    jobs: &[Job],
    cfg: &SchedulerConfig,
    tts_dir: &Path,
    ctx: &RenderContext,
) -> Result<(Vec<Narrated>, Vec<String>), SchedulerError> {
    let tts = &cfg.tts;
    let mut engine = KokoroSpeech::locate(
        tts.kokoro_python.as_deref(),
        tts.kokoro_models.as_deref(),
        cfg.child_path.as_deref(),
    )?;
    engine.ffmpeg = cfg.ffmpeg.clone();

    let mut langs: Vec<KokoroLang> = jobs
        .iter()
        .map(|j| KokoroLang::from_text(&j.text))
        .collect();
    let mut notes = Vec::new();
    let mut files = Vec::with_capacity(jobs.len());
    let mut missing = Vec::new();
    for (job, &lang) in jobs.iter().zip(&langs) {
        let scene = &timeline.scenes[job.index];
        let (voice, note) = resolve_kokoro_voice(job.voice.as_deref(), lang);
        if let Some(note) = note {
            notes.push(format!("scene `{}`: {note}", scene.id));
        }
        let file = format!(
            "{}-kokoro-{:016x}.wav",
            sanitize(&scene.id),
            engine.cache_key(&job.text, lang, &voice)
        );
        let dst = tts_dir.join(&file);
        let cached = dst.exists();
        if !cached {
            missing.push(KokoroItem {
                text: job.text.clone(),
                lang,
                voice,
                out: dst.clone(),
            });
        }
        files.push(Narrated {
            index: job.index,
            public_rel: format!("generated/tts/{file}"),
            path: dst,
            cached,
        });
    }
    if missing.is_empty() {
        return Ok((files, notes));
    }
    langs.retain(|l| missing.iter().any(|m| m.lang == *l));
    langs.dedup();
    engine.check_models(&langs)?;

    ctx.emit(Progress::Stage {
        name: format!("Narrating {} scene(s) with Kokoro", missing.len()),
    });
    let cancel = ctx.cancel.clone();
    let limits = Limits {
        timeout: Some(
            tts.kokoro_timeout
                .unwrap_or_else(|| batch_timeout(missing.iter().map(|m| m.text.as_str()))),
        ),
        cancelled: Arc::new(move || cancel.is_cancelled()),
    };
    let work_dir = cfg.work_dir.clone();
    let helper_notes =
        tokio::task::spawn_blocking(move || engine.synthesize_batch(&missing, &work_dir, &limits))
            .await
            .map_err(|e| SchedulerError::Join(e.to_string()))??;
    ctx.check()?;
    notes.extend(helper_notes.into_iter().map(|n| format!("Kokoro: {n}")));
    Ok((files, notes))
}

/// `say` into a temporary file renamed into place when complete, so the cache never holds a
/// half-written clip (a preview and a render may ask for the same clip at the same time; each
/// writer has its own temporary file).
fn synthesize_whole(
    speech: &dyn Speech,
    text: &str,
    voice: &str,
    dst: &Path,
) -> Result<(), MediaError> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let stem = dst
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let part = dst.with_file_name(format!(
        "{stem}.part{}-{}.wav",
        std::process::id(),
        NEXT.fetch_add(1, AtomicOrdering::Relaxed)
    ));
    let made = speech.synthesize(text, voice, &part).and_then(|()| {
        if part.is_file() {
            std::fs::rename(&part, dst).map_err(MediaError::from)
        } else {
            Err(MediaError::Speech(format!(
                "no audio was written for `{}`",
                dst.display()
            )))
        }
    });
    if made.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    made
}

async fn measure(
    probe: &Arc<dyn AudioProbe>,
    path: std::path::PathBuf,
) -> Result<u64, SchedulerError> {
    let probe = probe.clone();
    let ms = tokio::task::spawn_blocking(move || probe.duration_ms(&path))
        .await
        .map_err(|e| SchedulerError::Join(e.to_string()))??;
    Ok(ms)
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

/// 64-bit FNV-1a: a stable, dependency-free cache key.
fn fnv1a(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in s.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use vimmer_dsl::{MotionType, Narration, Resolution, Scene};
    use vimmer_media::{MediaError, Speech};

    use super::*;

    struct FakeSpeech(AtomicUsize);
    impl Speech for FakeSpeech {
        fn synthesize(&self, _text: &str, _voice: &str, out: &Path) -> Result<(), MediaError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            std::fs::write(out, b"wav").map_err(MediaError::from)
        }
    }

    struct FixedProbe(u64);
    impl AudioProbe for FixedProbe {
        fn duration_ms(&self, _path: &Path) -> Result<u64, MediaError> {
            Ok(self.0)
        }
    }

    fn scene(id: &str, ms: u64, text: Option<&str>, narration: bool) -> Scene {
        Scene {
            sfx: vec![],
            template: None,
            id: id.into(),
            duration_ms: ms,
            text: text.map(String::from),
            highlights: vec![],
            text_style: None,
            text_layout: None,
            text_pages: Vec::new(),
            image: None,
            motion: MotionType::Static,
            video: None,
            audio: None,
            narration: narration.then(Narration::default),
            transition_in: None,
            keyframes: vec![],
            layers: vec![],
            look: None,
        }
    }

    fn timeline(scenes: Vec<Scene>) -> VideoTimeline {
        VideoTimeline {
            brand: None,
            fps: 30,
            resolution: Resolution {
                width: 1920,
                height: 1080,
            },
            scenes,
            subtitles: vec![],
            music: None,
            subtitle_box: None,
            look: None,
            voiceover: vec![],
        }
    }

    fn cfg(public: PathBuf) -> SchedulerConfig {
        let mut cfg =
            SchedulerConfig::new(public.join("work"), public.join("out.mp4"), public.clone());
        cfg.public_dir = public;
        // Pinned, so a `$VIMMER_TTS` in the test environment cannot change the engine.
        cfg.tts.engine = Some(TtsEngine::Say);
        cfg
    }

    #[tokio::test]
    async fn attaches_audio_and_stretches_short_scenes_then_caches() {
        let dir = std::env::temp_dir().join(format!("vimmer-narrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = cfg(dir.clone());
        let speech = Arc::new(FakeSpeech(AtomicUsize::new(0)));
        let probe: Arc<dyn AudioProbe> = Arc::new(FixedProbe(2_000));
        let ctx = RenderContext::silent();

        let mut t = timeline(vec![
            scene("short", 1_000, Some("Hello"), true),
            scene("manual", 1_000, Some("Hi"), true),
            scene("silent", 1_000, Some("Quiet"), false),
        ]);
        t.scenes[1].audio = Some(AudioMapping {
            source: "x.wav".into(),
            offset_ms: 0,
        });
        let mut warnings = Vec::new();
        let n = narrate_scenes(
            &mut t,
            &cfg,
            speech.clone(),
            probe.clone(),
            &ctx,
            &mut warnings,
        )
        .await
        .unwrap();

        assert_eq!(
            n, 1,
            "only the scene without audio and with narration is narrated"
        );
        assert_eq!(speech.0.load(Ordering::SeqCst), 1);
        assert_eq!(t.scenes[0].duration_ms, 2_300, "stretched to speech + tail");
        assert!(t.scenes[0]
            .audio
            .as_ref()
            .unwrap()
            .source
            .starts_with("generated/tts/short-"));
        assert_eq!(t.scenes[1].audio.as_ref().unwrap().source, "x.wav");
        assert!(t.scenes[2].audio.is_none());
        assert_eq!(warnings.len(), 1);

        // Same text and voice: the cached file is reused, no second synthesis.
        let mut again = timeline(vec![scene("short", 1_000, Some("Hello"), true)]);
        narrate_scenes(
            &mut again,
            &cfg,
            speech.clone(),
            probe,
            &ctx,
            &mut Vec::new(),
        )
        .await
        .unwrap();
        assert_eq!(speech.0.load(Ordering::SeqCst), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A stand-in Kokoro interpreter: counts its runs, then writes each raw `out` named in the
    /// batch file (one item per line).
    const FAKE_PYTHON: &str = r#"#!/bin/sh
dir=$(dirname "$0")
echo run >> "$dir/runs.log"
for batch; do :; done
cp "$batch" "$dir/batch-seen.json"
sed -n 's/.*"out": "\([^"]*\)".*/\1/p' "$batch" | while read -r out; do printf 'RIFF' > "$out"; done
"#;

    /// A stand-in ffmpeg: copies the `-i` input to the last argument.
    const FAKE_FFMPEG: &str = r#"#!/bin/sh
while [ "$1" != "-i" ]; do shift; done
in=$2
for out; do :; done
cp "$in" "$out"
"#;

    fn executable(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, body).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A fresh dir with a Kokoro setup: fake interpreter (`python_body`), fake ffmpeg and
    /// placeholder model files, and a config that selects Kokoro.
    fn kokoro_setup(name: &str, python_body: &str) -> (PathBuf, SchedulerConfig) {
        let dir =
            std::env::temp_dir().join(format!("vimmer-narrate-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let bin = dir.join("bin");
        let models = dir.join("models");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&models).unwrap();
        for lang in [KokoroLang::Zh, KokoroLang::Ja, KokoroLang::En] {
            for f in lang.model_files() {
                std::fs::write(models.join(f), b"model").unwrap();
            }
        }
        executable(&bin.join("python"), python_body);
        executable(&bin.join("ffmpeg"), FAKE_FFMPEG);
        let mut cfg = cfg(dir.clone());
        cfg.ffmpeg = bin.join("ffmpeg");
        cfg.tts = TtsConfig {
            engine: Some(TtsEngine::Kokoro),
            kokoro_python: Some(bin.join("python")),
            kokoro_models: Some(models),
            kokoro_timeout: None,
        };
        (dir, cfg)
    }

    fn runs(dir: &Path) -> usize {
        std::fs::read_to_string(dir.join("bin/runs.log"))
            .map(|s| s.lines().count())
            .unwrap_or(0)
    }

    fn three_languages() -> VideoTimeline {
        let mut t = timeline(vec![
            scene("zh", 1_000, Some("西安钟楼"), true),
            scene("ja", 1_000, Some("桜が咲きました"), true),
            scene("en", 1_000, Some("Hello"), true),
        ]);
        t.scenes[0].narration = Some(Narration {
            voice: Some("Tingting".into()),
            text: None,
        });
        t.scenes[1].narration = Some(Narration {
            voice: Some("jm_kumo".into()),
            text: None,
        });
        t
    }

    async fn narrate(
        t: &mut VideoTimeline,
        cfg: &SchedulerConfig,
        speech: &Arc<FakeSpeech>,
        ctx: &RenderContext,
    ) -> (Result<usize, SchedulerError>, Vec<String>) {
        let mut warnings = Vec::new();
        let probe: Arc<dyn AudioProbe> = Arc::new(FixedProbe(2_000));
        let r = narrate_scenes(t, cfg, speech.clone(), probe, ctx, &mut warnings).await;
        (r, warnings)
    }

    #[tokio::test]
    async fn kokoro_narrates_all_scenes_in_one_batch_then_caches() {
        let (dir, cfg) = kokoro_setup("kokoro", FAKE_PYTHON);
        let speech = Arc::new(FakeSpeech(AtomicUsize::new(0)));
        let ctx = RenderContext::silent();

        let mut t = three_languages();
        let (n, warnings) = narrate(&mut t, &cfg, &speech, &ctx).await;
        assert_eq!(n.unwrap(), 3);
        assert_eq!(runs(&dir), 1, "one helper run for the whole timeline");
        assert_eq!(speech.0.load(Ordering::SeqCst), 0, "say is not used");
        for scene in &t.scenes {
            let source = &scene.audio.as_ref().unwrap().source;
            assert!(
                source.starts_with(&format!("generated/tts/{}-kokoro-", scene.id)),
                "{source}"
            );
            assert!(dir.join(source).is_file());
            assert_eq!(scene.duration_ms, 2_300);
        }
        let batch = std::fs::read_to_string(dir.join("bin/batch-seen.json")).unwrap();
        assert!(
            batch.contains(r#""lang": "zh", "voice": "zf_001""#),
            "{batch}"
        );
        assert!(
            batch.contains(r#""lang": "ja", "voice": "jm_kumo""#),
            "{batch}"
        );
        assert!(
            batch.contains(r#""lang": "en", "voice": "af_heart""#),
            "{batch}"
        );
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("scene `zh`") && w.contains("`Tingting` is a `say` voice")),
            "{warnings:?}"
        );
        assert!(
            !warnings.iter().any(|w| w.contains("instead: ")),
            "no fallback: {warnings:?}"
        );

        // Unchanged scenes come from the cache; a changed text is the only new clip.
        let mut again = three_languages();
        again.scenes[2].text = Some("Hello again".into());
        let (n, _) = narrate(&mut again, &cfg, &speech, &ctx).await;
        assert_eq!(n.unwrap(), 3);
        assert_eq!(runs(&dir), 2);
        let batch = std::fs::read_to_string(dir.join("bin/batch-seen.json")).unwrap();
        assert_eq!(batch.matches("\"out\"").count(), 1, "{batch}");
        let mut third = three_languages();
        third.scenes[2].text = Some("Hello again".into());
        narrate(&mut third, &cfg, &speech, &ctx).await.0.unwrap();
        assert_eq!(
            runs(&dir),
            2,
            "everything cached: the helper does not start"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn kokoro_falls_back_to_say_for_all_scenes_with_one_warning() {
        let cases: [(&str, &str, Option<Duration>); 4] = [
            ("no-python", "", None),
            (
                "helper-fails",
                "#!/bin/sh\necho 'ModuleNotFoundError: kokoro_onnx' >&2\nexit 1\n",
                None,
            ),
            (
                "helper-hangs",
                "#!/bin/sh\nsleep 30\n",
                Some(Duration::from_millis(300)),
            ),
            ("missing-model", FAKE_PYTHON, None),
        ];
        for (name, python, timeout) in cases {
            let (dir, mut cfg) = kokoro_setup(name, python);
            cfg.tts.kokoro_timeout = timeout;
            match name {
                "no-python" => cfg.tts.kokoro_python = Some(PathBuf::from("/nonexistent")),
                "missing-model" => {
                    std::fs::remove_file(dir.join("models/kokoro-v1.0.onnx")).unwrap()
                }
                _ => {}
            }
            let speech = Arc::new(FakeSpeech(AtomicUsize::new(0)));
            let mut t = three_languages();
            let (n, warnings) = narrate(&mut t, &cfg, &speech, &RenderContext::silent()).await;
            assert_eq!(n.unwrap(), 3, "{name}");
            assert_eq!(
                speech.0.load(Ordering::SeqCst),
                3,
                "{name}: every scene uses say"
            );
            let fallbacks: Vec<_> = warnings
                .iter()
                .filter(|w| w.contains("uses `say` instead"))
                .collect();
            assert_eq!(fallbacks.len(), 1, "{name}: {warnings:?}");
            assert!(
                !warnings.iter().any(|w| w.contains("is a `say` voice")),
                "{name}: Kokoro voice notes are dropped on fallback: {warnings:?}"
            );
            for scene in &t.scenes {
                let source = &scene.audio.as_ref().unwrap().source;
                assert!(!source.contains("-kokoro-"), "{name}: {source}");
            }
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[tokio::test]
    async fn cancelling_during_kokoro_cancels_the_render() {
        let (dir, cfg) = kokoro_setup("cancel", "#!/bin/sh\nsleep 30\n");
        let speech = Arc::new(FakeSpeech(AtomicUsize::new(0)));
        let ctx = RenderContext::silent();
        let cancel = ctx.cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            cancel.cancel();
        });
        let started = std::time::Instant::now();
        let mut t = three_languages();
        let (r, _) = narrate(&mut t, &cfg, &speech, &ctx).await;
        assert!(matches!(r, Err(SchedulerError::Cancelled)), "{r:?}");
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(
            speech.0.load(Ordering::SeqCst),
            0,
            "no say fallback after a cancel"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn say_keeps_its_cache_names_and_replaces_kokoro_voice_names() {
        struct Voices(std::sync::Mutex<Vec<String>>);
        impl Speech for Voices {
            fn synthesize(&self, _t: &str, voice: &str, out: &Path) -> Result<(), MediaError> {
                self.0.lock().unwrap().push(voice.to_string());
                std::fs::write(out, b"wav").map_err(MediaError::from)
            }
        }
        let dir = std::env::temp_dir().join(format!("vimmer-narrate-say-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = cfg(dir.clone());
        let speech = Arc::new(Voices(Default::default()));
        let mut t = three_languages();
        let mut warnings = Vec::new();
        let probe: Arc<dyn AudioProbe> = Arc::new(FixedProbe(500));
        narrate_scenes(
            &mut t,
            &cfg,
            speech.clone(),
            probe,
            &RenderContext::silent(),
            &mut warnings,
        )
        .await
        .unwrap();
        assert_eq!(*speech.0.lock().unwrap(), ["Tingting", "Kyoko", "Samantha"]);
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("`jm_kumo` is a Kokoro voice")),
            "{warnings:?}"
        );
        // The say cache key is unchanged: `voice\ntext`.
        assert_eq!(
            t.scenes[0].audio.as_ref().unwrap().source,
            format!("generated/tts/zh-{:016x}.wav", fnv1a("Tingting\n西安钟楼"))
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_spoken_line_is_what_is_synthesized_and_keyed() {
        struct Texts(std::sync::Mutex<Vec<String>>);
        impl Speech for Texts {
            fn synthesize(&self, text: &str, _v: &str, out: &Path) -> Result<(), MediaError> {
                self.0.lock().unwrap().push(text.to_string());
                std::fs::write(out, b"wav").map_err(MediaError::from)
            }
        }
        let dir = std::env::temp_dir().join(format!("vimmer-narrate-line-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = cfg(dir.clone());
        let speech = Arc::new(Texts(Default::default()));
        let mut t = timeline(vec![
            scene("old", 1_000, Some("Hello"), true),
            scene("line", 1_000, Some("Shown"), true),
            scene("blind", 1_000, None, true),
            scene("blank", 1_000, Some("Fallback"), true),
        ]);
        t.scenes[1].narration.as_mut().unwrap().text = Some("Said instead".into());
        t.scenes[2].narration.as_mut().unwrap().text = Some("Only heard".into());
        t.scenes[3].narration.as_mut().unwrap().text = Some("  ".into());
        let probe: Arc<dyn AudioProbe> = Arc::new(FixedProbe(500));
        let n = narrate_scenes(
            &mut t,
            &cfg,
            speech.clone(),
            probe,
            &RenderContext::silent(),
            &mut Vec::new(),
        )
        .await
        .unwrap();
        assert_eq!(n, 4);
        assert_eq!(
            *speech.0.lock().unwrap(),
            ["Hello", "Said instead", "Only heard", "Fallback"]
        );
        let src = |i: usize| t.scenes[i].audio.as_ref().unwrap().source.clone();
        // A scene without a spoken line keeps the key it had before the field existed
        // (the same name main gives it).
        assert_eq!(src(0), "generated/tts/old-751a583f147ef128.wav");
        assert_eq!(
            src(0),
            format!("generated/tts/old-{:016x}.wav", fnv1a("Samantha\nHello"))
        );
        assert_eq!(
            src(1),
            format!(
                "generated/tts/line-{:016x}.wav",
                fnv1a("Samantha\nSaid instead")
            )
        );
        assert_eq!(
            src(3),
            format!(
                "generated/tts/blank-{:016x}.wav",
                fnv1a("Samantha\nFallback")
            )
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    fn fresh_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("vimmer-preview-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn preview_timeline() -> VideoTimeline {
        let mut t = timeline(vec![
            scene("intro", 1_000, Some("Hello"), true),
            scene("body", 5_000, Some("西安钟楼"), true),
            scene("quiet", 1_000, Some("Shh"), false),
        ]);
        t.scenes[1].narration.as_mut().unwrap().text = Some("钟楼在西安".into());
        t
    }

    #[tokio::test]
    async fn a_preview_makes_the_files_the_render_uses_and_the_render_reuses_them() {
        let dir = fresh_dir("reuse");
        let cfg = cfg(dir.clone());
        let speech = Arc::new(FakeSpeech(AtomicUsize::new(0)));
        let probe: Arc<dyn AudioProbe> = Arc::new(FixedProbe(2_000));
        let ctx = RenderContext::silent();
        let t = preview_timeline();
        let before = t.clone();

        let p = preview_narration(&t, None, &cfg, speech.clone(), probe.clone(), &ctx)
            .await
            .unwrap();
        assert_eq!(t, before, "a preview never changes the timeline");
        assert_eq!(p.engine, "say");
        assert_eq!(speech.0.load(Ordering::SeqCst), 2);
        let ids: Vec<_> = p.clips.iter().map(|c| c.scene_id.as_str()).collect();
        assert_eq!(ids, ["intro", "body"], "only narrated scenes");
        let intro = &p.clips[0];
        assert_eq!(
            (intro.speech_ms, intro.scene_ms, intro.render_ms),
            (2_000, 1_000, 2_300)
        );
        assert!(!intro.cached);
        assert_eq!(
            p.clips[1].render_ms, 5_000,
            "long enough already: not stretched"
        );
        assert_eq!(
            p.clips[1].text, "钟楼在西安",
            "the spoken line is what is heard"
        );
        assert_eq!(
            intro.source,
            format!("generated/tts/intro-{:016x}.wav", fnv1a("Samantha\nHello"))
        );
        assert!(dir.join(&intro.source).is_file());
        let leftovers: Vec<_> = std::fs::read_dir(dir.join("generated/tts"))
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".part"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "no temporary files left: {leftovers:?}"
        );

        // The render attaches the very same files and synthesizes nothing.
        let mut rendered = t.clone();
        narrate_scenes(
            &mut rendered,
            &cfg,
            speech.clone(),
            probe.clone(),
            &ctx,
            &mut Vec::new(),
        )
        .await
        .unwrap();
        assert_eq!(
            speech.0.load(Ordering::SeqCst),
            2,
            "render reused the preview's clips"
        );
        for clip in &p.clips {
            let scene = &rendered.scenes[clip.index];
            assert_eq!(scene.audio.as_ref().unwrap().source, clip.source);
            assert_eq!(
                scene.duration_ms, clip.render_ms,
                "preview predicts the render's length"
            );
        }

        // And a preview after a render (or a second preview) is all cached.
        let again = preview_narration(
            &t,
            Some(&["body".to_string()]),
            &cfg,
            speech.clone(),
            probe,
            &ctx,
        )
        .await
        .unwrap();
        assert_eq!(again.clips.len(), 1);
        assert!(again.clips[0].cached);
        assert_eq!(speech.0.load(Ordering::SeqCst), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn changing_the_text_or_the_voice_makes_a_new_clip() {
        let dir = fresh_dir("invalidate");
        let cfg = cfg(dir.clone());
        let speech = Arc::new(FakeSpeech(AtomicUsize::new(0)));
        let probe: Arc<dyn AudioProbe> = Arc::new(FixedProbe(800));
        let ctx = RenderContext::silent();
        let only = ["intro".to_string()];
        let mut t = preview_timeline();
        let first = preview_narration(&t, Some(&only), &cfg, speech.clone(), probe.clone(), &ctx)
            .await
            .unwrap();
        t.scenes[0].text = Some("Hello there".into());
        let text = preview_narration(&t, Some(&only), &cfg, speech.clone(), probe.clone(), &ctx)
            .await
            .unwrap();
        t.scenes[0].narration.as_mut().unwrap().voice = Some("Daniel".into());
        let voice = preview_narration(&t, Some(&only), &cfg, speech.clone(), probe.clone(), &ctx)
            .await
            .unwrap();
        let sources = [&first, &text, &voice].map(|p| p.clips[0].source.clone());
        assert_ne!(sources[0], sources[1]);
        assert_ne!(sources[1], sources[2]);
        assert_eq!(
            speech.0.load(Ordering::SeqCst),
            3,
            "each change synthesized once"
        );
        assert!([&first, &text, &voice].iter().all(|p| !p.clips[0].cached));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn previews_never_synthesize_more_than_the_cap_at_once() {
        struct Slow {
            now: AtomicUsize,
            peak: AtomicUsize,
        }
        impl Speech for Slow {
            fn synthesize(&self, _t: &str, _v: &str, out: &Path) -> Result<(), MediaError> {
                let n = self.now.fetch_add(1, Ordering::SeqCst) + 1;
                self.peak.fetch_max(n, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(150));
                self.now.fetch_sub(1, Ordering::SeqCst);
                std::fs::write(out, b"wav").map_err(MediaError::from)
            }
        }
        let dir = fresh_dir("cap");
        let cfg = Arc::new(cfg(dir.clone()));
        let speech = Arc::new(Slow {
            now: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        });
        let probe: Arc<dyn AudioProbe> = Arc::new(FixedProbe(500));
        let mut tasks = Vec::new();
        for i in 0..6 {
            let (cfg, speech, probe) = (cfg.clone(), speech.clone(), probe.clone());
            tasks.push(tokio::spawn(async move {
                let t = timeline(vec![scene(
                    &format!("s{i}"),
                    1_000,
                    Some(&format!("Line {i}")),
                    true,
                )]);
                preview_narration(&t, None, &cfg, speech, probe, &RenderContext::silent()).await
            }));
        }
        for task in tasks {
            task.await.unwrap().unwrap();
        }
        let peak = speech.peak.load(Ordering::SeqCst);
        assert!((1..=PREVIEW_SLOTS).contains(&peak), "peak {peak}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn preview_errors_say_what_is_wrong() {
        struct Broken;
        impl Speech for Broken {
            fn synthesize(&self, _t: &str, _v: &str, _out: &Path) -> Result<(), MediaError> {
                Err(MediaError::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "say",
                )))
            }
        }
        let dir = fresh_dir("errors");
        let cfg = cfg(dir.clone());
        let probe: Arc<dyn AudioProbe> = Arc::new(FixedProbe(500));
        let ctx = RenderContext::silent();
        let t = preview_timeline();
        let quiet = preview_narration(
            &t,
            Some(&["quiet".to_string()]),
            &cfg,
            Arc::new(Broken),
            probe.clone(),
            &ctx,
        )
        .await
        .unwrap_err();
        assert!(
            quiet.contains("Scene `quiet` has nothing to narrate"),
            "{quiet}"
        );
        let none = preview_narration(
            &timeline(vec![]),
            None,
            &cfg,
            Arc::new(Broken),
            probe.clone(),
            &ctx,
        )
        .await
        .unwrap_err();
        assert_eq!(none, "No scene has narration to listen to.");
        let missing = preview_narration(&t, None, &cfg, Arc::new(Broken), probe.clone(), &ctx)
            .await
            .unwrap_err();
        assert!(
            missing.contains("`say` and `ffmpeg` must be on this Mac"),
            "{missing}"
        );
        let voice = preview_error(&SchedulerError::Media(MediaError::Speech(
            "Voice `Nobody' not found.".into(),
        )));
        assert!(voice.contains("not installed for macOS `say`"), "{voice}");
        assert!(preview_error(&SchedulerError::Cancelled).contains("stopped"));
        assert!(
            !dir.join("generated/tts").read_dir().unwrap().any(|_| true),
            "a failed synthesis leaves nothing in the cache"
        );

        // Kokoro chosen but not installed: like a render, `say` speaks, with a visible note.
        let mut kokoro = cfg.clone();
        kokoro.tts = TtsConfig {
            engine: Some(TtsEngine::Kokoro),
            kokoro_python: Some(PathBuf::from("/nonexistent/python")),
            ..TtsConfig::default()
        };
        let speech = Arc::new(FakeSpeech(AtomicUsize::new(0)));
        let p = preview_narration(&t, None, &kokoro, speech, probe, &ctx)
            .await
            .unwrap();
        assert_eq!(p.engine, "say");
        assert!(
            p.warnings
                .iter()
                .any(|w| w.contains("Kokoro narration failed, all narration uses `say` instead")),
            "{:?}",
            p.warnings
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn kokoro_previews_use_the_render_cache_names() {
        let (dir, cfg) = kokoro_setup("preview", FAKE_PYTHON);
        let speech = Arc::new(FakeSpeech(AtomicUsize::new(0)));
        let probe: Arc<dyn AudioProbe> = Arc::new(FixedProbe(2_000));
        let ctx = RenderContext::silent();
        let t = three_languages();
        let p = preview_narration(&t, None, &cfg, speech.clone(), probe.clone(), &ctx)
            .await
            .unwrap();
        assert_eq!(p.engine, "kokoro");
        assert_eq!(runs(&dir), 1);
        let mut rendered = three_languages();
        narrate(&mut rendered, &cfg, &speech, &ctx).await.0.unwrap();
        assert_eq!(runs(&dir), 1, "the render found every clip in the cache");
        for clip in &p.clips {
            assert_eq!(
                rendered.scenes[clip.index].audio.as_ref().unwrap().source,
                clip.source
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn engine_comes_from_config_then_env_then_say() {
        let mut w = Vec::new();
        let explicit = TtsConfig {
            engine: Some(TtsEngine::Kokoro),
            ..TtsConfig::default()
        };
        assert_eq!(explicit.resolve_engine(&mut w), TtsEngine::Kokoro);
        assert_eq!(engine_from_env(None, &mut w), TtsEngine::Say);
        assert_eq!(engine_from_env(Some(""), &mut w), TtsEngine::Say);
        assert_eq!(engine_from_env(Some("KOKORO"), &mut w), TtsEngine::Kokoro);
        assert!(w.is_empty());
        assert_eq!(engine_from_env(Some("espeak"), &mut w), TtsEngine::Say);
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn fnv_is_stable_and_discriminates() {
        assert_eq!(fnv1a("abc"), fnv1a("abc"));
        assert_ne!(fnv1a("abc"), fnv1a("abd"));
    }
}
