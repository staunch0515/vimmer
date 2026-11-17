//! 数字人讲解人 (narrator avatar) in the desktop app: whether this Mac can generate clips, which
//! clips of the timeline are ready, a face check on the chosen photo, and 「生成数字人」 as a
//! background job.
//!
//! The job is `vimmer avatar generate <timeline> --json` run in-process: the same scheduler call
//! (`render_timeline_with` in `AvatarMode::Generate`) with the render's own config, so the app
//! needs no separate `vimmer` binary. Progress goes to the webview as `avatar-progress` events
//! (the scheduler's `Progress`, as for a render) and the result as `avatar-finished`. 取消 sets the
//! cancel handle; the engine then kills the runner's whole process group and removes its
//! half-written `.part.mp4`, so a cancelled job leaves no clip behind.
//!
//! Status is cheap (no narration, no Python): the scheduler records each generation in
//! `generated/avatar/status.json` under the timeline's plan signature (`vimmer check` reads the
//! same file). With no entry the clips are unknown: never generated, or the text, voice or photo
//! changed since, so they need generating.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, State};
use vimmer_dsl::VideoTimeline;
use vimmer_media::{
    avatar_capability, avatar_estimate_seconds, AudioProbe, AvatarCapability, AvatarEngine, FfmpegProbe, SaySpeech,
    Speech,
};
use vimmer_scheduler::{
    render_timeline_with, AvatarMode, AvatarRunStatus, CancelHandle, Progress, RenderContext, SchedulerConfig,
    SchedulerError,
};

use crate::render::{default_remotion_dir, find_in_path, render_options, unix_millis, user_path, vimmer_env, PanelOptions};

pub const PROGRESS_EVENT: &str = "avatar-progress";
pub const FINISHED_EVENT: &str = "avatar-finished";
/// Where the scheduler records clip status, relative to the public folder.
const STATUS_FILE: &str = "generated/avatar/status.json";

#[derive(Default)]
pub struct AvatarState {
    current: Mutex<Option<Arc<CancelHandle>>>,
}

/// `$VIMMER_AVATAR_HOME` (also from the login shell), else the default install dir.
fn home() -> Option<PathBuf> {
    vimmer_env("VIMMER_AVATAR_HOME").map(PathBuf::from)
}

/// Whether this Mac can generate: `{"state": "ready"|"not_installed"|"unsupported", "message", "zh"}`.
pub fn env_value(cap: &AvatarCapability) -> Value {
    let (state, zh) = match cap {
        AvatarCapability::Ready { .. } => ("ready", "数字人引擎已就绪，可以生成。"),
        AvatarCapability::NotInstalled { .. } => {
            ("not_installed", "这台 Mac 还没安装数字人引擎：请先安装（见说明），或在 iMac 上生成。")
        }
        AvatarCapability::Unsupported { .. } => {
            ("unsupported", "这台电脑不能生成数字人（需要 Apple 芯片的 Mac）：请在 iMac 上生成。")
        }
    };
    json!({ "state": state, "message": cap.message(), "zh": zh })
}

#[tauri::command]
pub fn avatar_env() -> Value {
    env_value(&avatar_capability(home().as_deref()))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusArgs {
    pub timeline_json: String,
}

/// The clips of `timeline` as recorded in `public_dir`'s status file:
/// `{"ok", "known", "runs": [...], "missing_estimate_s", "speaking_estimate_s"}`.
/// `known: false` means no record for this exact timeline (never generated, or the speech or the
/// photo changed): every speaking scene with an avatar needs generating, in about
/// `speaking_estimate_s`. A run recorded ready whose file is gone counts as missing.
pub fn status_in(timeline: &VideoTimeline, public_dir: &Path) -> Value {
    let mut t = timeline.clone();
    // The render computes the signature after expanding templates and before staging.
    let _ = vimmer_dsl::expand_templates(&mut t);
    let Some(sig) = vimmer_dsl::avatar_plan_signature(&t) else {
        return json!({ "ok": true, "known": true, "runs": [], "missing_estimate_s": 0, "speaking_estimate_s": 0 });
    };
    let speaking_ms: u64 = (0..t.scenes.len())
        .filter(|&i| t.scene_avatar(i).is_some() && speaks(&t, i))
        .map(|i| t.scenes[i].duration_ms)
        .sum();
    let speaking_s = if speaking_ms > 0 { avatar_estimate_seconds(speaking_ms) } else { 0 };
    let recorded: Option<Vec<AvatarRunStatus>> = std::fs::read_to_string(public_dir.join(STATUS_FILE))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|doc| serde_json::from_value(doc.get(&sig)?.clone()).ok());
    match recorded {
        Some(mut runs) => {
            for r in &mut runs {
                r.ready = r.ready && public_dir.join(&r.clip).is_file();
            }
            let missing: u64 = runs.iter().filter(|r| !r.ready).map(|r| r.estimate_s).sum();
            json!({ "ok": true, "known": true, "runs": runs, "missing_estimate_s": missing, "speaking_estimate_s": speaking_s })
        }
        None => json!({ "ok": true, "known": false, "runs": [], "missing_estimate_s": speaking_s, "speaking_estimate_s": speaking_s }),
    }
}

