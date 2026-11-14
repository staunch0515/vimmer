use std::collections::HashSet;

use thiserror::Error;

use crate::layer::{
    clip_problems, is_hex_color, Layer, LayerContent, ShapeKind, MAX_ABS_NUMBER, MAX_AFFIX_CHARS,
    MAX_BARS, MAX_BAR_LABEL_CHARS, MAX_DECIMALS, MAX_LAYERS, MAX_LAYER_KEYFRAMES,
    MAX_LAYER_TEXT_CHARS, MAX_LOWER_THIRD_SUBTITLE_CHARS, MAX_LOWER_THIRD_TITLE_CHARS,
    MAX_RING_LABEL_CHARS, MAX_TYPING_CPS, MAX_VIDEO_LAYERS,
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
    #[error("scene `{0}` has no text, image, video or layers")]
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
    #[error("scene `{0}` has both an image and a video; a scene has one background")]
    ImageAndVideo(String),
    #[error("scene `{scene}` video: {message}")]
    BadSceneVideo { scene: String, message: String },
    #[error("scene `{scene}` has {count} video layers; the limit is {MAX_VIDEO_LAYERS}")]
    TooManyVideoLayers { scene: String, count: usize },
    #[error("scene `{scene}` has two layers with id `{layer}`")]
    DuplicateLayerId { scene: String, layer: String },
    #[error("scene `{scene}` layer `{layer}`: {message}")]
    BadLayer {
        scene: String,
        layer: String,
        message: String,
    },
    #[error("scene `{scene}` template: {message}")]
    BadTemplate { scene: String, message: String },
    #[error("brand: {0}")]
    BadBrand(String),
    #[error("scene `{scene}` sfx {index}: {message}")]
    BadSfx {
        scene: String,
        index: usize,
        message: String,
    },
    #[error("scene `{scene}` has {count} sound effects; the limit is {MAX_SFX_PER_SCENE}")]
    TooManySfx { scene: String, count: usize },
    #[error("music beats: {0}")]
    BadBeatGrid(String),
    #[error("music beat_sync: {0}")]
    BadBeatSync(String),
}

/// Most sound effects one scene may have.
pub const MAX_SFX_PER_SCENE: usize = 16;
/// File types a sound effect may be.
pub const SFX_EXTENSIONS: [&str; 7] = ["wav", "mp3", "m4a", "aac", "ogg", "flac", "opus"];

/// Every problem of a scene's sound effects: path and type, one of `at_ms` / `on`, a time inside
/// the scene, an existing layer, volume. `(index, message)` pairs.
pub fn sfx_problems(scene: &Scene) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (index, fx) in scene.sfx.iter().enumerate() {
        let mut push = |m: String| out.push((index, m));
        if let Some(p) = asset_path_problem(&fx.src) {
            push(format!("src: {p}"));
        } else {
            let ext = fx
                .src
                .rsplit_once('.')
                .map(|(_, e)| e.to_ascii_lowercase())
                .unwrap_or_default();
            if !SFX_EXTENSIONS.contains(&ext.as_str()) {
                push(format!(
                    "src must be an audio file (.{})",
                    SFX_EXTENSIONS.join(", .")
                ));
            }
        }
        match (&fx.at_ms, &fx.on) {
            (Some(_), Some(_)) => push("give at_ms or on, not both".into()),
            (None, None) => {
                push("give at_ms (ms from the scene start) or on (a layer event)".into())
            }
            (Some(at), None) => {
                if *at >= scene.duration_ms {
                    push(format!(
                        "at_ms ({at}) must be inside the scene (0..{} ms)",
                        scene.duration_ms
                    ));
                }
            }
            (None, Some(on)) => match scene.layers.iter().find(|l| l.id == on.layer) {
                None => push(format!("on: the scene has no layer `{}`", on.layer)),
                Some(_) => {
                    let at = fx.time_in(scene).unwrap_or(0);
                    if at >= scene.duration_ms {
                        push(format!(
                            "on: layer `{}` {} at {at} ms, at or after the scene ends ({} ms); give it an end_ms or an exit animation",
                            on.layer,
                            match on.event {
                                crate::model::LayerEvent::Enter => "enters",
                                crate::model::LayerEvent::Exit => "exits",
                            },
                            scene.duration_ms
                        ));
                    }
                }
            },
        }
        if !in_range(fx.volume, 0.0, 1.0) {
            push(format!("volume {} must be between 0 and 1", fx.volume));
        }
    }
    out
}

