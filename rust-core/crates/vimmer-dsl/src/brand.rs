//! Brand kit: colors, fonts and a logo shared by every scene of a video.
//!
//! Brand values are *defaults*. A field set on a layer always wins; a field left out falls back
//! to the brand, and without a brand (or without that brand field) to the built-in value the
//! renderer has always used. The built-in values are the `DEFAULT_*` constants below and are
//! mirrored in `remotion-renderer/src/brand.ts`, so a timeline without `brand` renders exactly as
//! it did before brands existed.
//!
//! Which brand color each element uses when it has no color of its own:
//!
//! | element                                   | brand color          | built-in    |
//! |-------------------------------------------|----------------------|-------------|
//! | scene text, text layer, counter           | `text`               | `#fafafa`   |
//! | lower-third title, ring label             | `text`               | `#fafafa`   |
//! | lower-third subtitle                      | `muted_text`         | `#d4d4d8`   |
//! | chart labels and values                   | `muted_text`         | `#e4e4e7`   |
//! | highlighted words (scene text, layers)    | `accent`             | `#a5b4fc`   |
//! | lower-third accent bar, chart bars, ring  | `primary`            | `#6366f1`   |
//! | frame behind scenes without a background  | `background`         | `#09090b`   |
//! | subtitle box                              | `subtitle_background`| `#09090bb8` |
//! | subtitle text                             | `text`               | `#fafafa`   |
//!
//! Fonts: `heading` is used by the scene text, lower-third titles, counters and template
//! headings (text layers with `font: heading`); `body` by subtitles, text layers, lower-third
//! subtitles, chart and ring labels. A font with a `src` is loaded from that file before the first
//! frame is drawn; without `src` the family must be installed on the machine that renders.

use serde::{Deserialize, Serialize};

use crate::layer::is_hex_color;
use crate::validation::asset_path_problem;

/// Built-in colors (no brand). Mirrored in `remotion-renderer/src/brand.ts`.
pub const DEFAULT_TEXT: &str = "#fafafa";
pub const DEFAULT_LOWER_THIRD_SUBTITLE: &str = "#d4d4d8";
pub const DEFAULT_CHART_LABEL: &str = "#e4e4e7";
pub const DEFAULT_ACCENT: &str = "#a5b4fc";
pub const DEFAULT_PRIMARY: &str = "#6366f1";
pub const DEFAULT_BACKGROUND: &str = "#09090b";
/// `rgba(9, 9, 11, 0.72)` as hex.
pub const DEFAULT_SUBTITLE_BACKGROUND: &str = "#09090bb8";

/// Font file extensions a brand font may have (lower case).
pub const FONT_EXTENSIONS: [&str; 4] = ["ttf", "otf", "woff", "woff2"];
/// Image file extensions a logo may have (lower case): the raster formats image layers use, since
/// templates draw the logo as an image layer.
pub const LOGO_EXTENSIONS: [&str; 3] = ["png", "jpg", "jpeg"];
/// Longest font family name.
pub const MAX_FAMILY_CHARS: usize = 64;

/// Brand colors, each `#rgb`, `#rrggbb` or `#rrggbbaa`. Every one is optional.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BrandColors {
    /// Graphics: lower-third accent bars, chart bars, progress rings, template shapes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary: Option<String>,
    /// Highlighted words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    /// Text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Secondary text: lower-third subtitles, chart labels, template sub-lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted_text: Option<String>,
    /// Behind scenes that have no image or video, and template backgrounds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// The box behind subtitles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle_background: Option<String>,
}

/// A font family, optionally loaded from a file under the public dir.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrandFont {
    /// CSS family name. With `src`, the file is registered under this name, so pick a name that
    /// is not an installed font (or the installed one may be used when the file is missing).
    pub family: String,
    /// A .ttf, .otf, .woff or .woff2 file relative to the public dir. One file is used for every
    /// weight (it is not made bolder or thinner).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BrandFonts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading: Option<BrandFont>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<BrandFont>,
}

