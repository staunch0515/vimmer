//! Tests for the in-process assistant with a stand-in Python: it prints `reply.json` with
//! `@ENV@` replaced by the key it was given, and records that key in `saw`. No network, no real
//! key.

use std::cell::Cell;
use std::fs;
use std::os::unix::fs::PermissionsExt;

use super::*;
use vimmer_storyboard::edit_claude::propose_edit_with_key_check;

const FAKE_PYTHON: &str = r#"#!/bin/sh
dir=$(dirname "$0")
echo run >> "$dir/runs"
printf '%s' "$ANTHROPIC_API_KEY" > "$dir/saw"
echo "helper saw $ANTHROPIC_API_KEY" >&2
sed "s/@ENV@/$ANTHROPIC_API_KEY/g" "$dir/reply.json"
"#;
const FAKE_KEY: &str = "sk-fake-test-value-000";

const TIMELINE: &str = r#"{"fps": 30, "resolution": {"width": 1920, "height": 1080},
  "scenes": [{"id": "scene-01", "duration_ms": 15000, "text": "Turn any idea into a video"}]}"#;

struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(name: &str, reply: Option<Value>) -> Self {
        let dir = std::env::temp_dir().join(format!("vimmer-desktop-assistant-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("public")).unwrap();
        let python = dir.join("python");
        fs::write(&python, FAKE_PYTHON).unwrap();
        fs::set_permissions(&python, fs::Permissions::from_mode(0o755)).unwrap();
        let body = reply.map(|r| r.to_string() + "\n").unwrap_or_default();
        fs::write(dir.join("reply.json"), body).unwrap();
        Self { dir }
    }

    fn helper(&self) -> LlmHelper {
        let mut h = LlmHelper::locate(Some(&self.dir.join("python"))).unwrap();
        h.dir = self.dir.join("files");
        h
    }

    fn env<'a>(
        &self,
        key: &'a dyn Fn() -> Option<HelperKey>,
        helper: &'a dyn Fn() -> Result<LlmHelper, String>,
    ) -> AssistantEnv<'a> {
        AssistantEnv {
            public_dir: Some(self.dir.join("public")),
            work_dir: self.dir.join("work"),
            limits: Limits::none(),
            key,
            helper,
        }
    }

    /// Proposes with the fake helper and a key read "from the shell".
    fn propose(&self, a: &ProposeArgs) -> Value {
        let helper = || Ok(self.helper());
        let key = || Some(HelperKey::FromShell(FAKE_KEY.into()));
        propose(&self.env(&key, &helper), a)
    }

    fn runs(&self) -> usize {
        fs::read_to_string(self.dir.join("runs")).map_or(0, |s| s.lines().count())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn args(instruction: &str) -> ProposeArgs {
    ProposeArgs {
        timeline_json: TIMELINE.into(),
        instruction: instruction.into(),
        scene_id: Some("scene-01".into()),
        model: "claude-sonnet-5-5".into(),
    }
}

fn usage() -> Value {
    json!({"input_tokens": 1000, "output_tokens": 200, "cache_creation_input_tokens": 0,
           "cache_read_input_tokens": 0, "cost_usd": 0.004})
}

fn reply(status: &str, patch: Value) -> Value {
    let edit = json!({"status": status, "explanation": "Scene 1 is slower.", "patch": patch});
    json!({"ok": true, "reply_json": edit.to_string(), "model": "claude-sonnet-5-5",
           "stop_reason": "end_turn", "usage": usage(), "warnings": ["note with @ENV@ inside"]})
}

fn slower() -> Value {
    reply("edited", json!([{"op": "replace", "path": "/scenes/0/duration_ms", "from": "", "value_json": "20000"}]))
}

fn api_error(kind: &str, message: &str) -> Value {
    json!({"ok": false, "error": {"kind": kind, "message": message}, "usage": null})
}

fn kind(v: &Value) -> &str {
    v["error"]["kind"].as_str().unwrap_or("")
}

fn message(v: &Value) -> String {
    v["error"]["message"].as_str().unwrap_or("").to_string()
}

#[test]
fn the_proposal_is_what_vimmer_edit_dry_run_json_prints() {
    let fx = Fixture::new("same", Some(slower()));
    let a = args("make scene 1 slower");
    let got = fx.propose(&a);

    // The CLI's `--dry-run --json` output for the same reply: `proposal_json(p, None)` from the
    // same library call (vimmer-cli's tests/edit_json.rs checks the CLI prints exactly that).
    let mut helper = fx.helper();
    helper.env.set(ENV_API_KEY, FAKE_KEY);
    let timeline: Value = serde_json::from_str(TIMELINE).unwrap();
    let opts = edit_options(&a, Some(&fx.dir.join("public")));
    let p = propose_edit_with_key_check(&helper, &timeline, &opts, &fx.dir.join("work2"), &Limits::none(), true)
        .unwrap();
    let mut cli = proposal_json(&p, None);
    // The fake helper put the key in a warning; the desktop scrubs it.
    let cli_text = cli.to_string().replace(FAKE_KEY, "[api key removed]");
    cli = serde_json::from_str(&cli_text).unwrap();
    assert_eq!(got, cli);

    assert_eq!(got["ok"], true);
    assert_eq!(got["status"], "edited");
    assert_eq!(got["retries"], 0);
    assert_eq!(got["written"], Value::Null);
    assert_eq!(got["outcome"]["timeline"]["scenes"][0]["duration_ms"], 20000);
    assert_eq!(got["patch"][0]["value"], 20000);
    assert_eq!(got["usage"]["cost_usd"], 0.004);
    assert!(!fx.dir.join("work").exists(), "request files are removed");
}

#[test]
fn clarify_and_refused_are_answers_not_errors() {
    for status in ["clarify", "refused"] {
        let fx = Fixture::new(status, Some(reply(status, json!([]))));
        let got = fx.propose(&args("do something"));
        assert_eq!(got["ok"], true, "{got}");
        assert_eq!(got["status"], status);
        assert_eq!(got["outcome"]["changed"], false);
        assert!(got.get("error").is_none());
    }
}

#[test]
fn each_failure_names_its_cause() {
    let cases: [(&str, Option<Value>, &str, &[&str]); 7] = [
        ("ratelimit", Some(api_error("rate_limit", "rate limited (429) after retries: busy")), "rate_limit",
         &["The Claude API call failed", "429"]),
        ("connection", Some(api_error("connection", "could not reach the API: dns")), "connection",
         &["The Claude API call failed", "could not reach"]),
        ("auth", Some(api_error("auth", "the API key in ANTHROPIC_API_KEY was rejected (401): bad")), "auth",
         &["rejected", "~/.zshrc"]),
        ("sdk", Some(api_error("sdk_missing",
            "the anthropic Python SDK is not installed for /x (No module named 'anthropic'); run rust-core/crates/vimmer-storyboard/scripts/install_llm.sh")),
         "helper_missing", &["not installed", "install_llm.sh"]),
        ("crash", None, "helper", &["The Claude helper failed", "printed no result"]),
        ("helperkey", Some(api_error("no_api_key", "ANTHROPIC_API_KEY is not set")), "no_api_key", &["~/.zshrc"]),
        ("invalid", Some(reply("edited", json!([{"op": "replace", "path": "/nope", "from": "", "value_json": "1"}]))),
         "invalid", &["still failed the checks"]),
    ];
    for (name, r, want, words) in cases {
        let fx = Fixture::new(name, r);
        let got = fx.propose(&args("make scene 1 slower"));
        assert_eq!(got["ok"], false, "{name}: {got}");
        assert_eq!(kind(&got), want, "{name}: {got}");
        for w in words {
            assert!(message(&got).contains(w), "{name}: {} lacks {w}", message(&got));
        }
        if want != "helper_missing" {
            assert!(!message(&got).contains("install_llm"), "{name}: no install hint: {}", message(&got));
        }
        assert!(got["usage"].is_object() && got["attempts"].is_array(), "{name}: the CLI's failure shape");
    }
}

#[test]
fn no_key_and_no_helper_are_reported_before_any_call() {
    let fx = Fixture::new("setup", Some(slower()));
    let helper = || Ok(fx.helper());
    let none = || None;
    let got = propose(&fx.env(&none, &helper), &args("x"));
    assert_eq!(kind(&got), "no_api_key");
    assert!(message(&got).contains("Set ANTHROPIC_API_KEY in ~/.zshrc"), "{got}");
    assert!(!message(&got).contains("install_llm"));

    let key = || Some(HelperKey::FromShell(FAKE_KEY.into()));
    let missing = || LlmHelper::locate(Some(Path::new("/nonexistent/python")));
    let got = propose(&fx.env(&key, &missing), &args("x"));
    assert_eq!(kind(&got), "helper_missing");
    assert!(message(&got).contains("install_llm.sh"), "{got}");
    assert_eq!(fx.runs(), 0, "the helper never ran");
}

#[test]
fn bad_input_is_reported_without_reading_the_key() {
    let fx = Fixture::new("input", Some(slower()));
    let asked = Cell::new(0);
    let key = || {
        asked.set(asked.get() + 1);
        Some(HelperKey::FromShell(FAKE_KEY.into()))
    };
    let helper = || Ok(fx.helper());
    let env = fx.env(&key, &helper);
    let mut a = args("x");
    a.scene_id = Some("scene-99".into());
    let got = propose(&env, &a);
    assert_eq!(kind(&got), "input");
    assert!(message(&got).contains("scene-99"), "{got}");
    a = args("x");
    a.timeline_json = "{not json".into();
    assert_eq!(kind(&propose(&env, &a)), "input");
    a = args("x");
    a.timeline_json = r#"{"fps": 30}"#.into();
    assert_eq!(kind(&propose(&env, &a)), "input");
    a = args("  ");
    assert_eq!(kind(&propose(&env, &a)), "input");
    a = args("x");
    a.model = "gpt-x".into();
    assert!(message(&propose(&env, &a)).contains("gpt-x"));
    assert!(check_args(&args(&"x".repeat(2001))).is_err());
    assert_eq!(asked.get(), 0, "the key was never read");
    assert_eq!(fx.runs(), 0);
}

#[test]
fn the_key_reaches_only_the_helper() {
    let replies = [
        slower(),
        api_error("api_error", "API error (500): failed using @ENV@"),
        api_error("auth", "the API key @ENV@ was rejected (401)"),
        api_error("refusal", "Claude declined the request (refusal, category x): @ENV@"),
    ];
    for (i, r) in replies.into_iter().enumerate() {
        let fx = Fixture::new(&format!("key{i}"), Some(r));
        let got = fx.propose(&args("make scene 1 slower"));
        assert_eq!(fs::read_to_string(fx.dir.join("saw")).unwrap(), FAKE_KEY, "the helper got the key");
        let said = got.to_string();
        assert!(!said.contains(FAKE_KEY), "{said}");
        assert!(said.contains("[api key removed]"), "{said}");
    }
}

#[test]
fn the_helper_missing_rule_matches_the_helpers_messages() {
    // `classify` treats a helper failure as "not installed" when it names install_llm.sh: both
    // the missing-Python message and the helper's sdk_missing message do.
    let locate = LlmHelper::locate(Some(Path::new("/nonexistent/python"))).unwrap_err();
    assert!(locate.contains("install_llm.sh"), "{locate}");
    let script = include_str!("../../../rust-core/crates/vimmer-storyboard/scripts/generate_storyboard.py");
    let sdk = &script[script.find("\"sdk_missing\"").unwrap()..];
    assert!(sdk[..300].contains("install_llm.sh"));
}

/// One real call through the desktop path (key from the environment or ~/.zshrc, the installed
/// helper): Sonnet, effort low, on fixtures/sample-timeline.json. Costs about $0.03; run with
/// `cargo test real_edit -- --ignored --nocapture`.
#[test]
#[ignore = "calls the Claude API (costs money)"]
fn real_edit_through_the_desktop_path() {
    let timeline = include_str!("../../../fixtures/sample-timeline.json");
    let a = ProposeArgs {
        timeline_json: timeline.into(),
        instruction: "make scene 1 slower".into(),
        scene_id: None,
        model: "claude-sonnet-5-5".into(),
    };
    let work = std::env::temp_dir().join(format!("vimmer-desktop-real-edit-{}", std::process::id()));
    let env = AssistantEnv {
        public_dir: Some(crate::assets::public_dir()),
        work_dir: work,
        limits: Limits::none(),
        key: &crate::api_key::helper_key,
        helper: &|| LlmHelper::locate(None),
    };
    let got = propose(&env, &a);
    println!(
        "ok={} status={} explanation={} summary={} error={} cost_usd={}",
        got["ok"], got["status"], got["explanation"], got["outcome"]["summary"], got["error"], got["usage"]["cost_usd"]
    );
    assert_eq!(got["ok"], true, "{}", got["error"]);
    assert_eq!(got["status"], "edited");
    assert!(got["outcome"]["timeline"]["scenes"][0]["duration_ms"].as_u64().unwrap() > 15000);
}
