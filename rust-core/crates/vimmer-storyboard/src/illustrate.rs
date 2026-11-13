//! Storyboard -> one Claude-drawn SVG illustration per scene -> PNGs for the timeline.
//!
//! 1. **Style**: one call (`scripts/illustrate.py`, kind `style`) turns the title and every
//!    scene's `image_prompt` into a style guide (palette, background, line, shapes, composition,
//!    mood, motifs). Every scene request then carries that guide, so the pictures of one video
//!    share one look.
//! 2. **Scenes**: up to `concurrency` helper runs at once, one per scene that has no `image` yet.
//!    The reply is untrusted: [`vimmer_media::sanitize_svg`] checks and rebuilds it; when it is
//!    rejected (or renders as a flat colour) the complaint goes back to Claude once. A scene that
//!    still fails stays text only, with a warning.
//! 3. **Raster**: the sanitized SVG is screenshotted by headless Chrome
//!    ([`vimmer_media::rasterize_svg`]) to a PNG the size of the frame, which the normal image
//!    pipeline then handles.
//!
//! Files live in `<public>/generated/illustrations/` (git-ignored), named by a hash of model,
//! effort, cache version, style guide, picture prompt and canvas size, so a re-run with the
//! same inputs makes no API call. The style guide is cached the same way (`style-<hash>.json`).
//!
//! Nothing here fails the video: every problem becomes a warning and the affected scene keeps
//! no image. When the API is not usable at all (no key, key rejected, no connection, no SDK),
//! the run stops early with [`IllustrationReport::unavailable`] set.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use vimmer_dsl::VideoTimeline;
use vimmer_media::{
    preflight_chrome, rasterize_svg, run_limited, sanitize_svg, Limits, MediaError, SvgError,
};

use crate::generate::{api_key_present, fnv1a, fnv1a_from, tail, LlmHelper, Usage, ENV_API_KEY};
use crate::model::{Language, Orientation, Storyboard};

pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
pub const DEFAULT_EFFORT: &str = "medium";
pub const DEFAULT_CONCURRENCY: usize = 3;
/// Time budget for one helper run (one API call, including the SDK's own retries).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);
/// Time budget for one Chrome screenshot.
pub const RASTER_TIMEOUT: Duration = Duration::from_secs(60);
/// API attempts per scene: the first reply plus one correction.
pub const MAX_ATTEMPTS: u32 = 2;
/// Folder under the public dir.
pub const ILLUSTRATION_DIR: &str = "generated/illustrations";
/// Part of every cache key; bump it when the prompts change enough that old pictures should
/// not be reused.
pub const CACHE_VERSION: &str = "illustrate-v1";

/// USD per million tokens (input, output), first-party API prices from the claude-api skill
/// (2026-11-03). Must match `PRICES` in `scripts/generate_storyboard.py` (a test checks).
pub const PRICES: &[(&str, f64, f64)] = &[
    ("claude-fable-5-2", 8.00, 40.00),
    ("claude-fable-5-1", 10.00, 50.00),
    ("claude-opus-5-5", 4.00, 20.00),
    ("claude-opus-5", 5.00, 25.00),
    ("claude-opus-4-8", 5.00, 25.00),
    ("claude-sonnet-5-5", 2.00, 10.00),
    ("claude-sonnet-5", 2.00, 10.00),
    ("claude-haiku-5-5", 0.10, 0.50),
];

/// Helper error kinds that mean no further call can succeed in this run.
const FATAL_KINDS: &[&str] = &[
    "no_api_key",
    "auth",
    "permission",
    "not_found",
    "connection",
    "sdk_missing",
];

/// Used when the style call fails for a reason other than the API being unavailable.
pub fn fallback_style() -> Value {
    json!({
        "name": "Calm flat dusk",
        "palette": ["#0f172a", "#1e293b", "#334155", "#3b82f6", "#f59e0b", "#e2e8f0"],
        "background": "Vertical gradient from #334155 at the top to #0f172a in the lower half.",
        "line": "No outlines; where needed 3px #0f172a strokes with round caps.",
        "shapes": "Simple geometric flat shapes, corner radius 8px, two tones per surface.",
        "composition": "Subject centred in the upper half; lower part is dark calm ground.",
        "mood": "Quiet, clear, slightly warm evening light.",
        "motifs": "A soft round sun or moon in #f59e0b, low rolling ground shapes."
    })
}

