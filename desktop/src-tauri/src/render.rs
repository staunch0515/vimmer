//! Renders the timeline from the desktop app. Progress and the final result are sent to the
//! webview as events; only one render runs at a time.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use vimmer_dsl::VideoTimeline;
use vimmer_media::{FfmpegProbe, SaySpeech, Speech};
use vimmer_scheduler::{render_timeline_with, CancelHandle, Progress, RenderContext, SchedulerConfig, SchedulerError};

pub const PROGRESS_EVENT: &str = "render-progress";
pub const FINISHED_EVENT: &str = "render-finished";

#[derive(Default)]
pub struct RenderState {
    current: Mutex<Option<Arc<CancelHandle>>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartArgs {
    /// The timeline as the editor holds it, including unsaved edits.
    pub timeline_json: String,
    /// Destination MP4 path, chosen by the user.
    pub output: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finished {
    pub ok: bool,
    pub cancelled: bool,
    pub output: Option<String>,
    pub error: Option<String>,
}

#[tauri::command]
pub fn start_render(app: AppHandle, state: State<'_, RenderState>, args: StartArgs) -> Result<(), String> {
    let timeline: VideoTimeline =
        serde_json::from_str(&args.timeline_json).map_err(|e| format!("The timeline is not valid JSON: {e}"))?;
    let remotion_dir = default_remotion_dir();
    if !remotion_dir.join("package.json").exists() {
        return Err(format!("Remotion project not found at {}", remotion_dir.display()));
    }

    let cancel = Arc::new(CancelHandle::default());
    {
        let mut current = state.current.lock().unwrap_or_else(|e| e.into_inner());
        if current.is_some() {
            return Err("A render is already running.".into());
        }
        *current = Some(cancel.clone());
    }

    let setup = || -> Result<(SchedulerConfig, PathBuf, Arc<dyn Speech>), String> {
        let cache = app.path().app_cache_dir().map_err(|e| format!("No cache folder: {e}"))?;
        let work_dir = cache.join(format!("render-{}", unix_millis()));
        let mut cfg = SchedulerConfig::new(work_dir.clone(), PathBuf::from(&args.output), remotion_dir.clone());
        let path = user_path();
        cfg.ffmpeg = find_in_path("ffmpeg", &path);
        let speech: Arc<dyn Speech> = Arc::new(SaySpeech {
            say: find_in_path("say", &path),
            ffmpeg: cfg.ffmpeg.clone(),
        });
        cfg.child_path = Some(path);
        Ok((cfg, work_dir, speech))
    };
    let (cfg, work_dir, speech) = match setup() {
        Ok(v) => v,
        Err(e) => {
            clear_current(&app);
            return Err(e);
        }
    };
    let probe = Arc::new(FfmpegProbe { binary: cfg.ffmpeg.clone() });

    let progress_app = app.clone();
    let ctx = RenderContext {
        cancel: cancel.clone(),
        progress: Arc::new(move |p: Progress| {
            let _ = progress_app.emit(PROGRESS_EVENT, p);
        }),
    };

    let task_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = render_timeline_with(timeline, cfg, probe, speech, ctx).await;
        let _ = std::fs::remove_dir_all(&work_dir);
        clear_current(&task_app);
        let finished = match result {
            Ok(report) => Finished {
                ok: true,
                cancelled: false,
                output: Some(report.output.display().to_string()),
                error: None,
            },
            Err(SchedulerError::Cancelled) => Finished { ok: false, cancelled: true, output: None, error: None },
            Err(e) => Finished { ok: false, cancelled: false, output: None, error: Some(e.to_string()) },
        };
        let _ = task_app.emit(FINISHED_EVENT, finished);
    });
    Ok(())
}

#[tauri::command]
pub fn cancel_render(state: State<'_, RenderState>) -> bool {
    let current = state.current.lock().unwrap_or_else(|e| e.into_inner());
    match current.as_ref() {
        Some(cancel) => {
            cancel.cancel();
            true
        }
        None => false,
    }
}

/// Shows a file in Finder with its folder open.
#[tauri::command]
pub fn reveal_in_finder(path: String) -> Result<(), String> {
    if !Path::new(&path).exists() {
        return Err(format!("{path} does not exist"));
    }
    Command::new("open").arg("-R").arg(&path).status().map_err(|e| e.to_string())?;
    Ok(())
}

fn clear_current(app: &AppHandle) {
    if let Some(state) = app.try_state::<RenderState>() {
        let mut current = state.current.lock().unwrap_or_else(|e| e.into_inner());
        *current = None;
    }
}

/// The Remotion project next to this repository. Only valid when running from the source tree.
fn default_remotion_dir() -> PathBuf {
    let here = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../remotion-renderer");
    here.canonicalize().unwrap_or(here)
}

/// A GUI app inherits a minimal PATH, so `npx` and `ffmpeg` would not be found. This asks the
/// user's login shell for its PATH once and reuses it for every child process.
fn user_path() -> String {
    static PATH: OnceLock<String> = OnceLock::new();
    PATH.get_or_init(|| {
        let shell_path = Command::new("/bin/zsh")
            .args(["-lc", "printf %s \"$PATH\""])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|p| !p.is_empty());
        let fallback = "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin";
        shell_path.unwrap_or_else(|| fallback.to_string())
    })
    .clone()
}

fn find_in_path(name: &str, path: &str) -> PathBuf {
    path.split(':')
        .map(|dir| Path::new(dir).join(name))
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| PathBuf::from(name))
}

fn unix_millis() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}
