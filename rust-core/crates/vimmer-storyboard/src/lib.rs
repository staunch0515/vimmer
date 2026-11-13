//! Storyboard JSON -> [`vimmer_dsl::VideoTimeline`].
//!
//! The storyboard is the format a language model is asked to produce (see
//! `prompts/storyboard.md`). Conversion is offline. [`generate`] asks Claude for a storyboard
//! through a Python helper (`scripts/generate_storyboard.py`) and validates the reply.

mod convert;
mod estimate;
pub mod generate;
mod model;

pub use convert::{
    convert, missing_assets, storyboard_to_timeline, Conversion, EmphasisMap, StoryboardError,
    Warning, DEFAULT_TRANSITION_MS,
};
pub use estimate::{estimate_duration_ms, MIN_ESTIMATED_MS};
pub use model::{Language, Orientation, SceneSpec, Storyboard};

#[cfg(test)]
mod generate_tests;
#[cfg(test)]
mod tests;
