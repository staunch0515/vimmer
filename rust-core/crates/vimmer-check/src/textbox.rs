//! Where the renderer draws text, in px: the box the glyphs (and any text background) take.
//! Shared by the logo-overlap and the contrast-over-media checks.
//!
//! * Scene text (`AnimatedText.tsx`): centered, bottom edge fixed at `height - 260 px`, growing
//!   upward. The div shrinks to its content up to 88% of the padded width; with several lines
//!   (`text-wrap: balance`) each line is about the total width over the line count, so the glyphs
//!   take that width (+10% for uneven breaks), centered.
//! * Text layers (`LayerKinds.tsx` `TextLayerView`): the layer box; a fixed-height box centers the
//!   text vertically. With a background the box is filled edge to edge; without one a single
//!   line takes its own width, placed by `align`.
//! * Lower thirds: the bar plus the padded text panel, which shrinks to its widest line.
//! * Counters: the box, one line tall unless `height` is set. Charts and rings: the box.
//!
//! Rotation and entrance/exit animations are ignored (the box at rest).

use vimmer_dsl::{Layer, LayerContent, Resolution, TextAlign, TextLayer};

use crate::fonts::Fonts;
use crate::layers::{
    lower_third_height, place, text_area, Rect, LT_BAR_EM, LT_PAD_H_EM, LT_SUB_SCALE,
};
use crate::layout::SceneTextArea;
use crate::text::scene_text_lines_with;

/// Width in px of `text` set as word spans (as `scene_text_lines`), without the trailing gap.
pub(crate) fn words_width(text: &str, area: &SceneTextArea, fonts: &Fonts, role: Role) -> f64 {
    let gap = area.word_gap_em * area.font_px;
    let total: f64 = text
        .split_whitespace()
        .map(|w| fonts.width(role, w, area.font_px, area.letter_spacing_em) + gap)
        .sum();
    (total - gap).max(0.0)
}

/// Which brand font sets a text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Heading,
    Body,
}

/// The scene text's box, or `None` without text.
pub(crate) fn scene_text(text: &str, res: Resolution, fonts: &Fonts) -> Option<Rect> {
    if text.trim().is_empty() {
        return None;
    }
    let area = SceneTextArea::new(res);
    let lines = scene_text_lines_with(text, &area, 1.0, fonts, Role::Heading).max(1);
    let total = words_width(text, &area, fonts, Role::Heading);
    let w = if lines == 1 {
        total.min(area.line_width)
    } else {
        (total / f64::from(lines) * 1.1).min(area.line_width)
    };
    let h = f64::from(lines) * area.line_height_px;
    let left = (f64::from(res.width) - w) / 2.0;
    Some((left, area.bottom_y - h, w, h))
}

/// The box a text layer's glyphs (or its background) take.
pub(crate) fn text_layer(t: &TextLayer, layer: &Layer, res: Resolution, fonts: &Fonts) -> Rect {
    let role = text_role(t);
    let (area, pad) = text_area(t, layer, res);
    let lines = scene_text_lines_with(&t.text, &area, 1.0, fonts, role).max(1);
    let content_h = f64::from(lines) * area.line_height_px + 2.0 * pad;
    let fixed_h = layer.height.map(|v| f64::from(v) * f64::from(res.height));
    let (l, top, w, h) = place(layer, res, fixed_h.unwrap_or(content_h));
    // A fixed box centers the text block vertically.
    let (top, h) = match fixed_h {
        Some(fh) if content_h < fh => (top + (fh - content_h) / 2.0, content_h),
        _ => (top, h),
    };
    if t.background.is_some() || lines > 1 {
        return (l, top, w, h);
    }
    let tw = words_width(&t.text, &area, fonts, role).min(w);
    let left = match t.align {
        TextAlign::Left => l,
        TextAlign::Center => l + (w - tw) / 2.0,
        TextAlign::Right => l + w - tw,
    };
    (left, top, tw, h)
}

pub(crate) fn text_role(t: &TextLayer) -> Role {
    match t.font {
        Some(vimmer_dsl::FontRole::Heading) => Role::Heading,
        _ => Role::Body,
    }
}

/// The box of any layer, as drawn at rest. Text-bearing layers use the boxes above.
pub(crate) fn layer_box(layer: &Layer, res: Resolution, fonts: &Fonts) -> Rect {
    let fh = f64::from(res.height);
    let fixed_h = layer.height.map(|v| f64::from(v) * fh);
    match &layer.content {
        LayerContent::Text(t) => text_layer(t, layer, res, fonts),
        LayerContent::LowerThird(lt) => {
            let h = fixed_h.unwrap_or_else(|| lower_third_height(lt, layer, res, fonts).0);
            let (l, top, w, h) = place(layer, res, h);
            let f = f64::from(lt.font_size) * fh;
            let area = |font_px| SceneTextArea {
                font_px,
                letter_spacing_em: crate::layout::SCENE_LETTER_SPACING_EM,
                word_gap_em: crate::layout::SCENE_WORD_GAP_EM,
                line_height_px: font_px,
                line_width: w,
                max_lines: u32::MAX,
                bottom_y: 0.0,
            };
            let title = words_width(&lt.title, &area(f), fonts, Role::Heading);
            let sub = lt.subtitle.as_deref().map_or(0.0, |s| {
                words_width(s, &area(f * LT_SUB_SCALE), fonts, Role::Body)
            });
            let panel = (LT_BAR_EM + 2.0 * LT_PAD_H_EM) * f + title.max(sub);
            (l, top, panel.min(w), h)
        }
        LayerContent::Counter(c) => {
            let h = fixed_h.unwrap_or(f64::from(c.font_size) * fh * 1.2);
            place(layer, res, h)
        }
        _ => place(layer, res, fixed_h.unwrap_or(0.0)),
    }
}
