//! Asset preprocessing: normalize images to the output frame and probe audio durations,
//! synthesize narration, and recognize it again to time subtitles word by word.

pub mod align;
pub mod audio;
pub mod image_norm;
pub mod speech;
pub mod transcribe;

pub use align::{align_words, is_cjk, split_units, AlignedWord, RecognizedToken};
pub use audio::{has_audio_stream, parse_ffmpeg_duration_ms, AudioProbe, FfmpegProbe};
pub use image_norm::{letterbox, normalize_image};
pub use speech::{default_voice_for, SaySpeech, Speech};
pub use transcribe::{
    guess_language, TranscribeRequest, Transcriber, WhisperCli, DEFAULT_MODEL_IN_HOME,
    ENV_WHISPER_CLI, ENV_WHISPER_MODEL,
};

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("image error: {0}")]
    Image(#[from] image::ImageError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("ffmpeg failed to probe {path}: {reason}")]
    Probe { path: String, reason: String },
    #[error("speech synthesis failed: {0}")]
    Speech(String),
    #[error("speech recognition failed: {0}")]
    Transcribe(String),
}
