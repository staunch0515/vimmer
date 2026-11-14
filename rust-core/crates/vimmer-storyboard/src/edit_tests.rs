//! Tests for `edit.rs`: JSON Patch semantics, model guards, hostile input, diff summary.

use std::fs;
use std::path::PathBuf;

use serde_json::{json, Value};

use crate::edit::*;

fn zh() -> Value {
    serde_json::from_str(include_str!("../examples/zh-xian.timeline.json")).unwrap()
}

fn small() -> Value {
    json!({
        "fps": 30,
        "resolution": {"width": 1920, "height": 1080},
        "scenes": [
            {"id": "a", "duration_ms": 3000, "text": "西安钟楼，古城的中心。", "highlights": [], "image": "assets/x.png"},
            {"id": "b", "duration_ms": 4000, "text": "second scene", "highlights": ["second"]}
        ],
        "subtitles": []
    })
}

fn ok(doc: &Value, patch: Value) -> EditOutcome {
    apply_edit(doc, &patch, &EditCheck::default()).unwrap_or_else(|e| panic!("{e}"))
}

fn err(doc: &Value, patch: Value) -> Rejection {
    match apply_edit(doc, &patch, &EditCheck::default()) {
        Ok(o) => panic!("expected a rejection, got {:?}", o.summary),
        Err(e) => e,
    }
}

fn text(r: &Rejection) -> String {
    r.to_string()
}

// --- pointers -----------------------------------------------------------------------------------

#[test]
fn pointer_parsing_and_escapes() {
    assert_eq!(parse_pointer("").unwrap(), Vec::<String>::new());
    assert_eq!(parse_pointer("/a~1b/c~0d").unwrap(), vec!["a/b", "c~d"]);
    assert_eq!(parse_pointer("/~01").unwrap(), vec!["~1"]);
    assert_eq!(parse_pointer("/").unwrap(), vec![""]);
    assert!(parse_pointer("a/b").is_err());
    assert!(parse_pointer("/a~2").is_err());
    assert!(parse_pointer("/a~").is_err());
    assert_eq!(escape_token("a/b~c"), "a~1b~0c");
}

// --- RFC 6902 ops (raw apply, no model) ---------------------------------------------------------

fn raw(doc: Value, patch: Value) -> Result<Value, Rejection> {
    apply_patch(&doc, &parse_patch(&patch)?)
}

#[test]
fn rfc_add_object_member_and_array_insert_and_append() {
    let out = raw(
        json!({"a": [1, 2]}),
        json!([
            {"op": "add", "path": "/b", "value": {"c": 1}},
            {"op": "add", "path": "/a/1", "value": 9},
            {"op": "add", "path": "/a/-", "value": 3},
            {"op": "add", "path": "/a/0", "value": 0}
        ]),
    )
    .unwrap();
    assert_eq!(out, json!({"a": [0, 1, 9, 2, 3], "b": {"c": 1}}));
}

#[test]
fn rfc_add_replaces_existing_member() {
    let out = raw(
        json!({"a": 1}),
        json!([{"op": "add", "path": "/a", "value": 2}]),
    )
    .unwrap();
    assert_eq!(out, json!({"a": 2}));
}

#[test]
fn rfc_add_rejects_bad_indices() {
    for path in ["/a/3", "/a/01", "/a/-1", "/a/x"] {
        assert!(
            raw(
                json!({"a": [1, 2]}),
                json!([{"op": "add", "path": path, "value": 0}])
            )
            .is_err(),
            "{path}"
        );
    }
    // Parent must exist.
    assert!(raw(
        json!({}),
        json!([{"op": "add", "path": "/x/y", "value": 0}])
    )
    .is_err());
}

#[test]
fn rfc_remove_replace() {
    let out = raw(
        json!({"a": [1, 2, 3], "b": 1}),
        json!([
            {"op": "remove", "path": "/a/1"},
            {"op": "replace", "path": "/b", "value": "x"}
        ]),
    )
    .unwrap();
    assert_eq!(out, json!({"a": [1, 3], "b": "x"}));
    assert!(raw(json!({"a": 1}), json!([{"op": "remove", "path": "/b"}])).is_err());
    assert!(raw(
        json!({"a": 1}),
        json!([{"op": "replace", "path": "/b", "value": 1}])
    )
    .is_err());
    assert!(raw(json!({"a": [1]}), json!([{"op": "remove", "path": "/a/-"}])).is_err());
}

#[test]
fn rfc_move_and_copy() {
    let out = raw(
        json!({"a": [1, 2, 3], "o": {"x": 1}}),
        json!([
            {"op": "move", "from": "/a/0", "path": "/a/-"},
            {"op": "copy", "from": "/o", "path": "/p"},
            {"op": "move", "from": "/o/x", "path": "/o/y"}
        ]),
    )
    .unwrap();
    assert_eq!(out, json!({"a": [2, 3, 1], "o": {"y": 1}, "p": {"x": 1}}));
    // Into its own child.
    assert!(raw(
        json!({"o": {"x": {}}}),
        json!([{"op": "move", "from": "/o", "path": "/o/x/z"}])
    )
    .is_err());
    // Missing source.
    assert!(raw(
        json!({}),
        json!([{"op": "copy", "from": "/n", "path": "/m"}])
    )
    .is_err());
}

#[test]
fn rfc_test_op_compares_numbers_by_value() {
    assert!(raw(
        json!({"a": 1}),
        json!([{"op": "test", "path": "/a", "value": 1.0}])
    )
    .is_ok());
    assert!(raw(
        json!({"a": [1, {"b": "c"}]}),
        json!([{"op": "test", "path": "/a", "value": [1, {"b": "c"}]}])
    )
    .is_ok());
    let e = raw(
        json!({"a": 1}),
        json!([{"op": "test", "path": "/a", "value": 2}]),
    )
    .unwrap_err();
    assert_eq!(e.stage, Stage::Apply);
    assert_eq!(e.problems[0].op, Some(0));
    assert!(e.to_string().contains("test failed"), "{e}");
}

#[test]
fn patch_is_atomic() {
    let doc = json!({"a": 1, "b": 2});
    let e = raw(
        doc.clone(),
        json!([
            {"op": "replace", "path": "/a", "value": 5},
            {"op": "remove", "path": "/zzz"}
        ]),
    )
    .unwrap_err();
    assert_eq!(e.problems[0].op, Some(1));
    // apply_patch works on a copy; the caller's document never changes.
    assert_eq!(doc, json!({"a": 1, "b": 2}));
}

#[test]
fn escaped_keys_resolve() {
    let out = raw(
        json!({"a/b": {"~": 1}}),
        json!([{"op": "replace", "path": "/a~1b/~0", "value": 2}]),
    )
    .unwrap();
    assert_eq!(out, json!({"a/b": {"~": 2}}));
}

// --- parsing ------------------------------------------------------------------------------------

