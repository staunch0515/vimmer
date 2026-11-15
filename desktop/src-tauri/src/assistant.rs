//! The editing assistant in the desktop app. Runs the edit in this process through
//! `vimmer_storyboard::edit_claude` (the same code as `vimmer edit - <instruction> --dry-run
//! --json`) and hands the same JSON to the webview, so no prebuilt CLI binary is involved and
//! nothing can be stale. Nothing is written; the editor applies an accepted proposal itself, as
//! one undo step.
//!
//! The API key is handled as in "generate from text" ([`crate::api_key`]): inherited from this
//! process, else read from the login shell once, kept in memory and handed to the helper
//! process alone. Every returned string is scrubbed of it.
//!
//! Failures keep the CLI's shape (`ok: false`, `error: {kind, message}`, `attempts`, `usage`)
//! with kinds that name the cause: `input`, `no_api_key`, `helper_missing` (the helper's Python
//! or its SDK is not installed), `helper` (it ran but failed), the helper's API error kind
//! (`auth`, `rate_limit`, `connection`, ...), `invalid`, `timed_out`, `busy`. A `clarify` or
//! `refused` answer is a normal proposal (`ok: true`), not a failure.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};
use vimmer_media::Limits;
use vimmer_storyboard::edit_claude::{
    failure_json, propose_edit_with_key_check, proposal_json, EditOptions, MAX_INSTRUCTION_CHARS,
};
use vimmer_storyboard::generate::{FailureKind, GenerateFailure, LlmHelper, ENV_API_KEY};

use crate::api_key::{helper_key, scrub, HelperKey, NO_KEY_MESSAGE};

const MODELS: [&str; 3] = ["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"];
/// The whole proposal, retries included, may take this long.
const TIMEOUT: Duration = Duration::from_secs(360);
/// One API call (the CLI's `--llm-timeout` default).
const CALL_TIMEOUT: Duration = Duration::from_secs(300);
const INSTALL_HINT: &str = "sh rust-core/crates/vimmer-storyboard/scripts/install_llm.sh";

static BUSY: AtomicBool = AtomicBool::new(false);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposeArgs {
    pub timeline_json: String,
    pub instruction: String,
    pub scene_id: Option<String>,
    pub model: String,
}

fn failure(kind: &str, message: impl Into<String>) -> Value {
    json!({"ok": false, "error": {"kind": kind, "message": message.into()}})
}

/// Checks the arguments; returns the problem in words.
pub fn check_args(a: &ProposeArgs) -> Result<(), String> {
    if a.instruction.trim().is_empty() {
        return Err("The instruction is empty.".into());
    }
    if a.instruction.chars().count() > MAX_INSTRUCTION_CHARS {
        return Err(format!(
            "The instruction is longer than {MAX_INSTRUCTION_CHARS} characters."
        ));
    }
    if !MODELS.contains(&a.model.as_str()) {
        return Err(format!("Unknown model {}.", a.model));
    }
    if a.scene_id.as_ref().is_some_and(|s| s.len() > 64) {
        return Err("The scene id is too long.".into());
    }
    Ok(())
}

/// The options `vimmer edit` would use for these arguments (its defaults otherwise).
fn edit_options(a: &ProposeArgs, public_dir: Option<&Path>) -> EditOptions {
    let mut opts = EditOptions::new(a.instruction.clone());
    opts.scene = a.scene_id.clone();
    opts.model = a.model.clone();
    opts.timeout = CALL_TIMEOUT;
    opts.public_dir = public_dir.map(Path::to_path_buf);
    opts
}

/// The kind and message the editor shows for a failed edit.
fn classify(f: &GenerateFailure) -> (String, String) {
    let m = f.message.trim();
    match &f.kind {
        FailureKind::NoApiKey => ("no_api_key".into(), NO_KEY_MESSAGE.into()),
        // Our helper names install_llm.sh only when its Python or the anthropic SDK is missing.
        FailureKind::Helper if m.contains("install_llm.sh") => (
            "helper_missing".into(),
            format!("The Claude helper is not installed ({m}). Install it: {INSTALL_HINT}"),
        ),
        FailureKind::Helper => ("helper".into(), format!("The Claude helper failed: {m}")),
        FailureKind::Api(k) if k == "auth" => (
            k.clone(),
            format!("Claude rejected the API key: {m}. Check ANTHROPIC_API_KEY in ~/.zshrc."),
        ),
        FailureKind::Api(k) if k == "refusal" => (k.clone(), m.to_string()),
        FailureKind::Api(k) => (k.clone(), format!("The Claude API call failed: {m}")),
        FailureKind::Invalid => ("invalid".into(), m.to_string()),
        FailureKind::TimedOut => ("timed_out".into(), m.to_string()),
        FailureKind::Cancelled => ("cancelled".into(), m.to_string()),
    }
}

