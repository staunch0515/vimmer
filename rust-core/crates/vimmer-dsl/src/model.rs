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

/// Generate narration for a scene from its `text` with a local speech engine.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Narration {
    /// Voice name for the speech engine. Empty = pick by language (CJK text gets a Chinese voice).
    #[serde(default)]
    pub voice: Option<String>,
}

fn default_scale() -> f32 {
    1.0
}

fn default_opacity() -> f32 {
    1.0
}

/// How a scene's keyframes ease between this keyframe and the next one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Easing {
    Linear,
    EaseIn,
    EaseOut,
    #[default]
    EaseInOut,
}

/// One animation pose of a scene's image layer. Values between keyframes are interpolated.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    /// Time from the start of the scene, in milliseconds.
    pub at_ms: u64,
    /// Zoom factor; 1.0 is the unscaled image.
    #[serde(default = "default_scale")]
    pub scale: f32,
    /// Horizontal offset as a percentage of the frame width.
    #[serde(default)]
    pub x: f32,
    /// Vertical offset as a percentage of the frame height.
    #[serde(default)]
    pub y: f32,
    /// Opacity of the image layer, from 0.0 (invisible) to 1.0 (opaque).
    #[serde(default = "default_opacity")]
    pub opacity: f32,
    /// Clockwise rotation in degrees.
    #[serde(default)]
    pub rotation: f32,
    /// Easing of the segment that leaves this keyframe, toward the next one.
    #[serde(default)]
    pub ease: Easing,
}

/// How the incoming scene appears over the held last frame of the previous scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionKind {
    Fade,
    SlideLeft,
    SlideRight,
    SlideUp,
    SlideDown,
    ZoomIn,
    WipeLeft,
}

/// Transition into a scene from the previous one. It overlaps the start of this scene:
/// the scene's nominal start and the timeline's total duration do not change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transition {
    pub kind: TransitionKind,
    pub duration_ms: u64,
}

/// One scene on the timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// When set and `audio` is empty, the scene's text is synthesized as narration at render time.
    #[serde(default)]
    pub narration: Option<Narration>,
    /// Transition from the previous scene. Ignored on the first scene (validation rejects it).
    #[serde(default)]
    pub transition_in: Option<Transition>,
    /// Animation of the image layer. When non-empty it replaces `motion`.
    #[serde(default)]
    pub keyframes: Vec<Keyframe>,
}

impl Scene {
    /// Duration in frames at `fps`, rounded to the nearest frame.
    pub fn duration_frames(&self, fps: u32) -> u32 {
        ((self.duration_ms * u64::from(fps) + 500) / 1000) as u32
    }
}

/// When one word (or CJK character) of a subtitle cue is spoken, in milliseconds from the
/// video start, like the cue itself. Produced by aligning speech recognition to the known text.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WordTiming {
    /// The word as it appears in the cue, including attached punctuation and trailing space.
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// A subtitle line on the global timeline, in milliseconds from the video start.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SubtitleCue {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    /// Per-word timing for karaoke-style highlighting. Empty means the line is shown as a whole.
    /// When present, the concatenated word texts are what the renderer shows for this cue.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<WordTiming>,
}

impl SubtitleCue {
    /// A cue without word timing.
    pub fn new(start_ms: u64, end_ms: u64, text: impl Into<String>) -> Self {
        Self {
            start_ms,
            end_ms,
            text: text.into(),
            words: Vec::new(),
        }
    }
}

fn default_music_volume() -> f32 {
    0.3
}

/// Background music for the whole video. Loops until the timeline ends, with optional fades.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Music {
    /// Path relative to the renderer's public dir.
    pub source: String,
    /// Linear gain from 0.0 (silent) to 1.0 (full level).
    #[serde(default = "default_music_volume")]
    pub volume: f32,
    #[serde(default)]
    pub fade_in_ms: u64,
    #[serde(default)]
    pub fade_out_ms: u64,
}

/// Root document: the full timeline handed from the Rust core to Remotion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoTimeline {
    pub fps: u32,
    pub resolution: Resolution,
    pub scenes: Vec<Scene>,
    /// Global subtitle track. Independent of scenes, so cues can span scene boundaries.
    #[serde(default)]
    pub subtitles: Vec<SubtitleCue>,
    /// Background music across the whole timeline.
    #[serde(default)]
    pub music: Option<Music>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_without_new_fields_gets_defaults() {
        let json = r#"{"id":"a","duration_ms":1000,"text":"hi"}"#;
        let scene: Scene = serde_json::from_str(json).unwrap();
        assert_eq!(scene.transition_in, None);
        assert!(scene.keyframes.is_empty());
        assert_eq!(scene.motion, MotionType::Static);
    }

    #[test]
    fn keyframe_fields_default_to_identity_pose() {
        let kf: Keyframe = serde_json::from_str(r#"{"at_ms":500}"#).unwrap();
        assert_eq!(
            kf,
            Keyframe {
                at_ms: 500,
                scale: 1.0,
                x: 0.0,
                y: 0.0,
                opacity: 1.0,
                rotation: 0.0,
                ease: Easing::EaseInOut,
            }
        );
    }

    #[test]
    fn easing_and_transition_kinds_use_snake_case() {
        let kf: Keyframe = serde_json::from_str(r#"{"at_ms":0,"ease":"ease_in_out"}"#).unwrap();
        assert_eq!(kf.ease, Easing::EaseInOut);
        let t: Transition =
            serde_json::from_str(r#"{"kind":"wipe_left","duration_ms":400}"#).unwrap();
        assert_eq!(t.kind, TransitionKind::WipeLeft);
    }

    #[test]
    fn cue_words_default_to_empty_and_are_not_written_when_empty() {
        let cue: SubtitleCue =
            serde_json::from_str(r#"{"start_ms":0,"end_ms":1000,"text":"hi"}"#).unwrap();
        assert!(cue.words.is_empty());
        let json = serde_json::to_string(&cue).unwrap();
        assert_eq!(json, r#"{"start_ms":0,"end_ms":1000,"text":"hi"}"#);
    }

    #[test]
    fn cue_words_round_trip() {
        let json = r#"{"start_ms":0,"end_ms":1000,"text":"hi you","words":[{"text":"hi ","start_ms":0,"end_ms":400},{"text":"you","start_ms":400,"end_ms":900}]}"#;
        let cue: SubtitleCue = serde_json::from_str(json).unwrap();
        assert_eq!(cue.words.len(), 2);
        assert_eq!(cue.words[1].start_ms, 400);
        assert_eq!(serde_json::to_string(&cue).unwrap(), json);
    }

    #[test]
    fn unknown_transition_kind_is_a_parse_error() {
        let err = serde_json::from_str::<Transition>(r#"{"kind":"spin","duration_ms":400}"#);
        assert!(err.is_err());
    }
}