#[test]
fn parse_accepts_value_json_strings() {
    let ops = parse_patch(&json!([
        {"op": "replace", "path": "/fps", "value_json": "30"},
        {"op": "remove", "path": "/music", "from": "", "value_json": ""}
    ]))
    .unwrap();
    assert_eq!(
        ops[0],
        PatchOp::Replace {
            path: "/fps".into(),
            value: json!(30)
        }
    );
    assert_eq!(
        ops[1],
        PatchOp::Remove {
            path: "/music".into()
        }
    );
}

#[test]
fn parse_rejects_malformed_ops() {
    for (patch, needle) in [
        (json!({"op": "add"}), "array"),
        (json!([{"op": "frobnicate", "path": "/a"}]), "unknown op"),
        (json!([{"op": "add", "path": "/a"}]), "needs a `value`"),
        (json!([{"op": "move", "path": "/a"}]), "from"),
        (
            json!([{"op": "add", "path": "/a", "value": 1, "evil": true}]),
            "unknown key",
        ),
        (
            json!([{"op": "add", "path": "/a", "value_json": "{nope"}]),
            "not valid JSON",
        ),
        (
            json!([{"op": "add", "path": "/a", "value": 1, "value_json": "1"}]),
            "not both",
        ),
        (json!(["add"]), "object"),
    ] {
        let e = parse_patch(&patch).unwrap_err();
        assert_eq!(e.stage, Stage::Parse);
        assert!(e.to_string().contains(needle), "{needle}: {e}");
    }
}

#[test]
fn patch_size_limits() {
    let many: Vec<Value> = (0..=MAX_OPS)
        .map(|_| json!({"op": "test", "path": "/fps", "value": 30}))
        .collect();
    assert!(text(&parse_patch(&Value::Array(many)).unwrap_err()).contains("operations"));
    let big = "x".repeat(MAX_VALUE_BYTES + 10);
    let e = parse_patch(&json!([{"op": "replace", "path": "/scenes/0/text", "value": big}]))
        .unwrap_err();
    assert!(text(&e).contains("bytes"), "{e}");
    let huge: Vec<Value> = (0..40)
        .map(|_| json!({"op": "test", "path": "/fps", "value": "y".repeat(MAX_VALUE_BYTES - 100)}))
        .collect();
    assert!(text(&parse_patch(&Value::Array(huge)).unwrap_err()).contains("limit"));
}

// --- edits on a real timeline -------------------------------------------------------------------

#[test]
fn faster_scene_with_fade_transition() {
    let doc = zh();
    let out = ok(
        &doc,
        json!([
            {"op": "replace", "path": "/scenes/1/duration_ms", "value": 3000},
            {"op": "replace", "path": "/scenes/1/transition_in", "value": {"kind": "fade", "duration_ms": 400}}
        ]),
    );
    assert!(out.changed);
    let line = out
        .summary
        .iter()
        .find(|l| l.starts_with("scene-02:"))
        .unwrap();
    assert!(line.contains("duration 4.11 s -> 3.00 s"), "{line}");
    assert!(
        line.contains("transition in fade 500 ms -> fade 400 ms"),
        "{line}"
    );
    assert!(out.summary.iter().any(|l| l.starts_with("total length:")));
    // Only the patched values differ from the input document.
    assert_eq!(out.timeline["scenes"][0], doc["scenes"][0]);
    assert_eq!(out.timeline["scenes"][1]["duration_ms"], json!(3000));
}

#[test]
fn switch_to_vertical() {
    let out = ok(
        &zh(),
        json!([{"op": "replace", "path": "/resolution", "value": {"width": 1080, "height": 1920}}]),
    );
    assert_eq!(out.summary, vec!["format: 1920x1080 -> 1080x1920"]);
}

#[test]
fn highlight_word_warns_when_not_in_text() {
    let out = ok(
        &zh(),
        json!([{"op": "add", "path": "/scenes/0/highlights/-", "value": "古城"}]),
    );
    assert!(
        out.summary[0].contains("\"钟楼\", \"古城\""),
        "{:?}",
        out.summary
    );
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let out = ok(
        &zh(),
        json!([{"op": "add", "path": "/scenes/0/highlights/-", "value": "长城"}]),
    );
    assert!(out.warnings[0].contains("长城"), "{:?}", out.warnings);
}

#[test]
fn reorder_add_and_remove_scenes_are_summarized() {
    let out = ok(
        &small(),
        json!([
            {"op": "move", "from": "/scenes/1", "path": "/scenes/0"},
            {"op": "copy", "from": "/scenes/0", "path": "/scenes/-"},
            {"op": "replace", "path": "/scenes/2/id", "value": "b-copy"}
        ]),
    );
    assert!(
        out.summary
            .iter()
            .any(|l| l.starts_with("b-copy: added as scene 3")),
        "{:?}",
        out.summary
    );
    assert!(
        out.summary.iter().any(|l| l == "scene order: a, b -> b, a"),
        "{:?}",
        out.summary
    );

    let out = ok(&small(), json!([{"op": "remove", "path": "/scenes/1"}]));
    assert!(
        out.summary.contains(&"b: removed".to_string()),
        "{:?}",
        out.summary
    );
}

#[test]
fn transition_moved_onto_first_scene_is_rejected() {
    // Moving scene-02 (which has a transition) to the front breaks a DSL rule.
    let e = err(
        &zh(),
        json!([{"op": "move", "from": "/scenes/1", "path": "/scenes/0"}]),
    );
    assert_eq!(e.stage, Stage::Validation);
    assert!(text(&e).contains("first scene"), "{e}");
}

#[test]
fn empty_patch_changes_nothing() {
    let out = ok(&zh(), json!([]));
    assert!(!out.changed);
    assert!(out.summary.is_empty());
}

#[test]
fn no_op_test_patch_is_unchanged() {
    let out = ok(&zh(), json!([{"op": "test", "path": "/fps", "value": 30}]));
    assert!(!out.changed);
}

// --- model guards -------------------------------------------------------------------------------

#[test]
fn made_up_paths_are_rejected_with_the_op() {
    for (patch, needle) in [
        (
            json!([{"op": "add", "path": "/secret", "value": 1}]),
            "`secret` is not a field of the timeline",
        ),
        (
            json!([{"op": "add", "path": "/scenes/0/script", "value": "x"}]),
            "`script` is not a field of a scene",
        ),
        (
            json!([{"op": "replace", "path": "/scenes/x/text", "value": "x"}]),
            "not an index",
        ),
        (
            json!([{"op": "replace", "path": "/fps/0", "value": 1}]),
            "no fields",
        ),
        (
            json!([{"op": "replace", "path": "", "value": {}}]),
            "whole timeline",
        ),
        (json!([{"op": "remove", "path": "/scenes"}]), "required"),
    ] {
        let e = err(&zh(), patch);
        assert_eq!(e.stage, Stage::Guard, "{e}");
        assert_eq!(e.problems[0].op, Some(0));
        assert!(text(&e).contains(needle), "{needle}: {e}");
    }
}

