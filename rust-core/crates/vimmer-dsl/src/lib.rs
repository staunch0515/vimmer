//! JSON DSL for describing a short video timeline.
//!
//! The same JSON document is consumed by the Remotion renderer, so field names
//! here are part of a cross-language contract (`snake_case`, no renames).

pub mod layer;
pub mod model;
pub mod srt;
pub mod validation;

pub use layer::{
    is_hex_color, Anchor, IconLayer, ImageFit, ImageLayer, Layer, LayerAnimation,
    LayerAnimationKind, LayerContent, ShapeKind, ShapeLayer, TextAlign, TextBackground, TextLayer,
    TextReveal, MAX_LAYERS, MAX_LAYER_KEYFRAMES, MAX_LAYER_TEXT_CHARS,
};
pub use model::{
    AudioMapping, Duck, Easing, Keyframe, MotionType, Music, Narration, Resolution, Scene,
    SubtitleCue, Transition, TransitionKind, VideoTimeline, WordTiming, MAX_DURATION_MS,
    MAX_FRAMES, TARGET_FPS,
};
pub use srt::{parse_srt, SrtError};
pub use validation::{validate, ValidationError};
