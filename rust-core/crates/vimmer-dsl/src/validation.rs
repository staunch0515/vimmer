use std::collections::HashSet;

use thiserror::Error;

use crate::layer::{
    is_hex_color, Layer, LayerContent, ShapeKind, MAX_LAYERS, MAX_LAYER_KEYFRAMES,
    MAX_LAYER_TEXT_CHARS, MAX_TYPING_CPS,
};
use crate::model::{
    Keyframe, Scene, SubtitleCue, VideoTimeline, MAX_DURATION_MS, MAX_FRAMES, TARGET_FPS,
};

const ALLOWED_RESOLUTIONS: [(u32, u32); 2] = [(1920, 1080), (1080, 1920)];

#[derive(Debug, Clone, PartialEq, Error)]
pub enum ValidationError {
    #[error("timeline has no scenes")]
    NoScenes,
    #[error("unsupported fps {0}; expected {TARGET_FPS}")]
    UnsupportedFps(u32),
    #[error("unsupported resolution {0}x{1}; expected 1920x1080 or 1080x1920")]
    UnsupportedResolution(u32, u32),
    #[error("duplicate scene id `{0}`")]
    DuplicateSceneId(String),
    #[error("scene `{0}` has zero duration")]
    ZeroDuration(String),
    #[error("scene `{0}` has no text, image or layers")]
    EmptyScene(String),
    #[error("scene `{0}` has an empty audio source")]
    EmptyAudioSource(String),
    #[error("timeline is {0} ms; maximum is {MAX_DURATION_MS} ms")]
    TooLong(u64),
    #[error("timeline is {0} frames; maximum is {MAX_FRAMES} frames")]
    TooManyFrames(u32),
    #[error("subtitle {index} must have start < end (got {start_ms}..{end_ms} ms)")]
    BadSubtitleRange {
        index: usize,
        start_ms: u64,
        end_ms: u64,
    },
    #[error("subtitle {index} ends at {end_ms} ms, after the timeline ends at {total_ms} ms")]
    SubtitleOutOfRange {
        index: usize,
        end_ms: u64,
        total_ms: u64,
    },
    #[error("subtitle {0} has empty text")]
    EmptySubtitle(usize),
    #[error("subtitle {cue} word {word} ({start_ms}..{end_ms} ms) is outside the cue ({cue_start_ms}..{cue_end_ms} ms) or ends before it starts")]
    WordOutOfCue {
        cue: usize,
        word: usize,
        start_ms: u64,
        end_ms: u64,
        cue_start_ms: u64,
        cue_end_ms: u64,
    },
    #[error("subtitle {cue} word {word} starts before the previous word ends; words must be in time order without overlap")]
    WordsNotMonotonic { cue: usize, word: usize },
    #[error("subtitle {cue} word {word} has empty text")]
    EmptyWord { cue: usize, word: usize },
    #[error("music has an empty source")]
    EmptyMusicSource,
    #[error("music volume {0} must be between 0 and 1")]
    BadMusicVolume(f32),
    #[error("music fades ({fade_in_ms} ms in, {fade_out_ms} ms out) are longer than the timeline")]
    MusicFadesTooLong { fade_in_ms: u64, fade_out_ms: u64 },
    #[error("music duck depth {0} dB must be between -40 and 0")]
    BadDuckDepth(f32),
    #[error("music duck attack ({attack_ms} ms) and release ({release_ms} ms) must each be between 10 and 5000 ms")]
    BadDuckRamp { attack_ms: u64, release_ms: u64 },
    #[error("scene `{0}` is the first scene, so it cannot have a transition_in")]
    TransitionOnFirstScene(String),
    #[error("scene `{0}` has a zero-length transition")]
    ZeroTransition(String),
    #[error(
        "scene `{scene}` transition is {transition_ms} ms, longer than the scene ({scene_ms} ms)"
    )]
    TransitionTooLong {
        scene: String,
        transition_ms: u64,
        scene_ms: u64,
    },
    #[error(
        "scene `{scene}` keyframe {index} is at {at_ms} ms, after the scene ends at {scene_ms} ms"
    )]
    KeyframeOutOfScene {
        scene: String,
        index: usize,
        at_ms: u64,
        scene_ms: u64,
    },
    #[error("scene `{scene}` keyframes must be sorted by at_ms (keyframe {index} is earlier than the one before it)")]
    KeyframesNotSorted { scene: String, index: usize },
    #[error("scene `{scene}` has two keyframes at {at_ms} ms")]
    DuplicateKeyframe { scene: String, at_ms: u64 },
    #[error(
        "scene `{scene}` keyframe {index} has scale {value}; scale must be a finite number above 0"
    )]
    BadKeyframeScale {
        scene: String,
        index: usize,
        value: f32,
    },
    #[error(
        "scene `{scene}` keyframe {index} has opacity {value}; opacity must be between 0 and 1"
    )]
    BadKeyframeOpacity {
        scene: String,
        index: usize,
        value: f32,
    },
    #[error("scene `{scene}` keyframe {index} has a non-finite x, y or rotation")]
    NonFiniteKeyframe { scene: String, index: usize },
    #[error("scene `{scene}` has {count} layers; the limit is {MAX_LAYERS}")]
    TooManyLayers { scene: String, count: usize },
    #[error("scene `{scene}` has two layers with id `{layer}`")]
    DuplicateLayerId { scene: String, layer: String },
    #[error("scene `{scene}` layer `{layer}`: {message}")]
    BadLayer {
        scene: String,
        layer: String,
        message: String,
    },
}

