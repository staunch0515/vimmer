//! Scene layers: graphics placed on top of (or under) a scene's text, each with its own box,
//! timing and animation.
//!
//! A [`Layer`] is the common part (id, box, timing, opacity, keyframes, enter/exit) plus a
//! [`LayerContent`] tagged by `kind`. New kinds are new [`LayerContent`] variants; the common
//! part, validation of the box and timing, stacking and animation stay as they are.
//!
//! Units, shared with the renderer (`remotion-renderer/src/motion/layers.ts`):
//!
//! * `x`, `y`, `width`, `height` are fractions of the frame width (x, width) and height
//!   (y, height), origin at the top-left corner of the frame.
//! * `anchor` says which point of the box `x`/`y` place: its top-left corner (the default) or its
//!   center. Rotation and keyframe scale always turn and grow the box about its center.
//! * Lengths that are not positions (font size, stroke width, corner radius) are fractions of
//!   the frame height, so a layout keeps its proportions at either resolution of one aspect.
//! * `start_ms`, `end_ms` and keyframe `at_ms` are milliseconds from the scene start.
//! * Keyframe `x`/`y` are offsets in percent of the frame width/height (as for the scene image);
//!   keyframe `scale` multiplies the box size, `opacity` multiplies the layer's `opacity`,
//!   `rotation` adds to the layer's `rotation`.
//!
//! Stacking inside a scene (bottom to top): the scene image, layers with `z < 0`, the scene
//! text, layers with `z >= 0`. Equal `z` keeps list order. Subtitles are always on top.

use serde::{Deserialize, Serialize};

use crate::model::{Easing, Keyframe};

/// Most layers one scene may have.
pub const MAX_LAYERS: usize = 32;
/// Longest text of one text layer, in characters.
pub const MAX_LAYER_TEXT_CHARS: usize = 400;
/// Most keyframes one layer may have.
pub const MAX_LAYER_KEYFRAMES: usize = 24;

/// Which point of the layer's box `x` and `y` place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    #[default]
    TopLeft,
    Center,
}

/// An entrance or exit animation of a layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerAnimationKind {
    /// Opacity 0 -> 1 (enter) or 1 -> 0 (exit).
    Fade,
    /// Fades while moving up: in from below (enter), out upward (exit).
    SlideUp,
    /// Fades while moving down: in from above, out downward.
    SlideDown,
    /// Fades while moving left: in from the right, out to the left.
    SlideLeft,
    /// Fades while moving right: in from the left, out to the right.
    SlideRight,
    /// Grows from half size with a small overshoot (enter), shrinks away (exit).
    Pop,
}

fn default_animation_ms() -> u64 {
    400
}

/// `enter` runs from the layer's start, `exit` ends at the layer's end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerAnimation {
    pub kind: LayerAnimationKind,
    #[serde(default = "default_animation_ms")]
    pub duration_ms: u64,
    /// Progress curve of the animation. Absent: the kind's own curve (ease-out cubic for an
    /// enter, ease-in cubic for an exit, back-out for `pop` entering).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ease: Option<Easing>,
}

/// How an image fills its box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageFit {
    /// Fills the whole box; the image's edges are cropped when the aspect ratios differ.
    #[default]
    Cover,
    /// The whole image is visible; the rest of the box stays transparent.
    Contain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    Left,
    #[default]
    Center,
    Right,
}

/// How a text layer's words appear.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextReveal {
    /// All at once (the layer's own enter animation still applies).
    #[default]
    None,
    /// Word by word, like the scene text.
    Words,
}

fn default_box_padding() -> f32 {
    0.4
}

fn default_box_radius() -> f32 {
    0.3
}

/// A filled box behind a text layer. `padding` and `radius` are fractions of the font size.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextBackground {
    pub color: String,
    #[serde(default = "default_box_padding")]
    pub padding: f32,
    #[serde(default = "default_box_radius")]
    pub radius: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShapeKind {
    #[default]
    Rect,
    Ellipse,
    /// From the box's top-left corner to its bottom-right corner (rotate it for other angles).
    /// `height` 0 is a horizontal line.
    Line,
}

fn default_font_size() -> f32 {
    0.06
}

fn default_text_color() -> String {
    "#fafafa".into()
}

fn default_weight() -> u16 {
    600
}

fn default_line_height() -> f32 {
    1.2
}

fn default_icon_color() -> String {
    "#fafafa".into()
}

/// An image file from the public dir.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageLayer {
    /// Path relative to the renderer's public dir.
    pub src: String,
    #[serde(default)]
    pub fit: ImageFit,
    /// Rounded corners, as a fraction of the frame height.
    #[serde(default)]
    pub corner_radius: f32,
}

/// A block of text. Lines wrap at the box width; without `height` the box is as tall as the
/// text, with `height` the text is centered vertically in it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextLayer {
    pub text: String,
    /// Fraction of the frame height.
    #[serde(default = "default_font_size")]
    pub font_size: f32,
    #[serde(default = "default_text_color")]
    pub color: String,
    /// CSS font weight, 100..900 in steps of 100.
    #[serde(default = "default_weight")]
    pub weight: u16,
    #[serde(default)]
    pub align: TextAlign,
    /// Line height as a multiple of the font size.
    #[serde(default = "default_line_height")]
    pub line_height: f32,
    #[serde(default)]
    pub background: Option<TextBackground>,
    /// Words drawn in the accent color, as in the scene text.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub highlights: Vec<String>,
    #[serde(default)]
    pub reveal: TextReveal,
}

