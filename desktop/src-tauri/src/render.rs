//! Renders the timeline from the desktop app. Progress and the final result are sent to the
//! webview as events; only one render runs at a time. The render panel's options become a
//! scheduler config through `vimmer_scheduler::RenderOptions`, the same builder the CLI uses,
//! and `check_before_render` runs the `vimmer check` rules on the timeline first.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use vimmer_dsl::VideoTimeline;
use vimmer_check::{check_timeline, CheckOptions, Report};
use vimmer_media::{FfmpegProbe, SaySpeech, Speech, TtsEngine};
use vimmer_scheduler::{
    render_timeline_with, AutoSubtitles, CancelHandle, Progress, RenderContext, RenderOptions, SchedulerConfig,
    SchedulerError,
};

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
    /// The render panel's choices. Missing fields keep the CLI's defaults.
    #[serde(default)]
    pub options: PanelOptions,
}

/// What the render panel sends; mirrors `RenderOptions` in remotion-renderer/editor/src/render.ts.
/// Every field is optional so an older editor (or an empty object) renders like the CLI without
/// flags.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PanelOptions {
    /// `say` or `kokoro`. Missing: `$VIMMER_TTS` (from the login shell too), else `say`.
    pub tts: Option<String>,
    /// `off`, `on` or `all`.
    pub auto_subtitles: Option<String>,
    pub word_subtitles: Option<bool>,
    pub chunk_ms: Option<u64>,
    pub preprocess_jobs: Option<usize>,
    pub render_jobs: Option<usize>,
    /// A whisper code or `auto`; empty means "guess from the text".
    pub whisper_language: Option<String>,
}

/// Defaults the panel shows before the user picks anything.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderDefaults {
    /// The engine a render uses without a choice: `$VIMMER_TTS`, else `say`.
    pub tts: &'static str,
    /// Whether that came from `$VIMMER_TTS`.
    pub tts_from_env: bool,
    pub chunk_ms: u64,
    pub preprocess_jobs: usize,
    pub render_jobs: usize,
}

/// Environment variables the scheduler reads. A GUI app started from Finder does not inherit
/// them, so values from the login shell fill the gaps (only these names are ever read).
const SHELL_VARS: [&str; 5] = [
    "VIMMER_TTS",
    "VIMMER_KOKORO_PYTHON",
    "VIMMER_KOKORO_MODELS",
    "VIMMER_WHISPER_CLI",
    "VIMMER_WHISPER_MODEL",
];

/// Turns the panel's choices into render options. `env` looks up a `VIMMER_*` variable.
pub fn render_options(panel: &PanelOptions, env: &dyn Fn(&str) -> Option<String>) -> Result<RenderOptions, String> {
    let tts = match panel.tts.as_deref() {
        Some(name) => TtsEngine::parse(name).ok_or_else(|| format!("Unknown voice engine `{name}`."))?,
        None => default_engine(env).0,
    };
    let mut opts = RenderOptions { tts: Some(tts), ..RenderOptions::default() };
    if let Some(mode) = &panel.auto_subtitles {
        opts.auto_subtitles = match mode.as_str() {
            "off" => AutoSubtitles::Off,
            "on" => AutoSubtitles::On,
            "all" => AutoSubtitles::All,
            other => return Err(format!("Unknown auto-subtitles mode `{other}`.")),
        };
    }
    if let Some(on) = panel.word_subtitles {
        opts.word_subtitles = on;
    }
    if let Some(ms) = panel.chunk_ms {
        if ms < 1000 {
            return Err("Chunk size must be at least 1000 ms.".into());
        }
        opts.chunk_ms = ms;
    }
    if let Some(n) = panel.preprocess_jobs {
        opts.preprocess_jobs = n.max(1);
    }
    if let Some(n) = panel.render_jobs {
        opts.render_jobs = n.max(1);
    }
    opts.whisper_language = panel.whisper_language.as_deref().map(str::trim).filter(|l| !l.is_empty()).map(String::from);
    let path = |name: &str| env(name).filter(|v| !v.trim().is_empty()).map(PathBuf::from);
    opts.kokoro_python = path("VIMMER_KOKORO_PYTHON");
    opts.kokoro_models = path("VIMMER_KOKORO_MODELS");
    opts.whisper_cli = path("VIMMER_WHISPER_CLI");
    opts.whisper_model = path("VIMMER_WHISPER_MODEL");
    Ok(opts)
}

