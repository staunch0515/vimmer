use std::{collections::BTreeMap, fmt, path::Path};

use serde_json::{json, Value};
use thiserror::Error;
use vimmer_dsl::{validate, ValidationError, VideoTimeline, MAX_DURATION_MS, TARGET_FPS};

use crate::{
    estimate::estimate_duration_ms,
    model::{Orientation, SceneSpec, Storyboard, StoryboardMusic},
};

/// Transition length used when a scene names a transition but no `transition_ms`.
pub const DEFAULT_TRANSITION_MS: u64 = 500;

/// Scene id -> emphasis words, in storyboard order. Kept next to the timeline because the DSL
/// `highlights` field only drives the on-screen text treatment; see the crate report on how
/// word-level subtitle highlighting can consume this map.
pub type EmphasisMap = BTreeMap<String, Vec<String>>;

/// A non-fatal problem; the conversion fell back to a safe default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    /// 1-based scene number, or `None` for storyboard-level fields.
    pub scene: Option<usize>,
    pub field: &'static str,
    pub message: String,
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.scene {
            Some(n) => write!(f, "scene {n}: field `{}`: {}", self.field, self.message),
            None => write!(f, "field `{}`: {}", self.field, self.message),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum StoryboardError {
    #[error("field `scenes`: storyboard has no scenes")]
    NoScenes,
    #[error("field `title`: must not be empty")]
    EmptyTitle,
    #[error("scene {scene}: field `{field}`: {message}")]
    Scene {
        scene: usize,
        field: &'static str,
        message: String,
    },
    #[error("field `music`: {0}")]
    Music(String),
    #[error("total duration is {total_ms} ms; the maximum is {MAX_DURATION_MS} ms (field `scenes[*].duration_ms`)")]
    TooLong { total_ms: u64 },
    #[error("converted timeline failed DSL validation: {}", join(.0))]
    Validation(Vec<ValidationError>),
    #[error("invalid storyboard JSON: {0}")]
    Json(String),
}

fn join(errors: &[ValidationError]) -> String {
    errors
        .iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>()
        .join("; ")
}

fn scene_err(scene: usize, field: &'static str, message: impl Into<String>) -> StoryboardError {
    StoryboardError::Scene {
        scene,
        field,
        message: message.into(),
    }
}

/// Result of a conversion.
#[derive(Debug, Clone, PartialEq)]
pub struct Conversion {
    pub timeline: VideoTimeline,
    pub warnings: Vec<Warning>,
    pub emphasis: EmphasisMap,
}

/// Convenience wrapper that discards warnings and emphasis.
pub fn storyboard_to_timeline(sb: &Storyboard) -> Result<VideoTimeline, StoryboardError> {
    convert(sb).map(|c| c.timeline)
}

enum Camera {
    Motion(&'static str),
    KenBurns,
}

fn parse_camera(raw: &str) -> Option<Camera> {
    Some(match raw {
        "static" => Camera::Motion("static"),
        "push_in" | "zoom_in" => Camera::Motion("zoom_in"),
        "pull_out" | "zoom_out" => Camera::Motion("zoom_out"),
        "pan_left" => Camera::Motion("pan_left"),
        "pan_right" => Camera::Motion("pan_right"),
        "ken_burns" => Camera::KenBurns,
        _ => return None,
    })
}

/// `Some(None)` = explicit "no transition"; `None` = unknown value.
fn parse_transition(raw: &str) -> Option<Option<&'static str>> {
    Some(match raw {
        "none" | "cut" => None,
        "fade" => Some("fade"),
        "slide_left" => Some("slide_left"),
        "slide_right" => Some("slide_right"),
        "slide_up" => Some("slide_up"),
        "slide_down" => Some("slide_down"),
        "zoom_in" => Some("zoom_in"),
        "wipe_left" => Some("wipe_left"),
        _ => return None,
    })
}

fn norm(s: &str) -> String {
    s.trim().to_ascii_lowercase().replace(['-', ' '], "_")
}

fn check_relative(scene: usize, field: &'static str, path: &str) -> Result<(), StoryboardError> {
    let p = path.trim();
    if p.is_empty() {
        return Err(scene_err(scene, field, "must not be empty when present"));
    }
    if p.starts_with('/') || p.contains("..") || p.contains("://") || p.contains('\\') {
        return Err(scene_err(
            scene,
            field,
            format!(
                "`{p}` must be a relative path under remotion-renderer/public/ (no `..`, no URL)"
            ),
        ));
    }
    Ok(())
}

/// Converts a storyboard into a validated timeline plus warnings and the emphasis map.
pub fn convert(sb: &Storyboard) -> Result<Conversion, StoryboardError> {
    if sb.title.trim().is_empty() {
        return Err(StoryboardError::EmptyTitle);
    }
    if sb.scenes.is_empty() {
        return Err(StoryboardError::NoScenes);
    }

    let mut warnings = Vec::new();
    let mut emphasis = EmphasisMap::new();
    let mut scenes: Vec<Value> = Vec::with_capacity(sb.scenes.len());
    let mut total_ms: u64 = 0;

    for (i, spec) in sb.scenes.iter().enumerate() {
        let n = i + 1;
        let id = format!("scene-{n:02}");
        let value = convert_scene(sb, spec, n, &id, &mut warnings)?;
        total_ms = total_ms.saturating_add(value["duration_ms"].as_u64().unwrap_or(0));
        emphasis.insert(
            id,
            spec.emphasis.iter().map(|w| w.trim().to_owned()).collect(),
        );
        scenes.push(value);
    }

    if total_ms > MAX_DURATION_MS {
        return Err(StoryboardError::TooLong { total_ms });
    }

    let (width, height) = match sb.orientation {
        Orientation::Landscape => (1920, 1080),
        Orientation::Portrait => (1080, 1920),
    };
    let mut root = json!({
        "fps": TARGET_FPS,
        "resolution": { "width": width, "height": height },
        "scenes": scenes,
    });
    match &sb.music {
        None => {}
        Some(StoryboardMusic::File(music)) => {
            if music.trim().is_empty() {
                return Err(StoryboardError::Music(
                    "must not be empty when present".into(),
                ));
            }
            let p = music.trim();
            if p.starts_with('/') || p.contains("..") || p.contains("://") {
                return Err(StoryboardError::Music(format!(
                    "`{p}` must be a relative path under remotion-renderer/public/"
                )));
            }
            root["music"] = json!({
                "source": p,
                "fade_in_ms": 500.min(total_ms / 4),
                "fade_out_ms": 1500.min(total_ms / 4),
            });
        }
        Some(StoryboardMusic::Mood(m)) => {
            let mood = m.mood.trim().to_lowercase();
            if mood.is_empty() {
                return Err(StoryboardError::Music("`mood` must not be empty".into()));
            }
            let mut pick = json!({ "mood": mood });
            if let Some(e) = m.energy {
                if !(0.0..=1.0).contains(&e) {
                    return Err(StoryboardError::Music(format!(
                        "`energy` {e} must be between 0 and 1"
                    )));
                }
                // Two decimals: 0.3 stays 0.3 in the timeline, not 0.30000001192092896.
                pick["energy"] = json!((f64::from(e) * 100.0).round() / 100.0);
            }
            root["music"] = json!({
                "pick": pick,
                "fade_in_ms": 500.min(total_ms / 4),
                "fade_out_ms": 1500.min(total_ms / 4),
            });
        }
    }

    // Built from JSON rather than struct literals: `Scene`/`VideoTimeline` do not implement
    // `Default`, and going through serde lets new DSL fields pick up their serde defaults.
    let timeline: VideoTimeline =
        serde_json::from_value(root).map_err(|e| StoryboardError::Json(e.to_string()))?;
    validate(&timeline).map_err(StoryboardError::Validation)?;

    Ok(Conversion {
        timeline,
        warnings,
        emphasis,
    })
}

/// Hiragana or katakana (the long mark ー included), which is how speech engines tell
/// Japanese text from Chinese.
fn has_kana(text: &str) -> bool {
    text.chars()
        .any(|c| matches!(c, '\u{3041}'..='\u{3096}' | '\u{30A1}'..='\u{30FA}' | 'ー'))
}

fn convert_scene(
    sb: &Storyboard,
    spec: &SceneSpec,
    n: usize,
    id: &str,
    warnings: &mut Vec<Warning>,
) -> Result<Value, StoryboardError> {
    let text = spec.text.trim();
    if text.is_empty() {
        return Err(scene_err(n, "text", "must not be empty"));
    }

    let duration_ms = match spec.duration_ms {
        Some(0) => return Err(scene_err(n, "duration_ms", "must be greater than 0")),
        Some(d) if d > MAX_DURATION_MS => {
            return Err(scene_err(
                n,
                "duration_ms",
                format!("{d} ms exceeds the {MAX_DURATION_MS} ms video limit"),
            ))
        }
        Some(d) => d,
        None => estimate_duration_ms(text, sb.language),
    };

    // Emphasis -> DSL `highlights` (only words that occur in the text can be highlighted).
    let mut highlights = Vec::new();
    for (k, word) in spec.emphasis.iter().enumerate() {
        let w = word.trim();
        if w.is_empty() {
            warnings.push(Warning {
                scene: Some(n),
                field: "emphasis",
                message: format!("entry {} is empty and was ignored", k + 1),
            });
        } else if !text.to_lowercase().contains(&w.to_lowercase()) {
            warnings.push(Warning {
                scene: Some(n),
                field: "emphasis",
                message: format!("`{w}` does not occur in text; not highlighted on screen"),
            });
        } else {
            highlights.push(w.to_owned());
        }
    }

    let mut scene = json!({
        "id": id,
        "duration_ms": duration_ms,
        "text": text,
        "highlights": highlights,
    });

    if let Some(img) = &spec.image {
        check_relative(n, "image", img)?;
        scene["image"] = json!(img.trim());
    }

    // Camera.
    let camera = match spec.camera.as_deref().map(norm) {
        None => Camera::Motion("static"),
        Some(raw) => parse_camera(&raw).unwrap_or_else(|| {
            warnings.push(Warning {
                scene: Some(n),
                field: "camera",
                message: format!("unknown camera `{raw}`; using `static`"),
            });
            Camera::Motion("static")
        }),
    };
    match camera {
        Camera::Motion(m) => scene["motion"] = json!(m),
        Camera::KenBurns => {
            scene["keyframes"] = json!([
                { "at_ms": 0, "scale": 1.0, "x": 0, "y": 0 },
                { "at_ms": duration_ms, "scale": 1.15, "x": -3, "y": -2, "ease": "ease_in_out" },
            ]);
        }
    }

    // Transition.
    let kind = match spec.transition.as_deref().map(norm) {
        None => None,
        Some(raw) => parse_transition(&raw).unwrap_or_else(|| {
            warnings.push(Warning {
                scene: Some(n),
                field: "transition",
                message: format!("unknown transition `{raw}`; using `none`"),
            });
            None
        }),
    };
    if let Some(kind) = kind {
        if n == 1 {
            warnings.push(Warning {
                scene: Some(n),
                field: "transition",
                message: "the first scene has no previous scene; transition dropped".into(),
            });
        } else {
            let requested = spec.transition_ms.unwrap_or(DEFAULT_TRANSITION_MS);
            if requested == 0 {
                warnings.push(Warning {
                    scene: Some(n),
                    field: "transition_ms",
                    message: "0 ms transition; treated as `none`".into(),
                });
            } else {
                let dur = requested.min(duration_ms);
                if dur < requested {
                    warnings.push(Warning {
                        scene: Some(n),
                        field: "transition_ms",
                        message: format!(
                            "{requested} ms is longer than the scene ({duration_ms} ms); clamped to {dur} ms"
                        ),
                    });
                }
                scene["transition_in"] = json!({ "kind": kind, "duration_ms": dur });
            }
        }
    } else if spec.transition_ms.is_some() && spec.transition.is_none() {
        warnings.push(Warning {
            scene: Some(n),
            field: "transition_ms",
            message: "given without `transition`; ignored".into(),
        });
    }

    if sb.narration {
        // No voice by default: each speech engine picks one from the text (kana → Japanese).
        // Only Japanese text without kana, which would be read as Chinese, gets `Kyoko`.
        let voice = sb.voice.clone().or_else(|| {
            (sb.language == crate::Language::Ja && !has_kana(text)).then(|| "Kyoko".to_owned())
        });
        scene["narration"] = json!({ "voice": voice });
    }

    Ok(scene)
}

/// Lists `image`/`music` files that do not exist under `public_dir`
/// (`remotion-renderer/public`). Returns `(scene number or None for music, path)`.
pub fn missing_assets(sb: &Storyboard, public_dir: &Path) -> Vec<(Option<usize>, String)> {
    let mut missing = Vec::new();
    for (i, s) in sb.scenes.iter().enumerate() {
        if let Some(img) = &s.image {
            if !public_dir.join(img.trim()).is_file() {
                missing.push((Some(i + 1), img.clone()));
            }
        }
    }
    if let Some(StoryboardMusic::File(m)) = &sb.music {
        if !public_dir.join(m.trim()).is_file() {
            missing.push((None, m.clone()));
        }
    }
    missing
}
