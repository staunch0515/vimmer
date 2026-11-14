//! Orchestration: validate → narrate → snap cuts to the music's beats → time subtitles word by word → preprocess assets →
//! plan chunks → render chunks (silent) → concatenate → mix the soundtrack once.

pub mod audio;
pub mod beatsync;
pub mod control;
pub mod duck;
pub mod narrate;
pub mod plan;
pub mod run;
pub mod video;
pub mod wordsubs;

pub use control::{CancelHandle, Progress, ProgressFn, RenderContext};
pub use narrate::TtsConfig;
pub use plan::{plan_chunks, Chunk, DEFAULT_CHUNK_MS};
pub use run::{render_timeline, render_timeline_with, RunReport, SchedulerConfig, SchedulerError};
pub use wordsubs::WordSubtitleConfig;