#[test]
fn unknown_fields_inside_values_are_rejected() {
    let e = err(
        &zh(),
        json!([{"op": "replace", "path": "/scenes/1/transition_in", "value": {"kind": "fade", "duration_ms": 400, "shader": "x"}}]),
    );
    assert!(
        text(&e).contains("unknown field `shader` in a transition"),
        "{e}"
    );
    let e = err(
        &zh(),
        json!([{"op": "add", "path": "/scenes/-", "value": {"id": "z", "duration_ms": 2000, "text": "t", "model3d": "a.glb"}}]),
    );
    assert!(text(&e).contains("model3d"), "{e}");
}

#[test]
fn move_between_kinds_is_rejected() {
    let e = err(
        &zh(),
        json!([{"op": "move", "from": "/scenes/0", "path": "/subtitles/-"}]),
    );
    assert!(text(&e).contains("cannot move a scene"), "{e}");
}

#[test]
fn wrong_types_are_model_errors() {
    let e = err(
        &zh(),
        json!([{"op": "replace", "path": "/scenes/0/duration_ms", "value": "fast"}]),
    );
    assert_eq!(e.stage, Stage::Model);
    let e = err(
        &zh(),
        json!([{"op": "replace", "path": "/scenes/0/motion", "value": "spin"}]),
    );
    assert_eq!(e.stage, Stage::Model);
}

#[test]
fn dsl_rules_still_apply() {
    for (patch, needle) in [
        (
            json!([{"op": "replace", "path": "/fps", "value": 60}]),
            "unsupported fps",
        ),
        (
            json!([{"op": "replace", "path": "/resolution/width", "value": 1280}]),
            "unsupported resolution",
        ),
        (
            json!([{"op": "replace", "path": "/scenes/0/duration_ms", "value": 0}]),
            "zero duration",
        ),
        (
            json!([{"op": "replace", "path": "/scenes/1/id", "value": "scene-01"}]),
            "duplicate scene id",
        ),
        (
            json!([{"op": "replace", "path": "/scenes", "value": []}]),
            "no scenes",
        ),
    ] {
        let e = err(&zh(), patch);
        assert_eq!(e.stage, Stage::Validation, "{e}");
        assert!(text(&e).contains(needle), "{needle}: {e}");
    }
}

#[test]
fn sanity_limits() {
    for (patch, needle) in [
        (
            json!([{"op": "replace", "path": "/scenes/0/duration_ms", "value": 100}]),
            "at least",
        ),
        (
            json!([{"op": "replace", "path": "/scenes/0/duration_ms", "value": 90000}]),
            "at most",
        ),
        (
            json!([{"op": "replace", "path": "/scenes/1/duration_ms", "value": 9000}, {"op": "replace", "path": "/scenes/1/transition_in/duration_ms", "value": 6000}]),
            "transition is 6000 ms",
        ),
        (
            json!([{"op": "replace", "path": "/scenes/0/text", "value": "长".repeat(MAX_TEXT_CHARS + 1)}]),
            "characters",
        ),
        (
            json!([{"op": "replace", "path": "/scenes/0/id", "value": "../x"}]),
            "scene id",
        ),
        (
            json!([{"op": "add", "path": "/scenes/0/keyframes", "value": [{"at_ms": 0, "scale": 50}]}]),
            "scale",
        ),
        (
            json!([{"op": "add", "path": "/scenes/0/highlights/-", "value": "钟楼"}]),
            "twice",
        ),
        (
            json!([{"op": "add", "path": "/scenes/0/keyframes", "value": [{"at_ms": 0, "x": 500}]}]),
            "x/y",
        ),
    ] {
        let e = err(&zh(), patch);
        assert!(text(&e).contains(needle), "{needle}: {e}");
    }
}

#[test]
fn pre_existing_errors_are_marked() {
    let mut doc = small();
    doc["scenes"][0]["duration_ms"] = json!(0);
    let e = err(
        &doc,
        json!([{"op": "replace", "path": "/scenes/1/text", "value": "x"}]),
    );
    assert!(text(&e).contains("already wrong before the patch"), "{e}");
    // Fixing it is accepted.
    ok(
        &doc,
        json!([{"op": "replace", "path": "/scenes/0/duration_ms", "value": 2000}]),
    );
}

#[test]
fn input_that_is_not_a_timeline_is_reported() {
    let e = apply_edit(&json!({"hello": 1}), &json!([]), &EditCheck::default()).unwrap_err();
    assert_eq!(e.stage, Stage::Model);
    assert!(text(&e).contains("timeline to edit"), "{e}");
}

// --- hostile input ------------------------------------------------------------------------------

#[test]
fn hostile_asset_paths_are_rejected() {
    for path in [
        "/etc/passwd",
        "../../secret",
        "assets/../../secret",
        "./assets/x.png",
        "https://evil.example/x.png",
        "file:///etc/passwd",
        "data:image/png;base64,AAAA",
        "javascript:alert(1)",
        "C:/Windows/win.ini",
        "~/.ssh/id_rsa",
        "assets\\x.png",
        "assets/%2e%2e/x",
        "assets//x.png",
        " assets/x.png",
        "assets/x\u{0}.png",
        "",
    ] {
        let e = err(
            &zh(),
            json!([{"op": "replace", "path": "/scenes/0/image", "value": path}]),
        );
        assert_eq!(e.stage, Stage::Validation, "{path:?}: {e}");
        assert!(text(&e).contains("image"), "{path:?}: {e}");
    }
    for (field, value) in [
        (
            "/scenes/0/audio",
            json!({"source": "/etc/passwd", "offset_ms": 0}),
        ),
        (
            "/music",
            json!({"source": "https://evil/x.mp3", "volume": 0.3, "fade_in_ms": 0, "fade_out_ms": 0}),
        ),
    ] {
        let e = err(
            &zh(),
            json!([{"op": "replace", "path": field, "value": value}]),
        );
        assert!(text(&e).contains("source"), "{field}: {e}");
    }
}

#[test]
fn hostile_voice_names_are_rejected() {
    let e = err(
        &zh(),
        json!([{"op": "replace", "path": "/scenes/0/narration", "value": {"voice": "../../bin/sh"}}]),
    );
    assert!(text(&e).contains("voice"), "{e}");
    ok(
        &zh(),
        json!([{"op": "replace", "path": "/scenes/0/narration", "value": {"voice": "Tingting"}}]),
    );
}

#[test]
fn delete_everything_is_rejected() {
    let e = err(&zh(), json!([{"op": "remove", "path": "/scenes"}]));
    assert_eq!(e.stage, Stage::Guard);
    let n = zh()["scenes"].as_array().unwrap().len();
    let ops: Vec<Value> = (0..n)
        .map(|_| json!({"op": "remove", "path": "/scenes/0"}))
        .collect();
    let e = err(&zh(), Value::Array(ops));
    assert!(text(&e).contains("no scenes"), "{e}");
}

#[test]
fn existing_asset_paths_are_not_rechecked() {
    // An odd path that was already there does not block unrelated edits.
    let mut doc = small();
    doc["scenes"][0]["image"] = json!("https://cdn.example/a.png");
    ok(
        &doc,
        json!([{"op": "replace", "path": "/scenes/1/text", "value": "changed"}]),
    );
}

