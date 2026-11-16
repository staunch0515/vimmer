//! Renderer geometry and the title-safe area.
//!
//! The constants mirror `remotion-renderer/src/components/AnimatedText.tsx` (scene text) and
//! `Subtitles.tsx` (subtitle band). CSS percentage padding is relative to the frame *width*,
//! also for top and bottom padding. When the renderer changes, change these too.
//!
//! The title-safe area is the inner 90% of the frame (5% margin on every side), EBU R95's
//! graphics-safe area for 16:9, used here for 9:16 as well. Phone apps cover more of a 9:16
//! frame with their own buttons and captions; that is not checked.

use vimmer_dsl::text_layout::top_inset_px;
use vimmer_dsl::{
    style_scale, Resolution, SubtitleBox, TextPoint, TextPosition, TextStyle, VideoTimeline,
};

use crate::fonts::Fonts;
use crate::text::{cue_lines_with, scene_text_lines_with, shown_text, MAX_CUE_LINES};
use crate::textbox::Role;
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
/// Extra space between the lines of a scene text box with a border, beyond its padding and border.
pub const SCENE_BORDER_LINE_GAP_EM: f64 = 0.12;
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
    /// Y of the text area's bottom edge (bottom text is aligned to it and grows upward).
    pub bottom_y: f64,
    /// Y of the text area's top edge (the top padding; `top` text hangs from it).
    pub top_y: f64,
    /// Where the text block sits between `top_y` and `bottom_y`.
    pub position: TextPosition,
    /// A free point (`text_layout.position: {"y": ..}`): the block's centre, which wins over
    /// `position`; `top_y` / `bottom_y` are then the frame's 8 % paddings.
    pub at: Option<TextPoint>,
}

/// How far a text style reaches beyond the glyphs, px.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct StyleReach {
    /// The box's padding on every side (0 without a box).
    pub pad: f64,
    /// The box's border width (0 without one).
    pub border: f64,
    /// The outline's width outside the glyphs (0 without one).
    pub outline: f64,
}

impl StyleReach {
    pub fn of(style: &TextStyle, font_px: f64, res: Resolution) -> Self {
        Self {
            pad: style
                .background
                .as_ref()
                .map_or(0.0, |b| f64::from(b.padding) * font_px),
            border: style.background.as_ref().map_or(0.0, |b| {
                f64::from(b.border_px()) * style_scale(res.width, res.height)
            }),
            outline: style.drawn_outline().map_or(0.0, |o| {
                f64::from(o.width) * style_scale(res.width, res.height)
            }),
        }
    }

    /// Beyond the glyphs on each side.
    pub fn total(self) -> f64 {
        self.pad + self.border + self.outline
    }
}

impl SceneTextArea {
    /// The text area of scene `i` as drawn: its text style, and its text layout's size and
    /// position (see `vimmer_dsl::text_layout`).
    pub fn for_scene(t: &VideoTimeline, i: usize) -> (Self, StyleReach) {
        let scene = &t.scenes[i];
        let brand = t.brand.as_ref();
        let style = scene.effective_text_style(brand);
        let layout = scene.effective_text_layout(brand);
        let top = top_inset_px(layout.position, brand, &scene.id, t.resolution);
        let (mut area, reach) = Self::styled_at(
            t.resolution,
            &style,
            layout.font_px(t.resolution),
            layout.position,
            top,
        );
        if let Some(p) = layout.at {
            // A point ignores the bottom inset and the logo: the whole frame inside the paddings.
            let (w, h) = (
                f64::from(t.resolution.width),
                f64::from(t.resolution.height),
            );
            let pad = w * SCENE_PADDING_FRAC;
            area.at = Some(p);
            area.top_y = pad;
            area.bottom_y = h - pad;
            area.max_lines = ((h - 2.0 * pad - 2.0 * reach.total()) / area.line_height_px)
                .floor()
                .max(0.0) as u32;
        }
        (area, reach)
    }