fn validate_transition(scene: &Scene, index: usize, errors: &mut Vec<ValidationError>) {
    let Some(transition) = &scene.transition_in else {
        return;
    };
    if index == 0 {
        errors.push(ValidationError::TransitionOnFirstScene(scene.id.clone()));
    }
    if transition.duration_ms == 0 {
        errors.push(ValidationError::ZeroTransition(scene.id.clone()));
    } else if transition.duration_ms > scene.duration_ms {
        errors.push(ValidationError::TransitionTooLong {
            scene: scene.id.clone(),
            transition_ms: transition.duration_ms,
            scene_ms: scene.duration_ms,
        });
    }
}

fn validate_keyframes(scene: &Scene, errors: &mut Vec<ValidationError>) {
    for (index, kf) in scene.keyframes.iter().enumerate() {
        if kf.at_ms > scene.duration_ms {
            errors.push(ValidationError::KeyframeOutOfScene {
                scene: scene.id.clone(),
                index,
                at_ms: kf.at_ms,
                scene_ms: scene.duration_ms,
            });
        }
        if !(kf.scale.is_finite() && kf.scale > 0.0) {
            errors.push(ValidationError::BadKeyframeScale {
                scene: scene.id.clone(),
                index,
                value: kf.scale,
            });
        }
        if !(0.0..=1.0).contains(&kf.opacity) {
            errors.push(ValidationError::BadKeyframeOpacity {
                scene: scene.id.clone(),
                index,
                value: kf.opacity,
            });
        }
        if !(kf.x.is_finite() && kf.y.is_finite() && kf.rotation.is_finite()) {
            errors.push(ValidationError::NonFiniteKeyframe {
                scene: scene.id.clone(),
                index,
            });
        }
        if index > 0 {
            let prev = &scene.keyframes[index - 1];
            if kf.at_ms < prev.at_ms {
                errors.push(ValidationError::KeyframesNotSorted {
                    scene: scene.id.clone(),
                    index,
                });
            } else if kf.at_ms == prev.at_ms {
                errors.push(ValidationError::DuplicateKeyframe {
                    scene: scene.id.clone(),
                    at_ms: kf.at_ms,
                });
            }
        }
    }
}

/// Why an asset path is not a plain relative path inside the public dir, if it is not. The same
/// rules as the edit assistant's `check_asset_path`, without the file system part.
pub fn asset_path_problem(p: &str) -> Option<&'static str> {
    if p.trim().is_empty() {
        return Some("the path is empty");
    }
    if p.trim() != p {
        return Some("the path has leading or trailing spaces");
    }
    if p.chars().any(char::is_control) || p.contains('\\') || p.contains('%') {
        return Some("the path has control characters, `\\` or `%`");
    }
    if p.starts_with('/') || p.starts_with('~') {
        return Some("the path must be relative to the public dir (no leading `/` or `~`)");
    }
    if p.split('/').next().is_some_and(|head| head.contains(':')) {
        return Some("URLs and schemes are not allowed; use a file inside public/");
    }
    if p.split('/')
        .any(|seg| seg.is_empty() || seg == "." || seg == "..")
    {
        return Some("`.`, `..` and empty segments are not allowed");
    }
    None
}

fn in_range(v: f32, lo: f32, hi: f32) -> bool {
    v.is_finite() && (lo..=hi).contains(&v)
}

