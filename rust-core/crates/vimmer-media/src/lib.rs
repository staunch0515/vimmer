//! Asset preprocessing: normalize images to the output frame and probe audio durations,
//! synthesize narration (macOS `say` or local Kokoro), and recognize it again to time subtitles word by word.

pub mod align;
pub mod audio;
pub mod image_norm;
pub mod kokoro;
pub mod process;
pub mod speech;
pub mod transcribe;

pub use align::{align_words, is_cjk, split_units, AlignedWord, RecognizedToken};
pub use audio::{has_audio_stream, parse_ffmpeg_duration_ms, AudioProbe, FfmpegProbe};
pub use image_norm::{letterbox, normalize_image};
pub use kokoro::{
    batch_timeout, looks_like_kokoro_voice, resolve_kokoro_voice, KokoroItem, KokoroLang,
    KokoroSpeech, TtsEngine, ENV_KOKORO_MODELS, ENV_KOKORO_PYTHON, ENV_TTS,
};
pub use process::{run_limited, Limits};
pub use speech::{default_voice_for, SaySpeech, Speech};
pub use transcribe::{
    guess_language, TranscribeRequest, Transcriber, WhisperCli, DEFAULT_MODEL_IN_HOME,
    ENV_WHISPER_CLI, ENV_WHISPER_DTW, ENV_WHISPER_GPU, ENV_WHISPER_MODEL,
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
    #[error("cancelled")]
    Cancelled,
    #[error("timed out after {secs} s")]
    TimedOut { secs: u64 },
}
