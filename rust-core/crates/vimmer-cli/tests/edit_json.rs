//! `vimmer edit --dry-run --json` prints exactly `edit_claude::proposal_json` /
//! `failure_json` of the library call. The desktop assistant returns the same objects, so the
//! two stay identical. A stand-in Python replays a prepared reply: no network, no real key.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Value};
use vimmer_media::Limits;
use vimmer_storyboard::edit_claude::{
    failure_json, proposal_json, propose_edit_with_key_check, EditOptions,
};
use vimmer_storyboard::generate::{LlmHelper, ENV_API_KEY};

const TIMELINE: &str = r#"{"fps": 30, "resolution": {"width": 1920, "height": 1080},
  "scenes": [{"id": "scene-01", "duration_ms": 15000, "text": "Turn any idea into a video"}]}"#;

fn fixture(name: &str, reply: &Value) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "vimmer-cli-edit-json-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("public")).unwrap();
    let python = dir.join("python");
    fs::write(&python, "#!/bin/sh\ncat \"$(dirname \"$0\")/reply.json\"\n").unwrap();
    fs::set_permissions(&python, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(dir.join("reply.json"), reply.to_string() + "\n").unwrap();
    dir
}

fn cli(dir: &Path) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_vimmer"))
        .args([
            "edit",
            "--dry-run",
            "--json",
            "--model",
            "claude-sonnet-5-5",
            "--scene",
            "scene-01",
        ])
        .arg("--public-dir")
        .arg(dir.join("public"))
        .arg("--llm-python")
        .arg(dir.join("python"))
        .args(["--", "-", "make scene 1 slower"])
        .env(ENV_API_KEY, "sk-fake-test-value-000")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(TIMELINE.as_bytes())
        .unwrap();
    String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap()
}

fn library(dir: &Path) -> Value {
    let mut helper = LlmHelper::locate(Some(&dir.join("python"))).unwrap();
    helper.dir = dir.join("files");
    let mut opts = EditOptions::new("make scene 1 slower");
    opts.scene = Some("scene-01".into());
    opts.model = "claude-sonnet-5-5".into();
    opts.public_dir = Some(dir.join("public"));
    let timeline: Value = serde_json::from_str(TIMELINE).unwrap();
    match propose_edit_with_key_check(
        &helper,
        &timeline,
        &opts,
        &dir.join("work"),
        &Limits::none(),
        true,
    ) {
        Ok(p) => proposal_json(&p, None),
        Err(f) => failure_json(&f),
    }
}

#[test]
fn the_cli_prints_the_library_json() {
    let usage = json!({"input_tokens": 1000, "output_tokens": 200, "cache_creation_input_tokens": 0,
                       "cache_read_input_tokens": 0, "cost_usd": 0.004});
    let edit = json!({"status": "edited", "explanation": "Slower.",
        "patch": [{"op": "replace", "path": "/scenes/0/duration_ms", "from": "", "value_json": "20000"}]});
    let replies = [
        (
            "edited",
            json!({"ok": true, "reply_json": edit.to_string(), "model": "claude-sonnet-5-5",
                          "stop_reason": "end_turn", "usage": usage, "warnings": ["a note"]}),
        ),
        (
            "failed",
            json!({"ok": false, "error": {"kind": "rate_limit", "message": "rate limited (429)"}, "usage": null}),
        ),
    ];
    for (name, reply) in replies {
        let dir = fixture(name, &reply);
        let printed = cli(&dir);
        assert_eq!(
            printed,
            serde_json::to_string(&library(&dir)).unwrap() + "\n",
            "{name}"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
