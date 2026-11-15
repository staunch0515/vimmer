//! "Generate a video from text" in the app: the same steps as `vimmer generate` (storyboard by
//! Claude, optional illustrations), ending in a new project instead of a render.
//!
//! Money: nothing is sent to Claude before the user has seen `generate_estimate` and confirmed
//! (`start_generate` refuses without `confirmed`). The finished event carries the actual cost.
//!
//! The API key: when `ANTHROPIC_API_KEY` is in this process's environment the helper inherits
//! it, as in the CLI. Otherwise (the app was started from Finder) it is read from the user's
//! shell on the first generation, kept in memory only, and handed to the helper process alone
//! (`LlmHelper.env`). It is never written to an event, an error, a log, the project folder or
//! the settings; as a last line of defence every message is scrubbed of the value before it
//! leaves this module.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, State};
use vimmer_media::{Limits, TtsEngine};
use vimmer_storyboard::generate::{
    generate_with_key_check, FailureKind, GenerateOptions, LlmHelper, Usage, ENV_API_KEY,
};
use vimmer_storyboard::illustrate::{self, illustrate_with_key_check, IllustrateOptions, IllustrationReport, PRICES};
use vimmer_storyboard::{Language, Orientation};

use crate::studio::{ProjectSummary, Studio, SOURCE_DIR, TIMELINE_FILE};

pub const PROGRESS_EVENT: &str = "generate-progress";
pub const FINISHED_EVENT: &str = "generate-finished";

/// Models offered in the dialog: Opus (the default) and the cheaper Sonnet.
pub const MODELS: [&str; 2] = ["claude-opus-5-5", "claude-sonnet-5-5"];
const MAX_TEXT_CHARS: usize = 60_000;
/// Input tokens of the fixed prompt and schema (README, "Cost").
const PROMPT_TOKENS: f64 = 2_600.0;
const NO_KEY_MESSAGE: &str =
    "ANTHROPIC_API_KEY was not found. Set ANTHROPIC_API_KEY in ~/.zshrc (export ANTHROPIC_API_KEY=...), then try again.";

