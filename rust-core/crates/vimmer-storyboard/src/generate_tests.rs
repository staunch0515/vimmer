//! Tests for [`crate::generate`] with a stand-in Python: a shell script that replays prepared
//! helper replies, one per run. No network, no API key.

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use vimmer_media::Limits;

use crate::generate::*;
use crate::model::{Language, Orientation};

/// Counts its runs, keeps a copy of each request file and prints `reply-<n>.json` (or
/// `reply-last.json` once those run out). `$FAKE_SLEEP` in `sleep.txt` makes it hang first.
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
if [ -f "$dir/sleep.txt" ]; then sleep "$(cat "$dir/sleep.txt")"; fi
echo "asking (fake) ..." >&2
if [ -f "$dir/reply-$n.json" ]; then cat "$dir/reply-$n.json"; else cat "$dir/reply-last.json"; fi
"#;

struct Fixture {
    dir: PathBuf,
    helper: LlmHelper,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("vimmer-llm-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let python = dir.join("python");
        fs::write(&python, FAKE_PYTHON).unwrap();
        fs::set_permissions(&python, fs::Permissions::from_mode(0o755)).unwrap();
        let mut helper = LlmHelper::locate(Some(&python)).unwrap();
        helper.dir = dir.join("helper");
        Self { dir, helper }
    }

    /// Reply number `n` (1-based); `None` sets the reply for every later run.
    fn reply(&self, n: Option<u32>, value: Value) {
        let name = n.map_or("reply-last.json".to_string(), |n| format!("reply-{n}.json"));
        fs::write(self.dir.join(name), value.to_string() + "\n").unwrap();
    }

    fn runs(&self) -> u32 {
        fs::read_to_string(self.dir.join("count"))
            .map(|s| s.trim().parse().unwrap())
            .unwrap_or(0)
    }

    fn request(&self, n: u32) -> Value {
        serde_json::from_str(
            &fs::read_to_string(self.dir.join(format!("request-{n}.json"))).unwrap(),
        )
        .unwrap()
    }