fn public_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vimmer-edit-test-{}", std::process::id()));
    fs::create_dir_all(dir.join("assets")).unwrap();
    fs::write(dir.join("assets/ok.png"), b"x").unwrap();
    dir
}

#[test]
fn public_dir_existence_and_symlink_escape() {
    let dir = public_dir();
    let check = EditCheck {
        public_dir: Some(dir.clone()),
    };
    let out = apply_edit(
        &small(),
        &json!([{"op": "replace", "path": "/scenes/0/image", "value": "assets/ok.png"}]),
        &check,
    )
    .unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let out = apply_edit(
        &small(),
        &json!([{"op": "replace", "path": "/scenes/0/image", "value": "assets/missing.png"}]),
        &check,
    )
    .unwrap();
    assert!(out.warnings[0].contains("not found"), "{:?}", out.warnings);
    #[cfg(unix)]
    {
        let link = dir.join("assets/escape.png");
        let _ = fs::remove_file(&link);
        std::os::unix::fs::symlink("/etc/hosts", &link).unwrap();
        let e = apply_edit(
            &small(),
            &json!([{"op": "replace", "path": "/scenes/0/image", "value": "assets/escape.png"}]),
            &check,
        )
        .unwrap_err();
        assert!(text(&e).contains("outside the public dir"), "{e}");
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn check_asset_path_accepts_normal_paths() {
    for p in [
        "assets/sample.png",
        "generated/illustrations/abc.png",
        "a.mp3",
        "音乐/曲.mp3",
    ] {
        assert_eq!(check_asset_path(p, None), Ok(None), "{p}");
    }
}

// --- warnings -----------------------------------------------------------------------------------

#[test]
fn shortening_narrated_scene_warns() {
    let mut doc = small();
    doc["scenes"][0]["narration"] = json!({"voice": null});
    let out = ok(
        &doc,
        json!([{"op": "replace", "path": "/scenes/0/duration_ms", "value": 1000}]),
    );
    assert!(
        out.warnings
            .iter()
            .any(|w| w.contains("lengthens the scene")),
        "{:?}",
        out.warnings
    );
}

#[test]
fn subtitles_follow_scene_timing_unless_the_patch_edits_them() {
    let mut doc = small();
    // One cue in scene a (0-3000), one in scene b (3000-7000).
    doc["subtitles"] = json!([
        {"start_ms": 500, "end_ms": 2500, "text": "first"},
        {"start_ms": 3500, "end_ms": 6000, "text": "second"}
    ]);
    let out = ok(
        &doc,
        json!([{"op": "replace", "path": "/scenes/0/duration_ms", "value": 2000}]),
    );
    let subs = &out.timeline["subtitles"];
    assert_eq!(
        subs[0],
        json!({"start_ms": 500, "end_ms": 2000, "text": "first"})
    );
    assert_eq!(
        subs[1],
        json!({"start_ms": 2500, "end_ms": 5000, "text": "second"})
    );
    assert_eq!(out.patch.as_array().unwrap().len(), 2, "{}", out.patch);
    assert_eq!(out.patch[1]["path"], json!("/subtitles"));
    assert!(
        out.summary
            .iter()
            .any(|l| l.contains("moved with their scenes")),
        "{:?}",
        out.summary
    );
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    // Replaying the full patch gives the same timeline.
    assert_eq!(ok(&doc, out.patch.clone()).timeline, out.timeline);

    // A patch that edits subtitles itself is taken as is.
    let out = ok(
        &doc,
        json!([
            {"op": "replace", "path": "/scenes/0/duration_ms", "value": 2000},
            {"op": "replace", "path": "/subtitles/0/end_ms", "value": 1900},
            {"op": "replace", "path": "/subtitles/1/start_ms", "value": 2500},
            {"op": "replace", "path": "/subtitles/1/end_ms", "value": 5000}
        ]),
    );
    assert_eq!(out.patch.as_array().unwrap().len(), 4);
    assert_eq!(out.timeline["subtitles"][0]["end_ms"], json!(1900));
}

#[test]
fn deleting_a_scene_drops_its_words_and_keeps_cues_apart() {
    let doc: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/word-subtitles-seam.json"
    ))
    .unwrap();
    let out = ok(&doc, json!([{"op": "remove", "path": "/scenes/1"}]));
    let t: vimmer_dsl::VideoTimeline = serde_json::from_value(out.timeline.clone()).unwrap();
    assert!(vimmer_dsl::validate(&t).is_ok());
    assert_eq!(t.total_duration_ms(), 6000);
    for pair in t.subtitles.windows(2) {
        assert!(pair[0].end_ms <= pair[1].start_ms, "{:?}", t.subtitles);
    }
    // Cue 0 ran from scene one into scene two: it now ends with scene one, and the words spoken
    // in scene two are gone from its text as well.
    let c0 = &t.subtitles[0];
    assert_eq!(c0.end_ms, 3000);
    let shown: String = c0.words.iter().map(|w| w.text.as_str()).collect();
    assert_eq!(shown.trim(), c0.text);
    // Cue 1 started in scene two: it now starts where scene three starts.
    assert!(t.subtitles[1].start_ms >= 3000, "{:?}", t.subtitles[1]);
}

#[test]
fn lengthening_a_scene_does_not_stretch_its_cues() {
    let doc: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/word-subtitles-cross-scene.json"
    ))
    .unwrap();
    let out = ok(
        &doc,
        json!([{"op": "replace", "path": "/scenes/1/duration_ms", "value": 6000}]),
    );
    assert_eq!(out.timeline["subtitles"], doc["subtitles"]);
    assert_eq!(out.patch.as_array().unwrap().len(), 1);
}

#[test]
fn reordering_scenes_moves_cues_with_them() {
    let mut doc = small();
    doc["subtitles"] = json!([
        {"start_ms": 500, "end_ms": 2500, "text": "in a"},
        {"start_ms": 3500, "end_ms": 6000, "text": "in b"}
    ]);
    let out = ok(
        &doc,
        json!([{"op": "move", "from": "/scenes/1", "path": "/scenes/0"}]),
    );
    let subs = &out.timeline["subtitles"];
    assert_eq!(
        subs[0],
        json!({"start_ms": 500, "end_ms": 3000, "text": "in b"})
    );
    assert_eq!(
        subs[1],
        json!({"start_ms": 4500, "end_ms": 6500, "text": "in a"})
    );
}

#[test]
fn cue_text_must_match_its_words() {
    let doc: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/word-subtitles-seam.json"
    ))
    .unwrap();
    let e = err(
        &doc,
        json!([{"op": "replace", "path": "/subtitles/0/text", "value": "Hello."}]),
    );
    assert!(text(&e).contains("remove `words`"), "{e}");
    ok(
        &doc,
        json!([
            {"op": "replace", "path": "/subtitles/0/text", "value": "Hello."},
            {"op": "remove", "path": "/subtitles/0/words"}
        ]),
    );
}