/// Every problem of one layer, as messages.
fn layer_problems(layer: &Layer, scene_ms: u64) -> Vec<String> {
    let mut out = Vec::new();
    let id_ok = !layer.id.is_empty()
        && layer.id.len() <= 64
        && layer
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !id_ok {
        out.push("the id must be 1-64 letters, digits, `-` or `_`".to_string());
    }
    if !in_range(layer.x, -1.0, 2.0) || !in_range(layer.y, -1.0, 2.0) {
        out.push(format!(
            "x and y must be between -1 and 2 (fractions of the frame), got {} and {}",
            layer.x, layer.y
        ));
    }
    let is_line = matches!(&layer.content, LayerContent::Shape(s) if s.shape == ShapeKind::Line);
    let size_ok = |v: f32| {
        if is_line {
            in_range(v, 0.0, 4.0)
        } else {
            v.is_finite() && v > 0.0 && v <= 4.0
        }
    };
    if !size_ok(layer.width) {
        out.push(format!(
            "width must be above 0 and at most 4 (fractions of the frame), got {}",
            layer.width
        ));
    }
    match layer.height {
        Some(h) if !size_ok(h) => out.push(format!(
            "height must be above 0 and at most 4 (fractions of the frame), got {h}"
        )),
        None if !matches!(layer.content, LayerContent::Text(_)) => {
            out.push(format!("a {} layer needs a height", layer.content.kind()))
        }
        _ => {}
    }
    if is_line && layer.width == 0.0 && layer.height == Some(0.0) {
        out.push("a line needs a width or a height above 0".to_string());
    }
    if !in_range(layer.opacity, 0.0, 1.0) {
        out.push(format!(
            "opacity must be between 0 and 1, got {}",
            layer.opacity
        ));
    }
    if !in_range(layer.rotation, -360.0, 360.0) {
        out.push(format!(
            "rotation must be within ±360 degrees, got {}",
            layer.rotation
        ));
    }
    let end = layer.end_ms_in(scene_ms);
    if layer.start_ms >= end || end > scene_ms {
        out.push(format!(
            "its time must satisfy start_ms < end_ms <= the scene length ({scene_ms} ms), got {}..{end} ms",
            layer.start_ms
        ));
    }
    let span = end.saturating_sub(layer.start_ms);
    let enter = layer.enter.map_or(0, |a| a.duration_ms);
    let exit = layer.exit.map_or(0, |a| a.duration_ms);
    if layer.enter.is_some_and(|a| a.duration_ms == 0)
        || layer.exit.is_some_and(|a| a.duration_ms == 0)
    {
        out.push("enter and exit animations need a duration above 0".to_string());
    } else if enter + exit > span {
        out.push(format!(
            "enter ({enter} ms) and exit ({exit} ms) animations are longer than the layer is shown ({span} ms)"
        ));
    }
    out.extend(layer_keyframe_problems(&layer.keyframes, scene_ms));
    let color = |what: &str, c: &str, out: &mut Vec<String>| {
        if !is_hex_color(c) {
            out.push(format!(
                "{what} `{c}` must be a color like #rgb, #rrggbb or #rrggbbaa"
            ));
        }
    };
    match &layer.content {
        LayerContent::Image(img) => {
            if let Some(p) = asset_path_problem(&img.src) {
                out.push(format!("src: {p}"));
            }
            if !in_range(img.corner_radius, 0.0, 0.5) {
                out.push("corner_radius must be between 0 and 0.5".to_string());
            }
        }
        LayerContent::Icon(icon) => {
            if let Some(p) = asset_path_problem(&icon.src) {
                out.push(format!("src: {p}"));
            } else if !icon.src.to_ascii_lowercase().ends_with(".svg") {
                out.push("an icon's src must be an .svg file".to_string());
            }
            color("color", &icon.color, &mut out);
        }
        LayerContent::Text(t) => {
            let n = t.text.chars().count();
            if t.text.trim().is_empty() {
                out.push("the text is empty".to_string());
            } else if n > MAX_LAYER_TEXT_CHARS {
                out.push(format!(
                    "the text is {n} characters; the limit is {MAX_LAYER_TEXT_CHARS}"
                ));
            }
            if !(t.font_size.is_finite() && t.font_size > 0.0 && t.font_size <= 0.5) {
                out.push(format!(
                    "font_size must be above 0 and at most 0.5 (fraction of the frame height), got {}",
                    t.font_size
                ));
            }
            if !(100..=900).contains(&t.weight) || t.weight % 100 != 0 {
                out.push(format!(
                    "weight must be 100, 200, ... 900, got {}",
                    t.weight
                ));
            }
            if !in_range(t.line_height, 0.8, 3.0) {
                out.push("line_height must be between 0.8 and 3".to_string());
            }
            color("color", &t.color, &mut out);
            if let Some(bg) = &t.background {
                color("background color", &bg.color, &mut out);
                if !in_range(bg.padding, 0.0, 3.0) || !in_range(bg.radius, 0.0, 3.0) {
                    out.push(
                        "background padding and radius must be between 0 and 3 (font sizes)"
                            .to_string(),
                    );
                }
            }
            if t.highlights.iter().any(|h| h.trim().is_empty()) {
                out.push("a highlight is empty".to_string());
            }
            if let Some(cps) = t.typing_cps {
                if !(cps.is_finite() && (1.0..=MAX_TYPING_CPS).contains(&cps)) {
                    out.push(format!(
                        "typing_cps must be between 1 and {MAX_TYPING_CPS} characters per second, got {cps}"
                    ));
                }
            }
        }
        LayerContent::Shape(sh) => {
            if let Some(c) = &sh.fill {
                color("fill", c, &mut out);
            }
            if let Some(c) = &sh.stroke {
                color("stroke", c, &mut out);
            }
            if sh.fill.is_none() && sh.stroke.is_none() {
                out.push("a shape needs a fill or a stroke color".to_string());
            }
            if sh.shape == ShapeKind::Line && sh.stroke.is_none() {
                out.push("a line needs a stroke color".to_string());
            }
            if !in_range(sh.stroke_width, 0.0, 0.1) {
                out.push("stroke_width must be between 0 and 0.1".to_string());
            }
            if !in_range(sh.corner_radius, 0.0, 0.5) {
                out.push("corner_radius must be between 0 and 0.5".to_string());
            }
        }
    }
    out
}