    /// The scene text's area with its style: a box (`AnimatedText.tsx` draws it per line, with
    /// the padding plus one word gap of horizontal room on each line) and an outline take width
    /// from every line, and room above the top line.
    pub fn styled(res: Resolution, style: &TextStyle) -> (Self, StyleReach) {
        let top = f64::from(res.width) * SCENE_PADDING_FRAC;
        Self::styled_at(res, style, SCENE_FONT_PX, TextPosition::Bottom, top)
    }

    /// [`Self::styled`] for a `font_px` text at `position` under a `top_y` top padding.
    pub fn styled_at(
        res: Resolution,
        style: &TextStyle,
        font_px: f64,
        position: TextPosition,
        top_y: f64,
    ) -> (Self, StyleReach) {
        let mut area = Self::at(res, font_px, position, top_y);
        let reach = StyleReach::of(style, area.font_px, res);
        if reach == StyleReach::default() {
            return (area, reach);
        }
        let gap = if style.background.is_some() {
            area.word_gap_em * area.font_px
        } else {
            0.0
        };
        area.line_width = (area.line_width - 2.0 * reach.total() - gap).max(1.0);
        // With a border every line is its own framed box: the lines move apart by the padding,
        // the border and a small gap (`AnimatedText.tsx` BoxedWords).
        if reach.border > 0.0 {
            let pad_em = style
                .background
                .as_ref()
                .map_or(0.0, |b| f64::from(b.padding));
            area.line_height_px = area.font_px
                * (SCENE_LINE_HEIGHT + 2.0 * pad_em + SCENE_BORDER_LINE_GAP_EM)
                + 2.0 * reach.border;
        }
        let h = f64::from(res.height);
        let avail = h - top_y - SCENE_BOTTOM_INSET_PX - reach.total();
        area.max_lines = (avail / area.line_height_px).floor().max(0.0) as u32;
        (area, reach)
    }

    /// The built-in area: 92 px text at the bottom.
    pub fn new(res: Resolution) -> Self {
        let top = f64::from(res.width) * SCENE_PADDING_FRAC;
        Self::at(res, SCENE_FONT_PX, TextPosition::Bottom, top)
    }

    /// The area of a `font_px` text at `position` under a `top_y` top padding, without a style.
    pub fn at(res: Resolution, font_px: f64, position: TextPosition, top_y: f64) -> Self {
        let (w, h) = (f64::from(res.width), f64::from(res.height));
        let line_width = w * (1.0 - 2.0 * SCENE_PADDING_FRAC) * SCENE_MAX_WIDTH_FRAC;
        let line_height_px = font_px * SCENE_LINE_HEIGHT;
        let avail = h - top_y - SCENE_BOTTOM_INSET_PX;
        Self {
            font_px,
            letter_spacing_em: SCENE_LETTER_SPACING_EM,
            word_gap_em: SCENE_WORD_GAP_EM,
            line_height_px,
            line_width,
            max_lines: (avail / line_height_px).floor().max(0.0) as u32,
            bottom_y: h - SCENE_BOTTOM_INSET_PX,
            top_y,
            position,
            at: None,
        }
    }

