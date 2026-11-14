//! Brand font metrics: advance widths read from the brand's own font files.
//!
//! The renderer registers `brand.fonts.heading` / `body` from their `src` file and sets text in
//! them (`components/Brand.tsx`, `brand.ts` `fontStack`): scene text, counters and lower-third
//! titles in the heading font, text layers in the body font (or heading with `font: heading`),
//! subtitles, lower-third subtitles and chart labels in the body font. When such a file is a
//! TrueType/OpenType font in the public dir, text widths come from its `hmtx` advances (via its
//! `cmap`), in em of `unitsPerEm`. Characters the font lacks fall back to the browser's next font,
//! estimated as before (`text::char_em`). Kerning is not applied (a few percent at most).
//!
//! Without a file (or with a .woff/.woff2 file, which is compressed), the estimate is used.
//! Measured widths are exact up to kerning, so the overflow checks use [`MEASURED_OPTIMISTIC`]
//! instead of the estimate's ±12 % margin.

use std::collections::HashMap;
use std::path::Path;

use vimmer_dsl::Brand;

use crate::text::char_em;
use crate::textbox::Role;

/// Width factor of the optimistic estimate with measured advances (kerning and shaping only).
pub const MEASURED_OPTIMISTIC: f64 = 0.97;

/// Advance widths of one font, in em.
#[derive(Debug, Clone, Default)]
pub struct Metrics {
    pub family: String,
    advances: HashMap<char, f64>,
}

impl Metrics {
    /// Advance of `c` in em, if the font has the character.
    pub fn em(&self, c: char) -> Option<f64> {
        self.advances.get(&c).copied()
    }

    /// Reads the advances of every character the font maps, from a TTF/OTF file.
    pub fn from_file(family: &str, path: &Path) -> Option<Metrics> {
        let data = std::fs::read(path).ok()?;
        Self::from_bytes(family, &data)
    }

    pub fn from_bytes(family: &str, data: &[u8]) -> Option<Metrics> {
        let advances = crate::sfnt::advances(data)?;
        (!advances.is_empty()).then(|| Metrics {
            family: family.to_string(),
            advances,
        })
    }
}

/// The brand fonts that could be measured.
#[derive(Debug, Clone, Default)]
pub struct Fonts {
    pub heading: Option<Metrics>,
    pub body: Option<Metrics>,
}

impl Fonts {
    /// No measured font: every width is the estimate.
    pub fn none() -> Fonts {
        Fonts::default()
    }

    /// Loads the brand's font files from `public_dir` (those that are present and parse).
    pub fn load(brand: Option<&Brand>, public_dir: Option<&Path>) -> Fonts {
        let (Some(brand), Some(dir)) = (brand, public_dir) else {
            return Fonts::none();
        };
        let load = |f: &Option<vimmer_dsl::brand::BrandFont>| {
            let f = f.as_ref()?;
            Metrics::from_file(&f.family, &dir.join(f.src.as_deref()?))
        };
        Fonts {
            heading: load(&brand.fonts.heading),
            body: load(&brand.fonts.body),
        }
    }

    pub fn get(&self, role: Role) -> Option<&Metrics> {
        match role {
            Role::Heading => self.heading.as_ref(),
            Role::Body => self.body.as_ref(),
        }
    }

    /// Whether text in `role` is measured rather than estimated.
    pub fn measured(&self, role: Role) -> bool {
        self.get(role).is_some()
    }

    /// The overflow checks' optimistic width factor for `role`.
    pub fn optimistic(&self, role: Role) -> f64 {
        if self.measured(role) {
            MEASURED_OPTIMISTIC
        } else {
            crate::text::OPTIMISTIC
        }
    }

    /// Advance of `c` in em in `role`'s font.
    pub fn em(&self, role: Role, c: char) -> f64 {
        self.get(role)
            .and_then(|m| m.em(c))
            .unwrap_or_else(|| char_em(c))
    }

    /// Width in px of `s` at `font_px`, with `letter_spacing_em` after each character.
    pub fn width(&self, role: Role, s: &str, font_px: f64, letter_spacing_em: f64) -> f64 {
        s.chars()
            .map(|c| (self.em(role, c) + letter_spacing_em) * font_px)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fira() -> Fonts {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../remotion-renderer/public");
        let brand: Brand = serde_json::from_str(
            r#"{"fonts":{"heading":{"family":"Thin","src":"assets/fonts/FiraSans-Thin.ttf"},
                "body":{"family":"Missing","src":"assets/fonts/nope.ttf"}}}"#,
        )
        .unwrap();
        Fonts::load(Some(&brand), Some(&dir))
    }

    #[test]
    fn measured_font_replaces_the_estimate_and_falls_back_per_character() {
        let f = fira();
        assert!(f.measured(Role::Heading));
        assert!(!f.measured(Role::Body));
        assert_eq!(f.em(Role::Heading, 'i'), 0.258);
        assert_eq!(f.em(Role::Body, 'i'), char_em('i'));
        // A character the font lacks is estimated.
        assert_eq!(f.em(Role::Heading, '中'), char_em('中'));
        assert_eq!(f.optimistic(Role::Heading), MEASURED_OPTIMISTIC);
        assert_eq!(f.optimistic(Role::Body), crate::text::OPTIMISTIC);
        let w = f.width(Role::Heading, "ii", 100.0, -0.02);
        assert!((w - 2.0 * (0.258 - 0.02) * 100.0).abs() < 1e-9);
    }

    #[test]
    fn no_brand_no_dir_or_unreadable_file_means_the_estimate() {
        assert!(!Fonts::load(None, Some(Path::new("/"))).measured(Role::Heading));
        let brand: Brand = serde_json::from_str(
            r#"{"fonts":{"heading":{"family":"X","src":"assets/fonts/OFL.txt"}}}"#,
        )
        .unwrap();
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../remotion-renderer/public");
        assert!(!Fonts::load(Some(&brand), Some(&dir)).measured(Role::Heading));
        assert!(!Fonts::load(Some(&brand), None).measured(Role::Heading));
    }
}
