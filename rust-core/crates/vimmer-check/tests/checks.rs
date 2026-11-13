//! One test per check, on purpose-built bad timelines (tests/fixtures/bad-*.json) with
//! generated images and audio. Audio tests need ffmpeg; without it they print a note and pass.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use vimmer_check::{check_timeline, CheckOptions, Finding, Report, Severity};
use vimmer_dsl::VideoTimeline;

fn fixture(name: &str) -> VideoTimeline {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vimmer-check-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn have_ffmpeg() -> bool {
    let ok = Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !ok {
        eprintln!("ffmpeg not found: skipping");
    }
    ok
}

fn ffmpeg(args: &[&str]) {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A mono 48 kHz 16-bit WAV of `expr` (an aevalsrc expression in t).
fn wav(dir: &Path, rel: &str, expr: &str, secs: f64) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let src = format!("aevalsrc={expr}:s=48000:d={secs}");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &src,
        "-c:a",
        "pcm_s16le",
        path.to_str().unwrap(),
    ]);
}

fn png(dir: &Path, rel: &str, w: u32, h: u32) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    image::RgbImage::from_pixel(w, h, image::Rgb([90, 120, 160]))
        .save(path)
        .unwrap();
}

fn run(t: &VideoTimeline, public: Option<&Path>) -> Vec<Finding> {
    let opts = CheckOptions {
        public_dir: public.map(Path::to_path_buf),
        ..CheckOptions::default()
    };
    check_timeline(t, &opts)
}

fn find<'a>(
    fs: &'a [Finding],
    code: &str,
    scene: Option<&str>,
    cue: Option<usize>,
) -> Vec<&'a Finding> {
    fs.iter()
        .filter(|f| f.code == code)
        .filter(|f| scene.is_none() || f.scene_id.as_deref() == scene)
        .filter(|f| cue.is_none() || f.cue_index == cue)
        .collect()
}

fn one<'a>(fs: &'a [Finding], code: &str, scene: Option<&str>, cue: Option<usize>) -> &'a Finding {
    let hits = find(fs, code, scene, cue);
    assert_eq!(
        hits.len(),
        1,
        "expected one {code} for {scene:?}/{cue:?}, got {hits:#?}\nall: {fs:#?}"
    );
    hits[0]
}

/// Applies a finding's suggestion to the timeline JSON (`/subtitles/N` + array splices).
fn apply(t: &VideoTimeline, f: &Finding) -> VideoTimeline {
    let mut v = serde_json::to_value(t).unwrap();
    let path = f.suggestion.path.as_deref().expect("path");
    let value = f.suggestion.value.clone().expect("value");
    if let (Some(rest), Value::Array(cues)) = (path.strip_prefix("/subtitles/"), &value) {
        if let Ok(i) = rest.parse::<usize>() {
            let subs = v["subtitles"].as_array_mut().unwrap();
            subs.splice(i..=i, cues.iter().cloned());
            return serde_json::from_value(v).unwrap();
        }
    }
    *v.pointer_mut(path).expect("pointer exists") = value;
    serde_json::from_value(v).unwrap()
}

#[test]
fn text_overflow_cue_length_and_reading_speed() {
    let t = fixture("bad-text.json");
    let fs = run(&t, None);
    let f = one(&fs, "text.overflow", Some("wall-of-text"), None);
    assert_eq!(f.severity, Severity::Error);
    assert_eq!(f.suggestion.path.as_deref(), Some("/scenes/0/text"));
    assert_eq!(
        one(&fs, "text.overflow", Some("cjk-wall"), None).severity,
        Severity::Error
    );
    assert!(find(&fs, "text.overflow", Some("ok"), None).is_empty());

    // Too long for the band: the split suggestion fixes it.
    let f = one(&fs, "subtitle.too_long", None, Some(0));
    let fixed = apply(&t, f);
    assert_eq!(fixed.subtitles.len(), 4);
    let after = run(&fixed, None);
    assert!(
        find(&after, "subtitle.too_long", None, None).is_empty(),
        "{after:#?}"
    );

    // 48 characters in one second.
    let f = one(&fs, "subtitle.reading_speed", None, Some(1));
    assert_eq!(f.severity, Severity::Warning);
    let fixed = apply(&t, f);
    assert!(find(&run(&fixed, None), "subtitle.reading_speed", None, Some(1)).is_empty());
    // The 300-word scene text is also far too fast for 4 s.
    one(&fs, "text.reading_speed", Some("wall-of-text"), None);
    assert!(find(&fs, "subtitle.reading_speed", None, Some(2)).is_empty());
}

