//! JSON DSL for describing a short video timeline.
//!
//! The same JSON document is consumed by the Remotion renderer, so field names
//! here are part of a cross-language contract (`snake_case`, no renames).

pub mod model;
pub mod srt;
pub mod validation;

pub use model::{
    AudioMapping, MotionType, Narration, Resolution, Scene, SubtitleCue, VideoTimeline,
    MAX_DURATION_MS, MAX_FRAMES, TARGET_FPS,
};
pub use srt::{parse_srt, SrtError};
pub use validation::{validate, ValidationError};