#[derive(Default)]
pub struct GenerateState {
    current: Mutex<Option<Arc<AtomicBool>>>,
    /// The key as read from the user's shell. Memory only.
    key: Mutex<Option<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateArgs {
    pub text: String,
    /// `zh`, `ja` or `en`.
    pub lang: String,
    /// `landscape` or `portrait`.
    pub orientation: String,
    #[serde(default)]
    pub target_seconds: Option<u32>,
    pub model: String,
    #[serde(default)]
    pub illustrate: bool,
    /// Project name; missing or blank: the storyboard's title.
    #[serde(default)]
    pub name: Option<String>,
    /// The user saw the estimate and pressed the confirm button.
    #[serde(default)]
    pub confirmed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Cost {
    /// What a normal run costs, in USD.
    pub typical: f64,
    /// When every call needs its corrections.
    pub worst: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Estimate {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Scenes the storyboard is expected to have.
    pub scenes: usize,
    pub storyboard: Cost,
    /// Present when illustrations are on.
    pub illustrate: Option<Cost>,
    pub total: Cost,
    /// Something that would stop a generation before any call (no Python helper): shown instead
    /// of letting the user confirm.
    pub problem: Option<String>,
}

fn language(code: &str) -> Result<Language, String> {
    match code {
        "zh" => Ok(Language::Zh),
        "ja" => Ok(Language::Ja),
        "en" => Ok(Language::En),
        other => Err(format!("Unknown language `{other}`.")),
    }
}

fn orientation(name: &str) -> Result<Orientation, String> {
    match name {
        "landscape" => Ok(Orientation::Landscape),
        "portrait" => Ok(Orientation::Portrait),
        other => Err(format!("Unknown orientation `{other}`.")),
    }
}

/// Checks the dialog's values; returns the problem in words.
pub fn check_args(args: &GenerateArgs) -> Result<(), String> {
    if args.text.trim().is_empty() {
        return Err("The text is empty.".into());
    }
    if args.text.chars().count() > MAX_TEXT_CHARS {
        return Err(format!("The text is longer than {MAX_TEXT_CHARS} characters."));
    }
    if !MODELS.contains(&args.model.as_str()) {
        return Err(format!("Unknown model {}.", args.model));
    }
    language(&args.lang)?;
    orientation(&args.orientation)?;
    if args.target_seconds.is_some_and(|s| !(5..=1_800).contains(&s)) {
        return Err("The target length must be between 5 and 1800 seconds.".into());
    }
    Ok(())
}

/// Tokens of a text, roughly: one per CJK (or other non-ASCII) character, one per four ASCII
/// characters. Errs on the high side.
fn text_tokens(text: &str) -> f64 {
    let wide = text.chars().filter(|c| !c.is_ascii()).count() as f64;
    let ascii = text.chars().filter(char::is_ascii).count() as f64;
    wide + ascii / 4.0
}

/// How many scenes to expect: one per six seconds of the target, else from the length of the
/// text (about 45 CJK characters or 30 words a scene).
fn expected_scenes(text: &str, target_seconds: Option<u32>) -> usize {
    let scenes = match target_seconds {
        Some(s) => (f64::from(s) / 6.0).ceil(),
        None => {
            let wide = text.chars().filter(|c| !c.is_ascii() && !c.is_whitespace()).count() as f64;
            let words = text.split_whitespace().filter(|w| w.is_ascii()).count() as f64;
            (wide / 45.0 + words / 30.0).ceil()
        }
    };
    (scenes as usize).clamp(3, 60)
}

/// The estimate shown before anything is sent: token counts times the prices of the README's
/// cost table. The storyboard's worst case is two corrections, each resending the conversation.
pub fn estimate(args: &GenerateArgs) -> Result<Estimate, String> {
    check_args(args)?;
    let (_, p_in, p_out) =
        PRICES.iter().find(|(m, _, _)| *m == args.model).ok_or_else(|| format!("No price known for {}.", args.model))?;
    let scenes = expected_scenes(&args.text, args.target_seconds);
    let input = PROMPT_TOKENS + text_tokens(&args.text);
    let output = 250.0 + 170.0 * scenes as f64;
    let call = |n: f64| ((input + n * (output + 300.0)) * p_in + output * p_out) / 1e6;
    let storyboard = Cost { typical: call(0.0), worst: call(0.0) + call(1.0) + call(2.0) };
    let pictures = match args.illustrate {
        true => illustrate::estimate_cost(&args.model, illustrate::DEFAULT_EFFORT, scenes)
            .map(|(typical, worst)| Cost { typical, worst }),
        false => None,
    };
    let total = Cost {
        typical: storyboard.typical + pictures.map_or(0.0, |c| c.typical),
        worst: storyboard.worst + pictures.map_or(0.0, |c| c.worst),
    };
    Ok(Estimate {
        model: args.model.clone(),
        input_tokens: input.round() as u64,
        output_tokens: output.round() as u64,
        scenes,
        storyboard,
        illustrate: pictures,
        total,
        problem: LlmHelper::locate(None).err(),
    })
}

/// What a generation reports while it runs.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateProgress {
    /// `storyboard`, `illustrations` or `project`.
    pub stage: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateFinished {
    pub ok: bool,
    pub cancelled: bool,
    pub project: Option<ProjectSummary>,
    /// What the calls made so far cost, in USD (also when the run failed or was cancelled).
    pub cost_usd: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub scenes: usize,
    pub illustrated: usize,
    pub warnings: Vec<String>,
    /// `no_api_key`, `helper`, `api`, `invalid`, `timed_out`, `project`; `None` when ok.
    pub error_kind: Option<String>,
    pub error: Option<String>,
}

impl GenerateFinished {
    fn failed(kind: &str, message: impl Into<String>, usage: &Usage) -> Self {
        Self {
            ok: false,
            cancelled: kind == "cancelled",
            project: None,
            cost_usd: usage.cost_usd,
            input_tokens: usage.input_tokens + usage.cache_creation_input_tokens + usage.cache_read_input_tokens,
            output_tokens: usage.output_tokens,
            scenes: 0,
            illustrated: 0,
            warnings: Vec::new(),
            error_kind: Some(kind.to_string()),
            error: Some(message.into()),
        }
    }

    /// Removes `secret` from every text in the result.
    fn scrubbed(mut self, secret: Option<&str>) -> Self {
        self.error = self.error.map(|e| scrub(&e, secret));
        self.warnings = self.warnings.iter().map(|w| scrub(w, secret)).collect();
        self
    }
}

/// `text` without `secret` in it.
fn scrub(text: &str, secret: Option<&str>) -> String {
    match secret.filter(|s| s.len() >= 8) {
        Some(s) => text.replace(s, "[api key removed]"),
        None => text.to_string(),
    }
}

/// Everything a generation needs besides the dialog's values.
pub struct GenerateEnv {
    pub studio: Studio,
    pub helper: LlmHelper,
    pub remotion_dir: PathBuf,
    /// Scratch folder for request files; removed by the caller.
    pub work_dir: PathBuf,
    /// Headless Chrome that turns the drawn SVGs into pictures; `None`: no illustrations.
    pub chrome: Option<PathBuf>,
    /// The engine the render will use; scenes are sized for it.
    pub tts: TtsEngine,
    pub timeout: Duration,
}

/// Runs the generation and makes the project. `on_event` gets progress; the key (if this module
/// handed one to the helper) is `secret`, removed from every message.
pub fn run_generation(
    env: &GenerateEnv,
    args: &GenerateArgs,
    limits: &Limits,
    secret: Option<&str>,
    on_event: &(dyn Fn(GenerateProgress) + Sync),
) -> GenerateFinished {
    run(env, args, limits, secret, on_event).scrubbed(secret)
}

fn run(
    env: &GenerateEnv,
    args: &GenerateArgs,
    limits: &Limits,
    secret: Option<&str>,
    on_event: &(dyn Fn(GenerateProgress) + Sync),
) -> GenerateFinished {
    let none = Usage::default();
    if let Err(e) = check_args(args) {
        return GenerateFinished::failed("helper", e, &none);
    }
    let (lang, orient) = (language(&args.lang).expect("checked"), orientation(&args.orientation).expect("checked"));
    let mut warnings: Vec<String> = Vec::new();
    let mut opts = GenerateOptions::new(args.text.clone(), lang);
    opts.orientation = orient;
    opts.target_seconds = args.target_seconds;
    opts.model = args.model.clone();
    opts.timeout = env.timeout;
    opts.tts = env.tts;
    match vimmer_dsl::MusicLibrary::load(&env.studio.public_dir) {
        Ok((library, _)) => opts.music_moods = library.main_moods(),
        Err(e) => warnings.push(format!("No background music: {e}")),
    }

    on_event(GenerateProgress { stage: "storyboard", message: format!("Writing the storyboard with {}", args.model) });
    let mut generated = match generate_with_key_check(&env.helper, &opts, &env.work_dir.join("llm"), limits, true) {
        Ok(g) => g,
        Err(failure) => {
            let kind = match &failure.kind {
                FailureKind::NoApiKey => "no_api_key",
                FailureKind::Helper => "helper",
                FailureKind::Api(kind) if kind == "auth" => "no_api_key",
                FailureKind::Api(_) => "api",
                FailureKind::Invalid => "invalid",
                FailureKind::TimedOut => "timed_out",
                FailureKind::Cancelled => "cancelled",
            };
            return GenerateFinished::failed(kind, failure.message.clone(), &failure.usage);
        }
    };
    let mut usage = generated.usage;
    warnings.extend(generated.conversion.warnings.iter().map(ToString::to_string));
    let scenes = generated.conversion.timeline.scenes.len();

    let mut report: Option<IllustrationReport> = None;
    if args.illustrate && !(limits.cancelled)() {
        match &env.chrome {
            None => warnings.push(format!(
                "No headless Chrome to turn the drawings into pictures (run `npm ci` in {}); made without illustrations.",
                env.remotion_dir.display()
            )),
            Some(chrome) => {
                on_event(GenerateProgress {
                    stage: "illustrations",
                    message: format!("Drawing {scenes} illustrations with {}", args.model),
                });
                let mut io = IllustrateOptions::new(env.studio.public_dir.clone(), chrome.clone());
                io.model = args.model.clone();
                io.timeout = env.timeout;
                let drawn = illustrate_with_key_check(
                    &env.helper,
                    &generated.storyboard,
                    &io,
                    &env.work_dir.join("illustrate"),
                    limits,
                    &|line| on_event(GenerateProgress { stage: "illustrations", message: scrub(&line, secret) }),
                    true,
                );
                usage.add(&drawn.usage);
                warnings.extend(drawn.all_warnings().into_iter().map(|w| format!("Illustration: {w}")));
                if let Some(reason) = &drawn.unavailable {
                    warnings.push(format!("Illustrations unavailable: {reason}."));
                }
                if drawn.cancelled {
                    warnings.push("Stopped while drawing; the project has the pictures made so far.".into());
                }
                if let Err(e) = drawn.apply(&mut generated.conversion.timeline) {
                    warnings.push(format!("Illustrations not used: {e}"));
                }
                report = Some(drawn);
            }
        }
    }

    // The storyboard is paid for: from here on the project is made even after a cancel.
    on_event(GenerateProgress { stage: "project", message: "Creating the project".into() });
    let mut timeline = generated.conversion.timeline.clone();
    let made = make_project(env, args, &mut timeline, &generated, report.as_ref(), &usage, secret);
    let (project, illustrated) = match made {
        Ok(done) => done,
        Err(e) => return GenerateFinished::failed("project", e, &usage),
    };
    GenerateFinished {
        ok: true,
        cancelled: false,
        project: Some(project),
        cost_usd: usage.cost_usd,
        input_tokens: usage.input_tokens + usage.cache_creation_input_tokens + usage.cache_read_input_tokens,
        output_tokens: usage.output_tokens,
        scenes,
        illustrated,
        warnings,
        error_kind: None,
        error: None,
    }
}

/// Writes the project: illustrations into `assets/illustrations/`, the timeline, and the text,
/// storyboard and usage into `source/`.
fn make_project(
    env: &GenerateEnv,
    args: &GenerateArgs,
    timeline: &mut vimmer_dsl::VideoTimeline,
    generated: &vimmer_storyboard::generate::Generated,
    report: Option<&IllustrationReport>,
    usage: &Usage,
    secret: Option<&str>,
) -> Result<(ProjectSummary, usize), String> {
    // The folder exists before the pictures are copied; the timeline is written again below with
    // their project paths.
    let name = args.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).unwrap_or(generated.storyboard.title.as_str());
    // Every file is scrubbed of the key: a helper message kept in `usage.json` must not carry it.
    let draft = scrub(&pretty(&*timeline), secret);
    let dir = env.studio.create_with(&scrub(name, secret), &draft, Some(&args.lang), "generated", None)?;
    let write = |rel: &str, body: String| {
        let file = dir.join(rel);
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Could not create {}: {e}", parent.display()))?;
        }
        crate::studio::write_atomic(&file, scrub(&body, secret).as_bytes())
    };

    let mut illustrated = 0;
    let prefix = "generated/illustrations/";
    for scene in &mut timeline.scenes {
        let Some(image) = scene.image.clone().filter(|i| i.starts_with(prefix)) else { continue };
        let source = env.studio.public_dir.join(&image);
        let rel = format!("assets/illustrations/{}", &image[prefix.len()..]);
        let target = dir.join(&rel);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Could not create {}: {e}", parent.display()))?;
        }
        // A picture that cannot be copied stays a reference to the public folder's cache.
        if fs::copy(&source, &target).is_ok() {
            scene.image = Some(rel);
            illustrated += 1;
        }
    }
    if illustrated > 0 {
        write(TIMELINE_FILE, pretty(&*timeline))?;
    }

