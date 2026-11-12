//! Orchestration: validate → preprocess assets → plan chunks → render chunks → concatenate.

pub mod control;
pub mod narrate;
pub mod plan;
pub mod run;

pub use control::{CancelHandle, Progress, ProgressFn, RenderContext};
pub use plan::{plan_chunks, Chunk, DEFAULT_CHUNK_MS};
pub use run::{render_timeline, render_timeline_with, RunReport, SchedulerConfig, SchedulerError};