/// `$VIMMER_TTS` when it names an engine, else `say`; and whether the variable decided it.
fn default_engine(env: &dyn Fn(&str) -> Option<String>) -> (TtsEngine, bool) {
    match env("VIMMER_TTS").as_deref().and_then(TtsEngine::parse) {
        Some(engine) => (engine, true),
        None => (TtsEngine::Say, false),
    }
}

#[tauri::command]
pub fn render_defaults() -> RenderDefaults {
    let (engine, from_env) = default_engine(&vimmer_env);
    let d = RenderOptions::default();
    RenderDefaults {
        tts: engine.as_str(),
        tts_from_env: from_env,
        chunk_ms: d.chunk_ms,
        preprocess_jobs: d.preprocess_jobs,
        render_jobs: d.render_jobs,
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckArgs {
    pub timeline_json: String,
    /// The engine chosen in the panel, for `narration.too_long`. Missing: the render's default.
    pub tts: Option<String>,
}

/// The `vimmer check` rules on the timeline as the editor holds it (the same report as
/// `vimmer check --json`, without a rendered file and without synthesizing narration).
#[tauri::command]
pub async fn check_before_render(args: CheckArgs) -> Result<Report, String> {
    let timeline: VideoTimeline =
        serde_json::from_str(&args.timeline_json).map_err(|e| format!("The timeline is not valid JSON: {e}"))?;
    let tts = match args.tts.as_deref() {
        Some(name) => TtsEngine::parse(name).ok_or_else(|| format!("Unknown voice engine `{name}`."))?,
        None => default_engine(&vimmer_env).0,
    };
    let remotion_dir = default_remotion_dir();
    let opts = CheckOptions {
        public_dir: Some(remotion_dir.join("public")).filter(|p| p.is_dir()),
        rendered: None,
        synth: false,
        ffmpeg: find_in_path("ffmpeg", &user_path()),
        tts,
    };
    // Asset checks read image files; keep that off the main thread.
    tauri::async_runtime::spawn_blocking(move || Report::new(check_timeline(&timeline, &opts), None, None))
        .await
        .map_err(|e| format!("The check stopped: {e}"))
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
        let options = render_options(&args.options, &vimmer_env)?;
        let cache = app.path().app_cache_dir().map_err(|e| format!("No cache folder: {e}"))?;
        let work_dir = cache.join(format!("render-{}", unix_millis()));
        let mut cfg = options.config(work_dir.clone(), PathBuf::from(&args.output), remotion_dir.clone());
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
pub(crate) fn user_path() -> String {
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

/// A `VIMMER_*` variable: this process's value, else the login shell's.
fn vimmer_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty()).or_else(|| {
        let i = SHELL_VARS.iter().position(|v| *v == name)?;
        shell_vars()[i].clone()
    })
}

/// The login shell's values of `SHELL_VARS`, asked once. Only these names are expanded, so no
/// other variable (an API key, say) is ever read.
fn shell_vars() -> &'static [Option<String>; SHELL_VARS.len()] {
    static VARS: OnceLock<[Option<String>; SHELL_VARS.len()]> = OnceLock::new();
    VARS.get_or_init(|| {
        let script = SHELL_VARS.iter().map(|v| format!("\"${v}\"")).collect::<Vec<_>>().join(" ");
        let out = Command::new("/bin/zsh")
            .args(["-lc", &format!("printf '%s\\0' {script}")])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        let mut values = out.split('\0').map(|v| Some(v.trim().to_string()).filter(|v| !v.is_empty()));
        std::array::from_fn(|_| values.next().flatten())
    })
}

pub(crate) fn find_in_path(name: &str, path: &str) -> PathBuf {
    path.split(':')
        .map(|dir| Path::new(dir).join(name))
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| PathBuf::from(name))
}

