//! Narration step: synthesizes scene text, attaches it as audio and stretches the scene to fit.
//!
//! Two engines: macOS `say` (the default) and Kokoro, a local neural TTS run through a Python
//! helper in one batch per render. When Kokoro is chosen but cannot run, all narration falls back
//! to `say` with one warning, so a render never fails because of it.

use std::path::{Path, PathBuf};
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
const TAIL_MS: u64 = 300;

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

    let mut jobs = Vec::new();
    for (index, scene) in timeline.scenes.iter().enumerate() {
        let Some(narration) = &scene.narration else {
            continue;
        };
        if scene.audio.is_some() {
            continue;
        }
        let Some(text) = scene.text.clone().filter(|t| !t.trim().is_empty()) else {
            continue;
        };
        jobs.push(Job {
            index,
            text,
            voice: narration.voice.clone().filter(|v| !v.trim().is_empty()),
        });
    }
    if jobs.is_empty() {
        return Ok(0);
    }

    let mut files = None;
    if cfg.tts.resolve_engine(warnings) == TtsEngine::Kokoro {
        match narrate_kokoro(timeline, &jobs, cfg, &tts_dir, ctx).await {
            Ok((narrated, notes)) => {
                warnings.extend(notes);
                files = Some(narrated);
            }
            Err(SchedulerError::Cancelled | SchedulerError::Media(MediaError::Cancelled)) => {
                return Err(SchedulerError::Cancelled)
            }
            Err(e) => warnings.push(format!(
                "Kokoro narration failed, all narration uses `say` instead: {e}"
            )),
        }
    }
    let files = match files {
        Some(files) => files,
        None => narrate_say(timeline, &jobs, speech, &tts_dir, ctx, warnings).await?,
    };

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
        if !dst.exists() {
            let (speech, text, dst) = (speech.clone(), job.text.clone(), dst.clone());
            tokio::task::spawn_blocking(move || speech.synthesize(&text, &voice, &dst))
                .await
                .map_err(|e| SchedulerError::Join(e.to_string()))??;
        }
        files.push(Narrated {
            index: job.index,
            public_rel: format!("generated/tts/{file}"),
            path: dst,
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
        if !dst.exists() {
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
            id: id.into(),
            duration_ms: ms,
            text: text.map(String::from),
            highlights: vec![],
            image: None,
            motion: MotionType::Static,
            audio: None,
            narration: narration.then(Narration::default),
            transition_in: None,
            keyframes: vec![],
            layers: vec![],
        }
    }

    fn timeline(scenes: Vec<Scene>) -> VideoTimeline {
        VideoTimeline {
            fps: 30,
            resolution: Resolution {
                width: 1920,
                height: 1080,
            },
            scenes,
            subtitles: vec![],
            music: None,
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
        });
        t.scenes[1].narration = Some(Narration {
            voice: Some("jm_kumo".into()),
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
