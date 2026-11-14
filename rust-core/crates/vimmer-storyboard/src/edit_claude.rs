//! Natural-language timeline edits: Claude writes a JSON Patch, [`crate::edit`] checks it.
//!
//! The helper (`scripts/edit_timeline.py`, embedded like the storyboard helper) makes one
//! Messages API call per run with structured output:
//!
//! ```json
//! {"status": "edited" | "clarify" | "refused", "explanation": "...",
//!  "patch": [{"op": "replace", "path": "/scenes/1/duration_ms", "from": "", "value_json": "3000"}]}
//! ```
//!
//! Each reply goes through [`apply_edit`]; a rejection (with the failing op and the reason) is
//! sent back as the next turn of the same conversation, up to `max_retries` times. Claude only
//! ever changes JSON: nothing here renders, and the input timeline is never written.
//!
//! The API key stays in the environment of the helper process (see [`crate::generate`]).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use vimmer_media::{run_limited, Limits, MediaError};

use crate::edit::{apply_edit, parse_patch, patch_to_json, EditCheck, EditOutcome};
use crate::generate::{
    api_key_present, tail, Attempt, FailureKind, GenerateFailure, LlmHelper, Usage, ENV_API_KEY,
};

pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
/// Edits are small; `low` keeps them quick. Raise it with `--effort` for tricky requests.
pub const DEFAULT_EFFORT: &str = "low";
pub const DEFAULT_MAX_RETRIES: u32 = 2;
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);
/// Longest instruction accepted, in characters.
pub const MAX_INSTRUCTION_CHARS: usize = 2_000;
/// Most asset file names listed for Claude.
pub const MAX_LISTED_ASSETS: usize = 200;

/// What Claude decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditStatus {
    /// The patch makes the change.
    Edited,
    /// The instruction is too vague; the explanation asks a question. No patch.
    Clarify,
    /// The instruction cannot or must not be done; the explanation says why. No patch.
    Refused,
}

/// What to ask for.
#[derive(Debug, Clone)]
pub struct EditOptions {
    pub instruction: String,
    /// The scene the user has selected (an id), as context.
    pub scene: Option<String>,
    pub model: String,
    pub effort: String,
    pub max_tokens: Option<u32>,
    pub max_retries: u32,
    pub timeout: Duration,
    /// Renderer public dir: asset files are listed for Claude and new paths checked against it.
    pub public_dir: Option<PathBuf>,
    /// When set, each raw API reply is saved here as `reply-<attempt>.json`.
    pub record_dir: Option<PathBuf>,
}

impl EditOptions {
    pub fn new(instruction: impl Into<String>) -> Self {
        Self {
            instruction: instruction.into(),
            scene: None,
            model: DEFAULT_MODEL.into(),
            effort: DEFAULT_EFFORT.into(),
            max_tokens: None,
            max_retries: DEFAULT_MAX_RETRIES,
            timeout: DEFAULT_TIMEOUT,
            public_dir: None,
            record_dir: None,
        }
    }
}

/// A proposal that passed every check. Nothing has been written.
#[derive(Debug, Clone, Serialize)]
pub struct Proposal {
    pub status: EditStatus,
    pub explanation: String,
    /// The patch in RFC 6902 form (`value`, not `value_json`). Empty unless `status` is edited.
    pub patch: Value,
    /// The patched timeline, the change summary and warnings.
    pub outcome: EditOutcome,
    pub attempts: Vec<Attempt>,
    pub usage: Usage,
}

impl Proposal {
    pub fn retries(&self) -> u32 {
        self.attempts.len().saturating_sub(1) as u32
    }
}

#[derive(Serialize)]
struct HelperRequest<'a> {
    timeline: &'a Value,
    instruction: &'a str,
    scene_id: Option<&'a str>,
    assets: &'a [String],
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
    reply_json: Option<String>,
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClaudeEdit {
    status: EditStatus,
    explanation: String,
    patch: Vec<Value>,
}

/// Checks one reply from Claude against the timeline. The error is the feedback for a retry.
pub fn check_reply(
    raw: &str,
    timeline: &Value,
    check: &EditCheck,
) -> Result<(EditStatus, String, Value, EditOutcome), String> {
    let reply: ClaudeEdit = serde_json::from_str(raw)
        .map_err(|e| format!("the reply is not valid JSON in the required shape: {e}"))?;
    let explanation = reply.explanation.trim().to_string();
    if explanation.is_empty() {
        return Err("`explanation` is empty; say what you changed (or why not)".into());
    }
    match reply.status {
        EditStatus::Edited => {
            if reply.patch.is_empty() {
                return Err(
                    "status is `edited` but the patch is empty; give the operations, or use `clarify` / `refused`"
                        .into(),
                );
            }
        }
        EditStatus::Clarify | EditStatus::Refused => {
            if !reply.patch.is_empty() {
                return Err(format!(
                    "status is `{}`, so the patch must be empty; either make no change or use `edited`",
                    if reply.status == EditStatus::Clarify { "clarify" } else { "refused" }
                ));
            }
        }
    }
    let ops = parse_patch(&Value::Array(reply.patch)).map_err(|e| e.to_string())?;
    let outcome = apply_edit(timeline, &patch_to_json(&ops), check).map_err(|e| e.to_string())?;
    if reply.status == EditStatus::Edited && !outcome.changed {
        return Err(
            "the patch changes nothing (the values are already like that); if nothing needs to change, use `clarify` and say so"
                .into(),
        );
    }
    let patch = outcome.patch.clone();
    Ok((reply.status, explanation, patch, outcome))
}