    write(&format!("{SOURCE_DIR}/input.txt"), args.text.clone())?;
    write(&format!("{SOURCE_DIR}/storyboard.json"), pretty(&generated.storyboard))?;
    let usage_doc = json!({
        "model": args.model,
        "language": args.lang,
        "orientation": args.orientation,
        "target_seconds": args.target_seconds,
        "attempts": generated.attempts,
        "retries": generated.retries(),
        "storyboard": generated.usage,
        "illustrations": report.map(|r| json!({
            "model": r.model, "usage": r.usage, "drawn": r.drawn(), "scenes": r.scenes.len()
        })),
        "total": usage,
    });
    write(&format!("{SOURCE_DIR}/usage.json"), pretty(&usage_doc))?;
    if let Some(report) = report {
        write(&format!("{SOURCE_DIR}/illustrations.json"), pretty(report))?;
    }
    Ok((env.studio.touch_thumbnail(&dir)?, illustrated))
}

/// Pretty JSON of a value, with a final newline.
fn pretty<T: Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "null".into()) + "\n"
}

/// The key when it is not in this process's environment: asked from the user's shell once and
/// remembered in memory. `None`: it is not set there either.
fn shell_key(state: &GenerateState) -> Option<String> {
    let mut cached = state.key.lock().unwrap_or_else(|e| e.into_inner());
    if cached.is_none() {
        *cached = read_key_from_shell();
    }
    cached.clone()
}

