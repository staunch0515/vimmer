//! Orchestration: validate → preprocess assets → plan chunks → render chunks → concatenate.

pub mod plan;
pub mod run;

pub use plan::{plan_chunks, Chunk, DEFAULT_CHUNK_MS};
pub use run::{render_timeline, RunReport, SchedulerConfig, SchedulerError};