#[test]
fn timing_cues_transitions_and_narration() {
    let t = fixture("bad-timing.json");
    let fs = run(&t, None);
    let f = one(&fs, "subtitle.too_short", None, Some(0));
    assert_eq!(f.severity, Severity::Error);
    assert_eq!(f.suggestion.value, Some(serde_json::json!(833)));

    let f = one(&fs, "subtitle.overlap", None, Some(1));
    assert_eq!(f.severity, Severity::Warning);
    let fixed = apply(&t, f);
    assert!(find(&run(&fixed, None), "subtitle.overlap", None, None).is_empty());

    let f = one(&fs, "subtitle.words_out_of_order", None, Some(3));
    assert_eq!(f.suggestion.value, Some(serde_json::json!([])));
    let f = one(&fs, "subtitle.outside_timeline", None, Some(4));
    assert_eq!(f.suggestion.value, Some(serde_json::json!(7500)));
    // The DSL's own message for the same problems is not repeated.
    assert!(find(&fs, "dsl.invalid", None, None).is_empty(), "{fs:#?}");

    let f = one(&fs, "transition.too_long", Some("b"), None);
    assert_eq!(
        f.suggestion.path.as_deref(),
        Some("/scenes/1/transition_in/duration_ms")
    );
    let f = one(&fs, "audio.offset_past_end", Some("b"), None);
    assert_eq!(f.severity, Severity::Error);

    let f = one(&fs, "narration.too_long", Some("c"), None);
    assert_eq!(f.severity, Severity::Warning);
    assert!(f.message.contains("drift"), "{}", f.message);
    let fixed = apply(&t, f);
    assert!(find(&run(&fixed, None), "narration.too_long", None, None).is_empty());

    // Without a public dir, asset checks say they did not run.
    one(&fs, "assets.public_dir_missing", None, None);
}

#[test]
fn audio_files_missing_and_cut_off() {
    if !have_ffmpeg() {
        return;
    }
    let t = fixture("bad-timing.json");
    let public = temp_dir("timing");
    wav(&public, "audio/long.wav", "0.25*sin(2*PI*330*t)", 3.0);
    let fs = run(&t, Some(&public));
    let f = one(&fs, "audio.missing", Some("d"), None);
    assert_eq!(f.severity, Severity::Error);
    assert_eq!(f.suggestion.path.as_deref(), Some("/scenes/3/audio"));
    let f = one(&fs, "audio.cut_off", Some("a"), None);
    assert_eq!(f.severity, Severity::Error);
    assert_eq!(f.suggestion.value, Some(serde_json::json!(3300)));
    // Scene b's audio starts after the scene ends: reported as offset_past_end, not cut_off.
    assert!(find(&fs, "audio.cut_off", Some("b"), None).is_empty());
    assert!(find(&fs, "assets.public_dir_missing", None, None).is_empty());
}

#[test]
fn audio_levels() {
    if !have_ffmpeg() {
        return;
    }
    let t = fixture("bad-audio.json");
    let public = temp_dir("levels");
    wav(&public, "audio/clipped.wav", "1.6*sin(2*PI*440*t)", 2.0);
    wav(&public, "audio/hot.wav", "sin(2*PI*440*t)", 2.0); // 0 dBFS, not clipped
    wav(&public, "audio/quiet.wav", "0.004*sin(2*PI*440*t)", 2.0);
    wav(&public, "audio/silent.wav", "0", 2.0);
    wav(&public, "audio/clean.wav", "0.2*sin(2*PI*440*t)", 2.0);
    wav(&public, "audio/loud-music.wav", "0.7*sin(2*PI*220*t)", 2.0);
    let fs = run(&t, Some(&public));

    one(&fs, "audio.clipping", Some("clipped"), None);
    assert!(find(&fs, "audio.hot_peak", Some("clipped"), None).is_empty());
    one(&fs, "audio.hot_peak", Some("hot"), None);
    assert!(find(&fs, "audio.clipping", Some("hot"), None).is_empty());
    one(&fs, "audio.quiet", Some("quiet"), None);
    one(&fs, "audio.silent", Some("silent"), None);
    for code in [
        "audio.clipping",
        "audio.hot_peak",
        "audio.quiet",
        "audio.silent",
    ] {
        assert!(
            find(&fs, code, Some("clean"), None).is_empty(),
            "{code} on clean audio"
        );
    }
    let f = one(&fs, "audio.music_too_loud", None, None);
    let v = f.suggestion.value.as_ref().unwrap().as_f64().unwrap();
    assert!(v > 0.0 && v < 0.2, "{v}");
}