#[test]
fn new_subtitle_overlaps_are_rejected() {
    let mut doc = small();
    doc["subtitles"] = json!([
        {"start_ms": 0, "end_ms": 1000, "text": "a"},
        {"start_ms": 1000, "end_ms": 2000, "text": "b"}
    ]);
    let e = err(
        &doc,
        json!([{"op": "replace", "path": "/subtitles/0/end_ms", "value": 1500}]),
    );
    assert!(text(&e).contains("overlaps"), "{e}");
}

#[test]
fn all_examples_and_fixtures_load_and_accept_an_empty_patch() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut files: Vec<PathBuf> = ["zh-xian", "en-coffee", "ja-sakura"]
        .iter()
        .map(|n| root.join(format!("examples/{n}.timeline.json")))
        .collect();
    for entry in fs::read_dir(root.join("../../../fixtures")).unwrap() {
        let path = entry.unwrap().path();
        // Brand files (`brand.*.json`) are not timelines.
        if !path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("brand.")
        {
            files.push(path);
        }
    }
    for f in files {
        let doc: Value = serde_json::from_str(&fs::read_to_string(&f).unwrap()).unwrap();
        let out = apply_edit(&doc, &json!([]), &EditCheck::default());
        assert!(out.is_ok(), "{}: {:?}", f.display(), out.err());
    }
}

// --- music ducking (added with the audio merge) --------------------------------------------------

fn with_music() -> Value {
    let mut doc = small();
    doc["music"] =
        json!({"source": "assets/pad-loop.m4a", "volume": 0.3, "fade_in_ms": 0, "fade_out_ms": 0});
    doc
}

#[test]
fn music_duck_can_be_added_and_changed() {
    let out = ok(
        &with_music(),
        json!([{"op": "add", "path": "/music/duck",
                "value": {"enabled": true, "depth_db": -18.0, "attack_ms": 200, "release_ms": 400}}]),
    );
    assert!(
        out.summary.iter().any(|l| l.contains("ducking -18 dB")),
        "{:?}",
        out.summary
    );
    let mut doc = with_music();
    doc["music"]["duck"] =
        json!({"enabled": true, "depth_db": -12.0, "attack_ms": 200, "release_ms": 400});
    let out = ok(
        &doc,
        json!([{"op": "replace", "path": "/music/duck/enabled", "value": false}]),
    );
    assert!(
        out.summary.iter().any(|l| l.contains("ducking off")),
        "{:?}",
        out.summary
    );
}

#[test]
fn music_duck_unknown_keys_and_bad_ranges_are_rejected() {
    let e = err(
        &with_music(),
        json!([{"op": "add", "path": "/music/duck",
                "value": {"enabled": true, "depth_db": -12.0, "attack_ms": 200, "release_ms": 400, "cmd": "rm"}}]),
    );
    assert_eq!(e.stage, Stage::Guard, "{e}");
    let e = err(
        &with_music(),
        json!([{"op": "add", "path": "/music/duck",
                "value": {"enabled": true, "depth_db": -90.0, "attack_ms": 200, "release_ms": 400}}]),
    );
    assert_eq!(e.stage, Stage::Validation, "{e}");
}

// --- music by mood -------------------------------------------------------------------------------

fn repo_public_dir() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../remotion-renderer/public"
    ))
}

fn ok_in_repo(doc: &Value, patch: Value) -> Result<EditOutcome, Rejection> {
    apply_edit(
        doc,
        &patch,
        &EditCheck {
            public_dir: Some(repo_public_dir()),
        },
    )
}

#[test]
fn music_file_can_become_a_mood_pick_and_the_mood_can_change() {
    let out = ok_in_repo(
        &with_music(),
        json!([{"op": "remove", "path": "/music/source"},
               {"op": "add", "path": "/music/pick", "value": {"mood": "upbeat", "energy": 0.8}}]),
    )
    .unwrap();
    assert_eq!(out.timeline["music"]["pick"]["mood"], "upbeat");
    assert!(
        out.summary
            .iter()
            .any(|l| l.contains("mood `upbeat` energy 0.8")),
        "{:?}",
        out.summary
    );
    assert!(
        out.warnings
            .iter()
            .any(|w| w.contains("the render will play `upbeat-sunny`")),
        "{:?}",
        out.warnings
    );
    let out = ok_in_repo(
        &out.timeline,
        json!([{"op": "replace", "path": "/music/pick/mood", "value": "calm"},
               {"op": "add", "path": "/music/pick/exclude", "value": ["calm-drift"]}]),
    )
    .unwrap();
    assert!(
        out.warnings.iter().any(|w| w.contains("`warm-hearth`")),
        "{:?}",
        out.warnings
    );
}

#[test]
fn music_pick_needs_a_known_mood_known_keys_and_no_source() {
    let mut doc = small();
    doc["music"] = json!({"pick": {"mood": "calm"}, "volume": 0.3});
    let e = ok_in_repo(
        &doc,
        json!([{"op": "replace", "path": "/music/pick/mood", "value": "sad"}]),
    )
    .unwrap_err();
    assert_eq!(e.stage, Stage::Validation, "{e}");
    assert!(text(&e).contains("available moods: ambient, calm"), "{e}");
    let e = err(
        &doc,
        json!([{"op": "add", "path": "/music/pick/track", "value": "x"}]),
    );
    assert_eq!(e.stage, Stage::Guard, "{e}");
    // A file and a pick together is not a timeline.
    let e = err(
        &doc,
        json!([{"op": "add", "path": "/music/source", "value": "assets/pad-loop.m4a"}]),
    );
    assert!(text(&e).contains("exactly one of `source`"), "{e}");
    // Without a public dir, the mood is not checked against a library.
    ok(
        &doc,
        json!([{"op": "replace", "path": "/music/pick/mood", "value": "sad"}]),
    );
}

// --- layers -------------------------------------------------------------------------------------

fn with_layers() -> Value {
    let mut doc = small();
    doc["scenes"][1]["layers"] = json!([
        {"id": "bar", "kind": "shape", "x": 0, "y": 0.9, "width": 1, "height": 0.1, "fill": "#000000aa", "z": -1},
        {"id": "title", "kind": "text", "x": 0.1, "y": 0.1, "width": 0.8, "text": "Hello"}
    ]);
    doc
}

#[test]
fn add_a_layer() {
    let out = ok(
        &small(),
        json!([{"op": "add", "path": "/scenes/0/layers", "value": [
            {"id": "logo", "kind": "icon", "src": "assets/icons/star.svg", "color": "#facc15",
             "x": 0.85, "y": 0.1, "width": 0.08, "height": 0.14, "enter": {"kind": "pop", "duration_ms": 300}}
        ]}]),
    );
    assert_eq!(out.timeline["scenes"][0]["layers"][0]["id"], "logo");
    assert!(
        out.summary
            .iter()
            .any(|l| l.contains("layer `logo` added (icon assets/icons/star.svg, z 0)")),
        "{:?}",
        out.summary
    );
    // Append to an existing list.
    let out = ok(
        &with_layers(),
        json!([{"op": "add", "path": "/scenes/1/layers/-", "value":
            {"id": "note", "kind": "text", "x": 0.1, "y": 0.5, "width": 0.4, "text": "Note", "background": {"color": "#fff"}}}]),
    );
    assert!(
        out.summary
            .iter()
            .any(|l| l.contains("layer `note` added (text \"Note\", z 0)")),
        "{:?}",
        out.summary
    );
}

