//! Tests for [`crate::illustrate`] with a stand-in Python (replays prepared helper replies) and a
//! stand-in Chrome (copies a prepared PNG). No network, no API key, no browser.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::{json, Value};
use vimmer_media::Limits;

use crate::generate::{LlmHelper, Usage, HELPER_SCRIPT};
use crate::illustrate::*;
use crate::model::Storyboard;

/// Same idea as the fake in generate_tests.rs: `reply-<n>.json`, else `reply-last.json`.
const FAKE_PYTHON: &str = r#"#!/bin/sh
dir=$(dirname "$0")
n=$(cat "$dir/count" 2>/dev/null || echo 0)
n=$((n + 1))
echo "$n" > "$dir/count"
while [ $# -gt 0 ]; do
  if [ "$1" = "--request" ]; then cp "$2" "$dir/request-$n.json"; fi
  shift
done
if [ -f "$dir/reply-$n.json" ]; then cat "$dir/reply-$n.json"; else cat "$dir/reply-last.json"; fi
"#;

/// Writes `shot.png` (or `flat.png` once `flat` exists; `small.png` for the 160x90 preflight)
/// to the --screenshot path and logs the SVG it was given. `broken` makes it write nothing;
/// `broken-scenes` only for the full-size pictures.
const FAKE_CHROME: &str = r#"#!/bin/sh
dir=$(dirname "$0")
out=""
small=""
for a in "$@"; do
  case "$a" in
    --screenshot=*) out="${a#--screenshot=}" ;;
    --window-size=160,90) small=1 ;;
    file://*) echo "${a#file://}" >> "$dir/chrome-inputs.txt" ;;
  esac
done
if [ -f "$dir/broken" ]; then exit 0; fi
if [ -n "$small" ]; then cp "$dir/small.png" "$out"; exit 0; fi
if [ -f "$dir/broken-scenes" ]; then exit 0; fi
if [ -f "$dir/flat" ]; then cp "$dir/flat.png" "$out"; else cp "$dir/shot.png" "$out"; fi
"#;