    fn run(&self, opts: &GenerateOptions, limits: &Limits) -> Result<Generated, GenerateFailure> {
        generate_with_key_check(&self.helper, opts, &self.dir.join("work"), limits, true)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn usage(input: u64, output: u64, cost: f64) -> Value {
    json!({"input_tokens": input, "output_tokens": output,
           "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0, "cost_usd": cost})
}

fn ok_reply(storyboard: &Value, cost: f64) -> Value {
    json!({"ok": true, "storyboard_json": storyboard.to_string(), "model": "claude-opus-5-5",
           "stop_reason": "end_turn", "usage": usage(1000, 500, cost), "warnings": []})
}

fn good_storyboard() -> Value {
    json!({"title": "西安钟楼", "orientation": "landscape", "language": "zh", "scenes": [
        {"text": "西安钟楼，古城的中心。", "emphasis": ["钟楼"], "camera": "push_in",
         "image_prompt": "黄昏的钟楼"},
        {"text": "它已有六百多年历史。", "camera": "pull_out", "transition": "fade"}]})
}

fn bad_storyboard() -> Value {
    json!({"title": "西安钟楼", "orientation": "landscape", "language": "zh", "scenes": [
        {"text": "西安钟楼。"}, {"text": "   "}]})
}

fn opts() -> GenerateOptions {
    GenerateOptions::new("西安钟楼位于古城中心。", Language::Zh)
}

#[test]
fn a_valid_first_reply_is_converted() {
    let fx = Fixture::new("ok");
    fx.reply(None, ok_reply(&good_storyboard(), 0.014));
    let mut o = opts();
    o.record_dir = Some(fx.dir.join("record"));
    let got = fx.run(&o, &Limits::none()).unwrap();
    assert_eq!(fx.runs(), 1);
    assert_eq!(got.retries(), 0);
    assert_eq!(got.storyboard.scenes.len(), 2);
    assert_eq!(
        got.storyboard.scenes[0].image_prompt.as_deref(),
        Some("黄昏的钟楼")
    );
    assert!(got.storyboard.scenes.iter().all(|s| s.image.is_none()));
    assert_eq!(got.conversion.timeline.scenes.len(), 2);
    assert_eq!(got.usage.input_tokens, 1000);
    assert!((got.usage.cost_usd - 0.014).abs() < 1e-9);
    assert_eq!(got.attempts[0].model.as_deref(), Some("claude-opus-5-5"));

    // The helper got its embedded files, the request and the record path; never a key.
    assert_eq!(
        fs::read_to_string(fx.helper.script_path()).unwrap(),
        HELPER_SCRIPT
    );
    assert_eq!(
        fs::read_to_string(fx.helper.schema_path()).unwrap(),
        STORYBOARD_SCHEMA
    );
    let args = fs::read_to_string(fx.dir.join("args-1.txt")).unwrap();
    let args: Vec<&str> = args.lines().collect();
    assert_eq!(args[0], "-I");
    assert!(args.contains(&"--request") && args.contains(&"--schema-file"));
    assert!(args.iter().any(|a| a.ends_with("reply-1.json")));
    let req = fx.request(1);
    assert_eq!(req["language"], "zh");
    assert_eq!(req["orientation"], "landscape");
    assert_eq!(req["model"], DEFAULT_MODEL);
    assert_eq!(req["effort"], DEFAULT_EFFORT);
    assert_eq!(req["history"], json!([]));
    assert!(!req.to_string().to_lowercase().contains("key"));
    // Request files are removed after each run.
    assert_eq!(fs::read_dir(fx.dir.join("work")).unwrap().count(), 0);
}

#[test]
fn a_rejected_reply_is_sent_back_and_the_correction_accepted() {
    let fx = Fixture::new("retry");
    fx.reply(Some(1), ok_reply(&bad_storyboard(), 0.01));
    fx.reply(None, ok_reply(&good_storyboard(), 0.02));
    let got = fx.run(&opts(), &Limits::none()).unwrap();
    assert_eq!(fx.runs(), 2);
    assert_eq!(got.retries(), 1);
    assert!(got.attempts[0].error.as_ref().unwrap().contains("scene 2"));
    assert!(got.attempts[1].error.is_none());
    assert_eq!(got.usage.input_tokens, 2000);
    assert_eq!(got.usage.output_tokens, 1000);
    assert!((got.usage.cost_usd - 0.03).abs() < 1e-9);
    let history = &fx.request(2)["history"];
    assert_eq!(history.as_array().unwrap().len(), 1);
    assert_eq!(history[0]["assistant"], bad_storyboard().to_string());
    assert!(history[0]["feedback"]
        .as_str()
        .unwrap()
        .contains("scene 2: field `text`"));
}

#[test]
fn retries_run_out_after_two_corrections() {
    let fx = Fixture::new("exhaust");
    fx.reply(None, ok_reply(&bad_storyboard(), 0.01));
    let err = fx.run(&opts(), &Limits::none()).unwrap_err();
    assert_eq!(err.kind, FailureKind::Invalid);
    assert_eq!(fx.runs(), 3);
    assert_eq!(err.attempts.len(), 3);
    assert_eq!(err.usage.input_tokens, 3000);
    assert!((err.usage.cost_usd - 0.03).abs() < 1e-9);
    assert!(
        err.message.contains("after 3 attempt(s)"),
        "{}",
        err.message
    );
    assert_eq!(fx.request(3)["history"].as_array().unwrap().len(), 2);

    let mut o = opts();
    o.max_retries = 0;
    let fx = Fixture::new("exhaust0");
    fx.reply(None, ok_reply(&bad_storyboard(), 0.01));
    assert_eq!(fx.run(&o, &Limits::none()).unwrap_err().attempts.len(), 1);
}

#[test]
fn replies_must_match_the_request_and_carry_no_asset_paths() {
    let mut sb = good_storyboard();
    sb["language"] = json!("ja");
    sb["scenes"][0]["image"] = json!("assets/sample.png");
    sb["music"] = json!("assets/bgm.mp3");
    let err = validate_reply(&sb.to_string(), Language::Zh, Orientation::Landscape).unwrap_err();
    assert!(err.contains("field `language`: must be `zh`"), "{err}");
    assert!(err.contains("scene 1: field `image`"), "{err}");
    assert!(err.contains("field `music`"), "{err}");
    let err = validate_reply(
        &good_storyboard().to_string(),
        Language::Zh,
        Orientation::Portrait,
    )
    .unwrap_err();
    assert!(err.contains("`portrait`"), "{err}");
    let err = validate_reply("{\"title\": 1}", Language::Zh, Orientation::Landscape).unwrap_err();
    assert!(err.starts_with("invalid storyboard JSON"), "{err}");
    let err = validate_reply(
        r#"{"title":"t","language":"zh","scenes":[{"text":"a","extra":1}]}"#,
        Language::Zh,
        Orientation::Landscape,
    )
    .unwrap_err();
    assert!(err.contains("unknown field `extra`"), "{err}");
}

#[test]
fn no_key_fails_before_running_anything() {
    let fx = Fixture::new("nokey");
    fx.reply(None, ok_reply(&good_storyboard(), 0.01));
    let err = generate_with_key_check(
        &fx.helper,
        &opts(),
        &fx.dir.join("work"),
        &Limits::none(),
        false,
    )
    .unwrap_err();
    assert_eq!(err.kind, FailureKind::NoApiKey);
    assert!(err.message.contains("ANTHROPIC_API_KEY") && err.message.contains("~/.zshrc"));
    assert_eq!(fx.runs(), 0);
    assert_eq!(err.usage, Usage::default());

    // The helper's own key check (e.g. a blank value) maps to the same kind.
    let fx = Fixture::new("nokey2");
    fx.reply(
        None,
        json!({"ok": false, "error": {"kind": "no_api_key", "message": "ANTHROPIC_API_KEY is not set"}, "usage": null}),
    );
    let err = fx.run(&opts(), &Limits::none()).unwrap_err();
    assert_eq!(err.kind, FailureKind::NoApiKey);
}

#[test]
fn api_failures_stop_at_once_and_keep_their_usage() {
    let fx = Fixture::new("refusal");
    fx.reply(Some(1), ok_reply(&bad_storyboard(), 0.01));
    fx.reply(
        None,
        json!({"ok": false, "error": {"kind": "refusal", "message": "Claude declined the request"},
               "usage": usage(900, 0, 0.0036), "model": "claude-opus-5-5", "stop_reason": "refusal"}),
    );
    let err = fx.run(&opts(), &Limits::none()).unwrap_err();
    assert_eq!(err.kind, FailureKind::Api("refusal".into()));
    assert_eq!(fx.runs(), 2, "an API failure is not retried");
    assert_eq!(err.usage.input_tokens, 1900);
    assert!((err.usage.cost_usd - 0.0136).abs() < 1e-9);
    assert_eq!(err.attempts[1].stop_reason.as_deref(), Some("refusal"));
    assert_eq!(err.to_string(), "Claude declined the request");
}

#[test]
fn a_helper_that_prints_no_result_is_reported_with_its_stderr() {
    let fx = Fixture::new("garbage");
    fs::write(fx.dir.join("reply-last.json"), "Traceback: boom\n").unwrap();
    let err = fx.run(&opts(), &Limits::none()).unwrap_err();
    assert_eq!(err.kind, FailureKind::Helper);
    assert!(err.message.contains("printed no result"), "{}", err.message);
    assert!(err.message.contains("asking (fake)"), "{}", err.message);

    let err = LlmHelper::locate(Some(Path::new("/nonexistent/python"))).unwrap_err();
    assert!(
        err.contains("install_llm.sh") && err.contains("--llm-python"),
        "{err}"
    );
}

#[test]
fn a_hung_call_times_out_and_a_cancel_stops_it() {
    let fx = Fixture::new("timeout");
    fx.reply(None, ok_reply(&good_storyboard(), 0.01));
    fs::write(fx.dir.join("sleep.txt"), "30").unwrap();
    let mut o = opts();
    o.timeout = Duration::from_millis(300);
    let started = Instant::now();
    let err = fx.run(&o, &Limits::none()).unwrap_err();
    assert_eq!(err.kind, FailureKind::TimedOut);
    assert!(err.message.contains("--llm-timeout"), "{}", err.message);
    assert!(started.elapsed() < Duration::from_secs(5));

    let flag = Arc::new(AtomicBool::new(false));
    let setter = flag.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        setter.store(true, Ordering::SeqCst);
    });
    let limits = Limits {
        timeout: None,
        cancelled: Arc::new(move || flag.load(Ordering::SeqCst)),
    };
    let started = Instant::now();
    let err = fx.run(&opts(), &limits).unwrap_err();
    assert_eq!(err.kind, FailureKind::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(5));
}

/// The schema sent to Claude mirrors `model.rs`: same field names (minus the asset paths this
/// flow leaves empty) and the camera / transition names the converter knows.
#[test]
fn schema_matches_the_storyboard_model() {
    let schema: Value = serde_json::from_str(STORYBOARD_SCHEMA).unwrap();
    let keys = |v: &Value| -> BTreeSet<String> {
        v["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect()
    };
    let full = crate::Storyboard {
        title: "t".into(),
        orientation: Orientation::Landscape,
        language: Language::Zh,
        music: Some("m".into()),
        narration: true,
        voice: Some("v".into()),
        scenes: vec![crate::SceneSpec {
            text: "a".into(),
            duration_ms: Some(1),
            emphasis: vec![],
            camera: Some("static".into()),
            transition: Some("fade".into()),
            transition_ms: Some(1),
            image: Some("i".into()),
            image_prompt: Some("p".into()),
        }],
    };
    let model = serde_json::to_value(&full).unwrap();
    let model_keys =
        |v: &Value| -> BTreeSet<String> { v.as_object().unwrap().keys().cloned().collect() };
    let mut expected = model_keys(&model);
    expected.remove("music");
    expected.remove("voice");
    assert_eq!(keys(&schema), expected);
    let mut expected = model_keys(&model["scenes"][0]);
    expected.remove("image");
    let scene = &schema["properties"]["scenes"]["items"];
    assert_eq!(keys(scene), expected);
    assert_eq!(scene["additionalProperties"], json!(false));
    assert_eq!(schema["additionalProperties"], json!(false));

    // Every enum value converts without a fallback warning.
    let names = |v: &Value| -> Vec<String> {
        v["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap().to_string())
            .collect()
    };
    let cameras = names(&scene["properties"]["camera"]);
    let transitions = names(&scene["properties"]["transition"]);
    let scenes: Vec<Value> = cameras
        .iter()
        .enumerate()
        .map(|(i, c)| json!({"text": format!("第{i}个场景。"), "camera": c,
                             "transition": if i == 0 { Value::Null } else { json!(transitions[i % transitions.len()]) }}))
        .chain(transitions.iter().map(|t| json!({"text": "转场。", "transition": t})))
        .collect();
    let mut sb = json!({"title": "t", "language": "zh", "scenes": scenes});
    for s in sb["scenes"].as_array_mut().unwrap() {
        s.as_object_mut().unwrap().retain(|_, v| !v.is_null());
    }
    let (_, conversion) =
        validate_reply(&sb.to_string(), Language::Zh, Orientation::Landscape).unwrap();
    assert!(conversion.warnings.is_empty(), "{:?}", conversion.warnings);
    for (field, value) in [
        ("language", ["zh", "en", "ja"]),
        ("orientation", ["landscape", "portrait", ""]),
    ] {
        let got = names(&schema["properties"][field]);
        let want: Vec<String> = value
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        assert_eq!(got, want, "{field}");
    }
}
