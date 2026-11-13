//! The editing assistant in the desktop app: runs `vimmer edit - <instruction> --dry-run --json`
//! with the editor's timeline on stdin and hands its JSON to the webview. Nothing is written;
//! the editor applies an accepted proposal itself, as one undo step.
//!
//! A GUI app does not inherit the shell environment, so the CLI is started through
//! `zsh -c 'source ~/.zshrc; exec "$@"'`: ANTHROPIC_API_KEY reaches only that child process,
//! never this process's own state or the webview.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};

const MODELS: [&str; 3] = ["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"];
const MAX_INSTRUCTION_CHARS: usize = 2_000;
const TIMEOUT: Duration = Duration::from_secs(360);

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

/// The CLI arguments: options first, then `--` so an instruction that starts with `-` stays text.
pub fn cli_args(a: &ProposeArgs, public_dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = vec![
        "edit".into(),
        "--dry-run".into(),
        "--json".into(),
        "--public-dir".into(),
        public_dir.display().to_string(),
        "--model".into(),
        a.model.clone(),
    ];
    if let Some(id) = &a.scene_id {
        out.push("--scene".into());
        out.push(id.clone());
    }
    out.push("--".into());
    out.push("-".into());
    out.push(a.instruction.clone());
    out
}

fn repo_root() -> PathBuf {
    let here = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    here.canonicalize().unwrap_or(here)
}

/// `$VIMMER_BIN`, else the release or debug build in rust-core/target.
fn find_vimmer() -> Option<PathBuf> {
    let root = repo_root();
    std::env::var_os("VIMMER_BIN")
        .map(PathBuf::from)
        .into_iter()
        .chain([
            root.join("rust-core/target/release/vimmer"),
            root.join("rust-core/target/debug/vimmer"),
        ])
        .find(|p| p.is_file())
}

fn run_cli(
    bin: &Path,
    args: &[String],
    stdin: &str,
) -> Result<(Option<i32>, String, String), String> {
    let mut cmd = if Path::new("/bin/zsh").is_file() {
        let mut c = Command::new("/bin/zsh");
        c.arg("-c")
            .arg(r#"source ~/.zshrc >/dev/null 2>&1; exec "$@""#)
            .arg("vimmer-edit")
            .arg(bin)
            .args(args);
        c
    } else {
        let mut c = Command::new(bin);
        c.args(args);
        c
    };
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not start {}: {e}", bin.display()))?;
    if let Some(mut input) = child.stdin.take() {
        input
            .write_all(stdin.as_bytes())
            .map_err(|e| e.to_string())?;
    }
    // Read output on threads so a full pipe cannot block the child while we wait.
    let mut out = child.stdout.take().ok_or("no stdout")?;
    let mut err = child.stderr.take().ok_or("no stderr")?;
    let out_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut out, &mut s);
        s
    });
    let err_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut err, &mut s);
        s
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if started.elapsed() > TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err("The assistant did not answer in time.".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    Ok((
        status.code(),
        out_t.join().unwrap_or_default(),
        err_t.join().unwrap_or_default(),
    ))
}

/// Proposes an edit. Always resolves to the CLI's JSON shape (`ok` true or false), so the
/// editor shows one kind of result; setup problems come back as `ok: false` with a kind.
#[tauri::command]
pub async fn propose_edit(args: ProposeArgs) -> Value {
    if let Err(e) = check_args(&args) {
        return failure("input", e);
    }
    let Some(bin) = find_vimmer() else {
        return failure(
            "no_binary",
            "The vimmer CLI was not found in rust-core/target.",
        );
    };
    if BUSY.swap(true, Ordering::SeqCst) {
        return failure("busy", "Another proposal is still running.");
    }
    let public_dir = repo_root().join("remotion-renderer/public");
    let cli = cli_args(&args, &public_dir);
    let result =
        tauri::async_runtime::spawn_blocking(move || run_cli(&bin, &cli, &args.timeline_json))
            .await;
    BUSY.store(false, Ordering::SeqCst);
    let (code, stdout, stderr) = match result {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => return failure("helper", e),
        Err(e) => return failure("helper", e.to_string()),
    };
    let last = stdout
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("");
    serde_json::from_str(last).unwrap_or_else(|_| {
        let tail: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).collect();
        let tail = tail[tail.len().saturating_sub(3)..].join(" | ");
        failure(
            "helper",
            format!(
                "vimmer edit failed (exit {code:?}): {}",
                if tail.is_empty() { "no output" } else { &tail }
            ),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(instruction: &str, model: &str) -> ProposeArgs {
        ProposeArgs {
            timeline_json: "{}".into(),
            instruction: instruction.into(),
            scene_id: Some("scene-02".into()),
            model: model.into(),
        }
    }

    #[test]
    fn instruction_goes_after_double_dash() {
        let a = args("--overwrite-input -o /etc/x", "claude-opus-5-5");
        let v = cli_args(&a, Path::new("/p"));
        let dd = v.iter().position(|s| s == "--").unwrap();
        assert_eq!(&v[dd + 1..], ["-", "--overwrite-input -o /etc/x"]);
        assert!(v.contains(&"--dry-run".to_string()));
        assert!(v.windows(2).any(|w| w == ["--scene", "scene-02"]));
    }

    #[test]
    fn arguments_are_checked() {
        assert!(check_args(&args("ok", "claude-opus-5-5")).is_ok());
        assert!(check_args(&args("  ", "claude-opus-5-5")).is_err());
        assert!(check_args(&args("ok", "gpt-x")).is_err());
        assert!(check_args(&args(&"x".repeat(2001), "claude-opus-5-5")).is_err());
    }
}