/// What to draw and how.
#[derive(Debug, Clone)]
pub struct IllustrateOptions {
    pub model: String,
    pub effort: String,
    pub concurrency: usize,
    /// Per API call.
    pub timeout: Duration,
    /// Output token cap per call; `None`: the helper's default.
    pub max_tokens: Option<u32>,
    /// `remotion-renderer/public`; images go to `<public_dir>/generated/illustrations/`.
    pub public_dir: PathBuf,
    /// Headless Chrome for rasterizing (see [`vimmer_media::find_chrome`]).
    pub chrome: PathBuf,
    /// When set, the raw API replies are saved here.
    pub record_dir: Option<PathBuf>,
}

impl IllustrateOptions {
    pub fn new(public_dir: impl Into<PathBuf>, chrome: impl Into<PathBuf>) -> Self {
        Self {
            model: DEFAULT_MODEL.into(),
            effort: DEFAULT_EFFORT.into(),
            concurrency: DEFAULT_CONCURRENCY,
            timeout: DEFAULT_TIMEOUT,
            max_tokens: None,
            public_dir: public_dir.into(),
            chrome: chrome.into(),
            record_dir: None,
        }
    }
}

/// The outcome for one storyboard scene.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SceneIllustration {
    /// 0-based scene index.
    pub scene: usize,
    /// PNG path relative to the public dir, when a picture is ready.
    pub image: Option<String>,
    /// SVG path relative to the public dir (sanitized).
    pub svg: Option<String>,
    /// Size of the sanitized SVG in bytes.
    pub svg_bytes: usize,
    /// True when no API call was needed.
    pub cached: bool,
    /// API calls made for this scene.
    pub calls: u32,
    pub usage: Usage,
    /// What the sanitizer removed from the accepted SVG.
    pub removed: Vec<String>,
    /// Why there is no picture, or what went wrong on the way.
    pub warnings: Vec<String>,
    /// The scene already had an image; nothing was drawn.
    pub skipped: bool,
}

/// The whole run.
#[derive(Debug, Clone, Default, Serialize)]
pub struct IllustrationReport {
    pub model: String,
    pub effort: String,
    pub style: Option<Value>,
    pub style_cached: bool,
    pub style_usage: Usage,
    pub scenes: Vec<SceneIllustration>,
    /// Style call plus every scene call.
    pub usage: Usage,
    pub warnings: Vec<String>,
    /// Set when the API could not be used at all; then no scene has a picture.
    pub unavailable: Option<String>,
    pub cancelled: bool,
}

impl IllustrationReport {
    pub fn drawn(&self) -> usize {
        self.scenes.iter().filter(|s| s.image.is_some()).count()
    }

    /// Writes each ready picture into the matching timeline scene (same order as the
    /// storyboard). Returns how many scenes got an image.
    pub fn apply(&self, timeline: &mut VideoTimeline) -> Result<usize, String> {
        let wanted = self.scenes.len();
        if wanted != timeline.scenes.len() {
            return Err(format!(
                "the timeline has {} scenes but the storyboard has {wanted}",
                timeline.scenes.len()
            ));
        }
        let mut n = 0;
        for s in &self.scenes {
            if let Some(img) = &s.image {
                timeline.scenes[s.scene].image = Some(img.clone());
                n += 1;
            }
        }
        Ok(n)
    }

    /// All warnings, with the scene number in front of scene warnings.
    pub fn all_warnings(&self) -> Vec<String> {
        let mut out = self.warnings.clone();
        for s in &self.scenes {
            for w in &s.warnings {
                out.push(format!("scene {}: {w}", s.scene + 1));
            }
        }
        out
    }
}

