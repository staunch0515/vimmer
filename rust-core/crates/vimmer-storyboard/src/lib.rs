//! Storyboard JSON -> [`vimmer_dsl::VideoTimeline`].
//!
//! The storyboard is the format a language model is asked to produce (see
//! `prompts/storyboard.md`). This crate is offline: it never calls any model or service.

mod convert;
mod estimate;
mod model;

pub use convert::{
    convert, missing_assets, storyboard_to_timeline, Conversion, EmphasisMap, StoryboardError,
    Warning, DEFAULT_TRANSITION_MS,
};
pub use estimate::{estimate_duration_ms, MIN_ESTIMATED_MS};
pub use model::{Language, Orientation, SceneSpec, Storyboard};

#[cfg(test)]
mod tests;
