//! Source text -> storyboard, written by Claude through a Python helper.
//!
//! The helper (`scripts/generate_storyboard.py`, embedded in this crate together with the prompt
//! template and the JSON schema) makes one Messages API call per run with structured output, so
//! the reply is always JSON in the storyboard shape. Every reply is then checked here with
//! [`convert`]; when that fails, the error goes back to Claude as the next turn of the same
//! conversation, up to `max_retries` times. Token usage and cost add up over all attempts.
//!
//! The API key is read by the helper from `ANTHROPIC_API_KEY` (inherited environment). This
//! module only checks that the variable is set; it never reads, prints or stores the value.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use vimmer_media::transcribe::find_executable;
use vimmer_media::{run_limited, Limits, MediaError};

use crate::convert::{convert, Conversion};
use crate::model::{Language, Orientation, Storyboard};

/// Environment variable holding the Anthropic API key (read by the helper only).
pub const ENV_API_KEY: &str = "ANTHROPIC_API_KEY";
/// Python interpreter that has the `anthropic` SDK installed.
pub const ENV_LLM_PYTHON: &str = "VIMMER_LLM_PYTHON";
/// Default interpreter: the venv made by `scripts/install_llm.sh`.
pub const DEFAULT_LLM_PYTHON_IN_HOME: &str = ".local/share/vimmer/llm-venv/bin/python";
pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
pub const DEFAULT_EFFORT: &str = "medium";
pub const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
/// Corrections asked for after the first reply fails validation.
pub const DEFAULT_MAX_RETRIES: u32 = 2;
/// Time budget for one helper run (one API call, including the SDK's own retries).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(900);

pub const HELPER_SCRIPT: &str = include_str!("../scripts/generate_storyboard.py");
/// Illustration helper (style guide and per-scene SVG); imports the storyboard helper.
pub const ILLUSTRATE_SCRIPT: &str = include_str!("../scripts/illustrate.py");
pub const PROMPT_TEMPLATE: &str = include_str!("../prompts/storyboard.md");
/// Timeline editing helper (JSON Patch from an instruction); imports the storyboard helper.
pub const EDIT_SCRIPT: &str = include_str!("../scripts/edit_timeline.py");
/// System prompt for timeline editing, with the field reference.
pub const EDIT_PROMPT: &str = include_str!("../prompts/edit.md");
/// JSON schema of the reply: the fields of [`Storyboard`] / `SceneSpec` minus the asset paths
/// (`music`, `voice`, `image`), which this flow never fills.
pub const STORYBOARD_SCHEMA: &str = include_str!("../schema/storyboard.schema.json");

/// What to generate.
#[derive(Debug, Clone)]
pub struct GenerateOptions {
    pub text: String,
    pub language: Language,
    pub orientation: Orientation,
    pub target_seconds: Option<u32>,
    pub model: String,
    pub effort: String,
    /// Output token cap per call; `None`: the helper's default.
    pub max_tokens: Option<u32>,
    pub max_retries: u32,
    pub timeout: Duration,
    /// When set, each raw API reply is saved here as `reply-<attempt>.json`.
    pub record_dir: Option<PathBuf>,
}

impl GenerateOptions {
    pub fn new(text: impl Into<String>, language: Language) -> Self {
        Self {
            text: text.into(),
            language,
            orientation: Orientation::Landscape,
            target_seconds: None,
            model: DEFAULT_MODEL.into(),
            effort: DEFAULT_EFFORT.into(),
            max_tokens: None,
            max_retries: DEFAULT_MAX_RETRIES,
            timeout: DEFAULT_TIMEOUT,
            record_dir: None,
        }
    }
}

/// Tokens and estimated cost (USD, from the helper's price table).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub cost_usd: f64,
}

impl Usage {
    pub fn add(&mut self, other: &Usage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_creation_input_tokens += other.cache_creation_input_tokens;
        self.cache_read_input_tokens += other.cache_read_input_tokens;
        self.cost_usd += other.cost_usd;
    }
}