/// Expected and worst-case cost (USD) of illustrating `scenes` scenes, or `None` for a model
/// without a known price. Output tokens per scene are the measured averages of Claude Opus 5.5
/// on the zh / ja examples at effort medium (~3,300; Sonnet 5.5 used ~1,750), rounded up; the
/// other efforts are scaled from a low/medium probe. The worst case assumes every scene needs
/// its correction call.
pub fn estimate_cost(model: &str, effort: &str, scenes: usize) -> Option<(f64, f64)> {
    let (_, p_in, p_out) = PRICES.iter().find(|(m, _, _)| *m == model)?;
    let scene_out: f64 = match effort {
        "low" => 1_800.0,
        "medium" => 3_500.0,
        "high" => 6_000.0,
        "xhigh" => 9_000.0,
        _ => 13_000.0,
    };
    let style = (1_200.0 * p_in + 1_000.0 * p_out) / 1e6;
    let scene = (1_700.0 * p_in + scene_out * p_out) / 1e6;
    // A correction call resends the first reply as input.
    let retry = ((1_900.0 + 4_000.0) * p_in + scene_out * p_out) / 1e6;
    let n = scenes as f64;
    Some((style + n * scene, style + n * (scene + retry)))
}

/// Cache key: 32 hex digits over the given parts.
pub fn cache_key(parts: &[&str]) -> String {
    let joined = parts.join("\u{1f}");
    format!(
        "{:016x}{:016x}",
        fnv1a(&joined),
        fnv1a_from(0x6c62_272e_07bb_0142, &joined)
    )
}

/// Canonical JSON text of a value (sorted object keys), so equal styles hash equally.
fn canonical(v: &Value) -> String {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let body: Vec<String> = keys
                .iter()
                .map(|k| format!("{}:{}", Value::String((*k).clone()), canonical(&map[*k])))
                .collect();
            format!("{{{}}}", body.join(","))
        }
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(canonical).collect::<Vec<_>>().join(",")
        ),
        other => other.to_string(),
    }
}

fn canvas(orientation: Orientation) -> (u32, u32) {
    match orientation {
        Orientation::Landscape => (1920, 1080),
        Orientation::Portrait => (1080, 1920),
    }
}

fn lang_code(l: Language) -> &'static str {
    match l {
        Language::Zh => "zh",
        Language::En => "en",
        Language::Ja => "ja",
    }
}

fn orientation_code(o: Orientation) -> &'static str {
    match o {
        Orientation::Landscape => "landscape",
        Orientation::Portrait => "portrait",
    }
}

/// The picture description of a scene: its `image_prompt`, else its text.
fn picture_prompt(sb: &Storyboard, i: usize) -> String {
    let scene = &sb.scenes[i];
    scene
        .image_prompt
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .unwrap_or(scene.text.trim())
        .to_string()
}

#[derive(Deserialize)]
struct HelperReply {
    ok: bool,
    #[serde(default)]
    style: Option<Value>,
    #[serde(default)]
    svg: Option<String>,
    #[serde(default)]
    usage: Option<Usage>,
    #[serde(default)]
    warnings: Vec<String>,
    #[serde(default)]
    error: Option<HelperError>,
}

#[derive(Deserialize)]
struct HelperError {
    kind: String,
    message: String,
}

/// One helper run's outcome.
enum Call {
    Reply(HelperReply),
    /// The helper or the API failed; `fatal` means stop the whole run.
    Failed {
        message: String,
        fatal: bool,
        usage: Usage,
    },
    Cancelled,
}

struct Runner<'a> {
    helper: &'a LlmHelper,
    opts: &'a IllustrateOptions,
    work_dir: &'a Path,
    limits: &'a Limits,
    seq: AtomicUsize,
}