/// Which brand font a text layer uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FontRole {
    Heading,
    #[default]
    Body,
}

/// Corner of the frame the logo sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Corner {
    TopLeft,
    #[default]
    TopRight,
    BottomLeft,
    BottomRight,
}

fn default_logo_size() -> f32 {
    0.08
}

fn default_logo_margin() -> f32 {
    0.05
}

fn default_one() -> f32 {
    1.0
}

/// A logo drawn on every scene (except `hide_on`), above the layers and below the subtitles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Logo {
    /// A .png, .jpg or .jpeg file relative to the public dir (transparent PNG works best).
    pub src: String,
    #[serde(default)]
    pub corner: Corner,
    /// Logo height as a fraction of the frame height; its width follows the image's shape.
    #[serde(default = "default_logo_size")]
    pub size: f32,
    /// Distance from the two nearest edges: a fraction of the frame width (left/right) and of
    /// the frame height (top/bottom). 0.05 puts the logo on the edge of the title-safe area.
    #[serde(default = "default_logo_margin")]
    pub margin: f32,
    #[serde(default = "default_one")]
    pub opacity: f32,
    /// Scene ids where the logo is not drawn (for example an intro that shows it large).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hide_on: Vec<String>,
}

/// Colors, fonts and logo of a video. See the module docs for which elements use what.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Brand {
    #[serde(default)]
    pub colors: BrandColors,
    #[serde(default)]
    pub fonts: BrandFonts,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo: Option<Logo>,
}

/// The color to use: the element's own, else the brand's, else the built-in one.
pub fn pick<'a>(own: Option<&'a str>, brand: Option<&'a str>, builtin: &'a str) -> &'a str {
    own.or(brand).unwrap_or(builtin)
}

impl Brand {
    pub fn primary(brand: Option<&Brand>) -> &str {
        pick(
            None,
            brand.and_then(|b| b.colors.primary.as_deref()),
            DEFAULT_PRIMARY,
        )
    }
    pub fn accent(brand: Option<&Brand>) -> &str {
        pick(
            None,
            brand.and_then(|b| b.colors.accent.as_deref()),
            DEFAULT_ACCENT,
        )
    }
    pub fn text(brand: Option<&Brand>) -> &str {
        pick(
            None,
            brand.and_then(|b| b.colors.text.as_deref()),
            DEFAULT_TEXT,
        )
    }
    /// The muted text color, with the element's own built-in when the brand has none.
    pub fn muted_text<'a>(brand: Option<&'a Brand>, builtin: &'a str) -> &'a str {
        pick(
            None,
            brand.and_then(|b| b.colors.muted_text.as_deref()),
            builtin,
        )
    }
    pub fn background(brand: Option<&Brand>) -> &str {
        pick(
            None,
            brand.and_then(|b| b.colors.background.as_deref()),
            DEFAULT_BACKGROUND,
        )
    }
    pub fn subtitle_background(brand: Option<&Brand>) -> &str {
        pick(
            None,
            brand.and_then(|b| b.colors.subtitle_background.as_deref()),
            DEFAULT_SUBTITLE_BACKGROUND,
        )
    }

    /// Every file the brand needs (fonts and logo), with what each one is.
    pub fn assets(&self) -> Vec<(&'static str, &str)> {
        let mut out = Vec::new();
        if let Some(src) = self.fonts.heading.as_ref().and_then(|f| f.src.as_deref()) {
            out.push(("heading font", src));
        }
        if let Some(src) = self.fonts.body.as_ref().and_then(|f| f.src.as_deref()) {
            out.push(("body font", src));
        }
        if let Some(logo) = &self.logo {
            out.push(("logo", logo.src.as_str()));
        }
        out
    }

