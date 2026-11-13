//! Tests for [`crate::edit_claude`] with a stand-in Python that replays prepared helper
//! replies, one per run. No network, no API key.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use serde_json::{json, Value};
use vimmer_media::Limits;

use crate::edit::EditCheck;
use crate::edit_claude::*;
use crate::generate::{FailureKind, GenerateFailure, LlmHelper};

const FAKE_PYTHON: &str = r#"#!/bin/sh
dir=$(dirname "$0")
n=$(cat "$dir/count" 2>/dev/null || echo 0)
n=$((n + 1))
echo "$n" > "$dir/count"
printf '%s\n' "$@" > "$dir/args-$n.txt"
while [ $# -gt 0 ]; do
  if [ "$1" = "--request" ]; then cp "$2" "$dir/request-$n.json"; fi
  shift
done
if [ -f "$dir/reply-$n.json" ]; then cat "$dir/reply-$n.json"; else cat "$dir/reply-last.json"; fi
"#;

struct Fixture {
    dir: PathBuf,
    helper: LlmHelper,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("vimmer-edit-llm-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let python = dir.join("python");
        fs::write(&python, FAKE_PYTHON).unwrap();
        fs::set_permissions(&python, fs::Permissions::from_mode(0o755)).unwrap();
        let mut helper = LlmHelper::locate(Some(&python)).unwrap();
        helper.dir = dir.join("helper");
        Self { dir, helper }
    }

    fn reply(&self, n: Option<u32>, edit: &Value) {
        let name = n.map_or("reply-last.json".to_string(), |n| format!("reply-{n}.json"));
        let body = json!({"ok": true, "reply_json": edit.to_string(), "model": "claude-sonnet-5-5",
            "stop_reason": "end_turn", "warnings": [],
            "usage": {"input_tokens": 100, "output_tokens": 50, "cache_creation_input_tokens": 0,
                      "cache_read_input_tokens": 2000, "cost_usd": 0.01}});
        fs::write(self.dir.join(name), body.to_string() + "\n").unwrap();
    }

    fn raw_reply(&self, body: &Value) {
        fs::write(self.dir.join("reply-last.json"), body.to_string() + "\n").unwrap();
    }

    fn runs(&self) -> u32 {
        fs::read_to_string(self.dir.join("count")).map_or(0, |s| s.trim().parse().unwrap())
    }

    fn request(&self, n: u32) -> Value {
        serde_json::from_str(
            &fs::read_to_string(self.dir.join(format!("request-{n}.json"))).unwrap(),
        )
        .unwrap()
    }

    fn run(&self, opts: &EditOptions) -> Result<Proposal, GenerateFailure> {
        propose_edit_with_key_check(
            &self.helper,
            &zh(),
            opts,
            &self.dir.join("work"),
            &Limits::none(),
            true,
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn zh() -> Value {
    serde_json::from_str(include_str!("../examples/zh-xian.timeline.json")).unwrap()
}

fn op(op: &str, path: &str, value_json: &str) -> Value {
    json!({"op": op, "path": path, "from": "", "value_json": value_json})
}

fn faster() -> Value {
    json!({"status": "edited", "explanation": "第二段缩短到 3 秒。",
           "patch": [op("replace", "/scenes/1/duration_ms", "3000")]})
}

#[test]
fn a_valid_first_reply_is_accepted() {
    let fx = Fixture::new("ok");
    fx.reply(None, &faster());
    let mut opts = EditOptions::new("第二段节奏快一点");
    opts.scene = Some("scene-02".into());
    let p = fx.run(&opts).unwrap();
    assert_eq!(p.status, EditStatus::Edited);
    assert_eq!(p.retries(), 0);
    assert_eq!(
        p.patch,
        json!([{"op": "replace", "path": "/scenes/1/duration_ms", "value": 3000}])
    );
    assert_eq!(p.outcome.timeline["scenes"][1]["duration_ms"], json!(3000));
    assert!(
        p.outcome.summary[0].starts_with("scene-02: duration"),
        "{:?}",
        p.outcome.summary
    );
    assert!((p.usage.cost_usd - 0.01).abs() < 1e-9);
    let req = fx.request(1);
    assert_eq!(req["instruction"], json!("第二段节奏快一点"));
    assert_eq!(req["scene_id"], json!("scene-02"));
    assert_eq!(req["effort"], json!(DEFAULT_EFFORT));
    assert_eq!(req["timeline"], zh());
    let args = fs::read_to_string(fx.dir.join("args-1.txt")).unwrap();
    assert!(
        args.contains("edit_timeline.py") && args.contains("edit.md"),
        "{args}"
    );
    assert!(fs::read_to_string(fx.helper.edit_prompt_path())
        .unwrap()
        .contains("JSON Patch"));
}

#[test]
fn a_rejected_patch_goes_back_with_the_failing_op_and_the_fix_is_accepted() {
    let fx = Fixture::new("retry");
    fx.reply(
        Some(1),
        &json!({"status": "edited", "explanation": "x",
                "patch": [op("replace", "/scenes/0/image", "\"/etc/passwd\"")]}),
    );
    fx.reply(Some(2), &json!({"status": "edited", "explanation": "x", "patch": [op("add", "/scenes/0/sparkle", "true")]}));
    fx.reply(None, &faster());
    let p = fx.run(&EditOptions::new("x")).unwrap();
    assert_eq!(p.retries(), 2);
    assert_eq!(fx.runs(), 3);
    let req = fx.request(3);
    let history = req["history"].as_array().unwrap();
    assert_eq!(history.len(), 2);
    let first = history[0]["feedback"].as_str().unwrap();
    assert!(
        first.contains("image") && first.contains("relative"),
        "{first}"
    );
    let second = history[1]["feedback"].as_str().unwrap();
    assert!(second.contains("op 0 (add /scenes/0/sparkle)"), "{second}");
    assert!((p.usage.cost_usd - 0.03).abs() < 1e-9);
}

#[test]
fn retries_run_out() {
    let fx = Fixture::new("out");
    fx.reply(
        None,
        &json!({"status": "edited", "explanation": "x", "patch": [op("remove", "/scenes", "")]}),
    );
    let e = fx.run(&EditOptions::new("delete everything")).unwrap_err();
    assert_eq!(e.kind, FailureKind::Invalid);
    assert_eq!(e.attempts.len(), 3);
    assert!(e.message.contains("required"), "{}", e.message);
}

#[test]
fn clarify_and_refused_carry_no_patch() {
    let fx = Fixture::new("refuse");
    fx.reply(
        None,
        &json!({"status": "refused", "explanation": "3D models are not supported.", "patch": []}),
    );
    let p = fx.run(&EditOptions::new("add a 3D model")).unwrap();
    assert_eq!(p.status, EditStatus::Refused);
    assert_eq!(p.patch, json!([]));
    assert!(!p.outcome.changed);
    assert_eq!(p.outcome.timeline, zh());
}

#[test]
fn reply_consistency_checks() {
    let check = EditCheck::default();
    let t = zh();
    for (reply, needle) in [
        (
            json!({"status": "refused", "explanation": "no", "patch": [op("remove", "/music", "")]}),
            "must be empty",
        ),
        (
            json!({"status": "edited", "explanation": "ok", "patch": []}),
            "patch is empty",
        ),
        (
            json!({"status": "edited", "explanation": " ", "patch": [op("replace", "/fps", "30")]}),
            "explanation",
        ),
        (
            json!({"status": "edited", "explanation": "ok", "patch": [op("replace", "/fps", "30")]}),
            "changes nothing",
        ),
        (
            json!({"status": "maybe", "explanation": "ok", "patch": []}),
            "required shape",
        ),
        (
            json!({"status": "edited", "explanation": "ok", "patch": [], "extra": 1}),
            "required shape",
        ),
    ] {
        let e = check_reply(&reply.to_string(), &t, &check).unwrap_err();
        assert!(e.contains(needle), "{needle}: {e}");
    }
}

#[test]
fn input_problems_stop_before_any_call() {
    let fx = Fixture::new("input");
    fx.reply(None, &faster());
    assert!(fx
        .run(&EditOptions::new("  "))
        .unwrap_err()
        .message
        .contains("empty"));
    let mut opts = EditOptions::new("x");
    opts.scene = Some("scene-99".into());
    assert!(fx.run(&opts).unwrap_err().message.contains("scene-99"));
    assert!(fx
        .run(&EditOptions::new("y".repeat(MAX_INSTRUCTION_CHARS + 1)))
        .is_err());
    let e = propose_edit_with_key_check(
        &fx.helper,
        &json!({"nope": 1}),
        &EditOptions::new("x"),
        &fx.dir,
        &Limits::none(),
        true,
    )
    .unwrap_err();
    assert!(e.message.contains("not a valid timeline"), "{}", e.message);
    let e = propose_edit_with_key_check(
        &fx.helper,
        &zh(),
        &EditOptions::new("x"),
        &fx.dir,
        &Limits::none(),
        false,
    )
    .unwrap_err();
    assert_eq!(e.kind, FailureKind::NoApiKey);
    assert_eq!(fx.runs(), 0);
}

#[test]
fn api_failures_keep_their_kind_and_usage() {
    let fx = Fixture::new("api");
    fx.raw_reply(
        &json!({"ok": false, "error": {"kind": "rate_limit", "message": "slow down"},
        "usage": {"input_tokens": 5, "output_tokens": 0, "cost_usd": 0.001}}),
    );
    let e = fx.run(&EditOptions::new("x")).unwrap_err();
    assert_eq!(e.kind, FailureKind::Api("rate_limit".into()));
    assert!((e.usage.cost_usd - 0.001).abs() < 1e-9);
    assert_eq!(fx.runs(), 1);
}

#[test]
fn assets_are_listed_relative_and_sorted() {
    let dir = std::env::temp_dir().join(format!("vimmer-edit-assets-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("assets/sub")).unwrap();
    fs::create_dir_all(dir.join(".hidden")).unwrap();
    for f in [
        "assets/b.png",
        "assets/a.JPG",
        "assets/sub/m.mp3",
        "assets/notes.txt",
        ".hidden/x.png",
    ] {
        fs::write(dir.join(f), b"x").unwrap();
    }
    assert_eq!(
        list_assets(&dir),
        vec!["assets/a.JPG", "assets/b.png", "assets/sub/m.mp3"]
    );
    let _ = fs::remove_dir_all(&dir);
}