impl Runner<'_> {
    fn call(&self, request: &Value, record_name: &str) -> Call {
        let n = self.seq.fetch_add(1, Ordering::SeqCst);
        let request_file = self
            .work_dir
            .join(format!("illustrate-{}-{n}.json", std::process::id()));
        if let Err(e) = fs::write(&request_file, request.to_string()) {
            return Call::Failed {
                message: format!("cannot write {}: {e}", request_file.display()),
                fatal: true,
                usage: Usage::default(),
            };
        }
        let record = self
            .opts
            .record_dir
            .as_ref()
            .map(|d| d.join(format!("{record_name}.json")));
        let output = run_limited(
            self.helper
                .illustrate_command(&request_file, record.as_deref()),
            &self.limits.with_timeout(self.opts.timeout),
        );
        let _ = fs::remove_file(&request_file);
        let output = match output {
            Ok(o) => o,
            Err(MediaError::Cancelled) => return Call::Cancelled,
            Err(MediaError::TimedOut { secs }) => {
                return Call::Failed {
                    message: format!("Claude did not answer within {secs} s"),
                    fatal: false,
                    usage: Usage::default(),
                }
            }
            Err(e) => {
                return Call::Failed {
                    message: format!("cannot run {}: {e}", self.helper.python.display()),
                    fatal: true,
                    usage: Usage::default(),
                }
            }
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        let reply: Option<HelperReply> = stdout
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .and_then(|l| serde_json::from_str(l).ok());
        let Some(reply) = reply else {
            return Call::Failed {
                message: format!(
                    "the illustration helper exited with {} and printed no result: {}",
                    output.status,
                    tail(&String::from_utf8_lossy(&output.stderr), 3)
                ),
                fatal: true,
                usage: Usage::default(),
            };
        };
        if reply.ok {
            return Call::Reply(reply);
        }
        let (kind, message) = reply.error.map_or_else(
            || ("unknown".to_string(), "the helper reported a failure".to_string()),
            |e| (e.kind, e.message),
        );
        Call::Failed {
            fatal: FATAL_KINDS.contains(&kind.as_str()),
            message,
            usage: reply.usage.unwrap_or_default(),
        }
    }
}

fn header(sb: &Storyboard, opts: &IllustrateOptions, kind: &str) -> Value {
    let (w, h) = canvas(sb.orientation);
    let mut v = json!({
        "kind": kind,
        "model": opts.model,
        "effort": opts.effort,
        "timeout_secs": opts.timeout.as_secs().max(1),
        "title": sb.title,
        "language": lang_code(sb.language),
        "orientation": orientation_code(sb.orientation),
        "width": w,
        "height": h,
    });
    if let Some(m) = opts.max_tokens {
        v["max_tokens"] = json!(m);
    }
    v
}

/// Writes `body` to `path` atomically.
fn write_atomic(path: &Path, body: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    fs::write(&tmp, body)?;
    fs::rename(&tmp, path)
}

/// Illustrates every scene of `sb` that has no `image`. Never fails: see the module docs.
/// `on_event` receives one-line progress messages.
pub fn illustrate(
    helper: &LlmHelper,
    sb: &Storyboard,
    opts: &IllustrateOptions,
    work_dir: &Path,
    limits: &Limits,
    on_event: &(dyn Fn(String) + Sync),
) -> IllustrationReport {
    illustrate_with_key_check(helper, sb, opts, work_dir, limits, on_event, api_key_present())
}