    /// Every problem of the brand, as messages (colors, font names and files, logo fields).
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        let c = &self.colors;
        for (name, v) in [
            ("primary", &c.primary),
            ("accent", &c.accent),
            ("text", &c.text),
            ("muted_text", &c.muted_text),
            ("background", &c.background),
            ("subtitle_background", &c.subtitle_background),
        ] {
            if let Some(v) = v {
                if !is_hex_color(v) {
                    out.push(format!(
                        "colors.{name} `{v}` must be a color like #rgb, #rrggbb or #rrggbbaa"
                    ));
                }
            }
        }
        for (name, font) in [("heading", &self.fonts.heading), ("body", &self.fonts.body)] {
            let Some(font) = font else { continue };
            let fam = font.family.trim();
            if fam.is_empty()
                || fam.chars().count() > MAX_FAMILY_CHARS
                || font
                    .family
                    .chars()
                    .any(|ch| ch.is_control() || matches!(ch, '"' | '\'' | ';' | ',' | '\\'))
            {
                out.push(format!(
                    "fonts.{name}.family must be 1-{MAX_FAMILY_CHARS} characters without quotes, commas, `;` or `\\`"
                ));
            }
            if let Some(src) = &font.src {
                if let Some(p) = file_problem(src, &FONT_EXTENSIONS, "a font") {
                    out.push(format!("fonts.{name}.src: {p}"));
                }
            }
        }
        if let Some(logo) = &self.logo {
            if let Some(p) = file_problem(&logo.src, &LOGO_EXTENSIONS, "a logo") {
                out.push(format!("logo.src: {p}"));
            }
            if !(logo.size.is_finite() && logo.size > 0.0 && logo.size <= 0.5) {
                out.push(format!(
                    "logo.size must be above 0 and at most 0.5 (fraction of the frame height), got {}",
                    logo.size
                ));
            }
            if !(logo.margin.is_finite() && (0.0..=0.4).contains(&logo.margin)) {
                out.push(format!(
                    "logo.margin must be between 0 and 0.4, got {}",
                    logo.margin
                ));
            }
            if !(logo.opacity.is_finite() && (0.0..=1.0).contains(&logo.opacity)) {
                out.push(format!(
                    "logo.opacity must be between 0 and 1, got {}",
                    logo.opacity
                ));
            }
        }
        out
    }
}

/// Why `src` is not a usable file path with one of `exts`, if it is not.
fn file_problem(src: &str, exts: &[&str], what: &str) -> Option<String> {
    if let Some(p) = asset_path_problem(src) {
        return Some(p.to_string());
    }
    let ext = src
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    if !exts.contains(&ext.as_str()) {
        return Some(format!("{what} must be a .{} file", exts.join(", .")));
    }
    None
}

/// Parses `#rgb`, `#rrggbb` or `#rrggbbaa` into (r, g, b, a) in 0..=255.
pub fn parse_hex(s: &str) -> Option<[u8; 4]> {
    if !is_hex_color(s) {
        return None;
    }
    let hex = &s[1..];
    let digit = |i: usize| u8::from_str_radix(&hex[i..=i], 16).ok();
    let pair = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some(match hex.len() {
        3 => [digit(0)? * 17, digit(1)? * 17, digit(2)? * 17, 255],
        6 => [pair(0)?, pair(2)?, pair(4)?, 255],
        _ => [pair(0)?, pair(2)?, pair(4)?, pair(6)?],
    })
}