impl fmt::Display for Usage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} input + {} output tokens, ${:.4}",
            self.input_tokens + self.cache_creation_input_tokens + self.cache_read_input_tokens,
            self.output_tokens,
            self.cost_usd
        )
    }
}

/// One API call and what became of its reply.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Attempt {
    /// 1-based.
    pub attempt: u32,
    /// The model that produced the reply (differs from the requested one after a fallback).
    pub model: Option<String>,
    pub stop_reason: Option<String>,
    pub usage: Usage,
    /// `None` when the reply passed validation; else the validation or API error.
    pub error: Option<String>,
    pub warnings: Vec<String>,
}

/// A storyboard that passed [`convert`].
#[derive(Debug, Clone)]
pub struct Generated {
    pub storyboard: Storyboard,
    /// The reply exactly as Claude wrote it.
    pub raw_json: String,
    pub conversion: Conversion,
    pub attempts: Vec<Attempt>,
    pub usage: Usage,
}

impl Generated {
    /// Validation retries that were needed (0: the first reply passed).
    pub fn retries(&self) -> u32 {
        self.attempts.len().saturating_sub(1) as u32
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FailureKind {
    /// `ANTHROPIC_API_KEY` is not set.
    NoApiKey,
    /// The helper or its Python could not run, or printed no result.
    Helper,
    /// The API call failed; the helper's error kind (`auth`, `refusal`, `max_tokens`, ...).
    Api(String),
    /// Every reply failed validation.
    Invalid,
    TimedOut,
    Cancelled,
}

/// Generation failed. `usage` still counts every call that was made.
#[derive(Debug, Clone)]
pub struct GenerateFailure {
    pub kind: FailureKind,
    pub message: String,
    pub attempts: Vec<Attempt>,
    pub usage: Usage,
}

impl fmt::Display for GenerateFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for GenerateFailure {}

/// True when `ANTHROPIC_API_KEY` is set to something other than blanks. The value is not kept.
pub fn api_key_present() -> bool {
    std::env::var_os(ENV_API_KEY).is_some_and(|v| !v.to_string_lossy().trim().is_empty())
}

/// The Python helper and where its files are written.
#[derive(Debug, Clone)]
pub struct LlmHelper {
    pub python: PathBuf,
    /// Holds the helper script, prompt template and schema, written from the embedded copies.
    pub dir: PathBuf,
}

impl LlmHelper {
    /// The interpreter from `python`, else `$VIMMER_LLM_PYTHON`, else the default venv.
    pub fn locate(python: Option<&Path>) -> Result<Self, String> {
        let name = python
            .map(|p| p.to_string_lossy().into_owned())
            .or_else(|| {
                std::env::var(ENV_LLM_PYTHON)
                    .ok()
                    .filter(|s| !s.trim().is_empty())
            })
            .or_else(|| {
                std::env::var_os("HOME").map(|h| {
                    PathBuf::from(h)
                        .join(DEFAULT_LLM_PYTHON_IN_HOME)
                        .to_string_lossy()
                        .into_owned()
                })
            })
            .unwrap_or_else(|| "python3".into());
        let python = find_executable(&name, None).ok_or_else(|| {
            format!(
                "Python `{name}` for Claude generation was not found (run rust-core/crates/vimmer-storyboard/scripts/install_llm.sh, or pass --llm-python / set {ENV_LLM_PYTHON})"
            )
        })?;
        Ok(Self {
            python,
            dir: default_helper_dir(),
        })
    }

    pub fn script_path(&self) -> PathBuf {
        self.dir.join("generate_storyboard.py")
    }
    pub fn illustrate_script_path(&self) -> PathBuf {
        self.dir.join("illustrate.py")
    }
    pub fn prompt_path(&self) -> PathBuf {
        self.dir.join("storyboard.md")
    }
    pub fn schema_path(&self) -> PathBuf {
        self.dir.join("storyboard.schema.json")
    }
    pub fn edit_script_path(&self) -> PathBuf {
        self.dir.join("edit_timeline.py")
    }
    pub fn edit_prompt_path(&self) -> PathBuf {
        self.dir.join("edit.md")
    }

    /// Writes the embedded helper files unless identical copies are already there.
    pub fn install_files(&self) -> std::io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        for (path, body) in [
            (self.script_path(), HELPER_SCRIPT),
            (self.prompt_path(), PROMPT_TEMPLATE),
            (self.schema_path(), STORYBOARD_SCHEMA),
            (self.illustrate_script_path(), ILLUSTRATE_SCRIPT),
            (self.edit_script_path(), EDIT_SCRIPT),
            (self.edit_prompt_path(), EDIT_PROMPT),
        ] {
            if fs::read_to_string(&path).is_ok_and(|s| s == body) {
                continue;
            }
            let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
            fs::write(&tmp, body)?;
            fs::rename(&tmp, &path)?;
        }
        Ok(())
    }

    /// The helper invocation. `-I` keeps the current directory off `sys.path`. The key is not
    /// an argument: the child inherits it from the environment.
    pub fn command(&self, request: &Path, record: Option<&Path>) -> Command {
        let mut cmd = Command::new(&self.python);
        cmd.arg("-I")
            .arg(self.script_path())
            .arg("--request")
            .arg(request)
            .arg("--prompt-file")
            .arg(self.prompt_path())
            .arg("--schema-file")
            .arg(self.schema_path());
        if let Some(path) = record {
            cmd.arg("--record").arg(path);
        }
        cmd
    }

    /// The timeline editing helper invocation (see [`crate::edit_claude`]).
    pub fn edit_command(&self, request: &Path, record: Option<&Path>) -> Command {
        let mut cmd = Command::new(&self.python);
        cmd.arg("-I")
            .arg(self.edit_script_path())
            .arg("--request")
            .arg(request)
            .arg("--prompt-file")
            .arg(self.edit_prompt_path());
        if let Some(path) = record {
            cmd.arg("--record").arg(path);
        }
        cmd
    }

    /// The illustration helper invocation (see [`crate::illustrate`]).
    pub fn illustrate_command(&self, request: &Path, record: Option<&Path>) -> Command {
        let mut cmd = Command::new(&self.python);
        cmd.arg("-I")
            .arg(self.illustrate_script_path())
            .arg("--request")
            .arg(request);
        if let Some(path) = record {
            cmd.arg("--record").arg(path);
        }
        cmd
    }
}

fn default_helper_dir() -> PathBuf {
    let key = format!(
        "{HELPER_SCRIPT}\0{PROMPT_TEMPLATE}\0{STORYBOARD_SCHEMA}\0{ILLUSTRATE_SCRIPT}\0{EDIT_SCRIPT}\0{EDIT_PROMPT}"
    );
    std::env::temp_dir().join(format!("vimmer-llm-{:016x}", fnv1a(&key)))
}

/// 64-bit FNV-1a, for a stable directory name.
pub(crate) fn fnv1a(s: &str) -> u64 {
    fnv1a_from(0xcbf2_9ce4_8422_2325, s)
}

/// FNV-1a with another starting value (two of them make a 128-bit cache key).
pub(crate) fn fnv1a_from(seed: u64, s: &str) -> u64 {
    let mut hash: u64 = seed;
    for byte in s.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

#[derive(Serialize)]
struct HelperRequest<'a> {
    text: &'a str,
    language: Language,
    orientation: Orientation,
    target_seconds: Option<u32>,
    model: &'a str,
    effort: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    timeout_secs: u64,
    history: &'a [Turn],
}

#[derive(Serialize)]
struct Turn {
    assistant: String,
    feedback: String,
}

#[derive(Deserialize)]
struct HelperReply {
    ok: bool,
    #[serde(default)]
    storyboard_json: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    stop_reason: Option<String>,
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

/// Checks a reply: it must parse as a storyboard, match the request and pass [`convert`].
/// The error text is what Claude gets back for a correction.
pub fn validate_reply(
    raw: &str,
    language: Language,
    orientation: Orientation,
) -> Result<(Storyboard, Conversion), String> {
    let sb: Storyboard =
        serde_json::from_str(raw).map_err(|e| format!("invalid storyboard JSON: {e}"))?;
    let mut problems = Vec::new();
    if sb.language != language {
        problems.push(format!(
            "field `language`: must be `{}`",
            lang_code(language)
        ));
    }
    if sb.orientation != orientation {
        problems.push(format!(
            "field `orientation`: must be `{}`",
            orientation_code(orientation)
        ));
    }
    if sb.music.is_some() {
        problems.push("field `music`: no music is available; remove the field".into());
    }
    if sb.voice.is_some() {
        problems.push("field `voice`: do not set a voice; remove the field".into());
    }
    for (i, scene) in sb.scenes.iter().enumerate() {
        if scene.image.is_some() {
            problems.push(format!(
                "scene {}: field `image`: no images are available; remove it and describe the picture in `image_prompt`",
                i + 1
            ));
        }
    }
    if !problems.is_empty() {
        return Err(problems.join("\n"));
    }
    let conversion = convert(&sb).map_err(|e| e.to_string())?;
    Ok((sb, conversion))
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

/// Generates a storyboard, checking `ANTHROPIC_API_KEY` first. `work_dir` holds the request
/// files while the helper runs. Each helper run is limited to `opts.timeout` and stops when
/// `limits` report a cancel.
pub fn generate(
    helper: &LlmHelper,
    opts: &GenerateOptions,
    work_dir: &Path,
    limits: &Limits,
) -> Result<Generated, GenerateFailure> {
    generate_with_key_check(helper, opts, work_dir, limits, api_key_present())
}

/// [`generate`] with the key check's outcome passed in (tests must not depend on the real
/// environment).
pub fn generate_with_key_check(
    helper: &LlmHelper,
    opts: &GenerateOptions,
    work_dir: &Path,
    limits: &Limits,
    key_present: bool,
) -> Result<Generated, GenerateFailure> {
    let mut attempts: Vec<Attempt> = Vec::new();
    let mut usage = Usage::default();
    let fail =
        |kind: FailureKind, message: String, attempts: &[Attempt], usage: Usage| GenerateFailure {
            kind,
            message,
            attempts: attempts.to_vec(),
            usage,
        };
    if !key_present {
        return Err(fail(
            FailureKind::NoApiKey,
            format!(
                "{ENV_API_KEY} is not set; export it in ~/.zshrc and open a new shell (see README)"
            ),
            &attempts,
            usage,
        ));
    }
    if opts.text.trim().is_empty() {
        return Err(fail(
            FailureKind::Helper,
            "the source text is empty".into(),
            &attempts,
            usage,
        ));
    }
    let setup = |e: std::io::Error| {
        fail(
            FailureKind::Helper,
            format!("cannot prepare the Claude helper: {e}"),
            &[],
            Usage::default(),
        )
    };
    helper.install_files().map_err(setup)?;
    fs::create_dir_all(work_dir).map_err(setup)?;
    if let Some(dir) = &opts.record_dir {
        fs::create_dir_all(dir).map_err(setup)?;
    }

    let mut history: Vec<Turn> = Vec::new();
    let call_limits = limits.with_timeout(opts.timeout);
    for number in 1..=opts.max_retries + 1 {
        let request = HelperRequest {
            text: &opts.text,
            language: opts.language,
            orientation: opts.orientation,
            target_seconds: opts.target_seconds,
            model: &opts.model,
            effort: &opts.effort,
            max_tokens: opts.max_tokens,
            timeout_secs: opts.timeout.as_secs().max(1),
            history: &history,
        };
        let request_file =
            work_dir.join(format!("llm-request-{}-{number}.json", std::process::id()));
        let body = serde_json::to_string(&request).expect("request serializes");
        fs::write(&request_file, body).map_err(|e| {
            fail(
                FailureKind::Helper,
                format!("cannot write {}: {e}", request_file.display()),
                &attempts,
                usage,
            )
        })?;
        let record = opts
            .record_dir
            .as_ref()
            .map(|d| d.join(format!("reply-{number}.json")));
        let output = run_limited(
            helper.command(&request_file, record.as_deref()),
            &call_limits,
        );
        let _ = fs::remove_file(&request_file);
        let output = match output {
            Ok(o) => o,
            Err(MediaError::Cancelled) => {
                return Err(fail(
                    FailureKind::Cancelled,
                    format!(
                        "cancelled during attempt {number}; tokens of that call are not counted"
                    ),
                    &attempts,
                    usage,
                ))
            }
            Err(MediaError::TimedOut { secs }) => return Err(fail(
                FailureKind::TimedOut,
                format!(
                    "Claude did not answer within {secs} s (attempt {number}; raise --llm-timeout)"
                ),
                &attempts,
                usage,
            )),
            Err(e) => {
                return Err(fail(
                    FailureKind::Helper,
                    format!("cannot run {}: {e}", helper.python.display()),
                    &attempts,
                    usage,
                ))
            }
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        let reply: Option<HelperReply> = stdout
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .and_then(|l| serde_json::from_str(l).ok());
        let Some(reply) = reply else {
            return Err(fail(
                FailureKind::Helper,
                format!(
                    "the Claude helper exited with {} and printed no result: {}",
                    output.status,
                    tail(&String::from_utf8_lossy(&output.stderr), 3)
                ),
                &attempts,
                usage,
            ));
        };
        let call_usage = reply.usage.unwrap_or_default();
        usage.add(&call_usage);
        let mut attempt = Attempt {
            attempt: number,
            model: reply.model,
            stop_reason: reply.stop_reason,
            usage: call_usage,
            error: None,
            warnings: reply.warnings,
        };
        if !reply.ok {
            let (kind, message) = reply.error.map_or_else(
                || {
                    (
                        "unknown".to_string(),
                        "the Claude helper reported a failure".to_string(),
                    )
                },
                |e| (e.kind, e.message),
            );
            attempt.error = Some(message.clone());
            attempts.push(attempt);
            let kind = if kind == "no_api_key" {
                FailureKind::NoApiKey
            } else if kind == "sdk_missing" || kind == "usage" || kind == "input" {
                FailureKind::Helper
            } else {
                FailureKind::Api(kind)
            };
            return Err(fail(kind, message, &attempts, usage));
        }
        let raw = reply.storyboard_json.unwrap_or_default();
        match validate_reply(&raw, opts.language, opts.orientation) {
            Ok((storyboard, conversion)) => {
                attempts.push(attempt);
                return Ok(Generated {
                    storyboard,
                    raw_json: raw,
                    conversion,
                    attempts,
                    usage,
                });
            }
            Err(problem) => {
                attempt.error = Some(problem.clone());
                attempts.push(attempt);
                history.push(Turn {
                    assistant: raw,
                    feedback: problem,
                });
            }
        }
    }
    let last = attempts
        .last()
        .and_then(|a| a.error.clone())
        .unwrap_or_default();
    Err(fail(
        FailureKind::Invalid,
        format!(
            "Claude's storyboard still failed validation after {} attempt(s): {last}",
            attempts.len()
        ),
        &attempts,
        usage,
    ))
}

/// The last `n` non-empty lines of a child's stderr.
pub(crate) fn tail(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join(" | ")
}
