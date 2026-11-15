//! Text style: an outline, a background box and a shadow that keep text readable over pictures.
//!
//! The same three parts style the scene text (`Scene.text_style`), text layers (their own
//! `outline`, `background` and `shadow` fields) and, as a default for both, the brand
//! (`brand.text_style`). Each part is picked on its own: the element's, else the brand's, else
//! the built-in one (no outline, no box; the scene text's soft shadow, no shadow on text layers).
//! Leaving everything unset renders exactly as before text styles existed.
//!
//! Lengths of the outline and of an explicit shadow are px on a 1080p frame and scale with the
//! frame: px × (shorter frame side / 1080). Both supported resolutions (1920x1080, 1080x1920) have
//! a shorter side of 1080, so the numbers are the px drawn. The box's `padding` and `radius` are
//! fractions of the font size, as the text layer `background` always had. The box may have a
//! `border` (color, width px at 1080p) that follows its corner radius; a box without `color` has no
//! fill, so a border alone frames the text.
//!
//! To switch off a part that the brand sets, set it on the element to "nothing": an outline of
//! `width` 0, `shadow: "none"`, or a box with a fully transparent color (`#00000000`).
//!
//! How the renderer draws them (`remotion-renderer/src/textStyle.ts`): the outline is
//! `-webkit-text-stroke` at twice the width with `paint-order: stroke fill`, so the stroke's inner
//! half is covered by the fill and the visible ring is `width` px outside the glyph (thin CJK
//! strokes keep their full weight); the scene text's box is drawn per line
//! (`box-decoration-break: clone`) so it hugs the lines; a text layer's box fills the layer's
//! width as it always did.

use serde::{Deserialize, Serialize};

use crate::layer::{is_hex_color, TextBackground};

/// Widest outline, px at 1080p.
pub const MAX_OUTLINE_PX: f32 = 16.0;
/// Widest box border, px at 1080p.
pub const MAX_BORDER_PX: f32 = 16.0;
/// Largest explicit shadow offset (either axis), px at 1080p.
pub const MAX_SHADOW_OFFSET_PX: f32 = 40.0;
/// Largest explicit shadow blur, px at 1080p.
pub const MAX_SHADOW_BLUR_PX: f32 = 80.0;

fn default_outline_color() -> String {
    "#000000".into()
}

fn default_outline_width() -> f32 {
    4.0
}

/// A ring of `color` around every glyph, `width` px (at 1080p) outside its edge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextOutline {
    #[serde(default = "default_outline_color")]
    pub color: String,
    /// px at 1080p, 0..=16. 0 draws no outline (it switches off the brand's).
    #[serde(default = "default_outline_width")]
    pub width: f32,
}

/// A named shadow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShadowPreset {
    None,
    /// `0 4px 32px rgba(0,0,0,.6)`: the scene text's built-in shadow.
    Soft,
    /// A tight dark shadow plus a wider one: `0 2px 4px rgba(0,0,0,.85), 0 4px 18px rgba(0,0,0,.75)`.
    Strong,
}

/// One explicit shadow, lengths in px at 1080p.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomShadow {
    pub color: String,
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    #[serde(default)]
    pub blur: f32,
}

/// `"none"`, `"soft"`, `"strong"`, or `{"color", "x", "y", "blur"}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TextShadow {
    Preset(ShadowPreset),
    Custom(CustomShadow),
}

/// Outline, box and shadow of a text. Every part is optional; see the module docs.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TextStyle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline: Option<TextOutline>,
    /// The box behind the text (the same type as a text layer's `background`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<TextBackground>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<TextShadow>,
}