/// A `w` x `h` RGB PNG, uncompressed (stored deflate blocks), so tests need no image crate and
/// no slow resizing.
fn png(w: u32, h: u32, pixel: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for &b in data {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
        out.extend((data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend(data);
        out.extend(&body);
        out.extend(crc32(&body).to_be_bytes());
    }
    let mut raw = Vec::with_capacity(((w * 3 + 1) * h) as usize);
    for y in 0..h {
        raw.push(0);
        for x in 0..w {
            raw.extend(pixel(x, y));
        }
    }
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
    for (i, block) in blocks.iter().enumerate() {
        z.push(u8::from(i + 1 == blocks.len()));
        let len = block.len() as u16;
        z.extend(len.to_le_bytes());
        z.extend((!len).to_le_bytes());
        z.extend(*block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in &raw {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    z.extend(((b << 16) | a).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend(w.to_be_bytes());
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

const GOOD_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1920 1080"><rect width="1920" height="1080" fill="#1f2a44"/><circle cx="960" cy="400" r="200" fill="#d9a441"/></svg>"##;
const EVIL_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1920 1080" onload="alert(1)"><script>alert(2)</script><rect width="1920" height="1080" fill="#1f2a44"/><image href="https://evil.example/x.png"/><circle r="9" fill="url(https://evil.example/y)"/></svg>"##;

struct Fixture {
    dir: PathBuf,
    helper: LlmHelper,
    opts: IllustrateOptions,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("vimmer-illus-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for (file, body) in [("python", FAKE_PYTHON), ("chrome", FAKE_CHROME)] {
            let path = dir.join(file);
            fs::write(&path, body).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let shot = png(1920, 1080, |x, _| {
            if x < 960 {
                [200, 120, 40]
            } else {
                [10, 20, 30]
            }
        });
        fs::write(dir.join("shot.png"), shot).unwrap();
        fs::write(dir.join("flat.png"), png(1920, 1080, |_, _| [10, 20, 30])).unwrap();
        let small = png(
            160,
            90,
            |x, _| if x < 80 { [200, 120, 40] } else { [10, 20, 30] },
        );
        fs::write(dir.join("small.png"), small).unwrap();
        let mut helper = LlmHelper::locate(Some(&dir.join("python"))).unwrap();
        helper.dir = dir.join("helper");
        let mut opts = IllustrateOptions::new(dir.join("public"), dir.join("chrome"));
        opts.concurrency = 1;
        Self { dir, helper, opts }
    }

    fn reply(&self, n: Option<u32>, value: Value) {
        let name = n.map_or("reply-last.json".to_string(), |n| format!("reply-{n}.json"));
        fs::write(self.dir.join(name), value.to_string() + "\n").unwrap();
    }

    fn clear_replies(&self) {
        for entry in fs::read_dir(&self.dir).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("reply-") {
                fs::remove_file(entry.path()).unwrap();
            }
        }
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

    fn run(&self, sb: &Storyboard, key: bool) -> (IllustrationReport, Vec<String>) {
        let events = Mutex::new(Vec::new());
        let report = illustrate_with_key_check(
            &self.helper,
            sb,
            &self.opts,
            &self.dir.join("work"),
            &Limits::none(),
            &|e| events.lock().unwrap().push(e),
            key,
        );
        (report, events.into_inner().unwrap())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn storyboard(scenes: usize) -> Storyboard {
    let scenes: Vec<Value> = (0..scenes)
        .map(|i| json!({"text": format!("第{}幕的文字。", i + 1), "image_prompt": format!("picture {}", i + 1)}))
        .collect();
    serde_json::from_value(json!({
        "title": "测试", "language": "zh", "orientation": "landscape", "scenes": scenes
    }))
    .unwrap()
}

fn usage(cost: f64) -> Value {
    json!({"input_tokens": 1000, "output_tokens": 2000, "cache_creation_input_tokens": 0,
           "cache_read_input_tokens": 0, "cost_usd": cost})
}

fn style_reply() -> Value {
    json!({"ok": true, "kind": "style", "model": "claude-opus-5-5", "usage": usage(0.02),
           "style": {"name": "Test style", "palette": ["#000000", "#ffffff"]}, "warnings": []})
}

fn svg_reply(svg: &str) -> Value {
    json!({"ok": true, "kind": "scene", "model": "claude-opus-5-5", "usage": usage(0.1),
           "svg": svg, "warnings": []})
}

#[test]
fn draws_every_scene_then_reuses_the_cache() {
    let f = Fixture::new("all");
    let sb = storyboard(3);
    f.reply(Some(1), style_reply());
    f.reply(None, svg_reply(GOOD_SVG));
    let (report, events) = f.run(&sb, true);
    assert_eq!(report.unavailable, None);
    assert_eq!(report.drawn(), 3, "{:?}", report.all_warnings());
    assert_eq!(f.runs(), 4, "one style call and one per scene");
    assert!((report.usage.cost_usd - 0.32).abs() < 1e-9);
    assert_eq!(report.style_usage.cost_usd, 0.02);
    assert!(events.iter().any(|e| e.contains("Test style")));
    // Every scene request carries the same style and its own picture prompt and canvas.
    for n in 2..=4 {
        let req = f.request(n);
        assert_eq!(req["kind"], "scene");
        assert_eq!(req["style"]["name"], "Test style");
        assert_eq!(req["width"], 1920);
        assert_eq!(req["height"], 1080);
        assert_eq!(req["model"], "claude-opus-5-5");
        assert_eq!(req["effort"], "medium");
    }
    let prompts: Vec<String> = (2..=4)
        .map(|n| {
            f.request(n)["scene"]["image_prompt"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(prompts, ["picture 1", "picture 2", "picture 3"]);
    for s in &report.scenes {
        let png = f.opts.public_dir.join(s.image.as_ref().unwrap());
        assert!(png.is_file());
        assert!(s
            .image
            .as_ref()
            .unwrap()
            .starts_with("generated/illustrations/"));
        let svg = fs::read_to_string(f.opts.public_dir.join(s.svg.as_ref().unwrap())).unwrap();
        assert!(svg.contains(r#"width="1920" height="1080""#));
        assert!(!s.cached);
        assert_eq!(s.calls, 1);
    }
    // Same scenes and prompts again: no API call at all.
    let (again, _) = f.run(&sb, true);
    assert_eq!(f.runs(), 4);
    assert!(again.style_cached);
    assert_eq!(again.drawn(), 3);
    assert!(again.scenes.iter().all(|s| s.cached));
    assert_eq!(again.usage, Usage::default());
    // Even without a key, cached pictures are used.
    let (no_key, _) = f.run(&sb, false);
    assert_eq!(no_key.drawn(), 3);
    assert_eq!(no_key.unavailable, None);
    // A different model is a different cache entry.
    let mut f2 = f;
    f2.opts.model = "claude-sonnet-5-5".into();
    let (sonnet, _) = f2.run(&sb, true);
    assert_eq!(f2.runs(), 8);
    assert!(sonnet.scenes.iter().all(|s| !s.cached));
}

#[test]
fn hostile_svg_is_sanitized_not_retried() {
    let f = Fixture::new("evil");
    let sb = storyboard(1);
    f.reply(Some(1), style_reply());
    f.reply(None, svg_reply(EVIL_SVG));
    let (report, _) = f.run(&sb, true);
    assert_eq!(report.drawn(), 1);
    assert_eq!(f.runs(), 2);
    let s = &report.scenes[0];
    assert!(
        s.removed.iter().any(|r| r.contains("script")),
        "{:?}",
        s.removed
    );
    let svg = fs::read_to_string(f.opts.public_dir.join(s.svg.as_ref().unwrap())).unwrap();
    for bad in ["script", "onload", "evil", "<image"] {
        assert!(!svg.contains(bad), "{bad} in {svg}");
    }
    // Chrome only ever sees sanitized files: the preflight picture, then the scene.
    let inputs = fs::read_to_string(f.dir.join("chrome-inputs.txt")).unwrap();
    let lines: Vec<&str> = inputs.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].ends_with("preflight.svg"));
    assert!(lines[1].contains("generated/illustrations/") && lines[1].ends_with(".svg"));
}

#[test]
fn invalid_svg_is_sent_back_once() {
    let f = Fixture::new("retry");
    let sb = storyboard(1);
    f.reply(Some(1), style_reply());
    f.reply(Some(2), svg_reply("<svg><rect></svg>"));
    f.reply(Some(3), svg_reply(GOOD_SVG));
    let (report, _) = f.run(&sb, true);
    assert_eq!(report.drawn(), 1, "{:?}", report.all_warnings());
    assert_eq!(report.scenes[0].calls, 2);
    let retry = f.request(3);
    let history = retry["history"].as_array().unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["assistant"], "<svg><rect></svg>");
    assert!(history[0]["feedback"]
        .as_str()
        .unwrap()
        .contains("not well-formed"));
    assert!((report.usage.cost_usd - 0.22).abs() < 1e-9);
}

#[test]
fn twice_invalid_leaves_the_scene_text_only_and_others_continue() {
    let f = Fixture::new("skip");
    let sb = storyboard(2);
    f.reply(Some(1), style_reply());
    // Scene 1: no viewBox, then the wrong aspect ratio. Scene 2: fine.
    f.reply(
        Some(2),
        svg_reply(r#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="5" height="5"/></svg>"#),
    );
    f.reply(
        Some(3),
        svg_reply(r#"<svg viewBox="0 0 100 100"><rect width="5" height="5"/></svg>"#),
    );
    f.reply(None, svg_reply(GOOD_SVG));
    let (report, _) = f.run(&sb, true);
    assert_eq!(report.unavailable, None);
    assert_eq!(report.scenes[0].image, None);
    assert_eq!(report.scenes[0].calls, 2);
    let warnings = report.all_warnings().join("\n");
    assert!(warnings.contains("scene 1: attempt 1: SVG rejected: the root <svg> has no viewBox"));
    assert!(warnings.contains("aspect ratio"));
    assert!(warnings.contains("text only"));
    assert!(report.scenes[1].image.is_some());
    assert_eq!(f.runs(), 4);
}

#[test]
fn flat_render_counts_as_invalid() {
    let f = Fixture::new("flat");
    let sb = storyboard(1);
    fs::write(f.dir.join("flat"), "").unwrap();
    f.reply(Some(1), style_reply());
    f.reply(None, svg_reply(GOOD_SVG));
    let (report, _) = f.run(&sb, true);
    assert_eq!(report.drawn(), 0);
    assert_eq!(report.scenes[0].calls, 2);
    assert!(f.request(3)["history"][0]["feedback"]
        .as_str()
        .unwrap()
        .contains("flat colour"));
}

#[test]
fn no_api_key_means_no_calls_and_a_clear_reason() {
    let f = Fixture::new("nokey");
    let (report, _) = f.run(&storyboard(2), false);
    assert_eq!(f.runs(), 0);
    assert!(report
        .unavailable
        .as_deref()
        .unwrap()
        .contains("ANTHROPIC_API_KEY"));
    assert_eq!(report.drawn(), 0);
}

#[test]
fn rejected_key_stops_the_run() {
    let f = Fixture::new("auth");
    f.reply(
        None,
        json!({"ok": false, "error": {"kind": "auth", "message": "the API key was rejected (401)"},
               "usage": null}),
    );
    let (report, _) = f.run(&storyboard(3), true);
    assert_eq!(f.runs(), 1, "no scene calls after the style call failed");
    assert!(report.unavailable.as_deref().unwrap().contains("401"));
    assert_eq!(report.drawn(), 0);
}

#[test]
fn connection_loss_mid_run_stops_remaining_scenes() {
    let f = Fixture::new("conn");
    f.reply(Some(1), style_reply());
    f.reply(Some(2), svg_reply(GOOD_SVG));
    f.reply(
        None,
        json!({"ok": false, "error": {"kind": "connection", "message": "could not reach the API"}}),
    );
    let (report, _) = f.run(&storyboard(4), true);
    assert_eq!(report.drawn(), 1);
    assert_eq!(f.runs(), 3, "scene 3 and 4 are not tried");
    assert!(report
        .unavailable
        .as_deref()
        .unwrap()
        .contains("could not reach"));
    assert!(report.scenes[3].warnings[0].contains("unavailable"));
}

#[test]
fn style_refusal_falls_back_to_the_built_in_style() {
    let f = Fixture::new("refusal");
    f.reply(
        Some(1),
        json!({"ok": false, "error": {"kind": "refusal", "message": "declined"}, "usage": usage(0.01)}),
    );
    f.reply(None, svg_reply(GOOD_SVG));
    let (report, _) = f.run(&storyboard(1), true);
    assert_eq!(report.drawn(), 1);
    assert_eq!(report.style.unwrap()["name"], fallback_style(Detail::Simple)["name"]);
    assert!(report.warnings[0].contains("built-in style"));
    assert!((report.usage.cost_usd - 0.11).abs() < 1e-9);
}

#[test]
fn scenes_with_an_image_are_kept_and_apply_fills_the_timeline() {
    let f = Fixture::new("apply");
    let mut sb = storyboard(2);
    sb.scenes[0].image = Some("assets/sample.png".into());
    f.reply(Some(1), style_reply());
    f.reply(None, svg_reply(GOOD_SVG));
    let (report, _) = f.run(&sb, true);
    assert!(report.scenes[0].skipped);
    assert_eq!(report.scenes[0].image, None);
    assert_eq!(f.runs(), 2);
    let mut timeline = crate::convert(&sb).unwrap().timeline;
    assert_eq!(report.apply(&mut timeline).unwrap(), 1);
    assert_eq!(
        timeline.scenes[0].image.as_deref(),
        Some("assets/sample.png")
    );
    assert!(timeline.scenes[1]
        .image
        .as_deref()
        .unwrap()
        .starts_with("generated/illustrations/"));
    timeline.scenes.pop();
    assert!(report.apply(&mut timeline).is_err());
}

#[test]
fn portrait_canvas_is_requested() {
    let f = Fixture::new("portrait");
    let mut sb = storyboard(1);
    sb.orientation = crate::Orientation::Portrait;
    f.reply(Some(1), style_reply());
    f.reply(None, svg_reply(GOOD_SVG)); // landscape viewBox: rejected twice
    let (report, _) = f.run(&sb, true);
    assert_eq!(f.request(2)["width"], 1080);
    assert_eq!(f.request(2)["height"], 1920);
    assert_eq!(report.drawn(), 0);
    f.clear_replies();
}

#[test]
fn estimate_grows_with_scenes_and_effort() {
    let (opus, opus_max) = estimate_cost("claude-opus-5-5", "medium", Detail::Simple, 8).unwrap();
    let (sonnet, _) = estimate_cost("claude-sonnet-5-5", "medium", Detail::Simple, 8).unwrap();
    let (low, _) = estimate_cost("claude-opus-5-5", "low", Detail::Simple, 8).unwrap();
    assert!(opus > sonnet && opus > low && opus_max > opus);
    assert!(opus > 0.5 && opus < 2.0, "{opus}");
    assert_eq!(estimate_cost("claude-unknown", "medium", Detail::Simple, 3), None);
}

#[test]
fn price_table_matches_the_python_helper() {
    for (model, p_in, p_out) in PRICES {
        let line = HELPER_SCRIPT
            .lines()
            .find(|l| l.trim_start().starts_with(&format!("\"{model}\":")))
            .unwrap_or_else(|| panic!("{model} missing from generate_storyboard.py"));
        let nums: Vec<f64> = line
            .split(['(', ')', ','])
            .filter_map(|p| p.trim().parse().ok())
            .collect();
        assert_eq!(nums[0], *p_in, "{model}");
        assert_eq!(nums[1], *p_out, "{model}");
    }
}

#[test]
fn cache_key_depends_on_every_part() {
    let base = cache_key(&["m", "s", "p", "1920x1080"]);
    assert_eq!(base.len(), 32);
    assert_eq!(base, cache_key(&["m", "s", "p", "1920x1080"]));
    assert_ne!(base, cache_key(&["m2", "s", "p", "1920x1080"]));
    assert_ne!(base, cache_key(&["m", "s", "p", "1080x1920"]));
    assert_ne!(base, cache_key(&["m", "sp", "", "1920x1080"]));
}

#[test]
fn broken_chrome_is_found_before_any_api_call() {
    let f = Fixture::new("nochrome");
    fs::write(f.dir.join("broken"), "").unwrap();
    f.reply(None, style_reply());
    let (report, _) = f.run(&storyboard(2), true);
    assert_eq!(f.runs(), 0);
    assert!(report
        .unavailable
        .as_deref()
        .unwrap()
        .contains("cannot rasterize"));
    assert_eq!(report.usage, Usage::default());
}

#[test]
fn raster_failure_keeps_the_svg_and_a_rerun_needs_no_api_call() {
    let f = Fixture::new("keepsvg");
    let sb = storyboard(1);
    fs::write(f.dir.join("broken-scenes"), "").unwrap();
    f.reply(Some(1), style_reply());
    f.reply(None, svg_reply(GOOD_SVG));
    let (report, _) = f.run(&sb, true);
    assert_eq!(report.drawn(), 0);
    assert_eq!(f.runs(), 2);
    let svg = report.scenes[0].svg.clone().expect("svg kept");
    assert!(f.opts.public_dir.join(&svg).is_file());
    assert!(report.all_warnings().join(" ").contains("run again"));
    fs::remove_file(f.dir.join("broken-scenes")).unwrap();
    let (again, _) = f.run(&sb, true);
    assert_eq!(f.runs(), 2, "the kept SVG is only rasterized");
    assert_eq!(again.drawn(), 1);
    assert!(again.scenes[0].cached);
}

// ---------------------------------------------------------------------------------------------
// Detail levels

#[test]
fn simple_cache_keys_are_unchanged() {
    // `simple` pictures cached before the detail levels existed must still be found: the key is
    // the original formula (cache version "illustrate-v1", no detail part).
    let f = Fixture::new("simplekey");
    f.reply(Some(1), style_reply());
    f.reply(None, svg_reply(GOOD_SVG));
    let (report, _) = f.run(&storyboard(1), true);
    assert_eq!(report.detail, Detail::Simple);
    let style = r##"{"name":"Test style","palette":["#000000","#ffffff"]}"##;
    let key = cache_key(&[
        "illustrate-v1",
        "scene",
        "claude-opus-5-5",
        "medium",
        style,
        "picture 1",
        "1920x1080",
    ]);
    assert_eq!(key, "bc729f8b70f1f9682fadfc91403face7");
    assert_eq!(
        report.scenes[0].image.as_deref(),
        Some(format!("generated/illustrations/{key}.png").as_str())
    );
    assert_eq!(f.request(2)["detail"], "simple");
}

#[test]
fn rich_pictures_have_their_own_requests_and_cache() {
    let mut f = Fixture::new("rich");
    let sb = storyboard(2);
    f.reply(Some(1), style_reply());
    f.reply(None, svg_reply(GOOD_SVG));
    let (simple, _) = f.run(&sb, true);
    assert_eq!(simple.drawn(), 2);
    assert_eq!(f.runs(), 3);

    f.opts.detail = Detail::Rich;
    f.reply(Some(4), style_reply());
    let (rich, events) = f.run(&sb, true);
    assert_eq!(rich.detail, Detail::Rich);
    assert_eq!(rich.drawn(), 2, "{:?}", rich.all_warnings());
    assert_eq!(f.runs(), 6, "a rich style call and one call per scene: nothing cached");
    assert!(!rich.style_cached);
    assert!(events.iter().any(|e| e.contains("Test style")));
    for n in 4..=6 {
        assert_eq!(f.request(n)["detail"], "rich");
    }
    for (s, r) in simple.scenes.iter().zip(&rich.scenes) {
        assert_ne!(s.image, r.image, "simple and rich pictures are separate files");
    }
    // Both levels stay cached side by side.
    let (again, _) = f.run(&sb, false);
    assert!(again.scenes.iter().all(|s| s.cached));
    f.opts.detail = Detail::Simple;
    let (back, _) = f.run(&sb, false);
    assert!(back.style_cached && back.scenes.iter().all(|s| s.cached));
    assert_eq!(
        back.scenes.iter().map(|s| &s.image).collect::<Vec<_>>(),
        simple.scenes.iter().map(|s| &s.image).collect::<Vec<_>>()
    );
    assert_eq!(f.runs(), 6);
}

#[test]
fn rich_fallback_style_has_the_rich_fields() {
    let simple = fallback_style(Detail::Simple);
    assert!(simple.get("lighting").is_none(), "the simple prompt stays as it was");
    let rich = fallback_style(Detail::Rich);
    for key in ["lighting", "depth", "texture"] {
        assert!(rich[key].as_str().is_some_and(|s| !s.is_empty()), "{key}");
    }
    assert_eq!(rich["palette"], simple["palette"]);
}

#[test]
fn detail_names_parse_both_ways() {
    for d in [Detail::Simple, Detail::Rich] {
        assert_eq!(Detail::parse(d.as_str()), Some(d));
        assert_eq!(d.to_string(), d.as_str());
    }
    assert_eq!(Detail::parse(" RICH "), Some(Detail::Rich));
    assert_eq!(Detail::parse("fancy"), None);
    assert_eq!(Detail::NAMES, ["simple", "rich"]);
    assert_eq!(Detail::default(), Detail::Simple);
    assert_eq!(serde_json::to_value(Detail::Rich).unwrap(), "rich");
}

#[test]
fn rich_estimate_stays_under_the_budget_and_above_simple() {
    for scenes in [1, 5, 10] {
        let (simple, simple_max) =
            estimate_cost("claude-opus-5-5", "medium", Detail::Simple, scenes).unwrap();
        let (rich, rich_max) =
            estimate_cost("claude-opus-5-5", "medium", Detail::Rich, scenes).unwrap();
        assert!(rich > 2.0 * simple && rich_max > rich && rich_max > simple_max);
        assert!(rich < 4.0 * simple, "{rich} vs {simple}");
    }
    // The target: a 5-scene video's rich pictures under about $1.50 with the defaults.
    let (five, _) = estimate_cost("claude-opus-5-5", DEFAULT_EFFORT, Detail::Rich, 5).unwrap();
    assert!(five > 0.8 && five < 1.5, "{five}");
}

// ---------------------------------------------------------------------------------------------
// The sanitizer on what the rich prompt asks for

/// The SVG of a real `rich` reply (claude-opus-5-5, effort medium, 2026-11-16, zh-xian scene 1),
/// saved with --record.
fn recorded_rich_svg() -> String {
    let raw: Value = serde_json::from_str(include_str!(
        "../scripts/tests/fixtures/recorded_rich_zh_scene.json"
    ))
    .unwrap();
    let text: String = raw["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|b| b["type"] == "text")
        .map(|b| b["text"].as_str().unwrap())
        .collect();
    let start = text.find("<svg").unwrap();
    let end = text.rfind("</svg>").unwrap() + "</svg>".len();
    text[start..end].to_string()
}

#[test]
fn a_recorded_rich_picture_passes_the_sanitizer_untouched() {
    let svg = recorded_rich_svg();
    let clean = vimmer_media::sanitize_svg(&svg, 1920, 1080).unwrap();
    assert_eq!(clean.removed, Vec::<String>::new());
    for needle in ["<feTurbulence", "<radialGradient", "<use ", "<filter", "<pattern"] {
        assert!(clean.svg.contains(needle), "{needle} kept");
    }
    assert!(svg.len() < 60 * 1024, "{} bytes", svg.len());
}

/// The building blocks of the rich prompt's example: grain filter, glow, haze, vignette, soft
/// shadow, symbol + use, pattern, mix-blend-mode.
const RICH_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1920 1080" width="1920" height="1080"><defs><linearGradient id="sky" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#1c2b4a"/><stop offset="1" stop-color="#0b1120"/></linearGradient><radialGradient id="glow" cx="0.7" cy="0.2" r="0.5"><stop offset="0" stop-color="#ffcf87" stop-opacity="0.6"/><stop offset="1" stop-color="#ffcf87" stop-opacity="0"/></radialGradient><filter id="soft" x="-30%" y="-30%" width="160%" height="160%"><feGaussianBlur stdDeviation="14"/></filter><filter id="grain" x="0" y="0" width="100%" height="100%"><feTurbulence type="fractalNoise" baseFrequency="0.85" numOctaves="2" seed="7"/><feColorMatrix type="saturate" values="0"/></filter><filter id="wobble"><feTurbulence baseFrequency="0.02" result="t"/><feDisplacementMap in="SourceGraphic" in2="t" scale="6"/></filter><pattern id="brick" width="40" height="20" patternUnits="userSpaceOnUse"><path d="M0 0h40v20H0z" fill="#3a2a2a"/><path d="M0 10h40M20 0v10" stroke="#2a1d1d"/></pattern><symbol id="win" viewBox="0 0 20 30"><rect width="20" height="30" rx="3" fill="#ffd28a"/></symbol></defs><rect width="1920" height="1080" fill="url(#sky)"/><rect width="1920" height="1080" fill="url(#glow)"/><rect x="700" y="300" width="500" height="300" fill="url(#brick)" filter="url(#wobble)"/><use href="#win" x="760" y="340" width="20" height="30"/><use xlink:href="#win" x="820" y="340" width="20" height="30"/><ellipse cx="960" cy="620" rx="300" ry="30" fill="#05070d" opacity="0.5" filter="url(#soft)"/><rect width="1920" height="1080" filter="url(#grain)" opacity="0.08" style="mix-blend-mode:overlay"/></svg>"##;

#[test]
fn the_rich_building_blocks_are_kept() {
    let clean = vimmer_media::sanitize_svg(RICH_SVG, 1920, 1080).unwrap();
    assert_eq!(clean.removed, Vec::<String>::new());
    for needle in [
        "<feTurbulence",
        "<feDisplacementMap",
        "<feColorMatrix",
        "<pattern",
        "<symbol",
        r##"href="#win""##,
        "mix-blend-mode:overlay",
        r##"filter="url(#grain)""##,
    ] {
        assert!(clean.svg.contains(needle), "{needle} kept");
    }
}

#[test]
fn unsafe_content_inside_rich_features_is_still_stripped() {
    let cases: &[(&str, &str, &str)] = &[
        // (inserted before </svg>, must not appear in the output, removal note)
        (
            r#"<filter id="f"><feImage href="https://evil.example/x.png"/><feTurbulence baseFrequency="0.9"/></filter>"#,
            "evil.example",
            "element <feImage>",
        ),
        (
            r#"<rect width="9" height="9" filter="url(https://evil.example/f.svg#f)"/>"#,
            "evil.example",
            "external url()",
        ),
        (
            r#"<rect width="9" height="9" style="mix-blend-mode:overlay;fill:url(http://evil.example/p)"/>"#,
            "evil.example",
            "external url()",
        ),
        (
            r#"<rect width="9" height="9" style="mix-blend-mode:overlay;background:image-set('x.png' 1x)"/>"#,
            "image-set",
            "script-like CSS",
        ),
        (
            r#"<pattern id="p" href="https://evil.example/pat.svg#p" width="9" height="9"><rect width="9" height="9"/></pattern>"#,
            "evil.example",
            "attribute href (external link)",
        ),
        (
            r#"<use href="https://evil.example/sprites.svg#tree"/>"#,
            "evil.example",
            "attribute href (external link)",
        ),
        (
            r#"<symbol id="s"><script>alert(1)</script><rect width="9" height="9"/></symbol>"#,
            "alert",
            "element <script>",
        ),
        (
            r#"<pattern id="q" width="9" height="9"><foreignObject><div>x</div></foreignObject></pattern>"#,
            "foreignObject",
            "element <foreignObject>",
        ),
        (
            r#"<radialGradient id="g"><animate attributeName="r" to="9"/><stop offset="0"/></radialGradient>"#,
            "animate",
            "element <animate>",
        ),
        (
            r#"<feTurbulence baseFrequency="0.9"><set attributeName="seed" to="2"/></feTurbulence>"#,
            "<set",
            "element <set>",
        ),
        (
            r#"<filter id="h" onload="alert(1)"><feGaussianBlur stdDeviation="9"/></filter>"#,
            "onload",
            "attribute onload",
        ),
        (
            r#"<style>@import url(https://evil.example/a.css); rect{mix-blend-mode:overlay}</style>"#,
            "evil.example",
            "CSS @import",
        ),
        (
            r#"<rect width="9" height="9" fill="javascript:alert(1)"/>"#,
            "javascript",
            "attribute fill (javascript: value)",
        ),
    ];
    for (insert, gone, note) in cases {
        let svg = RICH_SVG.replace("</svg>", &format!("{insert}</svg>"));
        let clean = vimmer_media::sanitize_svg(&svg, 1920, 1080)
            .unwrap_or_else(|e| panic!("{insert}: {e}"));
        assert!(!clean.svg.contains(gone), "{insert} -> {}", clean.svg);
        assert!(
            clean.removed.iter().any(|r| r == note),
            "{insert}: removed {:?}",
            clean.removed
        );
        assert!(clean.svg.contains("<feTurbulence"), "the safe parts stay");
    }
}
