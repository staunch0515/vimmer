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
    /// Destination MP4 path, chosen by the user. May be left out for a project: the render then
    /// goes to `<project>/renders/<date-time>.mp4`.
    #[serde(default)]
    pub output: Option<String>,
    /// The render panel's choices. Missing fields keep the CLI's defaults.
    #[serde(default)]
    pub options: PanelOptions,
    /// The Studio project being rendered. Its files win over the public folder, and the finished
    /// render is added to its history. Missing: a plain timeline file, rendered as before.
    #[serde(default)]
    pub project_dir: Option<String>,
}

/// What the render panel sends; mirrors `RenderOptions` in remotion-renderer/editor/src/render.ts.
/// Every field is optional so an older editor (or an empty object) renders like the CLI without
/// flags.
#[derive(Debug, Default, Deserialize, Serialize)]
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
pub(crate) fn default_engine(env: &dyn Fn(&str) -> Option<String>) -> (TtsEngine, bool) {
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
    /// The Studio project the timeline belongs to, so its own files are found. Missing: only the
    /// public folder is looked in, as before.
    #[serde(default)]
    pub project_dir: Option<String>,
}

/// The `vimmer check` rules on the timeline as the editor holds it (the same report as
/// `vimmer check --json`, without a rendered file and without synthesizing narration).
#[tauri::command]
pub async fn check_before_render(app: AppHandle, args: CheckArgs) -> Result<Report, String> {
    let project = match &args.project_dir {
        Some(dir) => Some(crate::studio::studio(&app)?.project(dir)?),
        None => None,
    };
    check_timeline_in(args, project).await
}