/// As `vimmer check`: a scene speaks when it has narration, its own audio, or there is a voice-over.
fn speaks(t: &VideoTimeline, i: usize) -> bool {
    let s = &t.scenes[i];
    s.spoken_text().is_some_and(|x| !x.trim().is_empty()) || s.audio.is_some() || !t.voiceover.is_empty()
}

#[tauri::command]
pub async fn avatar_status(args: StatusArgs) -> Value {
    let timeline: VideoTimeline = match serde_json::from_str(&args.timeline_json) {
        Ok(t) => t,
        Err(e) => return json!({ "ok": false, "error": format!("The timeline is not valid JSON: {e}") }),
    };
    let public = crate::assets::public_dir();
    tauri::async_runtime::spawn_blocking(move || status_in(&timeline, &public))
        .await
        .unwrap_or_else(|e| json!({ "ok": false, "error": e.to_string() }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeArgs {
    /// The photo as the timeline names it.
    pub photo: String,
    /// The open project's folder, whose files win over the public folder.
    #[serde(default)]
    pub project_dir: Option<String>,
}

/// Where a timeline path is on disk: the project's copy first, else the public folder.
pub fn locate_photo(photo: &str, project: Option<&Path>, public_dir: &Path) -> Option<PathBuf> {
    if vimmer_dsl::validation::asset_path_problem(photo).is_some() {
        return None;
    }
    project.map(|p| p.join(photo)).into_iter().chain([public_dir.join(photo)]).find(|p| p.is_file())
}

/// Looks for one clear face in the photo: `{"ok": true, "state": "found"|"no_face"|"unavailable"|"missing", "zh"}`.
pub fn probe_with(engine: Result<AvatarEngine, String>, photo: Option<PathBuf>) -> Value {
    let Some(photo) = photo else {
        return json!({ "ok": true, "state": "missing", "zh": "找不到这张照片。" });
    };
    let engine = match engine {
        Ok(e) => e,
        Err(message) => {
            return json!({ "ok": true, "state": "unavailable", "message": message,
                "zh": "这台电脑不能检查人脸；在 iMac 上生成时会再检查。" })
        }
    };
    match engine.probe_face(&photo) {
        Ok(p) if p.found => json!({ "ok": true, "state": "found", "width": p.width, "height": p.height, "zh": "✓ 找到清晰的正脸" }),
        Ok(_) => json!({ "ok": true, "state": "no_face", "zh": "没找到清晰的正脸" }),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

#[tauri::command]
pub async fn avatar_probe(app: AppHandle, args: ProbeArgs) -> Value {
    let project = match &args.project_dir {
        Some(dir) => match crate::studio::studio(&app).and_then(|s| s.project(dir)) {
            Ok(p) => Some(p),
            Err(e) => return json!({ "ok": false, "error": e }),
        },
        None => None,
    };
    tauri::async_runtime::spawn_blocking(move || {
        let photo = locate_photo(&args.photo, project.as_deref(), &crate::assets::public_dir());
        let engine = AvatarEngine::locate(home().as_deref(), Some(&user_path())).map_err(|e| e.to_string());
        probe_with(engine, photo)
    })
    .await
    .unwrap_or_else(|e| json!({ "ok": false, "error": e.to_string() }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateArgs {
    pub timeline_json: String,
    /// The render panel's choices (the voice engine decides the narration the avatar speaks).
    #[serde(default)]
    pub options: PanelOptions,
    #[serde(default)]
    pub project_dir: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Finished {
    pub ok: bool,
    pub cancelled: bool,
    pub error: Option<String>,
    pub runs: Vec<AvatarRunStatus>,
}

/// Generates the missing clips of `timeline` with `cfg` (its mode is set to generate here).
pub async fn generate_in(
    timeline: VideoTimeline,
    mut cfg: SchedulerConfig,
    probe: Arc<dyn AudioProbe>,
    speech: Arc<dyn Speech>,
    ctx: RenderContext,
) -> Finished {
    cfg.avatar.mode = AvatarMode::Generate;
    match render_timeline_with(timeline, cfg, probe, speech, ctx).await {
        Ok(report) => Finished { ok: true, cancelled: false, error: None, runs: report.avatar },
        Err(SchedulerError::Cancelled) => Finished { ok: false, cancelled: true, error: None, runs: vec![] },
        Err(e) => Finished { ok: false, cancelled: false, error: Some(e.to_string()), runs: vec![] },
    }
}

/// The job `start_avatar_generate` runs: the render's config for these panel options (so the
/// speech matches the next render), the project's files first, the user's PATH and avatar home.
/// The render's output is never written (generate mode stops before the render).
pub fn generate_setup(
    options: &PanelOptions,
    project: Option<PathBuf>,
    work_dir: &Path,
) -> Result<(SchedulerConfig, Arc<dyn AudioProbe>, Arc<dyn Speech>), String> {
    let options = render_options(options, &vimmer_env)?;
    let mut cfg = options.config(work_dir.to_path_buf(), work_dir.join("avatar-unused.mp4"), default_remotion_dir());
    if let Some(dir) = project {
        cfg.asset_dirs = vec![dir];
        cfg.asset_dirs_first = true;
    }
    let path = user_path();
    cfg.ffmpeg = find_in_path("ffmpeg", &path);
    cfg.avatar.home = home();
    let speech: Arc<dyn Speech> = Arc::new(SaySpeech { say: find_in_path("say", &path), ffmpeg: cfg.ffmpeg.clone() });
    cfg.child_path = Some(path);
    let probe: Arc<dyn AudioProbe> = Arc::new(FfmpegProbe { binary: cfg.ffmpeg.clone() });
    Ok((cfg, probe, speech))
}

#[tauri::command]
pub fn start_avatar_generate(app: AppHandle, state: State<'_, AvatarState>, args: GenerateArgs) -> Result<(), String> {
    let cap = avatar_capability(home().as_deref());
    if !matches!(cap, AvatarCapability::Ready { .. }) {
        return Err(env_value(&cap)["zh"].as_str().unwrap_or_default().to_string());
    }
    let timeline: VideoTimeline =
        serde_json::from_str(&args.timeline_json).map_err(|e| format!("The timeline is not valid JSON: {e}"))?;
    let project = match &args.project_dir {
        Some(dir) => Some(crate::studio::studio(&app)?.project(dir)?),
        None => None,
    };
    let cache = app.path().app_cache_dir().map_err(|e| format!("No cache folder: {e}"))?;
    let work_dir = cache.join(format!("avatar-{}", unix_millis()));
    let (cfg, probe, speech) = generate_setup(&args.options, project, &work_dir)?;

    let cancel = Arc::new(CancelHandle::default());
    {
        let mut current = state.current.lock().unwrap_or_else(|e| e.into_inner());
        if current.is_some() {
            return Err("数字人已经在生成了。".into());
        }
        *current = Some(cancel.clone());
    }
    let progress_app = app.clone();
    let ctx = RenderContext {
        cancel,
        progress: Arc::new(move |p: Progress| {
            let _ = progress_app.emit(PROGRESS_EVENT, p);
        }),
    };
    let task_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let finished = generate_in(timeline, cfg, probe, speech, ctx).await;
        let _ = std::fs::remove_dir_all(&work_dir);
        if let Some(state) = task_app.try_state::<AvatarState>() {
            *state.current.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        let _ = task_app.emit(FINISHED_EVENT, finished);
    });
    Ok(())
}

#[tauri::command]
pub fn cancel_avatar_generate(state: State<'_, AvatarState>) -> bool {
    match state.current.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        Some(cancel) => {
            cancel.cancel();
            true
        }
        None => false,
    }
}

#[cfg(test)]
#[path = "avatar_tests.rs"]
mod tests;
