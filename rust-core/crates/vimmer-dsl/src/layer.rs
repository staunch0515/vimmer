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
    /// Character by character at `typing_cps`, from the layer's start. Characters are counted
    /// as [`typing_units`] (an emoji or an accented letter is one character).
    Typewriter,
}

/// Default typing speed of the typewriter reveal, in characters per second.
pub const DEFAULT_TYPING_CPS: f32 = 20.0;
/// Highest typing speed.
pub const MAX_TYPING_CPS: f32 = 200.0;

/// Code points that attach to the code point before them instead of being typed on their own:
/// combining marks, variation selectors, zero-width joiner, emoji skin tones, tag characters.
/// Mirrors `joinsPrevious` in `remotion-renderer/src/motion/typewriter.ts`.
fn joins_previous(c: char) -> bool {
    matches!(u32::from(c),
        0x0300..=0x036F
        | 0x1AB0..=0x1AFF
        | 0x1DC0..=0x1DFF
        | 0x20D0..=0x20FF
        | 0xFE20..=0xFE2F
        | 0xFE00..=0xFE0F
        | 0x200D
        | 0x1F3FB..=0x1F3FF
        | 0xE0020..=0xE007F
        | 0xE0100..=0xE01EF)
}

fn is_regional_indicator(c: char) -> bool {
    (0x1F1E6..=0x1F1FF).contains(&u32::from(c))
}

/// The text split into the units a typewriter types one at a time: a code point plus everything
/// that attaches to it (combining marks, variation selectors, skin tones, ZWJ sequences, flag
/// pairs). An approximation of grapheme clusters that is good for CJK, emoji and accents and
/// needs no Unicode tables; the renderer uses the same rule.
pub fn typing_units(text: &str) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    let mut start = 0;
    let mut prev: Option<char> = None;
    // Regional indicators already in the current unit (a flag is two).
    let mut ri = 0;
    for (i, c) in text.char_indices() {
        let attach = match prev {
            None => false,
            Some(p) => {
                joins_previous(c) || p == '\u{200D}' || (is_regional_indicator(c) && ri == 1)
            }
        };
        if !attach && i > 0 {
            out.push(&text[start..i]);
            start = i;
            ri = 0;
        }
        if is_regional_indicator(c) {
            ri += 1;
        }
        prev = Some(c);
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
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
    /// Typing speed of the typewriter reveal, characters per second. Absent:
    /// [`DEFAULT_TYPING_CPS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typing_cps: Option<f32>,
    /// A blinking cursor after the typed text (typewriter reveal only).
    #[serde(default, skip_serializing_if = "is_false")]
    pub cursor: bool,
}

impl TextLayer {
    /// How long the typewriter takes to type the whole text, in ms (0 for other reveals).
    pub fn typing_ms(&self) -> u64 {
        if self.reveal != TextReveal::Typewriter {
            return 0;
        }
        let cps = f64::from(self.typing_cps.unwrap_or(DEFAULT_TYPING_CPS));
        (typing_units(&self.text).len() as f64 * 1000.0 / cps).ceil() as u64
    }
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

/// Longest lower-third title, in characters.
pub const MAX_LOWER_THIRD_TITLE_CHARS: usize = 120;
/// Longest lower-third subtitle, in characters.
pub const MAX_LOWER_THIRD_SUBTITLE_CHARS: usize = 160;
/// Built-in entrance and exit of a lower third, used when the layer has no `enter` / `exit`.
/// Each is shortened to 40 % / 30 % of the layer's time when the layer is short.
pub const LOWER_THIRD_IN_MS: u64 = 700;
pub const LOWER_THIRD_OUT_MS: u64 = 450;

fn default_lower_third_size() -> f32 {
    0.045
}

fn default_subtitle_color() -> String {
    "#d4d4d8".into()
}

fn default_accent_color() -> String {
    "#6366f1".into()
}

fn default_lower_third_background() -> Option<String> {
    Some("#09090bd9".into())
}

/// A name/title strip: an accent bar beside a title and an optional subtitle on a box. Without
/// `height` the box is as tall as its text. Unless the layer has its own `enter` / `exit`, the
/// accent bar grows, the box wipes in from the left and the text slides in; on the way out the
/// box wipes back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LowerThirdLayer {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    /// Title size as a fraction of the frame height; the subtitle is 0.62 of it.
    #[serde(default = "default_lower_third_size")]
    pub font_size: f32,
    /// Title color.
    #[serde(default = "default_text_color")]
    pub color: String,
    #[serde(default = "default_subtitle_color")]
    pub subtitle_color: String,
    #[serde(default = "default_accent_color")]
    pub accent_color: String,
    /// Box color; `null` draws no box.
    #[serde(default = "default_lower_third_background")]
    pub background_color: Option<String>,
}

