//! Orchestration: validate → narrate → time subtitles word by word → preprocess assets →
//! plan chunks → render chunks → concatenate.

pub mod control;
pub mod music;
pub mod narrate;
pub mod plan;
pub mod run;
pub mod wordsubs;

pub use control::{CancelHandle, Progress, ProgressFn, RenderContext};
pub use narrate::TtsConfig;
pub use plan::{plan_chunks, Chunk, DEFAULT_CHUNK_MS};
pub use run::{render_timeline, render_timeline_with, RunReport, SchedulerConfig, SchedulerError};
pub use wordsubs::WordSubtitleConfig;
