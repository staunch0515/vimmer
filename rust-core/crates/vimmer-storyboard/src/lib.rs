//! Storyboard JSON -> [`vimmer_dsl::VideoTimeline`].
//!
//! The storyboard is the format a language model is asked to produce (see
//! `prompts/storyboard.md`). Conversion is offline. [`generate`] asks Claude for a storyboard
//! through a Python helper (`scripts/generate_storyboard.py`) and validates the reply.

mod convert;
pub mod edit;
pub mod edit_claude;
mod estimate;
pub mod generate;
pub mod illustrate;
mod model;

pub use convert::{
    convert, missing_assets, storyboard_to_timeline, Conversion, EmphasisMap, StoryboardError,
    Warning, DEFAULT_TRANSITION_MS,
};
pub use estimate::{estimate_duration_ms, MIN_ESTIMATED_MS};
pub use model::{Language, Orientation, SceneSpec, Storyboard};

#[cfg(test)]
mod edit_claude_tests;
#[cfg(test)]
mod edit_tests;
#[cfg(test)]
mod generate_tests;
#[cfg(test)]
mod illustrate_tests;
#[cfg(test)]
mod tests;