#[test]
fn images_motion_and_safe_area() {
    let t = fixture("bad-visual.json");
    let public = temp_dir("visual");
    png(&public, "img/canvas.png", 1080, 1920);
    png(&public, "img/wide.png", 1920, 1080);
    png(&public, "img/tiny.png", 270, 480);
    std::fs::write(public.join("img/broken.png"), b"not a png").unwrap();
    let fs = run(&t, Some(&public));

    let f = one(&fs, "motion.uncovered_edge", Some("tilted"), None);
    let fixed = apply(&t, f);
    assert!(find(
        &run(&fixed, Some(&public)),
        "motion.uncovered_edge",
        None,
        None
    )
    .is_empty());

    let f = one(
        &fs,
        "assets.aspect_ratio",
        Some("landscape-on-portrait"),
        None,
    );
    assert_eq!(f.severity, Severity::Warning);
    assert_eq!(f.suggestion.path.as_deref(), Some("/scenes/1/keyframes"));
    assert!(find(&fs, "assets.aspect_ratio", Some("tilted"), None).is_empty());

    let f = one(&fs, "assets.low_resolution", Some("tiny"), None);
    assert_eq!(f.severity, Severity::Warning);
    assert!(
        f.suggestion.text.contains("1210x2151"),
        "{}",
        f.suggestion.text
    );
    assert_eq!(
        one(&fs, "assets.missing_image", Some("missing"), None).severity,
        Severity::Error
    );
    assert_eq!(
        one(&fs, "assets.unreadable_image", Some("broken"), None).severity,
        Severity::Error
    );

    // 9:16: the renderer's subtitle band sits below the title-safe bottom, reported once.
    let f = one(&fs, "safe_area.subtitle_band", None, None);
    assert_eq!(f.severity, Severity::Info);
}

#[test]
fn rendered_file_checks() {
    if !have_ffmpeg() {
        return;
    }
    let dir = temp_dir("rendered");
    let mp4 = dir.join("short.mp4");
    // 2 s at 640x360 with a clipped tone; the timeline says 3 s at 1920x1080 with music.
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "color=c=gray:s=640x360:r=30:d=2",
        "-f",
        "lavfi",
        "-i",
        "aevalsrc=1.6*sin(2*PI*440*t):s=48000:d=2",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-c:a",
        "pcm_s16le",
        "-shortest",
        dir.join("short.mov").to_str().unwrap(),
    ]);
    std::fs::rename(dir.join("short.mov"), &mp4).unwrap();
    let mut t = fixture("bad-text.json");
    t.scenes.truncate(1);
    t.scenes[0].duration_ms = 3000;
    t.subtitles.clear();
    t.music = Some(vimmer_dsl::Music {
        source: "m.wav".into(),
        volume: 0.3,
        fade_in_ms: 0,
        fade_out_ms: 0,
        duck: vimmer_dsl::Duck::default(),
    });
    let opts = CheckOptions {
        rendered: Some(mp4.clone()),
        ..CheckOptions::default()
    };
    let fs = check_timeline(&t, &opts);
    assert_eq!(
        one(&fs, "rendered.duration", None, None).severity,
        Severity::Error
    );
    assert_eq!(
        one(&fs, "rendered.frame_size", None, None).severity,
        Severity::Error
    );
    assert_eq!(
        one(&fs, "rendered.clipping", None, None).severity,
        Severity::Error
    );

    // Video only: no audio stream although the timeline has music.
    let silent = dir.join("video-only.mp4");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "color=c=gray:s=1920x1080:r=30:d=3",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        silent.to_str().unwrap(),
    ]);
    let opts = CheckOptions {
        rendered: Some(silent),
        ..CheckOptions::default()
    };
    let fs = check_timeline(&t, &opts);
    one(&fs, "rendered.no_audio", None, None);
    assert!(
        find(&fs, "rendered.duration", None, None).is_empty(),
        "{fs:#?}"
    );
    assert!(find(&fs, "rendered.frame_size", None, None).is_empty());
}