impl TextStyle {
    /// Each part of `self`, else of `fallback`.
    pub fn or(&self, fallback: Option<&TextStyle>) -> TextStyle {
        TextStyle {
            outline: self
                .outline
                .clone()
                .or_else(|| fallback.and_then(|f| f.outline.clone())),
            background: self
                .background
                .clone()
                .or_else(|| fallback.and_then(|f| f.background.clone())),
            shadow: self
                .shadow
                .clone()
                .or_else(|| fallback.and_then(|f| f.shadow.clone())),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.outline.is_none() && self.background.is_none() && self.shadow.is_none()
    }

    /// The outline that is drawn (width above 0), if any.
    pub fn drawn_outline(&self) -> Option<&TextOutline> {
        self.outline.as_ref().filter(|o| o.width > 0.0)
    }

    /// Every problem, as messages naming the field under `prefix` (e.g. `text_style.`).
    pub fn problems(&self, prefix: &str) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(o) = &self.outline {
            out.extend(outline_problems(o, prefix));
        }
        if let Some(b) = &self.background {
            out.extend(background_problems(b, prefix));
        }
        if let Some(s) = &self.shadow {
            out.extend(shadow_problems(s, prefix));
        }
        out
    }
}

fn color_problem(field: &str, v: &str) -> Option<String> {
    (!is_hex_color(v)).then(|| format!("{field} `{v}` must be a color like #rgb, #rrggbb or #rrggbbaa"))
}

fn in_range(v: f32, lo: f32, hi: f32) -> bool {
    v.is_finite() && (lo..=hi).contains(&v)
}

pub(crate) fn outline_problems(o: &TextOutline, prefix: &str) -> Vec<String> {
    let mut out: Vec<String> = color_problem(&format!("{prefix}outline color"), &o.color)
        .into_iter()
        .collect();
    if !in_range(o.width, 0.0, MAX_OUTLINE_PX) {
        out.push(format!(
            "{prefix}outline width must be between 0 and {MAX_OUTLINE_PX} (px at 1080p), got {}",
            o.width
        ));
    }
    out
}

pub(crate) fn background_problems(b: &TextBackground, prefix: &str) -> Vec<String> {
    let mut out: Vec<String> = color_problem(&format!("{prefix}background color"), &b.color)
        .into_iter()
        .collect();
    if !in_range(b.padding, 0.0, 3.0) || !in_range(b.radius, 0.0, 3.0) {
        out.push(format!(
            "{prefix}background padding and radius must be between 0 and 3 (font sizes)"
        ));
    }
    out.extend(border_problems(b, prefix));
    out
}

/// Problems of a box's `border` (color, width 0..=[`MAX_BORDER_PX`]).
pub(crate) fn border_problems(b: &TextBackground, prefix: &str) -> Vec<String> {
    let Some(border) = &b.border else {
        return Vec::new();
    };
    let mut out: Vec<String> =
        color_problem(&format!("{prefix}background border color"), &border.color)
            .into_iter()
            .collect();
    if !in_range(border.width, 0.0, MAX_BORDER_PX) {
        out.push(format!(
            "{prefix}background border width must be between 0 and {MAX_BORDER_PX} (px at 1080p), got {}",
            border.width
        ));
    }
    out
}

pub(crate) fn shadow_problems(s: &TextShadow, prefix: &str) -> Vec<String> {
    let TextShadow::Custom(c) = s else {
        return Vec::new();
    };
    let mut out: Vec<String> = color_problem(&format!("{prefix}shadow color"), &c.color)
        .into_iter()
        .collect();
    let off = MAX_SHADOW_OFFSET_PX;
    if !in_range(c.x, -off, off) || !in_range(c.y, -off, off) {
        out.push(format!(
            "{prefix}shadow x and y must be between -{off} and {off} (px at 1080p)"
        ));
    }
    if !in_range(c.blur, 0.0, MAX_SHADOW_BLUR_PX) {
        out.push(format!(
            "{prefix}shadow blur must be between 0 and {MAX_SHADOW_BLUR_PX} (px at 1080p), got {}",
            c.blur
        ));
    }
    out
}

