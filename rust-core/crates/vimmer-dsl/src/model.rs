use serde::{Deserialize, Serialize};

/// Target frame rate for every render.
pub const TARGET_FPS: u32 = 30;
/// Hard ceiling on total timeline length.
pub const MAX_DURATION_MS: u64 = 200_000;
/// `MAX_DURATION_MS` expressed in frames at `TARGET_FPS` (6,000).
pub const MAX_FRAMES: u32 = 6_000;

/// Pixel dimensions of the output video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
}

/// Camera motion applied to a scene's image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionType {
    #[default]
    Static,
    ZoomIn,
    ZoomOut,
    PanLeft,
    PanRight,
}

/// Maps a voiceover / music asset onto a scene.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioMapping {
    /// Path or URL of the audio asset, relative to the renderer's public dir.
    pub source: String,
    /// Delay from the start of the scene, in milliseconds.
    #[serde(default)]
    pub offset_ms: u64,
}

/// One scene on the timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scene {
    pub id: String,
    pub duration_ms: u64,
    #[serde(default)]
    pub text: Option<String>,
    /// Words in `text` to render with the animated accent treatment.
    #[serde(default)]
    pub highlights: Vec<String>,
    /// Path or URL of the background image, relative to the public dir.
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub motion: MotionType,
    #[serde(default)]
    pub audio: Option<AudioMapping>,
}

impl Scene {
    /// Duration in frames at `fps`, rounded to the nearest frame.
    pub fn duration_frames(&self, fps: u32) -> u32 {
        ((self.duration_ms * u64::from(fps) + 500) / 1000) as u32
    }
}

/// A subtitle line on the global timeline, in milliseconds from the video start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtitleCue {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

/// Root document: the full timeline handed from the Rust core to Remotion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoTimeline {
    pub fps: u32,
    pub resolution: Resolution,
    pub scenes: Vec<Scene>,
    /// Global subtitle track. Independent of scenes, so cues can span scene boundaries.
    #[serde(default)]
    pub subtitles: Vec<SubtitleCue>,
}

impl VideoTimeline {
    /// Sum of all scene durations, in milliseconds.
    pub fn total_duration_ms(&self) -> u64 {
        self.scenes.iter().map(|s| s.duration_ms).sum()
    }

    /// Total frames across scenes at the timeline's fps.
    pub fn total_frames(&self) -> u32 {
        self.scenes
            .iter()
            .map(|s| s.duration_frames(self.fps))
            .sum()
    }

    /// Start frame of each scene, in order. Useful for `<Sequence from>`.
    pub fn scene_start_frames(&self) -> Vec<u32> {
        let mut acc = 0;
        self.scenes
            .iter()
            .map(|s| {
                let start = acc;
                acc += s.duration_frames(self.fps);
                start
            })
            .collect()
    }
}
