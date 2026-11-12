//! Asset preprocessing: normalize images to the output frame and probe audio durations.

pub mod audio;
pub mod image_norm;

pub use audio::{parse_ffmpeg_duration_ms, AudioProbe, FfmpegProbe};
pub use image_norm::{letterbox, normalize_image};

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("image error: {0}")]
    Image(#[from] image::ImageError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("ffmpeg failed to probe {path}: {reason}")]
    Probe { path: String, reason: String },
}