fn validate_sfx(scene: &Scene, errors: &mut Vec<ValidationError>) {
    if scene.sfx.len() > MAX_SFX_PER_SCENE {
        errors.push(ValidationError::TooManySfx {
            scene: scene.id.clone(),
            count: scene.sfx.len(),
        });
    }
    for (index, message) in sfx_problems(scene) {
        errors.push(ValidationError::BadSfx {
            scene: scene.id.clone(),
            index,
            message,
        });
    }
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
        None if !layer.content.auto_height() => {
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
    let opt_color = |what: &str, c: &Option<String>, out: &mut Vec<String>| {
        if let Some(c) = c {
            color(what, c, out);
        }
    };
    match &layer.content {
        LayerContent::Video(clip) => out.extend(clip_problems(clip)),
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
            opt_color("color", &t.color, &mut out);
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
        LayerContent::LowerThird(lt) => {
            let n = lt.title.chars().count();
            if lt.title.trim().is_empty() {
                out.push("the title is empty".to_string());
            } else if n > MAX_LOWER_THIRD_TITLE_CHARS {
                out.push(format!(
                    "the title is {n} characters; the limit is {MAX_LOWER_THIRD_TITLE_CHARS}"
                ));
            }
            if let Some(sub) = &lt.subtitle {
                let n = sub.chars().count();
                if n > MAX_LOWER_THIRD_SUBTITLE_CHARS {
                    out.push(format!(
                        "the subtitle is {n} characters; the limit is {MAX_LOWER_THIRD_SUBTITLE_CHARS}"
                    ));
                }
            }
            if !(lt.font_size.is_finite() && lt.font_size > 0.0 && lt.font_size <= 0.2) {
                out.push(format!(
                    "font_size must be above 0 and at most 0.2 (fraction of the frame height), got {}",
                    lt.font_size
                ));
            }
            opt_color("color", &lt.color, &mut out);
            opt_color("subtitle_color", &lt.subtitle_color, &mut out);
            opt_color("accent_color", &lt.accent_color, &mut out);
            if let Some(c) = &lt.background_color {
                color("background_color", c, &mut out);
            }
        }
        LayerContent::Counter(c) => {
            for (name, v) in [("from", c.from), ("to", c.to)] {
                number(name, v, &mut out);
            }
            out.extend(number_format_problems(
                c.decimals,
                &c.prefix,
                &c.suffix,
                Some((&c.separator, &c.decimal_mark)),
            ));
            if let Some(ms) = c.duration_ms {
                if ms == 0 || ms > span {
                    out.push(format!(
                        "duration_ms must be above 0 and at most the layer's time ({span} ms), got {ms}"
                    ));
                }
            }
            if !(c.font_size.is_finite() && c.font_size > 0.0 && c.font_size <= 0.5) {
                out.push(format!(
                    "font_size must be above 0 and at most 0.5 (fraction of the frame height), got {}",
                    c.font_size
                ));
            }
            if !(100..=900).contains(&c.weight) || c.weight % 100 != 0 {
                out.push(format!(
                    "weight must be 100, 200, ... 900, got {}",
                    c.weight
                ));
            }
            opt_color("color", &c.color, &mut out);
        }
        LayerContent::BarChart(ch) => {
            if ch.bars.is_empty() || ch.bars.len() > MAX_BARS {
                out.push(format!(
                    "a chart needs 1 to {MAX_BARS} bars, got {}",
                    ch.bars.len()
                ));
            }
            for (i, bar) in ch.bars.iter().enumerate() {
                let n = bar.label.chars().count();
                if n > MAX_BAR_LABEL_CHARS {
                    out.push(format!(
                        "bar {i} label is {n} characters; the limit is {MAX_BAR_LABEL_CHARS}"
                    ));
                }
                if !(bar.value.is_finite() && (0.0..=MAX_ABS_NUMBER).contains(&bar.value)) {
                    out.push(format!(
                        "bar {i} value must be 0 or more (at most {MAX_ABS_NUMBER:e}), got {}",
                        bar.value
                    ));
                }
                if let Some(c) = &bar.color {
                    color(&format!("bar {i} color"), c, &mut out);
                }
            }
            let largest = ch.bars.iter().map(|b| b.value).fold(0.0, f64::max);
            match ch.max {
                Some(m) if !(m.is_finite() && m > 0.0 && m <= MAX_ABS_NUMBER) => {
                    out.push(format!("max must be above 0, got {m}"))
                }
                Some(m) if largest > m => out.push(format!(
                    "max ({m}) is below the largest value ({largest}); leave it out to use the largest value"
                )),
                None if largest <= 0.0 && !ch.bars.is_empty() => {
                    out.push("every bar is 0: give one a value above 0 or set max".to_string())
                }
                _ => {}
            }
            out.extend(number_format_problems(
                ch.decimals,
                &ch.prefix,
                &ch.suffix,
                None,
            ));
            if !(ch.font_size.is_finite() && ch.font_size > 0.0 && ch.font_size <= 0.2) {
                out.push(format!(
                    "font_size must be above 0 and at most 0.2 (fraction of the frame height), got {}",
                    ch.font_size
                ));
            }
            if ch.grow_ms == 0 || ch.grow_ms > 10_000 {
                out.push(format!("grow_ms must be 1..10000, got {}", ch.grow_ms));
            }
            if ch.stagger_ms > 2_000 {
                out.push(format!("stagger_ms must be 0..2000, got {}", ch.stagger_ms));
            }
            opt_color("color", &ch.color, &mut out);
            opt_color("label_color", &ch.label_color, &mut out);
        }
        LayerContent::ProgressRing(r) => {
            for (name, v) in [("value", r.value), ("from", r.from)] {
                if !(v.is_finite() && (0.0..=1.0).contains(&v)) {
                    out.push(format!(
                        "{name} must be between 0 and 1 (a fraction of the ring), got {v}"
                    ));
                }
            }
            if let Some(ms) = r.duration_ms {
                if ms == 0 || ms > span {
                    out.push(format!(
                        "duration_ms must be above 0 and at most the layer's time ({span} ms), got {ms}"
                    ));
                }
            }
            if !in_range(r.thickness, 0.02, 0.5) {
                out.push(format!(
                    "thickness must be between 0.02 and 0.5 (fraction of the ring's diameter), got {}",
                    r.thickness
                ));
            }
            if let Some(l) = &r.label {
                let n = l.chars().count();
                if n > MAX_RING_LABEL_CHARS {
                    out.push(format!(
                        "the label is {n} characters; the limit is {MAX_RING_LABEL_CHARS}"
                    ));
                }
            }
            out.extend(number_format_problems(r.decimals, "", "", None));
            opt_color("color", &r.color, &mut out);
            color("track_color", &r.track_color, &mut out);
            opt_color("label_color", &r.label_color, &mut out);
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

/// A finite number within ±[`MAX_ABS_NUMBER`].
fn number(name: &str, v: f64, out: &mut Vec<String>) {
    if !(v.is_finite() && v.abs() <= MAX_ABS_NUMBER) {
        out.push(format!(
            "{name} must be a number within ±{MAX_ABS_NUMBER:e}, got {v}"
        ));
    }
}

/// Decimals, prefix and suffix of a formatted number, and its separator and decimal mark when
/// the kind has them.
fn number_format_problems(
    decimals: u8,
    prefix: &str,
    suffix: &str,
    marks: Option<(&str, &str)>,
) -> Vec<String> {
    let mut out = Vec::new();
    if decimals > MAX_DECIMALS {
        out.push(format!(
            "decimals must be 0..{MAX_DECIMALS}, got {decimals}"
        ));
    }
    for (name, s) in [("prefix", prefix), ("suffix", suffix)] {
        if s.chars().count() > MAX_AFFIX_CHARS {
            out.push(format!(
                "{name} is longer than {MAX_AFFIX_CHARS} characters"
            ));
        }
    }
    if let Some((sep, mark)) = marks {
        let digit = |s: &str| s.chars().any(|c| c.is_ascii_digit());
        if sep.chars().count() > 2 || digit(sep) {
            out.push("separator must be at most 2 characters and no digits".to_string());
        }
        if mark.chars().count() != 1 || digit(mark) || mark == sep {
            out.push(
                "decimal_mark must be one character, not a digit and not the separator".to_string(),
            );
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
    let videos = scene
        .layers
        .iter()
        .filter(|l| matches!(l.content, LayerContent::Video(_)))
        .count();
    if videos > MAX_VIDEO_LAYERS {
        errors.push(ValidationError::TooManyVideoLayers {
            scene: scene.id.clone(),
            count: videos,
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
///
/// Template scenes are checked as they will be rendered: a copy of the timeline is expanded
/// (see [`crate::template`]) and the copy is validated. Bad template params are reported as
/// [`ValidationError::BadTemplate`] (the rest of the timeline is then not checked).
pub fn validate(timeline: &VideoTimeline) -> Result<(), Vec<ValidationError>> {
    let mut errors: Vec<ValidationError> = timeline
        .brand
        .as_ref()
        .map(|b| {
            b.problems()
                .into_iter()
                .map(ValidationError::BadBrand)
                .collect()
        })
        .unwrap_or_default();
    let result = if timeline.scenes.iter().any(|s| s.template.is_some()) {
        let mut expanded = timeline.clone();
        match crate::template::expand_templates(&mut expanded) {
            Ok(()) => validate_expanded(&expanded),
            Err(es) => Err(es
                .into_iter()
                .map(|e| ValidationError::BadTemplate {
                    scene: e.scene,
                    message: e.message,
                })
                .collect()),
        }
    } else {
        validate_expanded(timeline)
    };
    if let Err(more) = result {
        errors.extend(more);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// [`validate`] of a timeline without template scenes and without the brand.
fn validate_expanded(timeline: &VideoTimeline) -> Result<(), Vec<ValidationError>> {
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
        if !has_text && !has_image && scene.video.is_none() && scene.layers.is_empty() {
            errors.push(ValidationError::EmptyScene(scene.id.clone()));
        }
        if let Some(video) = &scene.video {
            if scene.image.is_some() {
                errors.push(ValidationError::ImageAndVideo(scene.id.clone()));
            }
            for message in clip_problems(video) {
                errors.push(ValidationError::BadSceneVideo {
                    scene: scene.id.clone(),
                    message,
                });
            }
        }
        if let Some(audio) = &scene.audio {
            if audio.source.trim().is_empty() {
                errors.push(ValidationError::EmptyAudioSource(scene.id.clone()));
            }
        }
        validate_transition(scene, i, &mut errors);
        validate_keyframes(scene, &mut errors);
        validate_layers(scene, &mut errors);
        validate_sfx(scene, &mut errors);
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
        if let Some(grid) = &music.beats {
            if !in_range(grid.bpm, 30.0, 300.0) {
                errors.push(ValidationError::BadBeatGrid(format!(
                    "bpm {} must be between 30 and 300",
                    grid.bpm
                )));
            }
            if grid.offset_ms > 60_000 {
                errors.push(ValidationError::BadBeatGrid(format!(
                    "offset_ms {} must be at most 60000",
                    grid.offset_ms
                )));
            }
        }
        if let Some(sync) = &music.beat_sync {
            if ![1, 2, 4].contains(&sync.every) {
                errors.push(ValidationError::BadBeatSync(format!(
                    "every must be 1, 2 or 4 (got {})",
                    sync.every
                )));
            }
            if !(1..=1_000).contains(&sync.max_shift_ms) {
                errors.push(ValidationError::BadBeatSync(format!(
                    "max_shift_ms must be between 1 and 1000 (got {})",
                    sync.max_shift_ms
                )));
            }
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
            sfx: vec![],
            template: None,
            id: id.into(),
            duration_ms: ms,
            text: Some("hello".into()),
            highlights: vec![],
            image: None,
            motion: MotionType::Static,
            video: None,
            audio: None,
            narration: None,
            transition_in: None,
            keyframes: vec![],
            layers: vec![],
        }
    }

    fn timeline(scenes: Vec<Scene>) -> VideoTimeline {
        VideoTimeline {
            brand: None,
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
            brand: None,
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
    fn checks_sound_effects() {
        let mut s = scene("a", 2_000);
        s.layers = vec![serde_json::from_value(serde_json::json!({
            "id": "t", "kind": "text", "x": 0.1, "y": 0.1, "width": 0.5, "text": "hi",
            "start_ms": 300, "end_ms": 1_500, "exit": {"kind": "fade", "duration_ms": 300}
        }))
        .unwrap()];
        let fx = |json: serde_json::Value| -> crate::model::SoundEffect {
            serde_json::from_value(json).unwrap()
        };
        s.sfx = vec![
            fx(serde_json::json!({"src": "assets/sfx/pop.wav", "at_ms": 1_999})),
            fx(
                serde_json::json!({"src": "assets/sfx/pop.wav", "on": {"layer": "t", "event": "exit"}}),
            ),
        ];
        assert!(validate(&timeline(vec![s.clone()])).is_ok());
        assert_eq!(s.sfx[1].time_in(&s), Some(1_200));
        assert_eq!(s.sfx[0].volume, 0.8);

        let bad = [
            (
                serde_json::json!({"src": "assets/sfx/pop.wav", "at_ms": 2_000}),
                "inside the scene",
            ),
            (
                serde_json::json!({"src": "assets/sfx/pop.wav"}),
                "give at_ms",
            ),
            (
                serde_json::json!({"src": "assets/sfx/pop.wav", "at_ms": 1, "on": {"layer": "t", "event": "enter"}}),
                "not both",
            ),
            (
                serde_json::json!({"src": "assets/sfx/pop.wav", "on": {"layer": "x", "event": "enter"}}),
                "no layer `x`",
            ),
            (
                serde_json::json!({"src": "/etc/pop.wav", "at_ms": 0}),
                "relative",
            ),
            (
                serde_json::json!({"src": "assets/pop.png", "at_ms": 0}),
                "audio file",
            ),
            (
                serde_json::json!({"src": "assets/sfx/pop.wav", "at_ms": 0, "volume": 1.5}),
                "volume 1.5",
            ),
        ];
        for (json, want) in bad {
            let mut t = s.clone();
            t.sfx = vec![fx(json.clone())];
            let errs = validate(&timeline(vec![t])).unwrap_err();
            assert!(
                errs.iter().any(|e| e.to_string().contains(want)),
                "{json}: {errs:?}"
            );
        }
        // A layer without an end or exit "exits" at the scene end: not a moment inside it.
        let mut t = s.clone();
        t.layers[0].end_ms = None;
        t.layers[0].exit = None;
        t.sfx = vec![fx(
            serde_json::json!({"src": "assets/sfx/pop.wav", "on": {"layer": "t", "event": "exit"}}),
        )];
        let errs = validate(&timeline(vec![t])).unwrap_err();
        assert!(
            errs[0].to_string().contains("give it an end_ms"),
            "{errs:?}"
        );
        let mut t = s.clone();
        t.sfx = vec![s.sfx[0].clone(); MAX_SFX_PER_SCENE + 1];
        let errs = validate(&timeline(vec![t])).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| matches!(e, ValidationError::TooManySfx { count: 17, .. })));
    }

    #[test]
    fn checks_beat_settings() {
        use crate::model::Music;
        let mut t = timeline(vec![scene("a", 5_000)]);
        let mut music: Music = serde_json::from_value(serde_json::json!({
            "source": "m.m4a", "beat_sync": {}, "beats": {"bpm": 96, "offset_ms": 750}
        }))
        .unwrap();
        let sync = music.beat_sync.unwrap();
        assert!(sync.enabled && sync.every == 1 && sync.max_shift_ms == 250);
        t.music = Some(music.clone());
        assert!(validate(&t).is_ok());
        music.beats.as_mut().unwrap().bpm = 20.0;
        music.beat_sync.as_mut().unwrap().every = 3;
        music.beat_sync.as_mut().unwrap().max_shift_ms = 0;
        t.music = Some(music);
        let errs: Vec<String> = validate(&t)
            .unwrap_err()
            .iter()
            .map(|e| e.to_string())
            .collect();
        assert_eq!(errs.len(), 3, "{errs:?}");
        assert!(errs[0].contains("bpm 20"));
        assert!(errs[1].contains("every must be 1, 2 or 4"));
        assert!(errs[2].contains("max_shift_ms"));
    }

    #[test]
    fn checks_music() {
        use crate::model::Music;
        let mut t = timeline(vec![scene("a", 5_000)]);
        t.music = Some(Music {
            beats: None,
            beat_sync: None,
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
            beats: None,
            beat_sync: None,
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
            beats: None,
            beat_sync: None,
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
    fn lower_third_rules() {
        let mut a = scene("a", 3_000);
        a.layers = vec![
            layer(
                r#"{"id":"ok","kind":"lower_third","x":0.06,"y":0.66,"width":0.5,"title":"Ada","subtitle":"Mathematician"}"#,
            ),
            layer(
                r##"{"id":"bad","kind":"lower_third","x":0.06,"y":0.66,"width":0.5,"title":" ","accent_color":"blue","font_size":0.5}"##,
            ),
        ];
        let errs = validate(&timeline(vec![a])).unwrap_err();
        assert!(layer_messages(&errs, "ok").is_empty(), "{errs:?}");
        let m = layer_messages(&errs, "bad");
        for needle in ["title is empty", "accent_color", "font_size"] {
            assert!(m.iter().any(|m| m.contains(needle)), "{needle}: {m:?}");
        }
        let long = "x".repeat(121);
        let mut b = scene("b", 3_000);
        b.layers = vec![layer(&format!(
            r#"{{"id":"long","kind":"lower_third","x":0,"y":0,"width":0.5,"title":"{long}"}}"#
        ))];
        let errs = validate(&timeline(vec![b])).unwrap_err();
        assert!(layer_messages(&errs, "long")[0].contains("limit is 120"));
    }

    #[test]
    fn counter_rules() {
        let mut a = scene("a", 3_000);
        a.layers = vec![
            layer(
                r#"{"id":"ok","kind":"counter","x":0.3,"y":0.3,"width":0.4,"from":-5,"to":1250.5,"decimals":1,"prefix":"$","separator":" ","decimal_mark":",","duration_ms":3000}"#,
            ),
            layer(
                r#"{"id":"bad","kind":"counter","x":0.3,"y":0.3,"width":0.4,"to":1e16,"decimals":7,"suffix":"a very long suffix!","separator":"1","decimal_mark":"..","duration_ms":4000}"#,
            ),
        ];
        let errs = validate(&timeline(vec![a])).unwrap_err();
        assert!(layer_messages(&errs, "ok").is_empty(), "{errs:?}");
        let m = layer_messages(&errs, "bad");
        for needle in [
            "to must be",
            "decimals",
            "suffix",
            "separator",
            "decimal_mark",
            "duration_ms",
        ] {
            assert!(m.iter().any(|m| m.contains(needle)), "{needle}: {m:?}");
        }
    }

    #[test]
    fn bar_chart_rules() {
        let bars = |n: usize| {
            (0..n)
                .map(|i| format!(r#"{{"label":"b{i}","value":{i}}}"#))
                .collect::<Vec<_>>()
                .join(",")
        };
        let mut a = scene("a", 3_000);
        a.layers = vec![
            layer(&format!(
                r#"{{"id":"ok","kind":"bar_chart","x":0.1,"y":0.1,"width":0.8,"height":0.6,"orientation":"horizontal","bars":[{}]}}"#,
                bars(12)
            )),
            layer(&format!(
                r#"{{"id":"many","kind":"bar_chart","x":0.1,"y":0.1,"width":0.8,"height":0.6,"bars":[{}]}}"#,
                bars(13)
            )),
            layer(
                r#"{"id":"bad","kind":"bar_chart","x":0.1,"y":0.1,"width":0.8,"height":0.6,"max":5,"grow_ms":0,
                    "bars":[{"label":"a","value":-1},{"label":"b","value":9,"color":"green"}]}"#,
            ),
            layer(
                r#"{"id":"zero","kind":"bar_chart","x":0.1,"y":0.1,"width":0.8,"height":0.6,"bars":[{"label":"a","value":0}]}"#,
            ),
            layer(
                r#"{"id":"nohead","kind":"bar_chart","x":0.1,"y":0.1,"width":0.8,"bars":[{"label":"a","value":1}]}"#,
            ),
        ];
        let errs = validate(&timeline(vec![a])).unwrap_err();
        assert!(layer_messages(&errs, "ok").is_empty(), "{errs:?}");
        let has = |id: &str, needle: &str| {
            let m = layer_messages(&errs, id);
            assert!(m.iter().any(|m| m.contains(needle)), "{id} {needle}: {m:?}");
        };
        has("many", "1 to 12 bars");
        has("bad", "bar 0 value");
        has("bad", "bar 1 color");
        has("bad", "below the largest");
        has("bad", "grow_ms");
        has("zero", "every bar is 0");
        has("nohead", "needs a height");
    }

    #[test]
    fn progress_ring_rules() {
        let mut a = scene("a", 3_000);
        a.layers = vec![
            layer(
                r#"{"id":"ok","kind":"progress_ring","x":0.4,"y":0.3,"width":0.2,"height":0.35,"value":0.72,"from":0.1,"label":"72 of 100"}"#,
            ),
            layer(
                r#"{"id":"bad","kind":"progress_ring","x":0.4,"y":0.3,"width":0.2,"height":0.35,"value":1.5,"thickness":0.9,"duration_ms":0,"track_color":"grey","label":"a label that is far too long"}"#,
            ),
        ];
        let errs = validate(&timeline(vec![a])).unwrap_err();
        assert!(layer_messages(&errs, "ok").is_empty(), "{errs:?}");
        let m = layer_messages(&errs, "bad");
        for needle in [
            "value must be",
            "thickness",
            "duration_ms",
            "track_color",
            "label is",
        ] {
            assert!(m.iter().any(|m| m.contains(needle)), "{needle}: {m:?}");
        }
    }

    #[test]
    fn video_background_and_layers() {
        use crate::layer::VideoClip;
        let mut s = scene("v", 3_000);
        s.text = None;
        s.video = Some(VideoClip::new("assets/clips/a.mp4"));
        assert!(
            validate(&timeline(vec![s.clone()])).is_ok(),
            "a video alone fills a scene"
        );
        s.image = Some("assets/sample.png".into());
        s.video.as_mut().unwrap().speed = 9.0;
        let errs = validate(&timeline(vec![s.clone()])).unwrap_err();
        assert_eq!(
            errs,
            vec![
                ValidationError::ImageAndVideo("v".into()),
                ValidationError::BadSceneVideo {
                    scene: "v".into(),
                    message: "speed must be between 0.25 and 4, got 9".into()
                }
            ]
        );
        let mut s = scene("w", 3_000);
        for i in 0..5 {
            s.layers.push(layer(&format!(
                r#"{{"id":"v{i}","kind":"video","x":0,"y":0,"width":0.2,"height":0.2,"src":"a.mp4","trim_start_ms":900,"trim_end_ms":100}}"#
            )));
        }
        let errs = validate(&timeline(vec![s])).unwrap_err();
        assert!(errs.contains(&ValidationError::TooManyVideoLayers {
            scene: "w".into(),
            count: 5
        }));
        assert_eq!(
            layer_messages(&errs, "v0"),
            ["trim_end_ms (100) must be after trim_start_ms (900)"]
        );
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
