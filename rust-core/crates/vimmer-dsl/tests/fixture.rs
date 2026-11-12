use std::path::PathBuf;

use vimmer_dsl::{validate, MotionType, VideoTimeline};

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/sample-timeline.json")
}

fn load_fixture() -> VideoTimeline {
    let raw = std::fs::read_to_string(fixture_path()).expect("fixture readable");
    serde_json::from_str(&raw).expect("fixture parses as VideoTimeline")
}

#[test]
fn sample_fixture_parses_and_validates() {
    let timeline = load_fixture();
    validate(&timeline).expect("fixture is valid");
}

#[test]
fn sample_fixture_is_a_15_second_zoom_scene() {
    let timeline = load_fixture();
    assert_eq!(timeline.total_duration_ms(), 15_000);
    assert_eq!(timeline.total_frames(), 450);
    assert_eq!(timeline.scenes[0].motion, MotionType::ZoomIn);
}

#[test]
fn fixture_round_trips_through_serde() {
    let timeline = load_fixture();
    let json = serde_json::to_string(&timeline).unwrap();
    let again: VideoTimeline = serde_json::from_str(&json).unwrap();
    assert_eq!(timeline, again);
}

#[test]
fn motion_demo_fixture_parses_and_validates() {
    let raw = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/motion-demo.json"),
    )
    .expect("motion demo readable");
    let timeline: VideoTimeline = serde_json::from_str(&raw).expect("motion demo parses");
    validate(&timeline).expect("motion demo is valid");
    assert_eq!(timeline.total_duration_ms(), 12_000);
    assert!(timeline.scenes[1].transition_in.is_some());
    assert!(!timeline.scenes[0].keyframes.is_empty());
    assert!(!timeline.scenes[2].keyframes.is_empty());
}
