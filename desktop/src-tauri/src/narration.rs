//! "Listen" in the editor: synthesizes scene narration before a render, with the render's own
//! code (`vimmer_scheduler::preview_narration`): the same engine choice as the render panel, the
//! same voices and the same cache files, so a clip heard here is reused by the render.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use vimmer_dsl::VideoTimeline;
use vimmer_media::{AudioProbe, FfmpegProbe, SaySpeech, Speech};
use vimmer_scheduler::{preview_narration, RenderContext};

use crate::render::{default_remotion_dir, find_in_path, render_options, user_path, vimmer_env, PanelOptions};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListenArgs {
    /// The timeline as the editor holds it, including unsaved edits.
    pub timeline_json: String,
    /// Only these scenes. Missing or empty: every narrated scene ("listen to all").
    #[serde(default)]
    pub scene_ids: Option<Vec<String>>,
    /// `say` or `kokoro`, as chosen in the render panel. Missing: the render's default.
    #[serde(default)]
    pub tts: Option<String>,
}

/// Returns `{"ok": true, "engine", "clips": [...], "warnings": [...]}` (clips as in
/// `vimmer tts --json`) or `{"ok": false, "error"}`. Runs off the UI thread; at most
/// `vimmer_scheduler::PREVIEW_SLOTS` previews synthesize at once.
#[tauri::command]
pub async fn listen_narration(app: AppHandle, args: ListenArgs) -> Value {
    let work = match app.path().app_cache_dir() {
        Ok(dir) => dir.join(format!("listen-{}", crate::render::unix_millis())),
        Err(e) => return json!({ "ok": false, "error": format!("No cache folder: {e}") }),
    };
    let path = user_path();
    let ffmpeg = find_in_path("ffmpeg", &path);
    let speech: Arc<dyn Speech> = Arc::new(SaySpeech { say: find_in_path("say", &path), ffmpeg: ffmpeg.clone() });
    let probe: Arc<dyn AudioProbe> = Arc::new(FfmpegProbe { binary: ffmpeg.clone() });
    let setup = Setup { remotion_dir: default_remotion_dir(), work, ffmpeg, child_path: Some(path) };
    listen_in(args, &setup, speech, probe, &vimmer_env).await
}

/// Where and with what the preview runs (the render's setup, for tests a temporary one).
pub struct Setup {
    pub remotion_dir: PathBuf,
    pub work: PathBuf,
    pub ffmpeg: PathBuf,
    pub child_path: Option<String>,
}

pub async fn listen_in(
    args: ListenArgs,
    setup: &Setup,
    speech: Arc<dyn Speech>,
    probe: Arc<dyn AudioProbe>,
    env: &(dyn Fn(&str) -> Option<String> + Sync),
) -> Value {
    match listen(args, setup, speech, probe, env).await {
        Ok(v) => v,
        Err(error) => json!({ "ok": false, "error": error }),
    }
}

async fn listen(
    args: ListenArgs,
    setup: &Setup,
    speech: Arc<dyn Speech>,
    probe: Arc<dyn AudioProbe>,
    env: &(dyn Fn(&str) -> Option<String> + Sync),
) -> Result<Value, String> {
    let mut timeline: VideoTimeline =
        serde_json::from_str(&args.timeline_json).map_err(|e| format!("The timeline is not valid JSON: {e}"))?;
    // A render narrates the expanded timeline; template errors are the render's to report.
    let _ = vimmer_dsl::expand_templates(&mut timeline);
    // The render panel's engine and the render's Kokoro paths, through the render's own builder.
    let panel = PanelOptions { tts: args.tts, ..PanelOptions::default() };
    let options = render_options(&panel, env)?;
    let mut cfg = options.config(setup.work.clone(), setup.work.join("unused.mp4"), setup.remotion_dir.clone());
    cfg.ffmpeg = setup.ffmpeg.clone();
    cfg.child_path = setup.child_path.clone();
    let only = args.scene_ids.filter(|ids| !ids.is_empty());
    let result = preview_narration(&timeline, only.as_deref(), &cfg, speech, probe, &RenderContext::silent()).await;
    remove_dir(&setup.work);
    let preview = result?;
    let mut value = serde_json::to_value(&preview).map_err(|e| e.to_string())?;
    value["ok"] = json!(true);
    Ok(value)
}

