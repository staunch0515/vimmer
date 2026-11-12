//! JSON DSL for describing a short video timeline.
//!
//! The same JSON document is consumed by the Remotion renderer, so field names
//! here are part of a cross-language contract (`snake_case`, no renames).

pub mod model;
pub mod validation;

pub use model::{
    AudioMapping, MotionType, Resolution, Scene, VideoTimeline, MAX_DURATION_MS, MAX_FRAMES,
    TARGET_FPS,
};
pub use validation::{validate, ValidationError};