#[test]
fn move_and_restyle_a_layer() {
    let out = ok(
        &with_layers(),
        json!([
            {"op": "replace", "path": "/scenes/1/layers/1/x", "value": 0.2},
            {"op": "add", "path": "/scenes/1/layers/1/z", "value": -2},
            {"op": "add", "path": "/scenes/1/layers/1/color", "value": "#ff0000"}
        ]),
    );
    let line = out
        .summary
        .iter()
        .find(|l| l.contains("layer `title`"))
        .expect("title line");
    assert!(line.contains("x 0.1 -> 0.2"), "{line}");
    assert!(line.contains("z none -> -2"), "{line}");
    // An unset color follows the brand (or the built-in #fafafa).
    assert!(line.contains("color none -> \"#ff0000\""), "{line}");
    // Reordering the list is reported as such.
    let out = ok(
        &with_layers(),
        json!([{"op": "move", "from": "/scenes/1/layers/1", "path": "/scenes/1/layers/0"}]),
    );
    assert!(
        out.summary
            .iter()
            .any(|l| l.contains("layer order bar, title -> title, bar")),
        "{:?}",
        out.summary
    );
    // Moving a layer to another scene.
    let out = ok(
        &with_layers(),
        json!([
            {"op": "add", "path": "/scenes/0/layers", "value": []},
            {"op": "move", "from": "/scenes/1/layers/0", "path": "/scenes/0/layers/-"}
        ]),
    );
    assert_eq!(out.timeline["scenes"][0]["layers"][0]["id"], "bar");
    assert!(out
        .summary
        .iter()
        .any(|l| l.starts_with("b:") && l.contains("layer `bar` removed")));
}

#[test]
fn remove_a_layer() {
    let out = ok(
        &with_layers(),
        json!([{"op": "remove", "path": "/scenes/1/layers/0"}]),
    );
    assert!(
        out.summary
            .iter()
            .any(|l| l.contains("layer `bar` removed")),
        "{:?}",
        out.summary
    );
    let out = ok(
        &with_layers(),
        json!([{"op": "remove", "path": "/scenes/1/layers"}]),
    );
    assert!(out.timeline["scenes"][1].get("layers").is_none());
}

#[test]
fn unknown_layer_keys_are_rejected() {
    // A key no layer has.
    let r = err(
        &with_layers(),
        json!([{"op": "add", "path": "/scenes/1/layers/0/blur", "value": 3}]),
    );
    assert_eq!(r.stage, Stage::Guard);
    assert!(
        text(&r).contains("`blur` is not a field of a layer"),
        "{}",
        text(&r)
    );
    // A key of another kind: `text` on a shape, inside a value.
    let r = err(
        &with_layers(),
        json!([{"op": "add", "path": "/scenes/1/layers/-", "value":
            {"id": "x", "kind": "shape", "x": 0, "y": 0, "width": 1, "height": 1, "fill": "#000", "text": "hi"}}]),
    );
    assert_eq!(r.stage, Stage::Guard);
    assert!(
        text(&r).contains("unknown field `text` in a shape layer"),
        "{}",
        text(&r)
    );
    // ... and by pointer, caught on the patched document.
    let r = err(
        &with_layers(),
        json!([{"op": "add", "path": "/scenes/1/layers/0/font_size", "value": 0.1}]),
    );
    assert_eq!(r.stage, Stage::Guard);
    assert!(
        text(&r).contains("unknown field `font_size` in a shape layer"),
        "{}",
        text(&r)
    );
    // Unknown keys inside nested layer objects.
    let r = err(
        &with_layers(),
        json!([{"op": "add", "path": "/scenes/1/layers/1/enter", "value": {"kind": "fade", "easing": "x"}}]),
    );
    assert!(text(&r).contains("unknown field `easing`"), "{}", text(&r));
    // An unknown kind is a model error.
    let r = err(
        &with_layers(),
        json!([{"op": "replace", "path": "/scenes/1/layers/0/kind", "value": "film"}]),
    );
    assert_eq!(r.stage, Stage::Model);
    // A layer value moved to a non-layer place.
    let r = err(
        &with_layers(),
        json!([{"op": "move", "from": "/scenes/1/layers/0", "path": "/scenes/0/keyframes/-"}]),
    );
    assert_eq!(r.stage, Stage::Guard);
}

#[test]
fn hostile_layer_asset_paths_are_rejected() {
    for src in [
        "/etc/passwd.svg",
        "../secret.svg",
        "assets/../../x.svg",
        "https://evil.example/x.svg",
        "file:///x.svg",
        "data:image/svg+xml,<svg onload=alert(1)>",
        "~/x.svg",
        "assets\\x.svg",
        "assets/%2e%2e/x.svg",
    ] {
        let r = err(
            &small(),
            json!([{"op": "add", "path": "/scenes/0/layers", "value": [
                {"id": "i", "kind": "icon", "src": src, "x": 0, "y": 0, "width": 0.1, "height": 0.1}
            ]}]),
        );
        assert_eq!(r.stage, Stage::Validation, "{src}");
        let r = err(
            &small(),
            json!([{"op": "add", "path": "/scenes/0/layers", "value": [
                {"id": "i", "kind": "image", "src": src, "x": 0, "y": 0, "width": 0.1, "height": 0.1}
            ]}]),
        );
        assert_eq!(r.stage, Stage::Validation, "{src}");
        assert!(
            text(&r).contains("layer `i` src") || text(&r).contains("layer `i`"),
            "{src}: {}",
            text(&r)
        );
    }
    // An icon that is not an SVG.
    let r = err(
        &small(),
        json!([{"op": "add", "path": "/scenes/0/layers", "value": [
            {"id": "i", "kind": "icon", "src": "assets/x.png", "x": 0, "y": 0, "width": 0.1, "height": 0.1}
        ]}]),
    );
    assert!(text(&r).contains(".svg"), "{}", text(&r));
}

#[test]
fn layer_rules_from_the_dsl_apply() {
    let r = err(
        &with_layers(),
        json!([
            {"op": "replace", "path": "/scenes/1/layers/1/id", "value": "bar"},
            {"op": "add", "path": "/scenes/1/layers/0/end_ms", "value": 9000},
            {"op": "replace", "path": "/scenes/1/layers/0/fill", "value": "red"}
        ]),
    );
    assert_eq!(r.stage, Stage::Validation);
    let t = text(&r);
    assert!(t.contains("two layers with id `bar`"), "{t}");
    assert!(t.contains("start_ms < end_ms"), "{t}");
    assert!(t.contains("`red` must be a color"), "{t}");
}