/// A rectangle, ellipse or line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapeLayer {
    #[serde(default)]
    pub shape: ShapeKind,
    #[serde(default)]
    pub fill: Option<String>,
    #[serde(default)]
    pub stroke: Option<String>,
    /// Fraction of the frame height. 0 with a `stroke` color draws a 0.004 (about 4 px) stroke.
    #[serde(default)]
    pub stroke_width: f32,
    /// Rounded corners of a rect, as a fraction of the frame height.
    #[serde(default)]
    pub corner_radius: f32,
}

/// An SVG file from the public dir, drawn as a silhouette in one color.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IconLayer {
    /// Path of an `.svg` file relative to the renderer's public dir.
    pub src: String,
    #[serde(default = "default_icon_color")]
    pub color: String,
}

/// What a layer draws, tagged by `kind` in the JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LayerContent {
    Image(ImageLayer),
    Text(TextLayer),
    Shape(ShapeLayer),
    Icon(IconLayer),
}

impl LayerContent {
    /// The `kind` tag.
    pub fn kind(&self) -> &'static str {
        match self {
            LayerContent::Image(_) => "image",
            LayerContent::Text(_) => "text",
            LayerContent::Shape(_) => "shape",
            LayerContent::Icon(_) => "icon",
        }
    }

    /// The asset file the layer draws, if any.
    pub fn asset(&self) -> Option<&str> {
        match self {
            LayerContent::Image(l) => Some(&l.src),
            LayerContent::Icon(l) => Some(&l.src),
            LayerContent::Text(_) | LayerContent::Shape(_) => None,
        }
    }
}

fn default_opacity() -> f32 {
    1.0
}

fn is_zero_u64(v: &u64) -> bool {
    *v == 0
}

fn is_zero_i32(v: &i32) -> bool {
    *v == 0
}

fn is_zero_f32(v: &f32) -> bool {
    *v == 0.0
}

fn is_one(v: &f32) -> bool {
    *v == 1.0
}

fn is_default_anchor(v: &Anchor) -> bool {
    *v == Anchor::TopLeft
}

/// One layer of a scene (see the module docs for units and stacking).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    /// Unique within the scene.
    pub id: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    /// Required except for text, whose box is then as tall as its text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f32>,
    #[serde(default, skip_serializing_if = "is_default_anchor")]
    pub anchor: Anchor,
    /// Stacking order: below the scene text when negative, above it otherwise.
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub z: i32,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub start_ms: u64,
    /// Absent: until the scene ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_ms: Option<u64>,
    #[serde(default = "default_opacity", skip_serializing_if = "is_one")]
    pub opacity: f32,
    /// Clockwise, in degrees, about the box center.
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub rotation: f32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keyframes: Vec<Keyframe>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enter: Option<LayerAnimation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<LayerAnimation>,
    #[serde(flatten)]
    pub content: LayerContent,
}

impl Layer {
    /// End of the layer in ms from the scene start, for a scene of `scene_ms`.
    pub fn end_ms_in(&self, scene_ms: u64) -> u64 {
        self.end_ms.unwrap_or(scene_ms)
    }
}

/// `#rgb`, `#rrggbb` or `#rrggbbaa`.
pub fn is_hex_color(s: &str) -> bool {
    let Some(hex) = s.strip_prefix('#') else {
        return false;
    };
    matches!(hex.len(), 3 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_layers_get_defaults() {
        let l: Layer = serde_json::from_str(
            r#"{"id":"t","kind":"text","x":0.1,"y":0.2,"width":0.5,"text":"hi"}"#,
        )
        .unwrap();
        assert_eq!(l.anchor, Anchor::TopLeft);
        assert_eq!(l.opacity, 1.0);
        assert_eq!(l.height, None);
        let LayerContent::Text(t) = &l.content else {
            panic!("text")
        };
        assert_eq!(t.font_size, 0.06);
        assert_eq!(t.color, "#fafafa");
        assert_eq!(t.weight, 600);
        assert_eq!(t.align, TextAlign::Center);
        let l: Layer = serde_json::from_str(
            r##"{"id":"s","kind":"shape","x":0,"y":0,"width":1,"height":0.1,"fill":"#000"}"##,
        )
        .unwrap();
        assert!(matches!(&l.content, LayerContent::Shape(s) if s.shape == ShapeKind::Rect));
    }

    #[test]
    fn layers_round_trip_without_default_keys() {
        let json = r##"{"id":"logo","x":0.5,"y":0.5,"width":0.1,"height":0.1,"anchor":"center","z":-1,"start_ms":200,"end_ms":900,"enter":{"kind":"pop","duration_ms":300},"kind":"icon","src":"assets/icons/star.svg","color":"#ffcc00"}"##;
        let l: Layer = serde_json::from_str(json).unwrap();
        assert_eq!(l.content.kind(), "icon");
        assert_eq!(l.content.asset(), Some("assets/icons/star.svg"));
        let back = serde_json::to_string(&l).unwrap();
        let again: Layer = serde_json::from_str(&back).unwrap();
        assert_eq!(again, l);
        assert!(!back.contains("rotation"));
        assert!(!back.contains("opacity"));
    }

    #[test]
    fn unknown_kind_is_a_parse_error() {
        assert!(serde_json::from_str::<Layer>(
            r#"{"id":"x","kind":"video","x":0,"y":0,"width":1,"height":1,"src":"a.mp4"}"#
        )
        .is_err());
    }

    #[test]
    fn hex_colors() {
        for ok in ["#fff", "#FFFFFF", "#11223344"] {
            assert!(is_hex_color(ok), "{ok}");
        }
        for bad in ["fff", "#ffff", "#ggg", "red", "#1234567", ""] {
            assert!(!is_hex_color(bad), "{bad}");
        }
    }
}