/// WCAG relative luminance of an sRGB color.
pub fn luminance(rgb: [u8; 3]) -> f64 {
    let lin = |c: u8| {
        let c = f64::from(c) / 255.0;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(rgb[0]) + 0.7152 * lin(rgb[1]) + 0.0722 * lin(rgb[2])
}

/// `top` (which may be translucent) composited over the opaque `bottom`.
pub fn over(top: [u8; 4], bottom: [u8; 3]) -> [u8; 3] {
    let a = f64::from(top[3]) / 255.0;
    let mix = |t: u8, b: u8| (f64::from(t) * a + f64::from(b) * (1.0 - a)).round() as u8;
    [
        mix(top[0], bottom[0]),
        mix(top[1], bottom[1]),
        mix(top[2], bottom[2]),
    ]
}

/// WCAG contrast ratio (1..=21) between two opaque colors.
pub fn contrast_ratio(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brand_round_trips_and_omits_empty_parts() {
        let json = r##"{"colors":{"primary":"#ff5500","text":"#fff"},"fonts":{"heading":{"family":"Demo Heading","src":"assets/fonts/demo.ttf"}},"logo":{"src":"assets/logo.png","corner":"bottom_left","size":0.1,"margin":0.05,"opacity":0.9,"hide_on":["intro"]}}"##;
        let b: Brand = serde_json::from_str(json).unwrap();
        assert_eq!(b.logo.as_ref().unwrap().corner, Corner::BottomLeft);
        assert_eq!(serde_json::to_string(&b).unwrap(), json);
        let empty: Brand = serde_json::from_str("{}").unwrap();
        assert_eq!(
            serde_json::to_string(&empty).unwrap(),
            r#"{"colors":{},"fonts":{}}"#
        );
        let logo: Logo = serde_json::from_str(r#"{"src":"a.png"}"#).unwrap();
        assert_eq!(
            (logo.corner, logo.size, logo.margin, logo.opacity),
            (Corner::TopRight, 0.08, 0.05, 1.0)
        );
    }

    #[test]
    fn defaults_without_a_brand_are_the_builtins() {
        assert_eq!(Brand::text(None), "#fafafa");
        assert_eq!(Brand::primary(None), "#6366f1");
        assert_eq!(Brand::background(None), "#09090b");
        let b: Brand = serde_json::from_str(r##"{"colors":{"primary":"#123456"}}"##).unwrap();
        assert_eq!(Brand::primary(Some(&b)), "#123456");
        assert_eq!(Brand::text(Some(&b)), "#fafafa");
        assert_eq!(
            Brand::muted_text(Some(&b), DEFAULT_CHART_LABEL),
            DEFAULT_CHART_LABEL
        );
        assert_eq!(pick(Some("#000"), Some("#111"), "#222"), "#000");
    }

    #[test]
    fn brand_problems() {
        let b: Brand = serde_json::from_str(
            r##"{"colors":{"accent":"orange"},"fonts":{"body":{"family":"A, B","src":"f.gif"}},"logo":{"src":"../l.png","size":2,"opacity":-1}}"##,
        )
        .unwrap();
        let p = b.problems();
        assert_eq!(p.len(), 6, "{p:?}");
        assert!(p[0].contains("colors.accent"));
        assert!(p
            .iter()
            .any(|m| m.contains("fonts.body.src: a font must be")));
        let ok: Brand = serde_json::from_str(
            r##"{"colors":{"accent":"#abc"},"fonts":{"body":{"family":"My Font","src":"assets/fonts/x.woff2"}},"logo":{"src":"assets/logo.png"}}"##,
        )
        .unwrap();
        assert!(ok.problems().is_empty());
        assert_eq!(ok.assets().len(), 2);
    }

    #[test]
    fn contrast_math_matches_wcag() {
        let white = [255, 255, 255];
        let black = [0, 0, 0];
        assert!((contrast_ratio(white, black) - 21.0).abs() < 1e-9);
        assert!((contrast_ratio(white, white) - 1.0).abs() < 1e-9);
        // #777 on white is about 4.48:1, just under AA.
        let grey = parse_hex("#777").unwrap();
        let r = contrast_ratio([grey[0], grey[1], grey[2]], white);
        assert!((r - 4.48).abs() < 0.01, "{r}");
        assert_eq!(parse_hex("#09090bb8"), Some([9, 9, 11, 184]));
        assert_eq!(over([255, 255, 255, 128], [0, 0, 0]), [128, 128, 128]);
        assert_eq!(parse_hex("red"), None);
    }
}