#[test]
fn layer_asset_files_are_checked_in_the_public_dir() {
    let dir = public_dir();
    let check = EditCheck {
        public_dir: Some(dir.clone()),
    };
    let out = apply_edit(
        &small(),
        &json!([{"op": "add", "path": "/scenes/0/layers", "value": [
            {"id": "i", "kind": "image", "src": "assets/nope.png", "x": 0, "y": 0, "width": 0.1, "height": 0.1}
        ]}]),
        &check,
    )
    .unwrap();
    assert!(
        out.warnings
            .iter()
            .any(|w| w.contains("layer `i` src") && w.contains("not found")),
        "{:?}",
        out.warnings
    );
}

#[test]
fn data_layers_are_guarded_like_other_layers() {
    // Add a counter, a chart and a ring; patch a bar value; summary names the title/kind.
    let out = ok(
        &small(),
        json!([{"op": "add", "path": "/scenes/0/layers", "value": [
            {"id": "n", "kind": "counter", "x": 0.3, "y": 0.3, "width": 0.4, "to": 1200, "prefix": "$", "ease": "back_out"},
            {"id": "c", "kind": "bar_chart", "x": 0.1, "y": 0.1, "width": 0.8, "height": 0.6,
             "bars": [{"label": "a", "value": 1}, {"label": "b", "value": 2}]},
            {"id": "r", "kind": "progress_ring", "x": 0.4, "y": 0.3, "width": 0.2, "height": 0.356, "value": 0.5},
            {"id": "lt", "kind": "lower_third", "x": 0.06, "y": 0.66, "width": 0.5, "title": "Ada Lovelace",
             "enter": {"kind": "slide_right", "duration_ms": 500, "ease": "elastic_out"}}
        ]}]),
    );
    assert!(
        out.summary
            .iter()
            .any(|l| l.contains("layer `lt` added (lower_third \"Ada Lovelace\"")),
        "{:?}",
        out.summary
    );
    let out = ok(
        &out.timeline,
        json!([{"op": "replace", "path": "/scenes/0/layers/1/bars/1/value", "value": 7}]),
    );
    assert_eq!(
        out.timeline["scenes"][0]["layers"][1]["bars"][1]["value"],
        7
    );
    // Unknown keys inside a bar, and a key of another data kind.
    let r = err(
        &out.timeline,
        json!([{"op": "add", "path": "/scenes/0/layers/1/bars/0/width", "value": 3}]),
    );
    assert_eq!(r.stage, Stage::Guard);
    assert!(
        text(&r).contains("`width` is not a field of a chart bar"),
        "{}",
        text(&r)
    );
    let r = err(
        &out.timeline,
        json!([{"op": "add", "path": "/scenes/0/layers/2/to", "value": 3}]),
    );
    assert!(
        text(&r).contains("unknown field `to` in a progress_ring layer"),
        "{}",
        text(&r)
    );
    // Model rules still apply: too many bars, a ring value above 1.
    let bars: Vec<Value> = (0..13)
        .map(|i| json!({"label": format!("b{i}"), "value": i}))
        .collect();
    let r = err(
        &out.timeline,
        json!([{"op": "replace", "path": "/scenes/0/layers/1/bars", "value": bars}]),
    );
    assert_eq!(r.stage, Stage::Validation);
    let r = err(
        &out.timeline,
        json!([{"op": "replace", "path": "/scenes/0/layers/2/value", "value": 1.5}]),
    );
    assert_eq!(r.stage, Stage::Validation);
}

#[test]
fn video_background_and_layer_edits() {
    // A clip as a scene background, in place of the image.
    let out = ok(
        &small(),
        json!([{"op": "add", "path": "/scenes/1/video", "value":
            {"src": "assets/clips/counter-6s.mp4", "trim_start_ms": 500, "speed": 2, "fit_time": "loop"}}]),
    );
    assert_eq!(out.timeline["scenes"][1]["video"]["speed"], 2.0);
    assert!(
        out.summary.iter().any(|l| l
            .contains("video none -> assets/clips/counter-6s.mp4 (trim 500..end ms, 2x, loop)")),
        "{:?}",
        out.summary
    );
    // A picture-in-picture video layer, then its speed changed by pointer.
    let out = ok(
        &with_layers(),
        json!([{"op": "add", "path": "/scenes/1/layers/-", "value":
            {"id": "pip", "kind": "video", "src": "assets/clips/pip-cellauto-2s.webm",
             "x": 0.65, "y": 0.6, "width": 0.3, "height": 0.3, "volume": 0.5}}]),
    );
    assert!(
        out.summary
            .iter()
            .any(|l| l.contains("layer `pip` added (video assets/clips/pip-cellauto-2s.webm, z 0)")),
        "{:?}",
        out.summary
    );
    let out2 = ok(
        &out.timeline,
        json!([{"op": "add", "path": "/scenes/1/layers/2/speed", "value": 0.5}]),
    );
    assert!(
        out2.summary.iter().any(|l| l.contains("layer `pip` speed")),
        "{:?}",
        out2.summary
    );
}

#[test]
fn bad_video_edits_are_rejected() {
    // Unknown key inside a scene video.
    let r = err(
        &small(),
        json!([{"op": "add", "path": "/scenes/1/video", "value": {"src": "a.mp4", "loop": true}}]),
    );
    assert_eq!(r.stage, Stage::Guard);
    assert!(
        text(&r).contains("unknown field `loop` in a scene background video"),
        "{}",
        text(&r)
    );
    // A text field on a video layer.
    let r = err(
        &with_layers(),
        json!([{"op": "add", "path": "/scenes/1/layers/-", "value":
            {"id": "v", "kind": "video", "src": "a.mp4", "x": 0, "y": 0, "width": 0.2, "height": 0.2, "text": "x"}}]),
    );
    assert!(
        text(&r).contains("unknown field `text` in a video layer"),
        "{}",
        text(&r)
    );
    // DSL rules: extension, speed, image and video together.
    let r = err(
        &small(),
        json!([{"op": "add", "path": "/scenes/1/video", "value": {"src": "assets/sample.png", "speed": 8}}]),
    );
    let t = text(&r);
    assert!(t.contains("must be a .mp4, .mov, .m4v, .webm file"), "{t}");
    assert!(t.contains("speed must be between 0.25 and 4"), "{t}");
    let r = err(
        &small(),
        json!([{"op": "add", "path": "/scenes/0/video", "value": {"src": "assets/clips/counter-6s.mp4"}}]),
    );
    assert!(
        text(&r).contains("both an image and a video"),
        "{}",
        text(&r)
    );
    // Hostile paths.
    let r = err(
        &small(),
        json!([{"op": "add", "path": "/scenes/1/video", "value": {"src": "../../etc/x.mp4"}}]),
    );
    assert!(text(&r).contains("not allowed"), "{}", text(&r));
}

// --- brand kit and templates ----------------------------------------------------------------------

