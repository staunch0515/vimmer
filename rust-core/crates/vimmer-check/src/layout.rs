//! Renderer geometry and the title-safe area.
//!
//! The constants mirror `remotion-renderer/src/components/AnimatedText.tsx` (scene text) and
//! `Subtitles.tsx` (subtitle band). CSS percentage padding is relative to the frame *width*,
//! also for top and bottom padding. When the renderer changes, change these too.
//!
//! The title-safe area is the inner 90% of the frame (5% margin on every side), EBU R95's
//! graphics-safe area for 16:9, used here for 9:16 as well. Phone apps cover more of a 9:16
//! frame with their own buttons and captions; that is not checked.

use vimmer_dsl::{Resolution, VideoTimeline};

use crate::text::{cue_lines, scene_text_lines, shown_text, MAX_CUE_LINES};
use crate::{scene_starts_ms, Finding, Severity, Suggestion};

/// Margin of the title-safe area, as a fraction of each side.
pub const TITLE_SAFE_MARGIN: f64 = 0.05;

// AnimatedText.tsx
pub const SCENE_FONT_PX: f64 = 92.0;
pub const SCENE_LINE_HEIGHT: f64 = 1.2;
pub const SCENE_LETTER_SPACING_EM: f64 = -0.02;
/// `marginRight: 0.25em` after every word span.
pub const SCENE_WORD_GAP_EM: f64 = 0.25;
/// `padding: "8% 8% 260px"`: top and sides are 8% of the frame width.
pub const SCENE_PADDING_FRAC: f64 = 0.08;
pub const SCENE_BOTTOM_INSET_PX: f64 = 260.0;
/// `maxWidth: "88%"` of the padded box.
pub const SCENE_MAX_WIDTH_FRAC: f64 = 0.88;

// Subtitles.tsx
/// `fontSize = round(min(width, height) * 0.05)`.
pub const SUB_FONT_FRAC: f64 = 0.05;
pub const SUB_LINE_HEIGHT: f64 = 1.3;
/// Vertical and horizontal padding as fractions of the font size (each rounded).
pub const SUB_PAD_V: f64 = 0.3;
pub const SUB_PAD_H: f64 = 0.6;
pub const SUB_MAX_WIDTH_FRAC: f64 = 0.86;
/// `paddingBottom: "4%"`, of the frame width.
pub const SUB_BOTTOM_FRAC: f64 = 0.04;

/// Where scene text is laid out.
#[derive(Debug, Clone, Copy)]
pub struct SceneTextArea {
    pub font_px: f64,
    pub letter_spacing_em: f64,
    pub word_gap_em: f64,
    pub line_height_px: f64,
    pub line_width: f64,
    /// Lines that fit between the top padding and the bottom inset.
    pub max_lines: u32,
    /// Y of the text's bottom edge (text is bottom-aligned and grows upward).
    pub bottom_y: f64,
}

impl SceneTextArea {
    pub fn new(res: Resolution) -> Self {
        let (w, h) = (f64::from(res.width), f64::from(res.height));
        let line_width = w * (1.0 - 2.0 * SCENE_PADDING_FRAC) * SCENE_MAX_WIDTH_FRAC;
        let line_height_px = SCENE_FONT_PX * SCENE_LINE_HEIGHT;
        let avail = h - w * SCENE_PADDING_FRAC - SCENE_BOTTOM_INSET_PX;
        Self {
            font_px: SCENE_FONT_PX,
            letter_spacing_em: SCENE_LETTER_SPACING_EM,
            word_gap_em: SCENE_WORD_GAP_EM,
            line_height_px,
            line_width,
            max_lines: (avail / line_height_px).floor().max(0.0) as u32,
            bottom_y: h - SCENE_BOTTOM_INSET_PX,
        }
    }
}

/// The subtitle band.
#[derive(Debug, Clone, Copy)]
pub struct SubtitleBand {
    pub font_px: f64,
    pub line_height_px: f64,
    pub pad_v: f64,
    /// Width available to the text inside the band's padding.
    pub text_width: f64,
    pub left_x: f64,
    pub bottom_y: f64,
}

impl SubtitleBand {
    pub fn new(res: Resolution) -> Self {
        let (w, h) = (f64::from(res.width), f64::from(res.height));
        let font_px = (w.min(h) * SUB_FONT_FRAC).round();
        let pad_v = (font_px * SUB_PAD_V).round();
        let pad_h = (font_px * SUB_PAD_H).round();
        Self {
            font_px,
            line_height_px: font_px * SUB_LINE_HEIGHT,
            pad_v,
            text_width: w * SUB_MAX_WIDTH_FRAC - 2.0 * pad_h,
            left_x: w * (1.0 - SUB_MAX_WIDTH_FRAC) / 2.0,
            bottom_y: h - w * SUB_BOTTOM_FRAC,
        }
    }