    /// Top edge of a `lines`-line text block with a style reaching `reach` px past the glyphs.
    pub fn text_top(&self, lines: u32, reach: f64) -> f64 {
        let h = f64::from(lines) * self.line_height_px;
        if let Some(p) = self.at {
            let frame_h = self.bottom_y + self.top_y;
            return f64::from(p.y) * frame_h - h / 2.0 - reach;
        }
        match self.position {
            TextPosition::Bottom => self.bottom_y - h - reach,
            TextPosition::Top => self.top_y - reach,
            TextPosition::Middle => (self.top_y + self.bottom_y - h) / 2.0 - reach,
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
    /// Left and right edges of the widest box (86 % of the frame width).
    pub left_x: f64,
    pub right_x: f64,
    pub bottom_y: f64,
}

impl SubtitleBand {
    /// The band where the timeline's subtitles are drawn: at its `subtitle_box`, or the
    /// renderer's built-in place.
    pub fn for_timeline(t: &VideoTimeline) -> Self {
        match t.subtitle_box {
            Some(b) => Self::at(t.resolution, b),
            None => Self::new(t.resolution),
        }
    }

    /// The band with the box centered at `b.x` and its bottom edge at `b.y` (Subtitles.tsx with
    /// `subtitle_box`: an 86 %-wide slot centered on x, its bottom at y, the box inside it).
    pub fn at(res: Resolution, b: SubtitleBox) -> Self {
        let (w, h) = (f64::from(res.width), f64::from(res.height));
        let left_x = w * (b.x - SUB_MAX_WIDTH_FRAC / 2.0);
        Self {
            left_x,
            right_x: left_x + w * SUB_MAX_WIDTH_FRAC,
            bottom_y: h * b.y,
            ..Self::new(res)
        }
    }

    /// The renderer's built-in place (no `subtitle_box`).
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
            right_x: w * (1.0 + SUB_MAX_WIDTH_FRAC) / 2.0,
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

/// Where a scene text at a free point crosses the title-safe bottom, the subtitle band (when the
/// video has subtitles) or the title-safe sides: `(what, fix)` per problem.
fn point_findings(
    t: &VideoTimeline,
    area: &SceneTextArea,
    reach: f64,
    p: TextPoint,
    text: &str,
    lines: u32,
    fonts: &Fonts,
) -> Vec<(String, &'static str)> {
    let res = t.resolution;
    let (safe_l, _, safe_r, safe_b) = title_safe(res);
    let top = area.text_top(lines, reach);
    let bottom = top + f64::from(lines) * area.line_height_px + 2.0 * reach;
    let mut out = Vec::new();
    if bottom > safe_b + 0.5 {
        out.push((
            format!("reaches y={bottom:.0} px, below the title-safe bottom at {safe_b:.0} px"),
            "move the text up (a smaller position.y)",
        ));
    } else if !t.subtitles.is_empty() {
        let band_top = SubtitleBand::for_timeline(t).top_y(1);
        if bottom > band_top + 0.5 {
            out.push((
                format!("reaches y={bottom:.0} px and covers the subtitles, which start at y={band_top:.0} px"),
                "move the text up (a smaller position.y)",
            ));
        }
    }
    let half = crate::text::scene_text_block_width(text, area, fonts, Role::Heading) / 2.0 + reach;
    let cx = f64::from(p.x_or_centre()) * f64::from(res.width);
    if cx - half < safe_l - 0.5 || cx + half > safe_r + 0.5 {
        out.push((
            format!(
                "spans x={:.0}..{:.0} px, past the title-safe sides at {safe_l:.0}..{safe_r:.0} px",
                cx - half,
                cx + half
            ),
            "move the text toward the middle (position.x nearer 0.5)",
        ));
    }
    out
}

pub fn check(timeline: &VideoTimeline) -> Vec<Finding> {
    check_with(timeline, &Fonts::none())
}

/// [`check`] with the brand's measured fonts.
pub fn check_with(timeline: &VideoTimeline, fonts: &Fonts) -> Vec<Finding> {
    let mut out = Vec::new();
    let res = timeline.resolution;
    let (safe_l, safe_t, _safe_r, safe_b) = title_safe(res);
    let starts = scene_starts_ms(timeline);

    // Scene text grows upward from a fixed bottom edge. Overflow past the padded area is
    // `text.overflow`; this catches text that fits there but crosses the safe area.
    for (i, scene) in timeline.scenes.iter().enumerate() {
        let pages = crate::pages::scene_pages(timeline, i);
        if pages.is_empty() {
            continue;
        }
        let (area, reach) = SceneTextArea::for_scene(timeline, i);
        let n = pages.len();
        for (k, page) in pages.iter().enumerate() {
            let lines = scene_text_lines_with(&page.text, &area, 1.0, fonts, Role::Heading);
            if lines > area.max_lines {
                continue;
            }
            let top = area.text_top(lines, reach.total());
            if top < safe_t {
                out.push(
                    Finding::new(
                        Severity::Warning,
                        "safe_area.text",
                        format!(
                            "{} ({lines} lines) reaches y={top:.0} px, above the title-safe top at {safe_t:.0} px",
                            crate::pages::page_name(k, n)
                        ),
                        Suggestion::field("shorten the text by a line or more", crate::scene_path(i, "text")),
                    )
                    .scene(&scene.id)
                    .at(starts[i] + page.from_ms),
                );
            }
            if let Some(p) = area.at {
                out.extend(
                    point_findings(timeline, &area, reach.total(), p, &page.text, lines, fonts)
                        .into_iter()
                        .map(|(what, fix)| {
                            Finding::new(
                                Severity::Warning,
                                "safe_area.text",
                                format!("{} ({lines} lines) {what}", crate::pages::page_name(k, n)),
                                Suggestion::field(
                                    fix,
                                    crate::scene_path(i, "text_layout/position"),
                                ),
                            )
                            .scene(&scene.id)
                            .at(starts[i] + page.from_ms)
                        }),
                );
            }
        }
    }

    let band = SubtitleBand::for_timeline(timeline);
    let boxed = timeline.subtitle_box;
    let (_, _, safe_r, _) = title_safe(res);
    let h = f64::from(res.height);
    // A fraction of the frame, rounded to 3 decimals.
    let r3 = |v: f64| (v * 1000.0).round() / 1000.0;
    if !timeline.subtitles.is_empty() {
        // The band's bottom edge is the same for every cue.
        if band.bottom_y > safe_b + 0.5 {
            let message = match boxed {
                None => format!(
                    "at {}x{} the renderer puts the subtitle band's bottom edge {:.0} px from the frame bottom; the title-safe margin is {:.0} px",
                    res.width,
                    res.height,
                    h - band.bottom_y,
                    h - safe_b
                ),
                Some(_) => format!(
                    "the subtitle box's bottom edge is {:.0} px from the frame bottom; the title-safe margin is {:.0} px",
                    h - band.bottom_y,
                    h - safe_b
                ),
            };
            let safe_y = r3(1.0 - TITLE_SAFE_MARGIN);
            out.push(match boxed {
                None => Finding::new(
                    Severity::Info,
                    "safe_area.subtitle_band",
                    message,
                    Suggestion::set(
                        "move the subtitles up into the title-safe area",
                        "/subtitle_box",
                        serde_json::json!({"x": 0.5, "y": safe_y}),
                    ),
                ),
                Some(_) => Finding::new(
                    Severity::Warning,
                    "safe_area.subtitle_band",
                    message,
                    Suggestion::set(
                        "move the subtitles up into the title-safe area",
                        "/subtitle_box/y",
                        serde_json::json!(safe_y),
                    ),
                ),
            });
        }
        if band.left_x < safe_l - 0.5 || band.right_x > safe_r + 0.5 {
            // Only a moved box can do this (the built-in band is 86 % wide, centered).
            let half = SUB_MAX_WIDTH_FRAC / 2.0;
            let x = boxed
                .map_or(0.5, |b| b.x)
                .clamp(TITLE_SAFE_MARGIN + half, 1.0 - TITLE_SAFE_MARGIN - half);
            out.push(Finding::new(
                Severity::Info,
                "safe_area.subtitle_band",
                "a long cue's box can reach outside the title-safe sides (the box is up to 86% of the frame width)",
                Suggestion::set("center the subtitles", "/subtitle_box/x", serde_json::json!(r3(x))),
            ));
        }
        for (i, cue) in timeline.subtitles.iter().enumerate() {
            let lines = cue_lines_with(&shown_text(cue), &band, 1.0, fonts);
            // At the built-in place, only a cue over the line limit can reach the top.
            if lines <= MAX_CUE_LINES && boxed.is_none() {
                continue;
            }
            let top = band.top_y(lines);
            if top < safe_t {
                let suggestion = match boxed {
                    // Lower the box just enough for this cue.
                    Some(b) if lines <= MAX_CUE_LINES => Suggestion::set(
                        "move the subtitles down",
                        "/subtitle_box/y",
                        serde_json::json!(r3(
                            (b.y + (safe_t - top) / h).min(1.0 - TITLE_SAFE_MARGIN)
                        )
                        .max(b.y)),
                    ),
                    _ => Suggestion::field("shorten the cue", crate::cue_path(i, "text")),
                };
                out.push(
                    Finding::new(
                        Severity::Error,
                        "safe_area.subtitle",
                        format!(
                            "cue ({lines} lines) reaches y={top:.0} px, above the title-safe top"
                        ),
                        suggestion,
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
    fn the_default_box_is_the_built_in_band() {
        for (width, height) in [(1920, 1080), (1080, 1920)] {
            let res = Resolution { width, height };
            let a = SubtitleBand::new(res);
            let b = SubtitleBand::at(res, SubtitleBox::default_for(res));
            assert!((a.left_x - b.left_x).abs() < 1e-9);
            assert!((a.right_x - b.right_x).abs() < 1e-9);
            assert!((a.bottom_y - b.bottom_y).abs() < 1e-9);
            assert_eq!(a.text_width, b.text_width);
        }
    }

    fn with_box(extra: &str, cue: &str) -> VideoTimeline {
        serde_json::from_str(&format!(
            r#"{{"fps":30,"resolution":{{"width":1920,"height":1080}},
            "scenes":[{{"id":"a","duration_ms":3000,"text":"Hi"}}],
            "subtitles":[{{"start_ms":0,"end_ms":1000,"text":"{cue}"}}]{extra}}}"#
        ))
        .unwrap()
    }

    #[test]
    fn a_moved_box_is_checked_against_the_safe_area() {
        let codes =
            |t: &VideoTimeline| -> Vec<&'static str> { check(t).iter().map(|f| f.code).collect() };
        // Built-in place and a box inside the safe area: nothing.
        assert!(codes(&with_box("", "hello")).is_empty());
        assert!(codes(&with_box(r#","subtitle_box":{"x":0.5,"y":0.5}"#, "hello")).is_empty());
        // Too low: a warning that moves it up.
        let t = with_box(r#","subtitle_box":{"x":0.5,"y":0.99}"#, "hello");
        let f = check(&t);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].severity, Severity::Warning);
        assert_eq!(f[0].suggestion.path.as_deref(), Some("/subtitle_box/y"));
        assert_eq!(f[0].suggestion.value, Some(serde_json::json!(0.95)));
        // Too high: the cue's top crosses the safe top; the fix lowers the box.
        let t = with_box(r#","subtitle_box":{"x":0.5,"y":0.08}"#, "hello");
        let f = check(&t);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].code, "safe_area.subtitle");
        assert_eq!(f[0].suggestion.path.as_deref(), Some("/subtitle_box/y"));
        let y = f[0].suggestion.value.as_ref().unwrap().as_f64().unwrap();
        let mut moved = t.clone();
        moved.subtitle_box.as_mut().unwrap().y = y;
        assert!(check(&moved).is_empty(), "{:?}", check(&moved));
        // Off to one side: the widest box leaves the safe sides.
        let t = with_box(r#","subtitle_box":{"x":0.2,"y":0.5}"#, "hello");
        let f = check(&t);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].suggestion.path.as_deref(), Some("/subtitle_box/x"));
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
