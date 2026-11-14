//! Advance widths of a TrueType/OpenType font, read with `ttf-parser`: every character of its
//! Unicode `cmap` subtables, mapped through `hmtx` and divided by `head.unitsPerEm`. Works for
//! .ttf, .otf and the first font of a .ttc collection. WOFF/WOFF2 are compressed and not read. A
//! damaged file gives `None`, never a panic.

use std::collections::HashMap;

/// Advance width in em of every character the font maps. `None` for anything that is not a
/// readable TTF/OTF/TTC.
pub fn advances(d: &[u8]) -> Option<HashMap<char, f64>> {
    let face = ttf_parser::Face::parse(d, 0).ok()?;
    let upem = f64::from(face.units_per_em());
    let cmap = face.tables().cmap?;
    if upem <= 0.0 {
        return None;
    }
    let mut out = HashMap::new();
    for sub in cmap.subtables.into_iter().filter(|s| s.is_unicode()) {
        sub.codepoints(|cp| {
            let Some(c) = char::from_u32(cp) else {
                return;
            };
            if out.contains_key(&c) {
                return;
            }
            let advance = sub
                .glyph_index(cp)
                .filter(|g| g.0 != 0)
                .and_then(|g| face.glyph_hor_advance(g));
            if let Some(a) = advance {
                out.insert(c, f64::from(a) / upem);
            }
        });
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_garbage_without_panicking() {
        assert!(advances(b"").is_none());
        assert!(advances(b"wOF2\0\0\0\0").is_none());
        assert!(advances(&[0, 1, 0, 0, 0, 200, 0, 0, 0, 0, 0, 0]).is_none());
    }
}

#[cfg(test)]
mod repo_font {
    /// The repo's Fira Sans Thin, against fontTools (`hmtx` advance / unitsPerEm 1000).
    #[test]
    fn fira_sans_thin_advances_match_font_tools() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../remotion-renderer/public/assets/fonts/FiraSans-Thin.ttf");
        let a = super::advances(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(a.len(), 1693);
        for (c, em) in [
            ('a', 0.528),
            ('m', 0.848),
            ('W', 0.79),
            ('i', 0.258),
            (' ', 0.296),
            ('é', 0.529),
        ] {
            assert_eq!(a.get(&c), Some(&em), "{c:?}");
        }
        assert_eq!(a.get(&'中'), None);
    }
}
