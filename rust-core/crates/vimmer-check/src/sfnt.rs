//! The minimum of TrueType/OpenType (`sfnt`) needed for advance widths: the table directory,
//! `head.unitsPerEm`, `hhea.numberOfHMetrics`, `hmtx` and a Unicode `cmap` (format 4 or 12).
//! Works for .ttf, .otf (CFF outlines keep their metrics in the same tables) and the first font
//! of a .ttc collection. WOFF/WOFF2 are compressed and not read. Every read is bounds-checked: a
//! damaged file gives `None`, never a panic.

use std::collections::HashMap;

fn u16_at(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

/// The tables of the font at `base`: tag -> (offset, length), offsets from the file start.
fn tables(d: &[u8], base: usize) -> Option<HashMap<[u8; 4], (usize, usize)>> {
    let version = u32_at(d, base)?;
    if !matches!(version, 0x0001_0000 | 0x7472_7565 | 0x4F54_544F) {
        return None; // not 1.0, 'true' or 'OTTO'
    }
    let n = usize::from(u16_at(d, base + 4)?);
    let mut out = HashMap::new();
    for i in 0..n {
        let rec = base + 12 + 16 * i;
        let tag: [u8; 4] = d.get(rec..rec + 4)?.try_into().ok()?;
        let off = u32_at(d, rec + 8)? as usize;
        let len = u32_at(d, rec + 12)? as usize;
        if off.checked_add(len)? > d.len() {
            return None;
        }
        out.insert(tag, (off, len));
    }
    Some(out)
}

/// Glyph id of every character the font maps, from its best Unicode `cmap` subtable.
fn char_map(d: &[u8], cmap: usize) -> Option<Vec<(char, u16)>> {
    let n = usize::from(u16_at(d, cmap + 2)?);
    // (score, offset): full-repertoire format 12 first, then BMP format 4.
    let mut best: Option<(u8, usize)> = None;
    for i in 0..n {
        let rec = cmap + 4 + 8 * i;
        let (platform, encoding) = (u16_at(d, rec)?, u16_at(d, rec + 2)?);
        let sub = cmap + u32_at(d, rec + 4)? as usize;
        let format = u16_at(d, sub)?;
        let unicode = platform == 0 || (platform == 3 && matches!(encoding, 1 | 10));
        let score = match (unicode, format) {
            (true, 12) => 2,
            (true, 4) => 1,
            _ => continue,
        };
        if best.is_none_or(|(s, _)| score > s) {
            best = Some((score, sub));
        }
    }
    let (_, sub) = best?;
    let mut out = Vec::new();
    match u16_at(d, sub)? {
        4 => {
            let segs = usize::from(u16_at(d, sub + 6)? / 2);
            let ends = sub + 14;
            let starts = ends + 2 * segs + 2;
            let deltas = starts + 2 * segs;
            let ranges = deltas + 2 * segs;
            for s in 0..segs {
                let end = u16_at(d, ends + 2 * s)?;
                let start = u16_at(d, starts + 2 * s)?;
                let delta = u16_at(d, deltas + 2 * s)?;
                let range_at = ranges + 2 * s;
                let range = usize::from(u16_at(d, range_at)?);
                if start > end {
                    continue;
                }
                for code in start..=end {
                    if code == 0xFFFF {
                        break;
                    }
                    let glyph = if range == 0 {
                        code.wrapping_add(delta)
                    } else {
                        let at = range_at + range + 2 * usize::from(code - start);
                        match u16_at(d, at)? {
                            0 => 0,
                            g => g.wrapping_add(delta),
                        }
                    };
                    if glyph != 0 {
                        if let Some(c) = char::from_u32(u32::from(code)) {
                            out.push((c, glyph));
                        }
                    }
                }
            }
        }
        12 => {
            let groups = u32_at(d, sub + 12)? as usize;
            for g in 0..groups {
                let rec = sub + 16 + 12 * g;
                let (start, end, glyph) =
                    (u32_at(d, rec)?, u32_at(d, rec + 4)?, u32_at(d, rec + 8)?);
                if start > end || end > 0x10_FFFF {
                    return None;
                }
                for code in start..=end {
                    let Ok(id) = u16::try_from(glyph + (code - start)) else {
                        break;
                    };
                    if let Some(c) = char::from_u32(code) {
                        out.push((c, id));
                    }
                }
            }
        }
        _ => return None,
    }
    Some(out)
}

/// Advance width in em of every character the font maps. `None` for anything that is not a
/// readable TTF/OTF/TTC.
pub fn advances(d: &[u8]) -> Option<HashMap<char, f64>> {
    let base = if d.get(0..4)? == b"ttcf" {
        u32_at(d, 12)? as usize
    } else {
        0
    };
    let t = tables(d, base)?;
    let head = t.get(b"head")?.0;
    let hhea = t.get(b"hhea")?.0;
    let (hmtx, hmtx_len) = *t.get(b"hmtx")?;
    let cmap = t.get(b"cmap")?.0;
    let upem = f64::from(u16_at(d, head + 18)?);
    let metrics = usize::from(u16_at(d, hhea + 34)?);
    if upem <= 0.0 || metrics == 0 || metrics * 4 > hmtx_len {
        return None;
    }
    // Glyphs past the last long metric share its advance.
    let advance = |g: u16| u16_at(d, hmtx + 4 * usize::from(g).min(metrics - 1));
    let mut out = HashMap::new();
    for (c, g) in char_map(d, cmap)? {
        out.insert(c, f64::from(advance(g)?) / upem);
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