/// px at 1080p to px on a `width` x `height` frame.
pub fn style_scale(width: u32, height: u32) -> f64 {
    f64::from(width.min(height)) / 1080.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_omits_unset_parts() {
        let json = r##"{"outline":{"color":"#000000","width":6.0},"background":{"color":"#000000aa","padding":0.3,"radius":0.2},"shadow":"strong"}"##;
        let s: TextStyle = serde_json::from_str(json).unwrap();
        assert_eq!(serde_json::to_string(&s).unwrap(), json);
        let empty: TextStyle = serde_json::from_str("{}").unwrap();
        assert!(empty.is_empty());
        assert_eq!(serde_json::to_string(&empty).unwrap(), "{}");
        let o: TextOutline = serde_json::from_str("{}").unwrap();
        assert_eq!((o.color.as_str(), o.width), ("#000000", 4.0));
        let c: TextStyle =
            serde_json::from_str(r##"{"shadow":{"color":"#000","y":3,"blur":10}}"##).unwrap();
        assert_eq!(
            c.shadow,
            Some(TextShadow::Custom(CustomShadow {
                color: "#000".into(),
                x: 0.0,
                y: 3.0,
                blur: 10.0
            }))
        );
        assert!(serde_json::from_str::<TextStyle>(r#"{"shadow":"huge"}"#).is_err());
    }

    #[test]
    fn element_parts_win_over_the_fallback_one_by_one() {
        let brand: TextStyle = serde_json::from_str(
            r##"{"outline":{"color":"#111","width":3},"shadow":"soft"}"##,
        )
        .unwrap();
        let own: TextStyle = serde_json::from_str(r##"{"shadow":"none"}"##).unwrap();
        let m = own.or(Some(&brand));
        assert_eq!(m.outline.as_ref().unwrap().color, "#111");
        assert_eq!(m.shadow, Some(TextShadow::Preset(ShadowPreset::None)));
        assert!(m.background.is_none());
        let off: TextStyle = serde_json::from_str(r#"{"outline":{"width":0}}"#).unwrap();
        assert!(off.or(Some(&brand)).drawn_outline().is_none());
        assert_eq!(TextStyle::default().or(None), TextStyle::default());
    }

    #[test]
    fn problems_name_the_field() {
        let s: TextStyle = serde_json::from_str(
            r##"{"outline":{"color":"black","width":40},"background":{"color":"#000","padding":9},"shadow":{"color":"#zzz","x":100,"blur":-1}}"##,
        )
        .unwrap();
        let p = s.problems("text_style.");
        assert_eq!(p.len(), 6, "{p:?}");
        assert!(p[0].starts_with("text_style.outline color `black`"));
        let ok: TextStyle = serde_json::from_str(
            r##"{"outline":{"color":"#000000cc","width":16},"shadow":{"color":"#00000088","x":-2,"y":4,"blur":12}}"##,
        )
        .unwrap();
        assert!(ok.problems("").is_empty());
        assert_eq!(style_scale(1920, 1080), 1.0);
        assert_eq!(style_scale(1080, 1920), 1.0);
    }

    #[test]
    fn box_border_is_optional_and_a_box_may_have_no_fill() {
        // An old background re-serializes without a border key.
        let old: TextBackground =
            serde_json::from_str(r##"{"color":"#000000aa"}"##).unwrap();
        assert_eq!(
            serde_json::to_string(&old).unwrap(),
            r##"{"color":"#000000aa","padding":0.4,"radius":0.3}"##
        );
        assert_eq!(old.border_px(), 0.0);
        let frame: TextStyle =
            serde_json::from_str(r##"{"background":{"border":{"color":"#ffffff"}}}"##).unwrap();
        let b = frame.background.as_ref().unwrap();
        assert_eq!((b.color.as_str(), b.border_px()), ("#00000000", 2.0));
        assert!(frame.problems("").is_empty());
        let bad: TextStyle = serde_json::from_str(
            r##"{"background":{"border":{"color":"white","width":17}}}"##,
        )
        .unwrap();
        let p = bad.problems("text_style.");
        assert_eq!(p.len(), 2, "{p:?}");
        assert!(p[0].contains("text_style.background border color `white`"));
        assert!(p[1].contains("border width"));
    }

}
