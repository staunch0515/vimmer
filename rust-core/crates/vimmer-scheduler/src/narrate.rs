//! Narration step: synthesizes scene text, attaches it as audio and stretches the scene to fit.

use std::sync::Arc;

use vimmer_dsl::{AudioMapping, VideoTimeline};
use vimmer_media::{default_voice_for, AudioProbe, Speech};

use crate::control::{Progress, RenderContext};
use crate::run::{SchedulerConfig, SchedulerError};

/// Pause after speech before a scene may end, so a sentence is not clipped.
const TAIL_MS: u64 = 300;

/// Synthesizes narration for every scene that asks for it and has no audio of its own.
/// Results are cached by voice and text, so an unchanged scene is not synthesized again.
/// Returns the number of scenes narrated and appends a warning for each scene stretched.
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
    let mut narrated = 0;

    for index in 0..timeline.scenes.len() {
        let scene = &timeline.scenes[index];
        let Some(narration) = scene.narration.clone() else {
            continue;
        };
        if scene.audio.is_some() {
            continue;
        }
        let Some(text) = scene.text.clone().filter(|t| !t.trim().is_empty()) else {
            continue;
        };
        let voice = narration
            .voice
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| default_voice_for(&text).to_string());

        let file = format!(
            "{}-{:016x}.wav",
            sanitize(&scene.id),
            fnv1a(&format!("{voice}\n{text}"))
        );
        let dst = tts_dir.join(&file);
        let public_rel = format!("generated/tts/{file}");

        ctx.emit(Progress::Stage {
            name: format!("Narrating {}", scene.id),
        });
        if !dst.exists() {
            let (speech, text, voice, dst) =
                (speech.clone(), text.clone(), voice.clone(), dst.clone());
            tokio::task::spawn_blocking(move || speech.synthesize(&text, &voice, &dst))
                .await
                .map_err(|e| SchedulerError::Join(e.to_string()))??;
        }
        let duration = measure(&probe, dst.clone()).await?;
        ctx.check()?;

        let scene = &mut timeline.scenes[index];
        scene.audio = Some(AudioMapping {
            source: public_rel,
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

    #[test]
    fn fnv_is_stable_and_discriminates() {
        assert_eq!(fnv1a("abc"), fnv1a("abc"));
        assert_ne!(fnv1a("abc"), fnv1a("abd"));
    }
}
