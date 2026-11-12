use vimmer_dsl::{validate, MotionType, TransitionKind, VideoTimeline};

use crate::*;

const EXAMPLES: [(&str, &str, &str); 3] = [
    (
        "zh",
        include_str!("../examples/zh-xian.storyboard.json"),
        include_str!("../examples/zh-xian.timeline.json"),
    ),
    (
        "en",
        include_str!("../examples/en-coffee.storyboard.json"),
        include_str!("../examples/en-coffee.timeline.json"),
    ),
    (
        "ja",
        include_str!("../examples/ja-sakura.storyboard.json"),
        include_str!("../examples/ja-sakura.timeline.json"),
    ),
];

fn sb(json: &str) -> Storyboard {
    serde_json::from_str(json).expect("storyboard parses")
}

fn base(scenes: &str) -> Storyboard {
    sb(&format!(
        r#"{{"title":"t","language":"en","scenes":[{scenes}]}}"#
    ))
}

fn err(s: &Storyboard) -> String {
    convert(s).unwrap_err().to_string()
}

#[test]
fn examples_convert_validate_and_match_checked_in_timelines() {
    for (name, storyboard, expected) in EXAMPLES {
        let conv = convert(&sb(storyboard)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(conv.warnings.is_empty(), "{name}: {:?}", conv.warnings);
        validate(&conv.timeline).unwrap();
        let expected: VideoTimeline = serde_json::from_str(expected).unwrap();
        assert_eq!(conv.timeline, expected, "{name}: regenerate examples/*.timeline.json");
        assert!(conv.timeline.total_duration_ms() <= 200_000);
    }
}

#[test]
fn examples_only_reference_existing_public_assets() {
    let public = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../remotion-renderer/public");
    for (name, storyboard, _) in EXAMPLES {
        assert!(missing_assets(&sb(storyboard), &public).is_empty(), "{name}");
    }
}

#[test]
fn orientation_maps_to_resolution() {
    let landscape = convert(&base(r#"{"text":"hi there"}"#)).unwrap();
    assert_eq!(landscape.timeline.resolution.width, 1920);
    let mut portrait = base(r#"{"text":"hi there"}"#);
    portrait.orientation = Orientation::Portrait;
    let p = convert(&portrait).unwrap();
    assert_eq!((p.timeline.resolution.width, p.timeline.resolution.height), (1080, 1920));
}

#[test]
fn camera_mapping() {
    let cases = [
        ("static", MotionType::Static),
        ("push_in", MotionType::ZoomIn),
        ("pull_out", MotionType::ZoomOut),
        ("pan_left", MotionType::PanLeft),
        ("pan_right", MotionType::PanRight),
        ("Push-In", MotionType::ZoomIn),
    ];
    for (camera, motion) in cases {
        let s = base(&format!(r#"{{"text":"x","camera":"{camera}"}}"#));
        let c = convert(&s).unwrap();
        assert_eq!(c.timeline.scenes[0].motion, motion, "{camera}");
        assert!(c.timeline.scenes[0].keyframes.is_empty());
        assert!(c.warnings.is_empty());
    }
}

#[test]
fn ken_burns_becomes_keyframes_spanning_the_scene() {
    let c = convert(&base(r#"{"text":"x","camera":"ken_burns","duration_ms":3000}"#)).unwrap();
    let kf = &c.timeline.scenes[0].keyframes;
    assert_eq!(kf.len(), 2);
    assert_eq!((kf[0].at_ms, kf[1].at_ms), (0, 3000));
    assert!(kf[1].scale > kf[0].scale);
}

#[test]
fn transition_mapping() {
    let cases = [
        ("fade", TransitionKind::Fade),
        ("slide_left", TransitionKind::SlideLeft),
        ("slide_right", TransitionKind::SlideRight),
        ("slide_up", TransitionKind::SlideUp),
        ("slide_down", TransitionKind::SlideDown),
        ("zoom_in", TransitionKind::ZoomIn),
        ("wipe_left", TransitionKind::WipeLeft),
    ];
    for (name, kind) in cases {
        let s = base(&format!(
            r#"{{"text":"a"}},{{"text":"b","transition":"{name}"}}"#
        ));
        let c = convert(&s).unwrap();
        let t = c.timeline.scenes[1].transition_in.unwrap();
        assert_eq!(t.kind, kind);
        assert_eq!(t.duration_ms, DEFAULT_TRANSITION_MS);
        assert!(c.timeline.scenes[0].transition_in.is_none());
    }
    for none in ["none", "cut"] {
        let s = base(&format!(r#"{{"text":"a"}},{{"text":"b","transition":"{none}"}}"#));
        let c = convert(&s).unwrap();
        assert!(c.timeline.scenes[1].transition_in.is_none());
        assert!(c.warnings.is_empty());
    }
}

#[test]
fn first_scene_transition_is_dropped_with_warning() {
    let c = convert(&base(r#"{"text":"a","transition":"fade"}"#)).unwrap();
    assert!(c.timeline.scenes[0].transition_in.is_none());
    assert_eq!(c.warnings.len(), 1);
    assert_eq!(c.warnings[0].scene, Some(1));
    assert_eq!(c.warnings[0].field, "transition");
}

#[test]
fn transition_longer_than_scene_is_clamped() {
    let s = base(r#"{"text":"a"},{"text":"b","duration_ms":800,"transition":"fade","transition_ms":5000}"#);
    let c = convert(&s).unwrap();
    assert_eq!(c.timeline.scenes[1].transition_in.unwrap().duration_ms, 800);
    assert!(c.warnings[0].to_string().contains("scene 2"));
    assert!(c.warnings[0].to_string().contains("transition_ms"));
}

#[test]
fn unknown_camera_and_transition_fall_back_with_warnings() {
    let s = base(r#"{"text":"a"},{"text":"b","camera":"dolly_zoom","transition":"spin"}"#);
    let c = convert(&s).unwrap();
    assert_eq!(c.timeline.scenes[1].motion, MotionType::Static);
    assert!(c.timeline.scenes[1].transition_in.is_none());
    let fields: Vec<_> = c.warnings.iter().map(|w| (w.scene, w.field)).collect();
    assert_eq!(fields, [(Some(2), "camera"), (Some(2), "transition")]);
}

#[test]
fn explicit_duration_wins_and_missing_duration_is_estimated() {
    let s = base(r#"{"text":"one two three four five","duration_ms":7777},{"text":"one two three four five"}"#);
    let c = convert(&s).unwrap();
    assert_eq!(c.timeline.scenes[0].duration_ms, 7777);
    assert_eq!(c.timeline.scenes[1].duration_ms, 400 * 5 + 600);
}

#[test]
fn emphasis_is_returned_and_in_text_words_become_highlights() {
    let s = base(r#"{"text":"Big news today","emphasis":["news","absent"," "]}"#);
    let c = convert(&s).unwrap();
    assert_eq!(c.timeline.scenes[0].highlights, ["news"]);
    assert_eq!(c.emphasis["scene-01"], ["news", "absent", ""]);
    assert_eq!(c.warnings.len(), 2);
}

#[test]
fn narration_and_voice() {
    let c = convert(&base(r#"{"text":"a"}"#)).unwrap();
    assert!(c.timeline.scenes[0].narration.is_some());
    let mut quiet = base(r#"{"text":"a"}"#);
    quiet.narration = false;
    assert!(convert(&quiet).unwrap().timeline.scenes[0].narration.is_none());
    let (_, ja, _) = EXAMPLES[2];
    let n = convert(&sb(ja)).unwrap().timeline.scenes[0].narration.clone().unwrap();
    assert_eq!(n.voice.as_deref(), Some("Kyoko"));
}

#[test]
fn music_is_mapped() {
    let mut s = base(r#"{"text":"a","duration_ms":10000}"#);
    s.music = Some("assets/bgm.mp3".into());
    let m = convert(&s).unwrap().timeline.music.unwrap();
    assert_eq!(m.source, "assets/bgm.mp3");
}

#[test]
fn errors_name_scene_and_field() {
    assert!(err(&base(r#"{"text":"ok"},{"text":"  "}"#)).contains("scene 2: field `text`"));
    assert!(err(&base(r#"{"text":"ok","duration_ms":0}"#)).contains("scene 1: field `duration_ms`"));
    assert!(err(&base(r#"{"text":"ok","image":"../secret.png"}"#)).contains("scene 1: field `image`"));
    assert!(err(&base(r#"{"text":"ok"},{"text":"ok","image":"/etc/x.png"}"#)).contains("scene 2: field `image`"));
    assert!(err(&base(r#"{"text":"ok","duration_ms":999999}"#)).contains("scene 1: field `duration_ms`"));
}

#[test]
fn empty_inputs_are_errors() {
    let empty = sb(r#"{"title":"t","language":"zh","scenes":[]}"#);
    assert_eq!(convert(&empty).unwrap_err(), StoryboardError::NoScenes);
    let untitled = sb(r#"{"title":" ","language":"zh","scenes":[{"text":"a"}]}"#);
    assert_eq!(convert(&untitled).unwrap_err(), StoryboardError::EmptyTitle);
    let mut bad_music = base(r#"{"text":"a"}"#);
    bad_music.music = Some("https://x.test/a.mp3".into());
    assert!(matches!(convert(&bad_music), Err(StoryboardError::Music(_))));
}

#[test]
fn total_over_limit_is_an_error() {
    let scenes = [r#"{"text":"a","duration_ms":60000}"#; 4].join(",");
    match convert(&base(&scenes)) {
        Err(StoryboardError::TooLong { total_ms }) => assert_eq!(total_ms, 240_000),
        other => panic!("{other:?}"),
    }
    let exactly = [r#"{"text":"a","duration_ms":50000}"#; 4].join(",");
    assert!(convert(&base(&exactly)).is_ok());
}

#[test]
fn unknown_fields_and_values_fail_to_parse() {
    assert!(serde_json::from_str::<Storyboard>(
        r#"{"title":"t","language":"fr","scenes":[]}"#
    )
    .is_err());
    assert!(serde_json::from_str::<Storyboard>(
        r#"{"title":"t","language":"en","scenes":[{"text":"a","duraton_ms":1}]}"#
    )
    .is_err());
}

#[test]
fn storyboard_to_timeline_wrapper_matches_convert() {
    let s = base(r#"{"text":"a"}"#);
    assert_eq!(storyboard_to_timeline(&s).unwrap(), convert(&s).unwrap().timeline);
}
