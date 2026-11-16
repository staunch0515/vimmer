//! Orchestration: validate → narrate → snap cuts to the music's beats → time subtitles word by word → time text pages → preprocess assets →
//! plan chunks → render chunks (silent) → concatenate → mix the soundtrack once.

pub mod align_scenes;
pub mod audio;
pub mod beatsync;
pub mod control;
pub mod duck;
pub mod localize;
pub mod narrate;
pub mod options;
pub mod pages;
pub mod plan;
pub mod run;
pub mod video;
pub mod wordsubs;

pub use align_scenes::{align_to_voiceover, Alignment, SceneChange};
pub use control::{CancelHandle, Progress, ProgressFn, RenderContext};
pub use narrate::{
    preview_error, preview_narration, NarrationPreview, PreviewClip, TtsConfig, PREVIEW_SLOTS,
};
pub use options::RenderOptions;
pub use plan::{plan_chunks, Chunk, DEFAULT_CHUNK_MS};
pub use run::{
    render_timeline, render_timeline_with, resolve_music, RunReport, SchedulerConfig,
    SchedulerError,
};
pub use wordsubs::{AutoSubtitles, WordSubtitleConfig};
