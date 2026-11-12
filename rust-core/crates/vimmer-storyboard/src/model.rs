use serde::{Deserialize, Serialize};

/// Screen orientation. Maps to 1920x1080 / 1080x1920.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    #[default]
    Landscape,
    Portrait,
}

/// Narration / on-screen text language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Zh,
    En,
    Ja,
}

fn default_true() -> bool {
    true
}

/// Root storyboard document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Storyboard {
    pub title: String,
    #[serde(default)]
    pub orientation: Orientation,
    pub language: Language,
    /// Background music path, relative to `remotion-renderer/public/`.
    #[serde(default)]
    pub music: Option<String>,
    /// Synthesize each scene's `text` as narration at render time (default true).
    #[serde(default = "default_true")]
    pub narration: bool,
    /// Override the speech voice. Default: chosen by language.
    #[serde(default)]
    pub voice: Option<String>,
    pub scenes: Vec<SceneSpec>,
}

/// One storyboard scene. `camera` and `transition` are free strings so that unknown
/// values can fall back to a safe default with a warning instead of failing the parse.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneSpec {
    /// Narration and on-screen text.
    pub text: String,
    /// Milliseconds. Omit to estimate from the text length.
    #[serde(default)]
    pub duration_ms: Option<u64>,
    /// Words / phrases in `text` to emphasise.
    #[serde(default)]
    pub emphasis: Vec<String>,
    /// static | push_in | pull_out | pan_left | pan_right | ken_burns
    #[serde(default)]
    pub camera: Option<String>,
    /// none | fade | slide_left | slide_right | slide_up | slide_down | zoom_in | wipe_left
    #[serde(default)]
    pub transition: Option<String>,
    /// Transition length in ms (default 500, clamped to the scene length).
    #[serde(default)]
    pub transition_ms: Option<u64>,
    /// Existing image under `remotion-renderer/public/`, e.g. `assets/sample.png`.
    #[serde(default)]
    pub image: Option<String>,
    /// Description of a wanted picture. Stored only; nothing consumes it yet.
    #[serde(default)]
    pub image_prompt: Option<String>,
}