/// `$ANTHROPIC_API_KEY` as a login shell that has read `~/.zshrc` sees it. Only stdout is read;
/// nothing is logged.
fn read_key_from_shell() -> Option<String> {
    if !Path::new("/bin/zsh").is_file() {
        return None;
    }
    let script = format!(r#"source ~/.zshrc >/dev/null 2>&1; printf %s "${ENV_API_KEY}""#);
    let out = Command::new("/bin/zsh")
        .args(["-lc", &script])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|k| !k.is_empty())
}

fn key_in_env() -> bool {
    std::env::var_os(ENV_API_KEY).is_some_and(|v| !v.to_string_lossy().trim().is_empty())
}

/// The cost estimate for the dialog. Makes no API call and does not read the key.
#[tauri::command]
pub async fn generate_estimate(args: GenerateArgs) -> Result<Estimate, String> {
    crate::studio::blocking(move || estimate(&args)).await
}

#[tauri::command]
pub fn start_generate(app: AppHandle, state: State<'_, GenerateState>, args: GenerateArgs) -> Result<(), String> {
    if !args.confirmed {
        return Err("Generating costs money: confirm the estimate first.".into());
    }
    check_args(&args)?;
    let studio = crate::studio::studio(&app)?;
    let cache = app.path().app_cache_dir().map_err(|e| format!("No cache folder: {e}"))?;
    let cancelled = Arc::new(AtomicBool::new(false));
    {
        let mut current = state.current.lock().unwrap_or_else(|e| e.into_inner());
        if current.is_some() {
            return Err("A generation is already running.".into());
        }
        *current = Some(cancelled.clone());
    }

    std::thread::spawn(move || {
        let state = app.state::<GenerateState>();
        let none = Usage::default();
        let finished = (|| {
            // The key: inherited when this process has it, else from the shell, to the helper only.
            let secret = if key_in_env() { None } else { shell_key(&state) };
            if !key_in_env() && secret.is_none() {
                return GenerateFinished::failed("no_api_key", NO_KEY_MESSAGE, &none);
            }
            let mut helper = match LlmHelper::locate(None) {
                Ok(h) => h,
                Err(e) => return GenerateFinished::failed("helper", e, &none),
            };
            if let Some(key) = &secret {
                helper.env.set(ENV_API_KEY, key.clone());
            }
            let remotion_dir = crate::render::default_remotion_dir();
            let work_dir = cache.join(format!("generate-{}", crate::render::unix_millis()));
            let env = GenerateEnv {
                studio,
                helper,
                chrome: vimmer_media::find_chrome(&remotion_dir),
                remotion_dir,
                work_dir: work_dir.clone(),
                tts: crate::render::default_engine(&crate::render::vimmer_env).0,
                timeout: vimmer_storyboard::generate::DEFAULT_TIMEOUT,
            };
            let flag = cancelled.clone();
            let limits = Limits { timeout: None, cancelled: Arc::new(move || flag.load(Ordering::Relaxed)) };
            let progress_app = app.clone();
            let done = run_generation(&env, &args, &limits, secret.as_deref(), &move |p| {
                let _ = progress_app.emit(PROGRESS_EVENT, p);
            });
            let _ = fs::remove_dir_all(&work_dir);
            done
        })();
        *state.current.lock().unwrap_or_else(|e| e.into_inner()) = None;
        crate::studio::allow_root(&app);
        let _ = app.emit(FINISHED_EVENT, finished);
    });
    Ok(())
}

