//! 按旁白对齐场景 in the desktop app: proposes scene lengths from a voice-over clip with the
//! render's own code (`vimmer_scheduler::align_to_voiceover`, local whisper). The editor shows the
//! proposal and applies it as one undo step; nothing is written here.

use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use vimmer_dsl::VideoTimeline;
use vimmer_media::{AudioProbe, FfmpegProbe};
use vimmer_scheduler::{align_to_voiceover, RenderContext};

use crate::narration::Setup;
use crate::render::{default_remotion_dir, find_in_path, render_options, user_path, vimmer_env, PanelOptions};
use crate::studio::{blocking, studio};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlignArgs {
    /// The timeline as the editor holds it, including unsaved edits.
    pub timeline_json: String,
    /// The voice-over clip (by id). Missing: the first one.
    #[serde(default)]
    pub clip: Option<String>,
    /// The open project's folder, whose files the timeline names. Missing: the public folder only.
    #[serde(default)]
    pub dir: Option<String>,
}

/// Returns `{"ok": true, "clip", "changes": [...], "durations": [...], "notes": [...]}` (as
/// `vimmer align-scenes --json`) or `{"ok": false, "error"}`.
#[tauri::command]
pub async fn align_scenes(app: AppHandle, args: AlignArgs) -> Value {
    let work = match app.path().app_cache_dir() {
        Ok(dir) => dir.join(format!("align-{}", crate::render::unix_millis())),
        Err(e) => return json!({ "ok": false, "error": format!("No cache folder: {e}") }),
    };
    let project = match &args.dir {
        Some(dir) => {
            let Ok(studio) = studio(&app) else {
                return json!({ "ok": false, "error": "The projects folder is not available." });
            };
            let dir = dir.clone();
            match blocking(move || studio.project(&dir)).await {
                Ok(p) => Some(p),
                Err(e) => return json!({ "ok": false, "error": e }),
            }
        }
        None => None,
    };
    let path = user_path();
    let ffmpeg = find_in_path("ffmpeg", &path);
    let probe: Arc<dyn AudioProbe> = Arc::new(FfmpegProbe { binary: ffmpeg.clone() });
    let setup = Setup { remotion_dir: default_remotion_dir(), work, ffmpeg, child_path: Some(path) };
    match align_in(&args, project, crate::assets::public_dir(), &setup, probe, &vimmer_env).await {
        Ok(v) => v,
        Err(error) => json!({ "ok": false, "error": error }),
    }
}

pub async fn align_in(
    args: &AlignArgs,
    project: Option<PathBuf>,
    public_dir: PathBuf,
    setup: &Setup,
    probe: Arc<dyn AudioProbe>,
    env: &(dyn Fn(&str) -> Option<String> + Sync),
) -> Result<Value, String> {
    let mut timeline: VideoTimeline =
        serde_json::from_str(&args.timeline_json).map_err(|e| format!("The timeline is not valid JSON: {e}"))?;
    if timeline.voiceover.is_empty() {
        return Err("There is no voice-over to align to: record or add one on the 旁白 track first.".into());
    }
    // The render's whisper settings (model, language, binary) through the render's own builder.
    let options = render_options(&PanelOptions::default(), env)?;
    let mut cfg = options.config(setup.work.clone(), setup.work.join("unused.mp4"), setup.remotion_dir.clone());
    cfg.ffmpeg = setup.ffmpeg.clone();
    cfg.child_path = setup.child_path.clone();
    cfg.public_dir = public_dir;
    cfg.asset_dirs = project.into_iter().collect();
    let mut notes = Vec::new();
    vimmer_scheduler::localize::stage_assets(&mut timeline, &cfg, &mut notes)
        .map_err(|e| format!("The voice-over file could not be read: {e}"))?;
    let result = align_to_voiceover(&timeline, args.clip.as_deref(), &cfg, probe, &RenderContext::silent()).await;
    let _ = std::fs::remove_dir_all(&setup.work);
    let alignment = result?;
    let mut value = serde_json::to_value(&alignment).map_err(|e| e.to_string())?;
    value["ok"] = json!(true);
    Ok(value)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use vimmer_media::MediaError;

    use super::*;

    struct FixedProbe;
    impl AudioProbe for FixedProbe {
        fn duration_ms(&self, _path: &Path) -> Result<u64, MediaError> {
            Ok(1_000)
        }
    }

    fn setup(name: &str) -> Setup {
        let root = std::env::temp_dir().join(format!("vimmer-align-{name}-{}", std::process::id()));
        Setup { remotion_dir: root.clone(), work: root.join("work"), ffmpeg: PathBuf::from("ffmpeg"), child_path: None }
    }

    const NO_VO: &str = r#"{"fps":30,"resolution":{"width":1920,"height":1080},
        "scenes":[{"id":"a","duration_ms":3000,"text":"Hello"}],"subtitles":[]}"#;

    #[test]
    fn explains_what_is_missing() {
        tauri::async_runtime::block_on(explains_what_is_missing_async());
    }

    async fn explains_what_is_missing_async() {
        let env = |_: &str| None;
        let args = AlignArgs { timeline_json: NO_VO.into(), clip: None, dir: None };
        let err = align_in(&args, None, PathBuf::from("."), &setup("none"), Arc::new(FixedProbe), &env).await.unwrap_err();
        assert!(err.contains("no voice-over"), "{err}");
        let args = AlignArgs { timeline_json: "{".into(), clip: None, dir: None };
        let err = align_in(&args, None, PathBuf::from("."), &setup("bad"), Arc::new(FixedProbe), &env).await.unwrap_err();
        assert!(err.contains("not valid JSON"), "{err}");
        // A clip id that does not exist.
        let with_vo = NO_VO.replace(r#""subtitles":[]"#, r#""subtitles":[],"voiceover":[{"id":"v","source":"rec/v.wav","start_ms":0}]"#);
        let args = AlignArgs { timeline_json: with_vo, clip: Some("x".into()), dir: None };
        let err = align_in(&args, None, PathBuf::from("."), &setup("clip"), Arc::new(FixedProbe), &env).await.unwrap_err();
        assert!(err.contains("no voice-over clip `x`"), "{err}");
    }
}