    /// Top edge of the band for a cue of `lines` lines.
    pub fn top_y(&self, lines: u32) -> f64 {
        self.bottom_y - f64::from(lines) * self.line_height_px - 2.0 * self.pad_v
    }
}

/// The title-safe rectangle: (left, top, right, bottom).
pub fn title_safe(res: Resolution) -> (f64, f64, f64, f64) {
    let (w, h) = (f64::from(res.width), f64::from(res.height));
    (
        w * TITLE_SAFE_MARGIN,
        h * TITLE_SAFE_MARGIN,
        w * (1.0 - TITLE_SAFE_MARGIN),
        h * (1.0 - TITLE_SAFE_MARGIN),
    )
}

pub fn check(timeline: &VideoTimeline) -> Vec<Finding> {
    let mut out = Vec::new();
    let res = timeline.resolution;
    let (safe_l, safe_t, _safe_r, safe_b) = title_safe(res);
    let area = SceneTextArea::new(res);
    let starts = scene_starts_ms(timeline);

    // Scene text grows upward from a fixed bottom edge. Overflow past the padded area is
    // `text.overflow`; this catches text that fits there but crosses the safe area.
    for (i, scene) in timeline.scenes.iter().enumerate() {
        let Some(text) = scene.text.as_deref().filter(|t| !t.trim().is_empty()) else {
            continue;
        };
        let lines = scene_text_lines(text, &area, 1.0);
        if lines > area.max_lines {
            continue;
        }
        let top = area.bottom_y - f64::from(lines) * area.line_height_px;
        if top < safe_t {
            out.push(
                Finding::new(
                    Severity::Warning,
                    "safe_area.text",
                    format!(
                        "scene text ({lines} lines) reaches y={top:.0} px, above the title-safe top at {safe_t:.0} px"
                    ),
                    Suggestion::field("shorten the text by a line or more", crate::scene_path(i, "text")),
                )
                .scene(&scene.id)
                .at(starts[i]),
            );
        }
    }

    let band = SubtitleBand::new(res);
    if !timeline.subtitles.is_empty() {
        // The band's bottom edge is fixed by the renderer, the same for every cue.
        if band.bottom_y > safe_b + 0.5 {
            out.push(Finding::new(
                Severity::Info,
                "safe_area.subtitle_band",
                format!(
                    "at {}x{} the renderer puts the subtitle band's bottom edge {:.0} px from the frame bottom; the title-safe margin is {:.0} px",
                    res.width,
                    res.height,
                    f64::from(res.height) - band.bottom_y,
                    f64::from(res.height) - safe_b
                ),
                Suggestion::text(
                    "renderer change, not a timeline field: Subtitles.tsx paddingBottom of at least 5% of the frame height",
                ),
            ));
        }
        if band.left_x < safe_l {
            out.push(Finding::new(
                Severity::Info,
                "safe_area.subtitle_band",
                "the subtitle band's full width reaches outside the title-safe sides",
                Suggestion::text("renderer change: Subtitles.tsx maxWidth of at most 90%"),
            ));
        }
        for (i, cue) in timeline.subtitles.iter().enumerate() {
            let lines = cue_lines(&shown_text(cue), &band, 1.0);
            if lines <= MAX_CUE_LINES {
                continue;
            }
            let top = band.top_y(lines);
            if top < safe_t {
                out.push(
                    Finding::new(
                        Severity::Error,
                        "safe_area.subtitle",
                        format!(
                            "cue ({lines} lines) reaches y={top:.0} px, above the title-safe top"
                        ),
                        Suggestion::field("shorten the cue", crate::cue_path(i, "text")),
                    )
                    .cue(i)
                    .at(cue.start_ms),
                );
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn landscape_geometry_matches_the_renderer() {
        let res = Resolution {
            width: 1920,
            height: 1080,
        };
        let band = SubtitleBand::new(res);
        assert_eq!(band.font_px, 54.0);
        assert_eq!(band.pad_v, 16.0);
        assert!((band.bottom_y - 1003.2).abs() < 0.01);
        // A two-line band stays below the scene text's bottom edge (1080 - 260 = 820).
        assert!(band.top_y(2) > SceneTextArea::new(res).bottom_y);
        assert!(band.top_y(3) < SceneTextArea::new(res).bottom_y);
        let (_, _, _, safe_b) = title_safe(res);
        assert!(band.bottom_y < safe_b);
    }

    #[test]
    fn portrait_band_sits_below_the_title_safe_bottom() {
        let res = Resolution {
            width: 1080,
            height: 1920,
        };
        let band = SubtitleBand::new(res);
        let (_, _, _, safe_b) = title_safe(res);
        assert!(band.bottom_y > safe_b);
    }
}