#[test]
fn report_counts_and_exit_codes() {
    let fs = run(&fixture("bad-timing.json"), None);
    let report = Report::new(fs, None, None);
    assert!(report.summary.errors > 0);
    assert_eq!(report.exit_code(false), 1);
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["schema"], "vimmer-check/1");
    let first = &json["findings"][0];
    for key in ["severity", "code", "message", "suggestion"] {
        assert!(first.get(key).is_some(), "{key} missing in {first}");
    }

    let warn_only = vec![Finding::new(
        Severity::Warning,
        "x",
        "m",
        vimmer_check::Suggestion::text("s"),
    )];
    let report = Report::new(warn_only, None, None);
    assert_eq!(report.exit_code(false), 0);
    assert_eq!(report.exit_code(true), 1);
}

#[test]
fn layer_text_assets_and_frame() {
    let t = fixture("bad-layers.json");
    vimmer_dsl::validate(&t).expect("the fixture is a valid timeline");
    let public = temp_dir("layers");
    png(&public, "img/tiny.png", 160, 90);
    png(&public, "img/big.png", 800, 800);
    std::fs::create_dir_all(public.join("icons")).unwrap();
    std::fs::write(public.join("icons/fake.svg"), b"\x89PNG not an svg").unwrap();
    std::fs::write(
        public.join("icons/fixed.svg"),
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"><circle cx="12" cy="12" r="10"/></svg>"#,
    )
    .unwrap();
    std::fs::write(
        public.join("icons/good.svg"),
        br#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="10"/></svg>"#,
    )
    .unwrap();
    let fs = run(&t, Some(&public));
    let about = |code: &str, layer: &str| -> Vec<&Finding> {
        find(&fs, code, None, None)
            .into_iter()
            .filter(|f| f.message.contains(&format!("`{layer}`")))
            .collect()
    };

    // Text layers: overflow of a fixed box, title-safe area, reading speed.
    let f = about("text.overflow", "crammed");
    assert_eq!(f.len(), 1, "{fs:#?}");
    assert_eq!(f[0].severity, Severity::Error);
    assert_eq!(
        f[0].suggestion.path.as_deref(),
        Some("/scenes/0/layers/1/text")
    );
    assert_eq!(about("safe_area.text", "edge").len(), 1);
    let f = about("text.reading_speed", "rushed");
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].time_ms, Some(3000));
    assert_eq!(
        f[0].suggestion.path.as_deref(),
        Some("/scenes/0/layers/3/end_ms")
    );
    for code in ["text.overflow", "safe_area.text", "text.reading_speed"] {
        assert!(about(code, "ok").is_empty(), "{code} on the good layer");
    }

    // Off frame: never on screen vs. sliding in through keyframes.
    let f = one(&fs, "layer.off_frame", Some("graphics"), None);
    assert!(f.message.contains("`gone`"));
    let fixed = apply(&t, f);
    assert!(find(&run(&fixed, Some(&public)), "layer.off_frame", None, None).is_empty());

    // Assets.
    assert_eq!(
        about("assets.missing_image", "missing")[0].severity,
        Severity::Error
    );
    let f = about("assets.low_resolution", "tiny");
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].severity, Severity::Warning);
    assert!(about("assets.low_resolution", "sharp").is_empty());
    assert_eq!(
        about("assets.missing_icon", "no-icon")[0].severity,
        Severity::Error
    );
    assert_eq!(
        about("assets.unreadable_icon", "fake-icon")[0].severity,
        Severity::Error
    );
    assert_eq!(about("assets.icon_no_viewbox", "fixed-icon").len(), 1);
    assert!(
        fs.iter().all(|f| !f.message.contains("`good-icon`")),
        "{fs:#?}"
    );
}

#[test]
fn layers_demo_fixture_has_no_errors() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/layers-demo.json");
    let t: VideoTimeline = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let public = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../remotion-renderer/public");
    let fs = run(&t, Some(&public));
    let bad: Vec<_> = fs.iter().filter(|f| f.severity > Severity::Info).collect();
    assert!(bad.is_empty(), "{bad:#?}");
}