fn unix_millis() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn empty_panel_renders_like_the_cli_without_flags() {
        let opts = render_options(&PanelOptions::default(), &no_env).unwrap();
        assert_eq!(opts, RenderOptions { tts: Some(TtsEngine::Say), ..RenderOptions::default() });
    }

    #[test]
    fn missing_engine_follows_vimmer_tts() {
        let env = |name: &str| (name == "VIMMER_TTS").then(|| "Kokoro".to_string());
        let opts = render_options(&PanelOptions::default(), &env).unwrap();
        assert_eq!(opts.tts, Some(TtsEngine::Kokoro));
        assert_eq!(default_engine(&env), (TtsEngine::Kokoro, true));
        let junk = |_: &str| Some("espeak".to_string());
        assert_eq!(default_engine(&junk), (TtsEngine::Say, false));
    }

    #[test]
    fn panel_choices_reach_the_options() {
        let panel: PanelOptions = serde_json::from_str(
            r#"{"tts":"kokoro","autoSubtitles":"on","wordSubtitles":false,"chunkMs":5000,
                "preprocessJobs":0,"renderJobs":3,"whisperLanguage":" ja "}"#,
        )
        .unwrap();
        let env = |name: &str| match name {
            "VIMMER_TTS" => Some("say".to_string()),
            "VIMMER_KOKORO_PYTHON" => Some("/k/python".to_string()),
            "VIMMER_WHISPER_MODEL" => Some(String::new()),
            _ => None,
        };
        let opts = render_options(&panel, &env).unwrap();
        assert_eq!(opts.tts, Some(TtsEngine::Kokoro), "an explicit choice beats $VIMMER_TTS");
        assert_eq!(opts.auto_subtitles, AutoSubtitles::On);
        assert!(!opts.word_subtitles);
        assert_eq!(opts.chunk_ms, 5000);
        assert_eq!(opts.preprocess_jobs, 1, "zero jobs would never finish");
        assert_eq!(opts.render_jobs, 3);
        assert_eq!(opts.whisper_language.as_deref(), Some("ja"));
        assert_eq!(opts.kokoro_python, Some(PathBuf::from("/k/python")));
        assert_eq!(opts.whisper_model, None, "an empty variable is unset");
        let cfg = opts.config("w".into(), "o.mp4".into(), "r".into());
        assert_eq!(cfg.word_subtitles.auto_subtitles, AutoSubtitles::On);
        assert_eq!(cfg.tts.engine, Some(TtsEngine::Kokoro));
    }

    #[test]
    fn bad_values_are_refused() {
        let bad = |json: &str| render_options(&serde_json::from_str(json).unwrap(), &no_env).unwrap_err();
        assert!(bad(r#"{"tts":"espeak"}"#).contains("espeak"));
        assert!(bad(r#"{"autoSubtitles":"some"}"#).contains("some"));
        assert!(bad(r#"{"chunkMs":10}"#).contains("1000"));
    }

    #[test]
    fn the_check_returns_the_vimmer_check_report_for_the_chosen_engine() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/word-subtitles-demo.json");
        let mut timeline: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(fixture).unwrap()).unwrap();
        timeline["scenes"][0]["duration_ms"] = 300.into(); // far too short for its narration
        timeline["scenes"][1]["image"] = "missing/nope.png".into(); // an error
        let run = |tts: &str| {
            let args = CheckArgs { timeline_json: timeline.to_string(), tts: Some(tts.into()) };
            serde_json::to_value(tauri::async_runtime::block_on(check_before_render(args)).unwrap()).unwrap()
        };
        let find = |report: &serde_json::Value, code: &str| {
            report["findings"].as_array().unwrap().iter().find(|f| f["code"] == code).cloned().unwrap()
        };
        let say = run("say");
        assert_eq!(say["schema"], "vimmer-check/1");
        assert!(say["summary"]["errors"].as_u64().unwrap() > 0);
        let missing = find(&say, "assets.missing_image");
        assert_eq!(missing["severity"], "error");
        assert_eq!(missing["suggestion"]["path"], "/scenes/1/image");
        let too_long = find(&say, "narration.too_long");
        assert_eq!(too_long["suggestion"]["path"], "/scenes/0/duration_ms");
        assert!(too_long["message"].as_str().unwrap().contains("say"));
        let kokoro = run("kokoro");
        assert!(find(&kokoro, "narration.too_long")["message"].as_str().unwrap().contains("kokoro"));

        let bad = CheckArgs { timeline_json: "{}".into(), tts: None };
        assert!(tauri::async_runtime::block_on(check_before_render(bad)).is_err());
    }

    #[test]
    fn start_args_without_options_still_parse() {
        let args: StartArgs = serde_json::from_str(r#"{"timelineJson":"{}","output":"o.mp4"}"#).unwrap();
        assert!(args.options.tts.is_none());
    }
}