/// [`illustrate`] with the key check's outcome passed in (for tests).
pub fn illustrate_with_key_check(
    helper: &LlmHelper,
    sb: &Storyboard,
    opts: &IllustrateOptions,
    work_dir: &Path,
    limits: &Limits,
    on_event: &(dyn Fn(String) + Sync),
    key_present: bool,
) -> IllustrationReport {
    let mut report = IllustrationReport {
        model: opts.model.clone(),
        effort: opts.effort.clone(),
        scenes: (0..sb.scenes.len())
            .map(|i| SceneIllustration {
                scene: i,
                skipped: sb.scenes[i].image.is_some(),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    let todo: Vec<usize> = (0..sb.scenes.len())
        .filter(|&i| sb.scenes[i].image.is_none())
        .collect();
    if todo.is_empty() {
        return report;
    }
    let dir = opts.public_dir.join(ILLUSTRATION_DIR);
    if let Err(e) = fs::create_dir_all(&dir)
        .and_then(|_| fs::create_dir_all(work_dir))
        .and_then(|_| match &opts.record_dir {
            Some(d) => fs::create_dir_all(d),
            None => Ok(()),
        })
        .and_then(|_| helper.install_files())
    {
        report.unavailable = Some(format!("cannot prepare the illustration files: {e}"));
        return report;
    }
    // Chrome must work before any money is spent on pictures it could not show.
    if let Err(e) = preflight_chrome(
        &opts.chrome,
        &work_dir.join(format!("chrome-preflight-{}", std::process::id())),
        &limits.with_timeout(RASTER_TIMEOUT),
    ) {
        if e == SvgError::Cancelled {
            report.cancelled = true;
            return report;
        }
        report.unavailable = Some(format!(
            "headless Chrome at {} cannot rasterize SVG: {e}",
            opts.chrome.display()
        ));
        return report;
    }
    let (w, h) = canvas(sb.orientation);
    let canvas_key = format!("{w}x{h}");
    let runner = Runner {
        helper,
        opts,
        work_dir,
        limits,
        seq: AtomicUsize::new(0),
    };

    // ---- style guide
    let mut style_parts: Vec<String> = vec![
        CACHE_VERSION.into(),
        "style".into(),
        opts.model.clone(),
        opts.effort.clone(),
        canvas_key.clone(),
        lang_code(sb.language).into(),
        sb.title.clone(),
    ];
    for i in 0..sb.scenes.len() {
        style_parts.push(sb.scenes[i].text.clone());
        style_parts.push(picture_prompt(sb, i));
    }
    let style_refs: Vec<&str> = style_parts.iter().map(String::as_str).collect();
    let style_file = dir.join(format!("style-{}.json", cache_key(&style_refs)));
    let cached_style: Option<Value> = fs::read_to_string(&style_file)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .filter(Value::is_object);
    let all_cached = |style: &Value| {
        todo.iter().all(|&i| {
            let key = scene_key(opts, style, &picture_prompt(sb, i), &canvas_key);
            dir.join(format!("{key}.png")).is_file() || dir.join(format!("{key}.svg")).is_file()
        })
    };
    let style = if let Some(style) = cached_style {
        report.style_cached = true;
        on_event("illustration style: cached".into());
        style
    } else {
        if !key_present {
            report.unavailable = Some(format!(
                "{ENV_API_KEY} is not set; no illustrations (export it in ~/.zshrc, see README)"
            ));
            return report;
        }
        let mut req = header(sb, opts, "style");
        req["scenes"] = Value::Array(
            (0..sb.scenes.len())
                .map(|i| json!({"text": sb.scenes[i].text, "image_prompt": picture_prompt(sb, i)}))
                .collect(),
        );
        on_event(format!(
            "illustration style: asking {} ...",
            opts.model
        ));
        match runner.call(&req, "illustration-style") {
            Call::Reply(reply) => {
                let usage = reply.usage.unwrap_or_default();
                report.style_usage = usage;
                report.usage.add(&usage);
                report.warnings.extend(reply.warnings);
                match reply.style.filter(Value::is_object) {
                    Some(style) => {
                        if let Err(e) = write_atomic(
                            &style_file,
                            (serde_json::to_string_pretty(&style).unwrap_or_default() + "\n")
                                .as_bytes(),
                        ) {
                            report
                                .warnings
                                .push(format!("cannot cache the style guide: {e}"));
                        }
                        on_event(format!("illustration style: \"{}\" ({usage})",
                            style.get("name").and_then(Value::as_str).unwrap_or("?")));
                        style
                    }
                    None => {
                        report
                            .warnings
                            .push("the style reply had no style; using a built-in style".into());
                        fallback_style()
                    }
                }
            }
            Call::Cancelled => {
                report.cancelled = true;
                return report;
            }
            Call::Failed {
                message,
                fatal,
                usage,
            } => {
                report.usage.add(&usage);
                report.style_usage = usage;
                if fatal {
                    report.unavailable = Some(message);
                    return report;
                }
                report.warnings.push(format!(
                    "the style call failed ({message}); using a built-in style"
                ));
                fallback_style()
            }
        }
    };
    if !key_present && !all_cached(&style) {
        report.unavailable = Some(format!(
            "{ENV_API_KEY} is not set; only cached illustrations are used"
        ));
    }
    report.style = Some(style.clone());

    // ---- scenes
    let next = AtomicUsize::new(0);
    let abort: Mutex<Option<String>> = Mutex::new(None);
    let stop = AtomicBool::new(false);
    let results: Mutex<Vec<SceneIllustration>> = Mutex::new(Vec::new());
    let workers = opts.concurrency.clamp(1, 8).min(todo.len());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                if stop.load(Ordering::SeqCst) || (limits.cancelled)() {
                    return;
                }
                let k = next.fetch_add(1, Ordering::SeqCst);
                let Some(&i) = todo.get(k) else { return };
                let outcome = draw_scene(
                    &runner,
                    sb,
                    i,
                    &style,
                    &dir,
                    &canvas_key,
                    key_present,
                );
                let mut result = outcome.result;
                match outcome.stop {
                    Stop::No => {}
                    Stop::Fatal(message) => {
                        stop.store(true, Ordering::SeqCst);
                        abort.lock().unwrap().get_or_insert(message);
                    }
                    Stop::Cancelled => stop.store(true, Ordering::SeqCst),
                }
                let status = if result.image.is_some() {
                    if result.cached {
                        "cached".to_string()
                    } else {
                        format!("ok, {} B SVG, {}", result.svg_bytes, result.usage)
                    }
                } else {
                    format!(
                        "no picture: {}",
                        result.warnings.last().cloned().unwrap_or_default()
                    )
                };
                on_event(format!(
                    "illustration {}/{}: {status}",
                    i + 1,
                    sb.scenes.len()
                ));
                result.scene = i;
                results.lock().unwrap().push(result);
            });
        }
    });
    if (limits.cancelled)() {
        report.cancelled = true;
    }
    if let Some(message) = abort.into_inner().unwrap() {
        report.unavailable = Some(message);
    }
    for r in results.into_inner().unwrap() {
        report.usage.add(&r.usage);
        let i = r.scene;
        report.scenes[i] = r;
    }
    for &i in &todo {
        let s = &mut report.scenes[i];
        if s.image.is_none() && s.warnings.is_empty() {
            s.warnings.push(if report.cancelled {
                "cancelled".into()
            } else {
                "not drawn: the API became unavailable".into()
            });
        }
    }
    report
}