/// The scene keyframe rules, for a layer's keyframes (times are from the scene start).
fn layer_keyframe_problems(keyframes: &[Keyframe], scene_ms: u64) -> Vec<String> {
    let mut out = Vec::new();
    if keyframes.len() > MAX_LAYER_KEYFRAMES {
        out.push(format!(
            "it has {} keyframes; the limit is {MAX_LAYER_KEYFRAMES}",
            keyframes.len()
        ));
    }
    for (i, kf) in keyframes.iter().enumerate() {
        if kf.at_ms > scene_ms {
            out.push(format!(
                "keyframe {i} is at {} ms, after the scene ends ({scene_ms} ms)",
                kf.at_ms
            ));
        }
        if i > 0 && kf.at_ms <= keyframes[i - 1].at_ms {
            out.push(format!(
                "keyframe {i} must be later than the one before it (sorted, no duplicates)"
            ));
        }
        if !(kf.scale.is_finite() && kf.scale > 0.0 && kf.scale <= 10.0) {
            out.push(format!("keyframe {i} scale must be above 0 and at most 10"));
        }
        if !in_range(kf.opacity, 0.0, 1.0) {
            out.push(format!("keyframe {i} opacity must be between 0 and 1"));
        }
        if !in_range(kf.x, -200.0, 200.0)
            || !in_range(kf.y, -200.0, 200.0)
            || !in_range(kf.rotation, -360.0, 360.0)
        {
            out.push(format!(
                "keyframe {i} x and y must be within ±200 (percent of the frame) and rotation within ±360"
            ));
        }
    }
    out
}

fn validate_layers(scene: &Scene, errors: &mut Vec<ValidationError>) {
    if scene.layers.len() > MAX_LAYERS {
        errors.push(ValidationError::TooManyLayers {
            scene: scene.id.clone(),
            count: scene.layers.len(),
        });
    }
    let mut ids = HashSet::new();
    for layer in &scene.layers {
        if !ids.insert(layer.id.as_str()) {
            errors.push(ValidationError::DuplicateLayerId {
                scene: scene.id.clone(),
                layer: layer.id.clone(),
            });
        }
        for message in layer_problems(layer, scene.duration_ms) {
            errors.push(ValidationError::BadLayer {
                scene: scene.id.clone(),
                layer: layer.id.clone(),
                message,
            });
        }
    }
}

/// Word timings must sit inside their cue, in order, without overlapping. Zero-length words are
/// allowed: chunking clamps words outside a chunk to the chunk edge, which keeps them in order.
fn validate_words(cue: &SubtitleCue, index: usize, errors: &mut Vec<ValidationError>) {
    let mut prev_end = cue.start_ms;
    for (word, w) in cue.words.iter().enumerate() {
        if w.text.trim().is_empty() {
            errors.push(ValidationError::EmptyWord { cue: index, word });
        }
        if w.start_ms > w.end_ms || w.start_ms < cue.start_ms || w.end_ms > cue.end_ms {
            errors.push(ValidationError::WordOutOfCue {
                cue: index,
                word,
                start_ms: w.start_ms,
                end_ms: w.end_ms,
                cue_start_ms: cue.start_ms,
                cue_end_ms: cue.end_ms,
            });
        } else if word > 0 && w.start_ms < prev_end {
            errors.push(ValidationError::WordsNotMonotonic { cue: index, word });
        }
        prev_end = prev_end.max(w.end_ms);
    }
}