impl LowerThirdLayer {
    /// Built-in (entrance, exit) lengths in ms for a layer shown `span_ms`.
    pub fn builtin_ms(span_ms: u64) -> (u64, u64) {
        (
            LOWER_THIRD_IN_MS.min(span_ms * 2 / 5),
            LOWER_THIRD_OUT_MS.min(span_ms * 3 / 10),
        )
    }
}

/// What a layer draws, tagged by `kind` in the JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LayerContent {
    Image(ImageLayer),
    Text(TextLayer),
    Shape(ShapeLayer),
    Icon(IconLayer),
    LowerThird(LowerThirdLayer),
}

impl LayerContent {
    /// The `kind` tag.
    pub fn kind(&self) -> &'static str {
        match self {
            LayerContent::Image(_) => "image",
            LayerContent::Text(_) => "text",
            LayerContent::Shape(_) => "shape",
            LayerContent::Icon(_) => "icon",
            LayerContent::LowerThird(_) => "lower_third",
        }
    }

    /// Kinds whose box may leave out `height` (it is then as tall as the content).
    pub fn auto_height(&self) -> bool {
        matches!(self, LayerContent::Text(_) | LayerContent::LowerThird(_))
    }

    /// The asset file the layer draws, if any.
    pub fn asset(&self) -> Option<&str> {
        match self {
            LayerContent::Image(l) => Some(&l.src),
            LayerContent::Icon(l) => Some(&l.src),
            _ => None,
        }
    }
}

fn default_opacity() -> f32 {
    1.0
}

fn is_false(v: &bool) -> bool {
    !*v
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
    fn typing_units_keep_emoji_accents_and_flags_whole() {
        assert_eq!(typing_units("abc"), ["a", "b", "c"]);
        assert_eq!(typing_units("你好，世界"), ["你", "好", "，", "世", "界"]);
        // e + combining acute, thumbs up + skin tone, family ZWJ sequence, flag, keycap.
        let s = "e\u{301}👍🏽👨\u{200D}👩\u{200D}👧🇯🇵1\u{FE0F}\u{20E3}!";
        let u = typing_units(s);
        assert_eq!(u.len(), 6, "{u:?}");
        assert_eq!(u.concat(), s);
        assert_eq!(u[3], "🇯🇵");
        // Two flags in a row are two units.
        assert_eq!(typing_units("🇯🇵🇫🇷").len(), 2);
        assert!(typing_units("").is_empty());
    }

    #[test]
    fn typewriter_fields_default_and_time() {
        let mut t: Layer = serde_json::from_str(
            r#"{"id":"t","kind":"text","x":0,"y":0,"width":1,"text":"你好世界!","reveal":"typewriter"}"#,
        )
        .unwrap();
        let LayerContent::Text(text) = &mut t.content else {
            panic!()
        };
        assert_eq!(text.typing_cps, None);
        assert!(!text.cursor);
        assert_eq!(text.typing_ms(), 250); // 5 units at 20 cps
        text.typing_cps = Some(3.0);
        assert_eq!(text.typing_ms(), 1667);
        let json = serde_json::to_string(&t).unwrap();
        assert!(!json.contains("cursor"));
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