/// A failure as the CLI prints it, with the desktop's kind and message.
fn failure_value(f: &GenerateFailure) -> Value {
    let mut v = failure_json(f);
    let (kind, message) = classify(f);
    v["error"] = json!({"kind": kind, "message": message});
    v
}

/// What a proposal needs besides the editor's values. Tests pass a fake helper and key.
pub struct AssistantEnv<'a> {
    /// Renderer public dir (asset files offered to Claude, new paths checked against it).
    pub public_dir: Option<PathBuf>,
    /// Scratch folder for request files; removed afterwards.
    pub work_dir: PathBuf,
    pub limits: Limits,
    pub key: &'a dyn Fn() -> Option<HelperKey>,
    pub helper: &'a dyn Fn() -> Result<LlmHelper, String>,
}

/// Proposes an edit: the same JSON as `vimmer edit --dry-run --json` on success, a failure with
/// a specific kind otherwise. The key is scrubbed from everything returned.
pub fn propose(env: &AssistantEnv, args: &ProposeArgs) -> Value {
    let mut secrets: Vec<String> = std::env::var(ENV_API_KEY).into_iter().collect();
    let result = run(env, args, &mut secrets);
    let mut text = result.to_string();
    for s in &secrets {
        text = scrub(&text, Some(s.trim()));
    }
    serde_json::from_str(&text).unwrap_or(result)
}

fn run(env: &AssistantEnv, args: &ProposeArgs, secrets: &mut Vec<String>) -> Value {
    if let Err(e) = check_args(args) {
        return failure("input", e);
    }
    let timeline: Value = match serde_json::from_str(&args.timeline_json) {
        Ok(v) => v,
        Err(e) => return failure("input", format!("The timeline is not valid JSON: {e}")),
    };
    let opts = edit_options(args, env.public_dir.as_deref());
    // Problems with the timeline or the scene id, found before the key is read. With no key
    // the library stops right after its input checks: nothing is started or written.
    let no_helper = LlmHelper {
        python: PathBuf::from("/nonexistent"),
        dir: env.work_dir.join("unused"),
        env: Default::default(),
    };
    if let Err(f) = propose_edit_with_key_check(&no_helper, &timeline, &opts, &env.work_dir, &env.limits, false) {
        if f.kind == FailureKind::Helper {
            return failure("input", f.message);
        }
    }

    let Some(key) = (env.key)() else {
        return failure("no_api_key", NO_KEY_MESSAGE);
    };
    secrets.extend(key.secret().map(str::to_string));
    let mut helper = match (env.helper)() {
        Ok(h) => h,
        Err(e) => {
            return failure(
                "helper_missing",
                format!("The Claude helper is not installed ({e}). Install it: {INSTALL_HINT}"),
            )
        }
    };
    key.give_to(&mut helper);
    let result = propose_edit_with_key_check(&helper, &timeline, &opts, &env.work_dir, &env.limits, true);
    let _ = std::fs::remove_dir_all(&env.work_dir);
    match result {
        Ok(p) => proposal_json(&p, None),
        Err(f) => failure_value(&f),
    }
}

/// Proposes an edit. Always resolves to the CLI's JSON shape (`ok` true or false), so the
/// editor shows one kind of result. Runs on a worker thread; the UI is never blocked.
#[tauri::command]
pub async fn propose_edit(args: ProposeArgs) -> Value {
    if let Err(e) = check_args(&args) {
        return failure("input", e);
    }
    if BUSY.swap(true, Ordering::SeqCst) {
        return failure("busy", "Another proposal is still running.");
    }
    let result = tauri::async_runtime::spawn_blocking(move || {
        let started = Instant::now();
        let over = Arc::new(move || started.elapsed() > TIMEOUT);
        let check = over.clone();
        let env = AssistantEnv {
            public_dir: Some(crate::assets::public_dir()).filter(|p| p.is_dir()),
            work_dir: std::env::temp_dir()
                .join(format!("vimmer-desktop-edit-{}-{}", std::process::id(), crate::render::unix_millis())),
            limits: Limits { timeout: None, cancelled: Arc::new(move || check()) },
            key: &helper_key,
            helper: &|| LlmHelper::locate(None),
        };
        let v = propose(&env, &args);
        if v["error"]["kind"] == "cancelled" && over() {
            return failure(
                "timed_out",
                format!("The assistant did not answer within {} minutes.", TIMEOUT.as_secs() / 60),
            );
        }
        v
    })
    .await;
    BUSY.store(false, Ordering::SeqCst);
    result.unwrap_or_else(|e| failure("helper", format!("The assistant stopped: {e}")))
}

#[cfg(test)]
#[path = "assistant_tests.rs"]
mod tests;
