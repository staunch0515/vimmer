//! Asset preprocessing: normalize images and video clips to the output frame and probe audio durations,
//! synthesize narration (macOS `say` or local Kokoro), and recognize it again to time subtitles word by word.

pub mod align;
pub mod audio;
pub mod beats;
pub mod estimate;
pub mod image_norm;
pub mod kokoro;
pub mod process;
pub mod speech;
pub mod svg;
pub mod transcribe;
pub mod video_norm;

pub use align::{align_words, is_cjk, split_units, AlignedWord, RecognizedToken};
pub use audio::{has_audio_stream, parse_ffmpeg_duration_ms, AudioProbe, FfmpegProbe};
pub use beats::{
    analyze, decode_mono, detect_beats, detect_beats_cached, BeatAnalysis, LOW_CONFIDENCE,
};
pub use estimate::{
    estimate_speech_ms, speech_features, speech_rates, SpeechFeatures, SpeechLang, SpeechRates,
};
pub use image_norm::{letterbox, normalize_image};
pub use kokoro::{
    batch_timeout, looks_like_kokoro_voice, resolve_kokoro_voice, KokoroItem, KokoroLang,
    KokoroSpeech, TtsEngine, ENV_KOKORO_MODELS, ENV_KOKORO_PYTHON, ENV_TTS,
};
pub use process::{run_limited, Limits};
pub use speech::{default_voice_for, SaySpeech, Speech};
pub use svg::{
    check_png, find_chrome, preflight_chrome, rasterize_svg, sanitize_svg, SanitizedSvg, SvgError,
    ENV_CHROME, MAX_SVG_BYTES,
};
pub use transcribe::{
    guess_language, TranscribeRequest, Transcriber, WhisperCli, DEFAULT_MODEL_IN_HOME,
    ENV_WHISPER_CLI, ENV_WHISPER_DTW, ENV_WHISPER_GPU, ENV_WHISPER_MODEL,
};
pub use video_norm::{
    atempo_chain, fingerprint, fingerprint_str, parse_video_info, probe_video, ClipJob, VideoInfo,
    CLIP_GOP, CLIP_SAMPLE_RATE,
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
    #[error("video {path}: {reason}")]
    Video { path: String, reason: String },
    #[error("cancelled")]
    Cancelled,
    #[error("timed out after {secs} s")]
    TimedOut { secs: u64 },
}