#[tauri::command]
pub fn cancel_generate(state: State<'_, GenerateState>) -> bool {
    let current = state.current.lock().unwrap_or_else(|e| e.into_inner());
    match current.as_ref() {
        Some(flag) => {
            flag.store(true, Ordering::Relaxed);
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::studio::tests::Fixture;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex as StdMutex;

    /// A stand-in for the Python helper: prints `reply.json` with `@ENV@` replaced by whatever
    /// key it was given, and repeats that on stderr. No network, no real key.
    const FAKE_PYTHON: &str = r#"#!/bin/sh
dir=$(dirname "$0")
echo run >> "$dir/runs"
echo "helper saw $ANTHROPIC_API_KEY" >&2
sed "s/@ENV@/$ANTHROPIC_API_KEY/g" "$dir/reply.json"
"#;
    const FAKE_KEY: &str = "sk-fake-test-value-000";

    fn args(text: &str) -> GenerateArgs {
        GenerateArgs {
            text: text.into(),
            lang: "zh".into(),
            orientation: "portrait".into(),
            target_seconds: None,
            model: "claude-sonnet-5-5".into(),
            illustrate: false,
            name: None,
            confirmed: true,
        }
    }

    fn env(fx: &Fixture, reply: serde_json::Value) -> GenerateEnv {
        let dir = fx.root.join("helper");
        fs::create_dir_all(&dir).unwrap();
        let python = dir.join("python");
        fs::write(&python, FAKE_PYTHON).unwrap();
        fs::set_permissions(&python, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(dir.join("reply.json"), reply.to_string() + "\n").unwrap();
        let mut helper = LlmHelper::locate(Some(&python)).unwrap();
        helper.dir = dir.join("files");
        helper.env.set(ENV_API_KEY, FAKE_KEY);
        GenerateEnv {
            studio: fx.studio.clone(),
            helper,
            remotion_dir: fx.root.join("remotion"),
            work_dir: fx.root.join("work"),
            chrome: None,
            tts: TtsEngine::Say,
            timeout: Duration::from_secs(30),
        }
    }

    fn good_reply() -> serde_json::Value {
        let storyboard = json!({"title": "西安/钟楼", "orientation": "portrait", "language": "zh", "scenes": [
            {"text": "西安钟楼，古城的中心。", "emphasis": ["钟楼"], "camera": "push_in"},
            {"text": "它已有六百多年历史。", "camera": "pull_out", "transition": "fade"}]});
        json!({"ok": true, "storyboard_json": storyboard.to_string(), "model": "claude-sonnet-5-5",
               "stop_reason": "end_turn", "warnings": ["note with @ENV@ inside"],
               "usage": {"input_tokens": 2700, "output_tokens": 600, "cache_creation_input_tokens": 0,
                         "cache_read_input_tokens": 0, "cost_usd": 0.0114}})
    }

    fn all_text_under(dir: &Path) -> String {
        let mut out = String::new();
        for entry in fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.push_str(&all_text_under(&path));
            } else {
                out.push_str(&String::from_utf8_lossy(&fs::read(&path).unwrap()));
            }
        }
        out
    }

    #[test]
    fn a_generation_becomes_a_project_and_the_key_stays_out_of_everything() {
        let fx = Fixture::new("generate");
        let env = env(&fx, good_reply());
        let events: StdMutex<Vec<GenerateProgress>> = StdMutex::new(Vec::new());
        let done = run_generation(&env, &args("西安钟楼位于古城中心。"), &Limits::none(), Some(FAKE_KEY), &|p| {
            events.lock().unwrap().push(p)
        });
        assert!(done.ok, "{:?}", done.error);
        assert_eq!(done.scenes, 2);
        assert!((done.cost_usd - 0.0114).abs() < 1e-9);
        assert_eq!((done.input_tokens, done.output_tokens), (2700, 600));

        let project = done.project.clone().unwrap();
        assert_eq!(project.name, "西安/钟楼", "the storyboard's title");
        assert!(project.dir.ends_with("/西安 钟楼"), "{}", project.dir);
        assert_eq!((project.origin.as_str(), project.lang.as_deref()), ("generated", Some("zh")));
        assert_eq!(project.orientation, "portrait");
        assert!(project.duration_ms > 0);
        let dir = Path::new(&project.dir);
        let timeline: vimmer_dsl::VideoTimeline =
            serde_json::from_str(&fs::read_to_string(dir.join(TIMELINE_FILE)).unwrap()).unwrap();
        vimmer_dsl::validate(&timeline).unwrap();
        assert_eq!(timeline.scenes.len(), 2);
        assert_eq!(fs::read_to_string(dir.join("source/input.txt")).unwrap(), "西安钟楼位于古城中心。");
        let storyboard: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.join("source/storyboard.json")).unwrap()).unwrap();
        assert_eq!(storyboard["scenes"].as_array().unwrap().len(), 2);
        let usage: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.join("source/usage.json")).unwrap()).unwrap();
        assert_eq!(usage["total"]["cost_usd"], 0.0114);
        assert_eq!(usage["model"], "claude-sonnet-5-5");
        assert_eq!(fx.studio.open(&project.dir).unwrap().project.name, project.name, "it opens");

        // The helper did get the key (it printed it), yet no event, result or file has it.
        let stages: Vec<&str> = events.lock().unwrap().iter().map(|e| e.stage).collect();
        assert_eq!(stages, ["storyboard", "project"]);
        let said = format!("{:?} {}", events.lock().unwrap(), serde_json::to_string(&done).unwrap());
        assert!(!said.contains(FAKE_KEY), "{said}");
        assert!(!all_text_under(dir).contains(FAKE_KEY));
        assert!(!all_text_under(&fx.root.join("config")).contains(FAKE_KEY));
        assert!(!format!("{:?}", env.helper).contains(FAKE_KEY));
        assert!(!fx.root.join("work/llm").read_dir().is_ok_and(|mut d| d.next().is_some()), "request files are removed");
    }

    #[test]
    fn a_failed_call_reports_its_cost_makes_no_project_and_hides_the_key() {
        let fx = Fixture::new("generate-fail");
        let reply = json!({"ok": false, "error_kind": "api", "error": "overloaded while using @ENV@",
            "usage": {"input_tokens": 100, "output_tokens": 0, "cache_creation_input_tokens": 0,
                      "cache_read_input_tokens": 0, "cost_usd": 0.0002}});
        let env = env(&fx, reply);
        let done = run_generation(&env, &args("text"), &Limits::none(), Some(FAKE_KEY), &|_| {});
        assert!(!done.ok && !done.cancelled);
        assert!(done.project.is_none());
        assert!(fx.studio.list().is_empty());
        let shown = serde_json::to_string(&done).unwrap();
        assert!(!shown.contains(FAKE_KEY), "{shown}");
        assert!(done.error.is_some());
    }

    #[test]
    fn a_cancel_before_the_call_stops_without_a_project_or_a_charge() {
        let fx = Fixture::new("generate-cancel");
        let env = env(&fx, good_reply());
        let limits = Limits { timeout: None, cancelled: Arc::new(|| true) };
        let done = run_generation(&env, &args("text"), &limits, None, &|_| {});
        assert!(done.cancelled && !done.ok);
        assert_eq!(done.error_kind.as_deref(), Some("cancelled"));
        assert_eq!(done.cost_usd, 0.0);
        assert!(fx.studio.list().is_empty());
    }

    #[test]
    fn bad_values_never_reach_the_helper() {
        let fx = Fixture::new("generate-args");
        let env = env(&fx, good_reply());
        let mut bad = args("  ");
        assert!(check_args(&bad).is_err());
        assert!(!run_generation(&env, &bad, &Limits::none(), None, &|_| {}).ok);
        bad = args("ok");
        bad.model = "gpt-x".into();
        assert!(check_args(&bad).unwrap_err().contains("gpt-x"));
        bad = args("ok");
        bad.lang = "fr".into();
        assert!(check_args(&bad).is_err());
        bad = args("ok");
        bad.target_seconds = Some(2);
        assert!(check_args(&bad).is_err());
        assert!(!fx.root.join("helper/runs").exists(), "the helper was never started");
    }

    #[test]
    fn the_estimate_follows_the_readme_cost_table() {
        // The README's examples: about 250 characters, measured at $0.029-0.035 (Opus) and
        // $0.012-0.016 (Sonnet) per storyboard.
        let text = "西".repeat(250);
        let mut a = args(&text);
        a.model = "claude-opus-5-5".into();
        let opus = estimate(&a).unwrap();
        assert!((0.025..=0.045).contains(&opus.storyboard.typical), "{opus:?}");
        assert!(opus.storyboard.worst > 2.5 * opus.storyboard.typical && opus.storyboard.worst < 0.2, "{opus:?}");
        assert_eq!(opus.illustrate, None);
        assert_eq!(opus.total, opus.storyboard);
        a.model = "claude-sonnet-5-5".into();
        let sonnet = estimate(&a).unwrap();
        assert!((0.010..=0.022).contains(&sonnet.storyboard.typical), "{sonnet:?}");

        // Illustrations cost far more and scale with the scenes (about $0.10 a scene with Opus).
        a.model = "claude-opus-5-5".into();
        a.illustrate = true;
        a.target_seconds = Some(60);
        let drawn = estimate(&a).unwrap();
        assert_eq!(drawn.scenes, 10);
        let pictures = drawn.illustrate.unwrap();
        assert!((0.5..=1.2).contains(&pictures.typical), "{pictures:?}");
        assert!((drawn.total.typical - drawn.storyboard.typical - pictures.typical).abs() < 1e-12);
        assert!(drawn.total.worst > drawn.total.typical);

        assert!(estimate(&args("")).is_err());
        assert_eq!(expected_scenes("word ".repeat(165).as_str(), None), 6);
        assert_eq!(expected_scenes("x", None), 3);
    }

    #[test]
    fn nothing_starts_without_the_users_confirmation() {
        let parsed: GenerateArgs =
            serde_json::from_str(r#"{"text":"t","lang":"en","orientation":"landscape","model":"claude-opus-5-5"}"#).unwrap();
        assert!(!parsed.confirmed, "confirmation is never assumed");
        assert!(!parsed.illustrate);
        assert_eq!(scrub("a sk-fake-test-value-000 b", Some(FAKE_KEY)), "a [api key removed] b");
        assert_eq!(scrub("short secrets are left", Some("ab")), "short secrets are left");
    }
}