#[test]
fn brand_can_be_added_and_changed() {
    let out = ok(
        &small(),
        json!([{"op": "add", "path": "/brand", "value": {"colors": {"primary": "#ff6b35", "text": "#fff8f0"}}}]),
    );
    let doc = out.timeline;
    let out = ok(
        &doc,
        json!([{"op": "replace", "path": "/brand/colors/primary", "value": "#22d3ee"}]),
    );
    assert_eq!(out.timeline["brand"]["colors"]["primary"], "#22d3ee");
    // Unknown fields, bad colors and odd font names are rejected.
    let e = err(
        &doc,
        json!([{"op": "add", "path": "/brand/colors/secondary", "value": "#000"}]),
    );
    assert!(
        text(&e).contains("`secondary` is not a field of the brand colors"),
        "{e}"
    );
    let e = err(
        &doc,
        json!([{"op": "replace", "path": "/brand/colors/text", "value": "white"}]),
    );
    assert!(text(&e).contains("colors.text `white`"), "{e}");
    let e = err(
        &doc,
        json!([{"op": "add", "path": "/brand/logo", "value": {"src": "../secret.png"}}]),
    );
    assert!(text(&e).contains("logo"), "{e}");
}

#[test]
fn template_scenes_are_checked_per_template() {
    let scene = |params: Value| {
        json!([{"op": "add", "path": "/scenes/-", "value": {
            "id": "q", "duration_ms": 5000, "transition_in": {"kind": "fade", "duration_ms": 400},
            "template": {"name": "quote", "params": params}}}])
    };
    let out = ok(
        &small(),
        scene(json!({"quote": "Less is more.", "author": "Mies"})),
    );
    assert!(
        out.summary.iter().any(|l| l.contains("q")),
        "{:?}",
        out.summary
    );
    let e = err(&small(), scene(json!({"quote": "Q", "items": ["a"]})));
    assert!(
        text(&e).contains("unknown param `items` for template `quote`"),
        "{e}"
    );
    let e = err(&small(), scene(json!({"quote": "x".repeat(221)})));
    assert!(text(&e).contains("221 characters; the limit is 220"), "{e}");
    let e = err(&small(), scene(json!({"author": "only"})));
    assert!(text(&e).contains("`quote` is required"), "{e}");
    let bad = json!([{"op": "add", "path": "/scenes/-", "value": {"id": "x", "duration_ms": 3000, "template": {"name": "slideshow"}}}]);
    assert!(text(&err(&small(), bad)).contains("unknown template `slideshow`"));
    // A param path is checked against every template's params, then validated for this one.
    let doc = ok(&small(), scene(json!({"quote": "Q"}))).timeline;
    let out = ok(
        &doc,
        json!([{"op": "add", "path": "/scenes/2/template/params/author", "value": "Someone"}]),
    );
    assert_eq!(
        out.timeline["scenes"][2]["template"]["params"]["author"],
        "Someone"
    );
    let e = err(
        &doc,
        json!([{"op": "add", "path": "/scenes/2/template/params/colour", "value": "red"}]),
    );
    assert!(text(&e).contains("not a param of any template"), "{e}");
    let e = err(
        &doc,
        json!([{"op": "add", "path": "/scenes/2/template/params/items", "value": ["a"]}]),
    );
    assert!(text(&e).contains("unknown param `items`"), "{e}");
}

// --- beat sync and sound effects -------------------------------------------------------------

#[test]
fn beat_sync_and_sound_effects_are_editable() {
    let mut doc = small();
    doc["music"] = json!({"source": "assets/music/beat-120bpm.m4a", "volume": 0.3, "fade_in_ms": 0, "fade_out_ms": 0});
    let out = ok(
        &doc,
        json!([
            {"op": "add", "path": "/music/beat_sync", "value": {"enabled": true, "every": 2, "max_shift_ms": 300, "mode": "later"}},
            {"op": "add", "path": "/music/beats", "value": {"bpm": 96, "offset_ms": 750}},
            {"op": "add", "path": "/scenes/0/sfx", "value": [{"src": "assets/sfx/whoosh.wav", "at_ms": 2500, "volume": 0.6}]},
        ]),
    );
    assert_eq!(out.timeline["music"]["beat_sync"]["every"], 2);
    assert_eq!(out.timeline["scenes"][0]["sfx"][0]["at_ms"], 2500);
    // A layer-event effect needs the layer.
    let mut with_layer = out.timeline.clone();
    with_layer["scenes"][1]["layers"] = json!([{"id": "t", "kind": "text", "x": 0.1, "y": 0.1, "width": 0.5, "text": "hi", "start_ms": 400}]);
    let out = ok(
        &with_layer,
        json!([{"op": "add", "path": "/scenes/1/sfx", "value": [{"src": "assets/sfx/pop.wav", "on": {"layer": "t", "event": "enter"}}]}]),
    );
    assert_eq!(out.timeline["scenes"][1]["sfx"][0]["on"]["event"], "enter");
}

#[test]
fn bad_beat_sync_and_sound_effects_are_rejected() {
    let mut doc = small();
    doc["music"] =
        json!({"source": "assets/m.m4a", "volume": 0.3, "fade_in_ms": 0, "fade_out_ms": 0});
    let r = err(
        &doc,
        json!([{"op": "add", "path": "/music/beat_sync", "value": {"every": 3}}]),
    );
    assert!(text(&r).contains("every must be 1, 2 or 4"), "{}", text(&r));
    let r = err(
        &doc,
        json!([{"op": "add", "path": "/music/beat_sync/bars", "value": 2}]),
    );
    assert!(text(&r).contains("bars"), "{}", text(&r));
    let r = err(
        &doc,
        json!([{"op": "add", "path": "/scenes/0/sfx", "value": [{"src": "assets/sfx/pop.wav", "at_ms": 100, "pan": -1}]}]),
    );
    assert!(text(&r).contains("unknown field `pan`"), "{}", text(&r));
    let r = err(
        &doc,
        json!([{"op": "add", "path": "/scenes/0/sfx", "value": [{"src": "../pop.wav", "at_ms": 100}]}]),
    );
    assert!(text(&r).contains("sfx 0"), "{}", text(&r));
    let r = err(
        &doc,
        json!([{"op": "add", "path": "/scenes/0/sfx", "value": [{"src": "assets/sfx/pop.wav", "at_ms": 3000}]}]),
    );
    assert!(text(&r).contains("inside the scene"), "{}", text(&r));
    let r = err(
        &doc,
        json!([{"op": "add", "path": "/scenes/0/sfx", "value": [{"src": "assets/sfx/pop.wav", "on": {"layer": "nope", "event": "enter"}}]}]),
    );
    assert!(text(&r).contains("no layer `nope`"), "{}", text(&r));
    let r = err(
        &doc,
        json!([{"op": "add", "path": "/scenes/0/sfx", "value": [{"src": "assets/a.png", "at_ms": 0}]}]),
    );
    assert!(text(&r).contains("audio file"), "{}", text(&r));
}