fn scene_key(opts: &IllustrateOptions, style: &Value, prompt: &str, canvas_key: &str) -> String {
    cache_key(&[
        CACHE_VERSION,
        "scene",
        &opts.model,
        &opts.effort,
        &canonical(style),
        prompt,
        canvas_key,
    ])
}

enum Stop {
    No,
    Fatal(String),
    Cancelled,
}

struct Outcome {
    result: SceneIllustration,
    stop: Stop,
}

#[allow(clippy::too_many_arguments)]
fn draw_scene(
    runner: &Runner<'_>,
    sb: &Storyboard,
    i: usize,
    style: &Value,
    dir: &Path,
    canvas_key: &str,
    key_present: bool,
) -> Outcome {
    let opts = runner.opts;
    let (w, h) = canvas(sb.orientation);
    let prompt = picture_prompt(sb, i);
    let key = scene_key(opts, style, &prompt, canvas_key);
    let svg_path = dir.join(format!("{key}.svg"));
    let png_path = dir.join(format!("{key}.png"));
    let rel_png = format!("{ILLUSTRATION_DIR}/{key}.png");
    let rel_svg = format!("{ILLUSTRATION_DIR}/{key}.svg");
    let mut result = SceneIllustration {
        scene: i,
        ..Default::default()
    };
    let done = |mut result: SceneIllustration, stop: Stop| {
        if result.image.is_none() && result.warnings.is_empty() {
            result.warnings.push("no picture".into());
        }
        Outcome { result, stop }
    };
    let profile = runner
        .work_dir
        .join(format!("chrome-{}-{i}", std::process::id()));
    let raster_limits = runner.limits.with_timeout(RASTER_TIMEOUT);

    // Cache: a PNG that passes the check, or a sanitized SVG that only needs rasterizing.
    if png_path.is_file() && vimmer_media::check_png(&png_path, w, h).is_ok() {
        result.cached = true;
        result.image = Some(rel_png);
        result.svg_bytes = fs::metadata(&svg_path).map_or(0, |m| m.len() as usize);
        result.svg = svg_path.is_file().then_some(rel_svg);
        return done(result, Stop::No);
    }
    if let Ok(text) = fs::read_to_string(&svg_path) {
        // Sanitize again: the file could have been edited by hand.
        if let Ok(clean) = sanitize_svg(&text, w, h) {
            if write_atomic(&svg_path, clean.svg.as_bytes()).is_ok()
                && raster(&opts.chrome, &svg_path, &png_path, w, h, &profile, &raster_limits)
                    .is_ok()
            {
                result.cached = true;
                result.image = Some(rel_png);
                result.svg_bytes = clean.svg.len();
                result.svg = Some(rel_svg);
                return done(result, Stop::No);
            }
        }
    }
    if !key_present {
        result
            .warnings
            .push(format!("{ENV_API_KEY} is not set and the picture is not cached"));
        return done(result, Stop::No);
    }

    let mut history: Vec<Value> = Vec::new();
    for attempt in 1..=MAX_ATTEMPTS {
        let mut req = header(sb, opts, "scene");
        req["style"] = style.clone();
        req["scene"] = json!({"text": sb.scenes[i].text, "image_prompt": prompt});
        req["history"] = Value::Array(history.clone());
        let reply = match runner.call(&req, &format!("illustration-{:02}-{attempt}", i + 1)) {
            Call::Reply(r) => r,
            Call::Cancelled => {
                result.warnings.push("cancelled".into());
                return done(result, Stop::Cancelled);
            }
            Call::Failed {
                message,
                fatal,
                usage,
            } => {
                result.calls += 1;
                result.usage.add(&usage);
                result.warnings.push(message.clone());
                return done(
                    result,
                    if fatal { Stop::Fatal(message) } else { Stop::No },
                );
            }
        };
        result.calls += 1;
        result.usage.add(&reply.usage.unwrap_or_default());
        for w in reply.warnings {
            result.warnings.push(w);
        }
        let raw = reply.svg.unwrap_or_default();
        let problem = match sanitize_svg(&raw, w, h) {
            Err(e) => e.to_string(),
            Ok(clean) => {
                if let Err(e) = write_atomic(&svg_path, clean.svg.as_bytes()) {
                    result.warnings.push(format!("cannot write the SVG: {e}"));
                    return done(result, Stop::No);
                }
                match raster(&opts.chrome, &svg_path, &png_path, w, h, &profile, &raster_limits) {
                    Ok(()) => {
                        result.image = Some(rel_png);
                        result.svg = Some(rel_svg);
                        result.svg_bytes = clean.svg.len();
                        result.removed = clean.removed;
                        // Earlier attempts' complaints are history now.
                        result.warnings.retain(|w| !w.starts_with("attempt "));
                        return done(result, Stop::No);
                    }
                    Err(SvgError::Cancelled) => {
                        let _ = fs::remove_file(&svg_path);
                        result.warnings.push("cancelled".into());
                        return done(result, Stop::Cancelled);
                    }
                    Err(e @ SvgError::Raster(_)) if e.to_string().contains("flat colour") => {
                        let _ = fs::remove_file(&svg_path);
                        "the picture renders as one flat colour; draw visible shapes".to_string()
                    }
                    Err(e) => {
                        // Keep the paid-for, sanitized SVG: a later run only rasterizes it.
                        result.svg = Some(rel_svg);
                        result.svg_bytes = clean.svg.len();
                        result.warnings.push(format!(
                            "{e} (the SVG is kept; run again to retry without an API call)"
                        ));
                        return done(result, Stop::No);
                    }
                }
            }
        };
        result
            .warnings
            .push(format!("attempt {attempt}: SVG rejected: {problem}"));
        history.push(json!({"assistant": truncate(&raw, 60_000), "feedback": problem}));
    }
    result
        .warnings
        .push(format!("SVG still invalid after {MAX_ATTEMPTS} attempts; scene stays text only"));
    done(result, Stop::No)
}

/// [`rasterize_svg`], tried a second time when it times out (a busy machine, e.g. while
/// another video renders, can make one Chrome run very slow).
fn raster(
    chrome: &Path,
    svg: &Path,
    png: &Path,
    w: u32,
    h: u32,
    profile: &Path,
    limits: &Limits,
) -> Result<(), SvgError> {
    match rasterize_svg(chrome, svg, png, w, h, profile, limits) {
        Err(SvgError::TimedOut { .. }) => rasterize_svg(chrome, svg, png, w, h, profile, limits),
        other => other,
    }
}

/// At most `max` bytes of `s`, cut at a character boundary.
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}