fn remove_dir(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use vimmer_media::MediaError;
    use vimmer_scheduler::narrate::narrate_scenes;

    use super::*;

    struct CountingSpeech(AtomicUsize);
    impl Speech for CountingSpeech {
        fn synthesize(&self, _text: &str, _voice: &str, out: &Path) -> Result<(), MediaError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            std::fs::write(out, b"RIFF").map_err(MediaError::from)
        }
    }

    struct FixedProbe;
    impl AudioProbe for FixedProbe {
        fn duration_ms(&self, _path: &Path) -> Result<u64, MediaError> {
            Ok(4_200)
        }
    }

    const TIMELINE: &str = r#"{
      "fps": 30, "resolution": {"width": 1920, "height": 1080},
      "scenes": [
        {"id": "a", "duration_ms": 3000, "text": "Hello", "narration": {"voice": "Samantha"}},
        {"id": "b", "duration_ms": 6000, "text": "Shown", "narration": {"voice": null, "text": "Said"}},
        {"id": "c", "duration_ms": 2000, "text": "Silent"}
      ],
      "subtitles": []
    }"#;

    fn setup(name: &str) -> Setup {
        let root = std::env::temp_dir().join(format!("vimmer-listen-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("public")).unwrap();
        Setup { remotion_dir: root.clone(), work: root.join("work"), ffmpeg: PathBuf::from("ffmpeg"), child_path: None }
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn args(scenes: Option<&[&str]>, tts: Option<&str>) -> ListenArgs {
        ListenArgs {
            timeline_json: TIMELINE.into(),
            scene_ids: scenes.map(|s| s.iter().map(|id| id.to_string()).collect()),
            tts: tts.map(String::from),
        }
    }

    #[test]
    fn listening_makes_the_clips_the_render_attaches() {
        tauri::async_runtime::block_on(listening_makes_the_clips_the_render_attaches_async());
    }

    async fn listening_makes_the_clips_the_render_attaches_async() {
        let s = setup("reuse");
        let speech = Arc::new(CountingSpeech(AtomicUsize::new(0)));
        let out = listen_in(args(Some(&["a"]), Some("say")), &s, speech.clone(), Arc::new(FixedProbe), &no_env).await;
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["engine"], "say");
        let clip = &out["clips"][0];
        assert_eq!(clip["scene_id"], "a");
        assert_eq!(clip["speech_ms"], 4_200);
        assert_eq!(clip["scene_ms"], 3_000);
        assert_eq!(clip["render_ms"], 4_500);
        assert_eq!(clip["cached"], false);
        let source = clip["source"].as_str().unwrap().to_string();
        assert!(s.remotion_dir.join("public").join(&source).is_file());
        assert!(!s.work.exists(), "scratch folder removed");

        // Listen to all: `a` comes from the cache, `b` speaks its spoken line, `c` is silent.
        let all = listen_in(args(None, Some("say")), &s, speech.clone(), Arc::new(FixedProbe), &no_env).await;
        let clips = all["clips"].as_array().unwrap();
        assert_eq!(clips.len(), 2, "{all}");
        assert_eq!(clips[0]["cached"], true);
        assert_eq!(clips[1]["text"], "Said");
        assert_eq!(speech.0.load(Ordering::SeqCst), 2);

        // The render picks up both files and synthesizes nothing.
        let mut timeline: VideoTimeline = serde_json::from_str(TIMELINE).unwrap();
        let mut cfg = vimmer_scheduler::RenderOptions {
            tts: Some(vimmer_media::TtsEngine::Say),
            ..Default::default()
        }
        .config(s.work.clone(), s.work.join("o.mp4"), s.remotion_dir.clone());
        cfg.ffmpeg = PathBuf::from("ffmpeg");
        narrate_scenes(&mut timeline, &cfg, speech.clone(), Arc::new(FixedProbe), &RenderContext::silent(), &mut Vec::new())
            .await
            .unwrap();
        assert_eq!(speech.0.load(Ordering::SeqCst), 2, "no re-synthesis in the render");
        assert_eq!(timeline.scenes[0].audio.as_ref().unwrap().source, source);
        assert_eq!(timeline.scenes[1].audio.as_ref().unwrap().source, clips[1]["source"].as_str().unwrap());
        let _ = std::fs::remove_dir_all(&s.remotion_dir);
    }

    #[test]
    fn problems_come_back_as_messages() {
        tauri::async_runtime::block_on(problems_come_back_as_messages_async());
    }

    async fn problems_come_back_as_messages_async() {
        let s = setup("errors");
        let speech = Arc::new(CountingSpeech(AtomicUsize::new(0)));
        let bad = ListenArgs { timeline_json: "{".into(), scene_ids: None, tts: None };
        let out = listen_in(bad, &s, speech.clone(), Arc::new(FixedProbe), &no_env).await;
        assert_eq!(out["ok"], false);
        assert!(out["error"].as_str().unwrap().starts_with("The timeline is not valid JSON"), "{out}");

        let out = listen_in(args(None, Some("espeak")), &s, speech.clone(), Arc::new(FixedProbe), &no_env).await;
        assert_eq!(out["error"], "Unknown voice engine `espeak`.");

        let out = listen_in(args(Some(&["c"]), None), &s, speech.clone(), Arc::new(FixedProbe), &no_env).await;
        assert!(out["error"].as_str().unwrap().contains("Scene `c` has nothing to narrate"), "{out}");

        // Kokoro chosen in the panel but not installed: `say` speaks, with the render's note.
        let env = |name: &str| (name == "VIMMER_KOKORO_PYTHON").then(|| "/nonexistent/python".to_string());
        let out = listen_in(args(Some(&["a"]), Some("kokoro")), &s, speech.clone(), Arc::new(FixedProbe), &env).await;
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["engine"], "say");
        assert!(out["warnings"][0].as_str().unwrap().contains("uses `say` instead"), "{out}");
        assert_eq!(speech.0.load(Ordering::SeqCst), 1);
        let _ = std::fs::remove_dir_all(&s.remotion_dir);
    }
}