/// The check, with the project folder already resolved.
pub async fn check_timeline_in(args: CheckArgs, project: Option<PathBuf>) -> Result<Report, String> {
    let timeline: VideoTimeline = match &project {
        Some(dir) => {
            let mut value: serde_json::Value = serde_json::from_str(&args.timeline_json)
                .map_err(|e| format!("The timeline is not valid JSON: {e}"))?;
            point_at_project_files(&mut value, dir);
            serde_json::from_value(value).map_err(|e| format!("The timeline is not valid JSON: {e}"))?
        }
        None => {
            serde_json::from_str(&args.timeline_json).map_err(|e| format!("The timeline is not valid JSON: {e}"))?
        }
    };
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

/// Replaces every asset path that names a file of the project by that file's absolute path, so
/// the check (which joins paths onto the public folder; joining an absolute path gives that
/// path) finds project files. Paths that are not in the project stay as they are.
pub fn point_at_project_files(timeline: &mut serde_json::Value, project: &Path) {
    crate::paths::for_each_asset(timeline, &mut |path, _| {
        if crate::paths::is_local_relative(path) {
            let file = project.join(path.as_str());
            if file.is_file() {
                *path = file.display().to_string();
            }
        }
    });
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finished {
    pub ok: bool,
    pub cancelled: bool,
    pub output: Option<String>,
    pub error: Option<String>,
    /// For a project render: its new history entry and the project as the Home screen shows it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub render: Option<crate::studio::RenderRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<crate::studio::ProjectSummary>,
    /// What the render noted (which music track, a project file used instead of a public one).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// Where the render goes: the path given, else the project's `renders/` folder.
pub fn output_for(output: Option<&str>, project: Option<&Path>) -> Result<PathBuf, String> {
    match (output.map(str::trim).filter(|o| !o.is_empty()), project) {
        (Some(path), _) => Ok(PathBuf::from(path)),
        (None, Some(dir)) => Ok(crate::renders::default_output(dir)),
        (None, None) => Err("No output file was chosen.".into()),
    }
}

#[tauri::command]
pub fn start_render(app: AppHandle, state: State<'_, RenderState>, args: StartArgs) -> Result<(), String> {
    let timeline: VideoTimeline =
        serde_json::from_str(&args.timeline_json).map_err(|e| format!("The timeline is not valid JSON: {e}"))?;
    let remotion_dir = default_remotion_dir();
    if !remotion_dir.join("package.json").exists() {
        return Err(format!("Remotion project not found at {}", remotion_dir.display()));
    }
    let studio = crate::studio::studio(&app)?;
    let project = match &args.project_dir {
        Some(dir) => Some(studio.project(dir)?),
        None => None,
    };
    let output = output_for(args.output.as_deref(), project.as_deref())?;
    let panel_options = serde_json::to_value(&args.options).unwrap_or_default();

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
        let mut cfg = options.config(work_dir.clone(), output.clone(), remotion_dir.clone());
        if let Some(dir) = &project {
            // A project's own files come first, as `vimmer <project>/timeline.json` does.
            cfg.asset_dirs = vec![dir.clone()];
            cfg.asset_dirs_first = true;
        }
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
        let failed = |cancelled: bool, error: Option<String>| Finished {
            ok: false,
            cancelled,
            output: None,
            error,
            render: None,
            project: None,
            notes: Vec::new(),
        };
        let finished = match result {
            Ok(report) => {
                // A project render goes into the project's history; a problem there must not
                // turn a finished render into a failed one.
                let recorded = match project {
                    Some(dir) => {
                        let out = report.output.clone();
                        tauri::async_runtime::spawn_blocking(move || {
                            let record = crate::renders::record_render(&studio, &dir, &out, panel_options, false).ok()?;
                            let meta = crate::studio::read_meta(&dir).ok()?;
                            Some((record, crate::studio::summary(&dir, &meta)))
                        })
                        .await
                        .ok()
                        .flatten()
                    }
                    None => None,
                };
                let (render, project) = recorded.map_or((None, None), |(r, p)| (Some(r), Some(p)));
                Finished {
                    ok: true,
                    cancelled: false,
                    output: Some(report.output.display().to_string()),
                    error: None,
                    render,
                    project,
                    notes: report.notes,
                }
            }
            Err(SchedulerError::Cancelled) => failed(true, None),
            Err(e) => failed(false, Some(e.to_string())),
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
pub(crate) fn default_remotion_dir() -> PathBuf {
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
pub(crate) fn vimmer_env(name: &str) -> Option<String> {
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

pub(crate) fn unix_millis() -> u128 {
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
            let args = CheckArgs { timeline_json: timeline.to_string(), tts: Some(tts.into()), project_dir: None };
            serde_json::to_value(tauri::async_runtime::block_on(check_timeline_in(args, None)).unwrap()).unwrap()
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

        let bad = CheckArgs { timeline_json: "{}".into(), tts: None, project_dir: None };
        assert!(tauri::async_runtime::block_on(check_timeline_in(bad, None)).is_err());
    }

    #[test]
    fn start_args_without_options_still_parse() {
        let args: StartArgs = serde_json::from_str(r#"{"timelineJson":"{}","output":"o.mp4"}"#).unwrap();
        assert!(args.options.tts.is_none());
        // What the editor sends for a plain timeline file: no project, the chosen path.
        assert!(args.project_dir.is_none());
        assert_eq!(output_for(args.output.as_deref(), None).unwrap(), PathBuf::from("o.mp4"));
        assert!(output_for(None, None).is_err(), "a plain file needs a chosen path");
        assert!(output_for(Some("  "), None).is_err());
    }

    #[test]
    fn a_project_render_defaults_to_its_renders_folder() {
        let args: StartArgs = serde_json::from_str(r#"{"timelineJson":"{}","projectDir":"/p/Film"}"#).unwrap();
        assert_eq!(args.project_dir.as_deref(), Some("/p/Film"));
        let out = output_for(args.output.as_deref(), Some(Path::new("/p/Film"))).unwrap();
        assert!(out.starts_with("/p/Film/renders") && out.extension().unwrap() == "mp4", "{}", out.display());
        // A path the user chose wins.
        assert_eq!(output_for(Some("/tmp/x.mp4"), Some(Path::new("/p/Film"))).unwrap(), PathBuf::from("/tmp/x.mp4"));
    }

    #[test]
    fn the_check_finds_a_projects_own_files() {
        let dir = std::env::temp_dir().join(format!("vimmer-check-project-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        let img = image::RgbImage::from_pixel(64, 36, image::Rgb([40, 40, 40]));
        img.save(dir.join("assets/mine.png")).unwrap();
        let timeline = serde_json::json!({"fps": 30, "resolution": {"width": 1920, "height": 1080},
            "scenes": [{"id": "a", "duration_ms": 3000, "text": "Hello", "image": "assets/mine.png"},
                       {"id": "b", "duration_ms": 3000, "text": "World", "image": "assets/sample.jpeg"}]});
        let codes = |project: Option<PathBuf>| {
            let args = CheckArgs { timeline_json: timeline.to_string(), tts: Some("say".into()), project_dir: None };
            let report = serde_json::to_value(tauri::async_runtime::block_on(check_timeline_in(args, project)).unwrap()).unwrap();
            report["findings"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|f| f["code"] == "assets.missing_image")
                .map(|f| f["suggestion"]["path"].as_str().unwrap_or("").to_string())
                .collect::<Vec<_>>()
        };
        // Without the project the file is unknown; with it, found. The public file is found both ways.
        assert_eq!(codes(None), ["/scenes/0/image"]);
        assert!(codes(Some(dir.clone())).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