/// Checks a timeline against the DSL rules. Returns every violation found,
/// not just the first, so callers can report them all at once.
pub fn validate(timeline: &VideoTimeline) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();

    if timeline.scenes.is_empty() {
        errors.push(ValidationError::NoScenes);
    }
    if timeline.fps != TARGET_FPS {
        errors.push(ValidationError::UnsupportedFps(timeline.fps));
    }
    let res = (timeline.resolution.width, timeline.resolution.height);
    if !ALLOWED_RESOLUTIONS.contains(&res) {
        errors.push(ValidationError::UnsupportedResolution(res.0, res.1));
    }

    let mut seen = HashSet::new();
    for (i, scene) in timeline.scenes.iter().enumerate() {
        if !seen.insert(scene.id.as_str()) {
            errors.push(ValidationError::DuplicateSceneId(scene.id.clone()));
        }
        if scene.duration_ms == 0 {
            errors.push(ValidationError::ZeroDuration(scene.id.clone()));
        }
        let has_text = scene.text.as_deref().is_some_and(|t| !t.trim().is_empty());
        let has_image = scene.image.as_deref().is_some_and(|i| !i.trim().is_empty());
        if !has_text && !has_image && scene.layers.is_empty() {
            errors.push(ValidationError::EmptyScene(scene.id.clone()));
        }
        if let Some(audio) = &scene.audio {
            if audio.source.trim().is_empty() {
                errors.push(ValidationError::EmptyAudioSource(scene.id.clone()));
            }
        }
        validate_transition(scene, i, &mut errors);
        validate_keyframes(scene, &mut errors);
        validate_layers(scene, &mut errors);
    }

    let total_ms = timeline.total_duration_ms();
    if total_ms > MAX_DURATION_MS {
        errors.push(ValidationError::TooLong(total_ms));
    }
    let total_frames = timeline.total_frames();
    if total_frames > MAX_FRAMES {
        errors.push(ValidationError::TooManyFrames(total_frames));
    }

    for (index, cue) in timeline.subtitles.iter().enumerate() {
        if cue.start_ms >= cue.end_ms {
            errors.push(ValidationError::BadSubtitleRange {
                index,
                start_ms: cue.start_ms,
                end_ms: cue.end_ms,
            });
        } else if cue.end_ms > total_ms {
            errors.push(ValidationError::SubtitleOutOfRange {
                index,
                end_ms: cue.end_ms,
                total_ms,
            });
        }
        if cue.text.trim().is_empty() {
            errors.push(ValidationError::EmptySubtitle(index));
        }
        validate_words(cue, index, &mut errors);
    }

    if let Some(music) = &timeline.music {
        if music.source.trim().is_empty() {
            errors.push(ValidationError::EmptyMusicSource);
        }
        if !(0.0..=1.0).contains(&music.volume) {
            errors.push(ValidationError::BadMusicVolume(music.volume));
        }
        if music.fade_in_ms + music.fade_out_ms > total_ms {
            errors.push(ValidationError::MusicFadesTooLong {
                fade_in_ms: music.fade_in_ms,
                fade_out_ms: music.fade_out_ms,
            });
        }
        let duck = &music.duck;
        if !(-40.0..=0.0).contains(&duck.depth_db) {
            errors.push(ValidationError::BadDuckDepth(duck.depth_db));
        }
        let ramp = 10..=5_000;
        if !ramp.contains(&duck.attack_ms) || !ramp.contains(&duck.release_ms) {
            errors.push(ValidationError::BadDuckRamp {
                attack_ms: duck.attack_ms,
                release_ms: duck.release_ms,
            });
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{MotionType, Resolution, Scene, SubtitleCue, WordTiming};

    fn scene(id: &str, ms: u64) -> Scene {
        Scene {
            id: id.into(),
            duration_ms: ms,
            text: Some("hello".into()),
            highlights: vec![],
            image: None,
            motion: MotionType::Static,
            audio: None,
            narration: None,
            transition_in: None,
            keyframes: vec![],
            layers: vec![],
        }
    }

    fn timeline(scenes: Vec<Scene>) -> VideoTimeline {
        VideoTimeline {
            fps: 30,
            resolution: Resolution {
                width: 1920,
                height: 1080,
            },
            scenes,
            subtitles: vec![],
            music: None,
        }
    }

    #[test]
    fn valid_timeline_passes() {
        assert!(validate(&timeline(vec![scene("a", 15_000)])).is_ok());
    }

    #[test]
    fn reports_all_errors_at_once() {
        let mut bad = scene("dup", 0);
        bad.text = None;
        let t = VideoTimeline {
            fps: 24,
            resolution: Resolution {
                width: 640,
                height: 480,
            },
            scenes: vec![bad, scene("dup", 1000)],
            subtitles: vec![],
            music: None,
        };
        let errs = validate(&t).unwrap_err();
        assert!(errs.contains(&ValidationError::UnsupportedFps(24)));
        assert!(errs.contains(&ValidationError::UnsupportedResolution(640, 480)));
        assert!(errs.contains(&ValidationError::DuplicateSceneId("dup".into())));
        assert!(errs.contains(&ValidationError::ZeroDuration("dup".into())));
        assert!(errs.contains(&ValidationError::EmptyScene("dup".into())));
    }

    #[test]
    fn rejects_over_200_seconds() {
        let t = timeline(vec![scene("a", 120_000), scene("b", 100_000)]);
        let errs = validate(&t).unwrap_err();
        assert!(errs.contains(&ValidationError::TooLong(220_000)));
        assert!(errs.contains(&ValidationError::TooManyFrames(6_600)));
    }

    #[test]
    fn exactly_200_seconds_is_allowed() {
        let t = timeline(vec![scene("a", 200_000)]);
        assert!(validate(&t).is_ok());
    }

    #[test]
    fn checks_subtitle_cues() {
        let mut t = timeline(vec![scene("a", 5_000)]);
        t.subtitles = vec![
            SubtitleCue {
                start_ms: 2_000,
                end_ms: 1_000,
                text: "backwards".into(),
                words: Vec::new(),
            },
            SubtitleCue {
                start_ms: 4_000,
                end_ms: 6_000,
                text: "too late".into(),
                words: Vec::new(),
            },
            SubtitleCue {
                start_ms: 0,
                end_ms: 1_000,
                text: "  ".into(),
                words: Vec::new(),
            },
        ];
        let errs = validate(&t).unwrap_err();
        assert!(errs.contains(&ValidationError::BadSubtitleRange {
            index: 0,
            start_ms: 2_000,
            end_ms: 1_000
        }));
        assert!(errs.contains(&ValidationError::SubtitleOutOfRange {
            index: 1,
            end_ms: 6_000,
            total_ms: 5_000
        }));
        assert!(errs.contains(&ValidationError::EmptySubtitle(2)));
    }

    fn word(text: &str, start_ms: u64, end_ms: u64) -> WordTiming {
        WordTiming {
            text: text.into(),
            start_ms,
            end_ms,
        }
    }

    #[test]
    fn valid_words_pass_including_zero_length_edges() {
        let mut t = timeline(vec![scene("a", 5_000)]);
        let mut cue = SubtitleCue::new(1_000, 3_000, "你好 world");
        cue.words = vec![
            word("你", 1_000, 1_000),
            word("好 ", 1_000, 1_400),
            word("world", 1_500, 3_000),
        ];
        t.subtitles = vec![cue];
        assert_eq!(validate(&t), Ok(()));
    }

    #[test]
    fn words_that_do_not_spell_the_text_are_still_valid() {
        // An edited cue keeps its old words until they are timed again; the renderer draws it as
        // a plain line. Validation must not reject such a timeline.
        let mut t = timeline(vec![scene("a", 2000)]);
        let mut cue = SubtitleCue::new(0, 1000, "new text");
        cue.words = vec![WordTiming {
            text: "old ".into(),
            start_ms: 0,
            end_ms: 500,
        }];
        t.subtitles = vec![cue];
        assert_eq!(validate(&t), Ok(()));
    }

    #[test]
    fn words_must_sit_inside_their_cue_in_order() {
        let mut t = timeline(vec![scene("a", 5_000)]);
        let mut cue = SubtitleCue::new(1_000, 3_000, "a b c d e");
        cue.words = vec![
            word("a", 900, 1_200),
            word("b", 1_200, 1_600),
            word("c", 1_500, 1_800),
            word("d", 2_000, 1_900),
            word(" ", 2_900, 3_100),
        ];
        t.subtitles = vec![cue];
        let errs = validate(&t).unwrap_err();
        assert!(errs.iter().any(|e| matches!(
            e,
            ValidationError::WordOutOfCue {
                cue: 0,
                word: 0,
                ..
            }
        )));
        assert!(errs.contains(&ValidationError::WordsNotMonotonic { cue: 0, word: 2 }));
        assert!(errs
            .iter()
            .any(|e| matches!(e, ValidationError::WordOutOfCue { word: 3, .. })));
        assert!(errs
            .iter()
            .any(|e| matches!(e, ValidationError::WordOutOfCue { word: 4, .. })));
        assert!(errs.contains(&ValidationError::EmptyWord { cue: 0, word: 4 }));
        assert!(!errs.contains(&ValidationError::WordsNotMonotonic { cue: 0, word: 1 }));
    }

    #[test]
    fn checks_music() {
        use crate::model::Music;
        let mut t = timeline(vec![scene("a", 5_000)]);
        t.music = Some(Music {
            source: " ".into(),
            volume: 1.5,
            fade_in_ms: 4_000,
            fade_out_ms: 4_000,
            duck: Default::default(),
        });
        let errs = validate(&t).unwrap_err();
        assert!(errs.contains(&ValidationError::EmptyMusicSource));
        assert!(errs.contains(&ValidationError::BadMusicVolume(1.5)));
        assert!(errs.contains(&ValidationError::MusicFadesTooLong {
            fade_in_ms: 4_000,
            fade_out_ms: 4_000
        }));
        t.music = Some(Music {
            source: "assets/m.mp3".into(),
            volume: 0.3,
            fade_in_ms: 500,
            fade_out_ms: 500,
            duck: Default::default(),
        });
        assert!(validate(&t).is_ok());
    }

    #[test]
    fn checks_duck_ranges() {
        use crate::model::{Duck, Music};
        let mut t = timeline(vec![scene("a", 5_000)]);
        let music = |duck: Duck| Music {
            source: "assets/m.mp3".into(),
            volume: 0.3,
            fade_in_ms: 0,
            fade_out_ms: 0,
            duck,
        };
        for duck in [
            Duck::default(),
            Duck {
                enabled: false,
                ..Duck::default()
            },
            Duck {
                depth_db: 0.0,
                attack_ms: 10,
                release_ms: 5_000,
                ..Duck::default()
            },
            Duck {
                depth_db: -40.0,
                ..Duck::default()
            },
        ] {
            t.music = Some(music(duck));
            assert!(validate(&t).is_ok(), "{duck:?}");
        }
        t.music = Some(music(Duck {
            depth_db: 3.0,
            ..Duck::default()
        }));
        assert_eq!(
            validate(&t).unwrap_err(),
            vec![ValidationError::BadDuckDepth(3.0)]
        );
        t.music = Some(music(Duck {
            depth_db: f32::NAN,
            ..Duck::default()
        }));
        assert!(matches!(
            validate(&t).unwrap_err()[0],
            ValidationError::BadDuckDepth(_)
        ));
        t.music = Some(music(Duck {
            attack_ms: 0,
            release_ms: 6_000,
            ..Duck::default()
        }));
        assert_eq!(
            validate(&t).unwrap_err(),
            vec![ValidationError::BadDuckRamp {
                attack_ms: 0,
                release_ms: 6_000
            }]
        );
    }

    #[test]
    fn rejects_empty_timeline() {
        let errs = validate(&timeline(vec![])).unwrap_err();
        assert_eq!(errs, vec![ValidationError::NoScenes]);
    }

    fn kf(at_ms: u64) -> crate::model::Keyframe {
        crate::model::Keyframe {
            at_ms,
            scale: 1.0,
            x: 0.0,
            y: 0.0,
            opacity: 1.0,
            rotation: 0.0,
            ease: crate::model::Easing::EaseInOut,
        }
    }

    fn fade(ms: u64) -> Option<crate::model::Transition> {
        Some(crate::model::Transition {
            kind: crate::model::TransitionKind::Fade,
            duration_ms: ms,
        })
    }

    #[test]
    fn transition_on_first_scene_is_rejected() {
        let mut first = scene("a", 5_000);
        first.transition_in = fade(500);
        let errs = validate(&timeline(vec![first])).unwrap_err();
        assert_eq!(
            errs,
            vec![ValidationError::TransitionOnFirstScene("a".into())]
        );
    }

    #[test]
    fn transition_on_later_scene_is_allowed() {
        let mut second = scene("b", 5_000);
        second.transition_in = fade(500);
        assert!(validate(&timeline(vec![scene("a", 5_000), second])).is_ok());
    }

    #[test]
    fn transition_longer_than_its_scene_is_rejected() {
        let mut second = scene("b", 1_000);
        second.transition_in = fade(1_500);
        let errs = validate(&timeline(vec![scene("a", 5_000), second])).unwrap_err();
        assert_eq!(
            errs,
            vec![ValidationError::TransitionTooLong {
                scene: "b".into(),
                transition_ms: 1_500,
                scene_ms: 1_000,
            }]
        );
    }

    #[test]
    fn transition_equal_to_scene_length_is_allowed() {
        let mut second = scene("b", 1_000);
        second.transition_in = fade(1_000);
        assert!(validate(&timeline(vec![scene("a", 5_000), second])).is_ok());
    }

    #[test]
    fn zero_length_transition_is_rejected() {
        let mut second = scene("b", 1_000);
        second.transition_in = fade(0);
        let errs = validate(&timeline(vec![scene("a", 5_000), second])).unwrap_err();
        assert_eq!(errs, vec![ValidationError::ZeroTransition("b".into())]);
    }

    #[test]
    fn keyframes_must_be_sorted_without_duplicates() {
        let mut a = scene("a", 4_000);
        a.keyframes = vec![kf(0), kf(3_000), kf(1_000)];
        let errs = validate(&timeline(vec![a])).unwrap_err();
        assert_eq!(
            errs,
            vec![ValidationError::KeyframesNotSorted {
                scene: "a".into(),
                index: 2,
            }]
        );

        let mut b = scene("b", 4_000);
        b.keyframes = vec![kf(1_000), kf(1_000)];
        let errs = validate(&timeline(vec![b])).unwrap_err();
        assert_eq!(
            errs,
            vec![ValidationError::DuplicateKeyframe {
                scene: "b".into(),
                at_ms: 1_000,
            }]
        );
    }

    #[test]
    fn keyframe_past_scene_end_is_rejected_and_end_is_allowed() {
        let mut a = scene("a", 4_000);
        a.keyframes = vec![kf(0), kf(4_000), kf(4_001)];
        let errs = validate(&timeline(vec![a])).unwrap_err();
        assert_eq!(
            errs,
            vec![ValidationError::KeyframeOutOfScene {
                scene: "a".into(),
                index: 2,
                at_ms: 4_001,
                scene_ms: 4_000,
            }]
        );
    }

    #[test]
    fn keyframe_values_are_range_checked() {
        let mut a = scene("a", 4_000);
        let mut bad = kf(0);
        bad.scale = 0.0;
        let mut bad2 = kf(1_000);
        bad2.opacity = 1.5;
        let mut bad3 = kf(2_000);
        bad3.x = f32::NAN;
        a.keyframes = vec![bad, bad2, bad3];
        let errs = validate(&timeline(vec![a])).unwrap_err();
        assert!(errs.contains(&ValidationError::BadKeyframeScale {
            scene: "a".into(),
            index: 0,
            value: 0.0,
        }));
        assert!(errs.contains(&ValidationError::BadKeyframeOpacity {
            scene: "a".into(),
            index: 1,
            value: 1.5,
        }));
        assert!(errs.contains(&ValidationError::NonFiniteKeyframe {
            scene: "a".into(),
            index: 2,
        }));
    }

    #[test]
    fn valid_keyframes_and_transition_pass() {
        let mut a = scene("a", 4_000);
        a.keyframes = vec![kf(0), kf(2_000), kf(4_000)];
        let mut b = scene("b", 4_000);
        b.transition_in = fade(1_000);
        assert!(validate(&timeline(vec![a, b])).is_ok());
    }

    fn layer(json: &str) -> crate::layer::Layer {
        serde_json::from_str(json).unwrap()
    }

    fn layer_messages(errs: &[ValidationError], id: &str) -> Vec<String> {
        errs.iter()
            .filter_map(|e| match e {
                ValidationError::BadLayer { layer, message, .. } if layer == id => {
                    Some(message.clone())
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn valid_layers_pass_and_make_a_scene_non_empty() {
        let mut a = scene("a", 4_000);
        a.text = None;
        a.layers = vec![
            layer(
                r##"{"id":"t","kind":"text","x":0.1,"y":0.1,"width":0.8,"text":"Hello","background":{"color":"#000000aa"}}"##,
            ),
            layer(
                r##"{"id":"r","kind":"shape","x":0,"y":0.8,"width":1,"height":0.2,"fill":"#123","z":-1,"start_ms":500,"end_ms":4000,"enter":{"kind":"slide_up","duration_ms":300},"exit":{"kind":"fade"}}"##,
            ),
            layer(
                r##"{"id":"l","kind":"shape","shape":"line","x":0.1,"y":0.5,"width":0.8,"height":0,"stroke":"#fff"}"##,
            ),
            layer(
                r##"{"id":"i","kind":"icon","x":0.5,"y":0.5,"width":0.1,"height":0.1,"anchor":"center","src":"assets/icons/star.svg","keyframes":[{"at_ms":0,"scale":0.5},{"at_ms":1000,"rotation":90}]}"##,
            ),
            layer(
                r#"{"id":"p","kind":"image","x":0.6,"y":0.1,"width":0.3,"height":0.3,"src":"assets/sample.png","corner_radius":0.02}"#,
            ),
        ];
        assert_eq!(validate(&timeline(vec![a])), Ok(()));
    }

    #[test]
    fn layer_rules() {
        let mut a = scene("a", 2_000);
        a.layers = vec![
            layer(
                r##"{"id":"dup","kind":"shape","x":0,"y":0,"width":1,"height":1,"fill":"#000"}"##,
            ),
            layer(r##"{"id":"dup","kind":"shape","x":0,"y":0,"width":1,"height":1,"fill":"red"}"##),
            layer(
                r##"{"id":"time","kind":"shape","x":0,"y":0,"width":1,"height":1,"fill":"#000","start_ms":1500,"end_ms":3000}"##,
            ),
            layer(
                r##"{"id":"anim","kind":"shape","x":0,"y":0,"width":1,"height":1,"fill":"#000","start_ms":1000,"enter":{"kind":"pop","duration_ms":800},"exit":{"kind":"fade","duration_ms":800}}"##,
            ),
            layer(
                r#"{"id":"img","kind":"image","x":5,"y":0,"width":0,"height":1,"src":"../secret.png"}"#,
            ),
            layer(
                r#"{"id":"icon","kind":"icon","x":0,"y":0,"width":0.1,"height":0.1,"src":"assets/logo.png"}"#,
            ),
            layer(
                r#"{"id":"url","kind":"icon","x":0,"y":0,"width":0.1,"height":0.1,"src":"https://example.com/a.svg"}"#,
            ),
            layer(
                r#"{"id":"abs","kind":"image","x":0,"y":0,"width":0.1,"height":0.1,"src":"/etc/passwd"}"#,
            ),
            layer(r#"{"id":"nohead","kind":"image","x":0,"y":0,"width":0.1,"src":"a.png"}"#),
            layer(
                r#"{"id":"txt","kind":"text","x":0,"y":0,"width":0.5,"text":" ","font_size":0.9,"weight":650}"#,
            ),
            layer(
                r#"{"id":"kf","kind":"text","x":0,"y":0,"width":0.5,"text":"a","keyframes":[{"at_ms":500},{"at_ms":500},{"at_ms":9000,"scale":0}]}"#,
            ),
            layer(r#"{"id":"bad id!","kind":"text","x":0,"y":0,"width":0.5,"text":"a"}"#),
            layer(r#"{"id":"empty","kind":"shape","x":0,"y":0,"width":0.5,"height":0.5}"#),
            layer(
                r#"{"id":"type","kind":"text","x":0,"y":0,"width":0.5,"text":"a","reveal":"typewriter","typing_cps":0}"#,
            ),
        ];
        let errs = validate(&timeline(vec![a])).unwrap_err();
        assert!(errs.contains(&ValidationError::DuplicateLayerId {
            scene: "a".into(),
            layer: "dup".into()
        }));
        let has = |id: &str, needle: &str| {
            let m = layer_messages(&errs, id);
            assert!(
                m.iter().any(|m| m.contains(needle)),
                "{id}: no `{needle}` in {m:?}"
            );
        };
        has("dup", "red");
        has("time", "start_ms < end_ms");
        has("anim", "longer than the layer is shown");
        has("img", "x and y");
        has("img", "width");
        has("img", "`..`");
        has("icon", ".svg");
        has("url", "URLs");
        has("abs", "relative");
        has("nohead", "needs a height");
        has("txt", "empty");
        has("txt", "font_size");
        has("txt", "weight");
        has("kf", "later than");
        has("kf", "after the scene ends");
        has("kf", "scale");
        has("bad id!", "id must be");
        has("empty", "fill or a stroke");
        has("type", "typing_cps");
    }

    #[test]
    fn layer_cap() {
        let mut a = scene("a", 2_000);
        a.layers = (0..33)
            .map(|i| layer(&format!(r##"{{"id":"l{i}","kind":"shape","x":0,"y":0,"width":1,"height":1,"fill":"#000"}}"##)))
            .collect();
        let errs = validate(&timeline(vec![a])).unwrap_err();
        assert_eq!(
            errs,
            vec![ValidationError::TooManyLayers {
                scene: "a".into(),
                count: 33
            }]
        );
    }

    #[test]
    fn asset_paths() {
        assert_eq!(asset_path_problem("assets/icons/star.svg"), None);
        for bad in [
            "",
            " a.svg",
            "/a.svg",
            "~/a.svg",
            "a/../b.svg",
            "./a.svg",
            "a//b.svg",
            "C:/a.svg",
            "data:image/svg+xml,x",
            "a\\b.svg",
            "a%2e.svg",
        ] {
            assert!(asset_path_problem(bad).is_some(), "{bad}");
        }
    }
}