/// Media files under the public dir, relative and sorted, for Claude to choose from.
pub fn list_assets(public_dir: &Path) -> Vec<String> {
    const EXT: [&str; 19] = [
        "png", "jpg", "jpeg", "webp", "gif", "svg", "mp3", "wav", "m4a", "aac", "ogg", "mp4",
        "mov", "m4v", "webm", "ttf", "otf", "woff", "woff2",
    ];
    // Normalized clips and their sound, written by the renderer: not assets to choose.
    let skip = public_dir.join("generated").join("videos");
    let mut out = Vec::new();
    let mut stack = vec![(public_dir.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() && depth < 4 && path != skip {
                stack.push((path, depth + 1));
            } else if ft.is_file() {
                let ext = path
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                if EXT.contains(&ext.as_str()) {
                    if let Ok(rel) = path.strip_prefix(public_dir) {
                        out.push(rel.to_string_lossy().replace('\\', "/"));
                    }
                }
            }
        }
    }
    out.sort();
    out.truncate(MAX_LISTED_ASSETS);
    out
}

/// Asks Claude for an edit, checking `ANTHROPIC_API_KEY` first.
pub fn propose_edit(
    helper: &LlmHelper,
    timeline: &Value,
    opts: &EditOptions,
    work_dir: &Path,
    limits: &Limits,
) -> Result<Proposal, GenerateFailure> {
    propose_edit_with_key_check(helper, timeline, opts, work_dir, limits, api_key_present())
}

/// [`propose_edit`] with the key check's outcome passed in (for tests).
pub fn propose_edit_with_key_check(
    helper: &LlmHelper,
    timeline: &Value,
    opts: &EditOptions,
    work_dir: &Path,
    limits: &Limits,
    key_present: bool,
) -> Result<Proposal, GenerateFailure> {
    let mut attempts: Vec<Attempt> = Vec::new();
    let mut usage = Usage::default();
    let fail =
        |kind: FailureKind, message: String, attempts: &[Attempt], usage: Usage| GenerateFailure {
            kind,
            message,
            attempts: attempts.to_vec(),
            usage,
        };
    let check = EditCheck {
        public_dir: opts.public_dir.clone(),
    };
    // Problems with the input are reported before any call is made.
    let input_problem = if opts.instruction.trim().is_empty() {
        Some("the instruction is empty".to_string())
    } else if opts.instruction.chars().count() > MAX_INSTRUCTION_CHARS {
        Some(format!(
            "the instruction is longer than {MAX_INSTRUCTION_CHARS} characters"
        ))
    } else if let Err(e) = apply_edit(timeline, &Value::Array(vec![]), &EditCheck::default()) {
        e.problems
            .iter()
            .find(|p| p.message.contains("not a valid timeline"))
            .map(|p| p.message.clone())
    } else {
        None
    };
    if let Some(message) = input_problem {
        return Err(fail(FailureKind::Helper, message, &attempts, usage));
    }
    if let Some(id) = &opts.scene {
        let known = timeline["scenes"]
            .as_array()
            .is_some_and(|s| s.iter().any(|s| s["id"].as_str() == Some(id.as_str())));
        if !known {
            return Err(fail(
                FailureKind::Helper,
                format!("scene `{id}` is not in the timeline"),
                &attempts,
                usage,
            ));
        }
    }
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
    let assets = opts
        .public_dir
        .as_deref()
        .map(list_assets)
        .unwrap_or_default();

    let mut history: Vec<Turn> = Vec::new();
    let call_limits = limits.with_timeout(opts.timeout);
    for number in 1..=opts.max_retries + 1 {
        let request = HelperRequest {
            timeline,
            instruction: opts.instruction.trim(),
            scene_id: opts.scene.as_deref(),
            assets: &assets,
            model: &opts.model,
            effort: &opts.effort,
            max_tokens: opts.max_tokens,
            timeout_secs: opts.timeout.as_secs().max(1),
            history: &history,
        };
        let request_file =
            work_dir.join(format!("edit-request-{}-{number}.json", std::process::id()));
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
            helper.edit_command(&request_file, record.as_deref()),
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
            Err(MediaError::TimedOut { secs }) => {
                return Err(fail(
                    FailureKind::TimedOut,
                    format!("Claude did not answer within {secs} s (attempt {number})"),
                    &attempts,
                    usage,
                ))
            }
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
            let kind = match kind.as_str() {
                "no_api_key" => FailureKind::NoApiKey,
                "sdk_missing" | "usage" | "input" => FailureKind::Helper,
                _ => FailureKind::Api(kind),
            };
            return Err(fail(kind, message, &attempts, usage));
        }
        let raw = reply.reply_json.unwrap_or_default();
        match check_reply(&raw, timeline, &check) {
            Ok((status, explanation, patch, outcome)) => {
                attempts.push(attempt);
                return Ok(Proposal {
                    status,
                    explanation,
                    patch,
                    outcome,
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
            "Claude's edit still failed the checks after {} attempt(s): {last}",
            attempts.len()
        ),
        &attempts,
        usage,
    ))
}
