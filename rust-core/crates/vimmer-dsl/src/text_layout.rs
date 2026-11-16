//! Text layout of the scene text: its size, where it sits, and paging (文字分页).
//!
//! `scene.text_layout` (and a default for every scene in `brand.text_layout`) has three optional
//! parts, each picked on its own: the scene's, else the brand's, else the built-in one.
//!
//! * `size`: font size in px on a 1080p frame (scaled by the shorter frame side / 1080, which is
//!   1 for both supported resolutions). 40..=160; built-in 92.
//! * `position`: `"top"`, `"middle"` or `"bottom"` (built-in, where the text always was:
//!   bottom-aligned above the subtitle band). Every position keeps the 260 px bottom inset that
//!   leaves the subtitle band free; `top` and `middle` also start below a logo in a top corner.
//! * `paging`: `"off"` (built-in: the whole text at once), `"auto"` (one page per sentence:
//!   split after 。！？.!? and at line breaks, very short sentences merged with a neighbour, a
//!   sentence too long for one page split at commas, then between words) or `"manual"` (a page
//!   per paragraph: a blank line in the text starts a new page; pages are not split further).
//!
//! Leaving `text_layout` out (scene and brand) renders exactly as before it existed.
//!
//! Pages share the scene's background and camera motion; each page reveals word by word like the
//! whole text did, and pages cross-fade. When the pages change is decided at render time
//! ([`page_starts`]) and handed to the renderer as `scene.text_pages`, never saved in a timeline:
//! a narrated scene that speaks its own text changes page when the narration reaches the page's
//! first word (word timings from speech recognition), else proportionally to the characters over
//! the narration clip; any other scene splits its duration by characters, at least
//! [`MIN_PAGE_MS`] per page when it is long enough.
//!
//! Everything here is mirrored by `remotion-renderer/src/textPages.ts` (the editor preview pages
//! the text itself); `tests/text_pages_vectors.json` is checked by both.

use serde::{Deserialize, Serialize};

use crate::brand::{Brand, Corner};
use crate::model::Resolution;
use crate::text_style::style_scale;

/// Built-in font size of the scene text, px at 1080p.
pub const DEFAULT_TEXT_PX: f32 = 92.0;
pub const MIN_TEXT_PX: f32 = 40.0;
pub const MAX_TEXT_PX: f32 = 160.0;

/// Most lines one auto page fills.
pub const PAGE_MAX_LINES: u32 = 3;
/// Share of a page's lines the text may fill (lines rarely break exactly at the edge).
pub const PAGE_FILL: f64 = 0.85;
/// A sentence or clause up to this much over a page's capacity stays whole (pages leave room).
pub const PAGE_SLACK: f64 = 1.15;
/// A sentence shorter than this share of a page is merged with a neighbour.
pub const SHORT_PAGE_SHARE: f64 = 0.3;
/// Shortest page of a scene without narration, when the scene is long enough for it.
pub const MIN_PAGE_MS: u64 = 1_200;
/// A narrated page shows this long before its first word is spoken.
pub const PAGE_LEAD_MS: u64 = 150;
/// Pages follow each other at least this far apart.
pub const MIN_PAGE_GAP_MS: u64 = 100;

// Geometry shared with the renderer (`AnimatedText.tsx`) and vimmer-check (`layout.rs`).
/// Top and side padding: 8 % of the frame width.
pub const PADDING_FRAC: f64 = 0.08;
/// Bottom inset that keeps the text above the subtitle band, px.
pub const BOTTOM_INSET_PX: f64 = 260.0;
/// `maxWidth: 88%` of the padded box.
pub const MAX_WIDTH_FRAC: f64 = 0.88;
pub const LINE_HEIGHT: f64 = 1.2;
/// Space kept under a top-corner logo before `top` / `middle` text, as a share of the frame height.
pub const LOGO_GAP_FRAC: f64 = 0.02;

/// Where the scene text sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextPosition {
    Top,
    Middle,
    #[default]
    Bottom,
}

/// How a long scene text is split into pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Paging {
    #[default]
    Off,
    Auto,
    Manual,
}

/// Size, position and paging of the scene text. Every part is optional; see the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct TextLayout {
    /// Font size, px at 1080p (40..=160).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<TextPosition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paging: Option<Paging>,
}

impl TextLayout {
    /// Each part of `self`, else of `fallback`.
    pub fn or(&self, fallback: Option<&TextLayout>) -> TextLayout {
        TextLayout {
            size: self.size.or(fallback.and_then(|f| f.size)),
            position: self.position.or(fallback.and_then(|f| f.position)),
            paging: self.paging.or(fallback.and_then(|f| f.paging)),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.size.is_none() && self.position.is_none() && self.paging.is_none()
    }

    /// Every problem, as messages naming the field under `prefix` (e.g. `text_layout.`).
    pub fn problems(&self, prefix: &str) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(s) = self.size {
            if !(s.is_finite() && (MIN_TEXT_PX..=MAX_TEXT_PX).contains(&s)) {
                out.push(format!(
                    "{prefix}size {s} must be between {MIN_TEXT_PX} and {MAX_TEXT_PX} (px on a 1080p frame; the default is {DEFAULT_TEXT_PX})"
                ));
            }
        }
        out
    }

    /// The layout with every part settled (built-in values for the parts nobody sets).
    pub fn resolved(&self) -> ResolvedTextLayout {
        ResolvedTextLayout {
            size: self.size.unwrap_or(DEFAULT_TEXT_PX),
            position: self.position.unwrap_or_default(),
            paging: self.paging.unwrap_or_default(),
        }
    }
}

/// A [`TextLayout`] with every part decided.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedTextLayout {
    /// px at 1080p.
    pub size: f32,
    pub position: TextPosition,
    pub paging: Paging,
}

impl ResolvedTextLayout {
    /// Font size in px on a `res` frame.
    pub fn font_px(&self, res: Resolution) -> f64 {
        f64::from(self.size) * style_scale(res.width, res.height)
    }
}

/// One page of a scene's text, as handed to the renderer: shown from `start_ms` (from the scene
/// start) until the next page starts. Written by the render into its render props only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextPage {
    pub text: String,
    pub start_ms: u64,
}

/// Top padding of the scene text area in px: 8 % of the frame width, or for `top` / `middle`
/// text below a logo in a top corner (shown on this scene), whichever is lower.
pub fn top_inset_px(
    position: TextPosition,
    brand: Option<&Brand>,
    scene_id: &str,
    res: Resolution,
) -> f64 {
    let (w, h) = (f64::from(res.width), f64::from(res.height));
    let pad = w * PADDING_FRAC;
    if position == TextPosition::Bottom {
        return pad;
    }
    let Some(logo) = brand.and_then(|b| b.logo.as_ref()) else {
        return pad;
    };
    if logo.hide_on.iter().any(|id| id == scene_id)
        || !matches!(logo.corner, Corner::TopLeft | Corner::TopRight)
    {
        return pad;
    }
    let below = (f64::from(logo.margin) + f64::from(logo.size) + LOGO_GAP_FRAC) * h;
    pad.max(below)
}

// ---- Text measure -------------------------------------------------------------------------

/// Characters set full width (CJK, kana, hangul, full-width forms and CJK punctuation).
pub fn is_wide(c: char) -> bool {
    matches!(c,
        '\u{1100}'..='\u{115F}'
        | '\u{2E80}'..='\u{303E}'
        | '\u{3040}'..='\u{33FF}'
        | '\u{3400}'..='\u{4DBF}'
        | '\u{4E00}'..='\u{9FFF}'
        | '\u{A960}'..='\u{A97F}'
        | '\u{AC00}'..='\u{D7A3}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{FE30}'..='\u{FE4F}'
        | '\u{FF00}'..='\u{FF60}'
        | '\u{FFE0}'..='\u{FFE6}'
        | '\u{20000}'..='\u{3FFFD}')
}

/// Rough width of `text` in em: 1 per wide character, 0.28 per space, 0.55 otherwise.
pub fn text_em(text: &str) -> f64 {
    text.chars()
        .map(|c| {
            if c.is_whitespace() {
                0.28
            } else if is_wide(c) {
                1.0
            } else {
                0.55
            }
        })
        .sum()
}

/// Characters that count for timing: letters and digits.
pub fn key_len(text: &str) -> u64 {
    text.chars().filter(|c| c.is_alphanumeric()).count() as u64
}

/// How much text one auto page holds, in em (see [`text_em`]): [`PAGE_MAX_LINES`] lines (fewer
/// when fewer fit) of the text area's width, filled to [`PAGE_FILL`].
pub fn page_cap_em(font_px: f64, res: Resolution) -> f64 {
    let (w, h) = (f64::from(res.width), f64::from(res.height));
    let line_width = w * (1.0 - 2.0 * PADDING_FRAC) * MAX_WIDTH_FRAC;
    let avail = h - w * PADDING_FRAC - BOTTOM_INSET_PX;
    let lines = ((avail / (font_px * LINE_HEIGHT)).floor().max(1.0) as u32).min(PAGE_MAX_LINES);
    line_width / font_px * f64::from(lines) * PAGE_FILL
}

// ---- Splitting ------------------------------------------------------------------------------

fn is_terminator(c: char) -> bool {
    matches!(c, '。' | '！' | '？' | '｡' | '．' | '!' | '?' | '.' | '…')
}

/// Ends a sentence wherever it is (CJK terminators); the others need a space, a wide character
/// or the end of the text after them (`3.14`, `e.g.x` stay whole).
fn is_cjk_terminator(c: char) -> bool {
    matches!(c, '。' | '！' | '？' | '｡' | '．')
}

fn is_closer(c: char) -> bool {
    matches!(
        c,
        '"' | '\'' | '”' | '’' | ')' | '）' | '」' | '』' | '】' | '》' | '〉' | ']'
    )
}

fn is_clause_end(c: char) -> bool {
    matches!(c, '，' | '、' | '；' | '：' | ',' | ';' | ':')
}

/// Punctuation that stays with the character before it.
fn sticks_to_previous(c: char) -> bool {
    is_closer(c) || is_terminator(c) || is_clause_end(c)
}

type Range = (usize, usize);

/// `text[a..b]` without surrounding whitespace, or `None` when nothing is left.
fn trimmed(text: &str, a: usize, b: usize) -> Option<Range> {
    let s = &text[a..b];
    let start = a + (s.len() - s.trim_start().len());
    let end = b - (s.len() - s.trim_end().len());
    (start < end).then_some((start, end))
}

/// Splits `text[r]` after every `ends(c, next)` run (with the closers that follow it) and at
/// line breaks when `lines` is set.
fn split_after(
    text: &str,
    r: Range,
    lines: bool,
    ends: &dyn Fn(char, Option<char>) -> bool,
    starts: &dyn Fn(char) -> bool,
) -> Vec<Range> {
    let chars: Vec<(usize, char)> = text[r.0..r.1]
        .char_indices()
        .map(|(i, c)| (r.0 + i, c))
        .collect();
    let mut out = Vec::new();
    let mut start = r.0;
    let mut i = 0;
    while i < chars.len() {
        let (b, c) = chars[i];
        if lines && c == '\n' {
            out.extend(trimmed(text, start, b));
            start = b + 1;
            i += 1;
            continue;
        }
        if starts(c) {
            let mut j = i + 1;
            while j < chars.len() && (starts(chars[j].1) || is_closer(chars[j].1)) {
                j += 1;
            }
            let next = chars.get(j).map(|&(_, n)| n);
            if ends(c, next) {
                let end = chars.get(j).map_or(r.1, |&(e, _)| e);
                out.extend(trimmed(text, start, end));
                start = end;
            }
            i = j;
            continue;
        }
        i += 1;
    }
    out.extend(trimmed(text, start, r.1));
    out
}

fn free_after(next: Option<char>) -> bool {
    next.is_none_or(|n| n.is_whitespace() || is_wide(n))
}

fn sentences(text: &str) -> Vec<Range> {
    split_after(
        text,
        (0, text.len()),
        true,
        &|c, next| is_cjk_terminator(c) || free_after(next),
        &is_terminator,
    )
}

fn clauses(text: &str, r: Range) -> Vec<Range> {
    split_after(
        text,
        r,
        false,
        &|c, next| !c.is_ascii() || free_after(next),
        &is_clause_end,
    )
}

/// Words: a run of narrow characters with the spaces after it, or one wide character with the
/// punctuation and spaces after it.
fn words(text: &str, r: Range) -> Vec<Range> {
    let chars: Vec<(usize, char)> = text[r.0..r.1]
        .char_indices()
        .map(|(i, c)| (r.0 + i, c))
        .collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let start = chars[i].0;
        let mut j = i + 1;
        if is_wide(chars[i].1) {
            while j < chars.len() && sticks_to_previous(chars[j].1) {
                j += 1;
            }
        } else {
            while j < chars.len() && !chars[j].1.is_whitespace() && !is_wide(chars[j].1) {
                j += 1;
            }
            // Wide closing punctuation after a word stays with it.
            while j < chars.len() && is_wide(chars[j].1) && sticks_to_previous(chars[j].1) {
                j += 1;
            }
        }
        while j < chars.len() && chars[j].1.is_whitespace() {
            j += 1;
        }
        let end = chars.get(j).map_or(r.1, |&(e, _)| e);
        out.extend(trimmed(text, start, end));
        i = j;
    }
    out
}

fn em_of(text: &str, r: Range) -> f64 {
    text_em(&text[r.0..r.1])
}

/// Packs consecutive pieces into groups of at most `cap` em (a piece wider than that alone).
fn pack(text: &str, pieces: &[Range], cap: f64) -> Vec<Range> {
    let mut out: Vec<Range> = Vec::new();
    for &p in pieces {
        match out.last_mut() {
            Some(last) if em_of(text, (last.0, p.1)) <= cap => last.1 = p.1,
            _ => out.push(p),
        }
    }
    out
}

/// Merges pages shorter than [`SHORT_PAGE_SHARE`] of `cap` into the next page, else into the
/// previous one, when the merged page still fits.
fn merge_short(text: &str, mut pages: Vec<Range>, cap: f64) -> Vec<Range> {
    let short = cap * SHORT_PAGE_SHARE;
    let mut i = 0;
    while i + 1 < pages.len() {
        if em_of(text, pages[i]) < short && em_of(text, (pages[i].0, pages[i + 1].1)) <= cap {
            pages[i].1 = pages[i + 1].1;
            pages.remove(i + 1);
        } else {
            i += 1;
        }
    }
    let mut i = pages.len();
    while i > 1 {
        i -= 1;
        if em_of(text, pages[i]) < short && em_of(text, (pages[i - 1].0, pages[i].1)) <= cap {
            pages[i - 1].1 = pages[i].1;
            pages.remove(i);
        }
    }
    pages
}

/// Auto pages of `text` with at most `cap_em` em each (see the module docs).
pub fn auto_pages(text: &str, cap_em: f64) -> Vec<String> {
    let mut pages: Vec<Range> = Vec::new();
    let whole = cap_em * PAGE_SLACK;
    for s in sentences(text) {
        if em_of(text, s) <= whole {
            pages.push(s);
            continue;
        }
        for c in pack(text, &clauses(text, s), cap_em) {
            if em_of(text, c) <= whole {
                pages.push(c);
            } else {
                pages.extend(pack(text, &words(text, c), cap_em));
            }
        }
    }
    merge_short(text, pages, cap_em)
        .into_iter()
        .map(|(a, b)| text[a..b].to_string())
        .collect()
}

/// Manual pages: the paragraphs of `text` (a line holding only whitespace separates them).
pub fn manual_pages(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    for line in text.split('\n') {
        if line.trim().is_empty() {
            if !cur.is_empty() {
                out.push(cur.join("\n").trim().to_string());
                cur.clear();
            }
        } else {
            cur.push(line);
        }
    }
    if !cur.is_empty() {
        out.push(cur.join("\n").trim().to_string());
    }
    out
}

/// The pages `text` is shown in: one page (the text) when paging is off, else the auto or
/// manual pages for a `font_px` text on a `res` frame. Empty for blank text.
pub fn split_pages(text: &str, paging: Paging, font_px: f64, res: Resolution) -> Vec<String> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    let pages = match paging {
        Paging::Off => vec![text.to_string()],
        Paging::Auto => auto_pages(text, page_cap_em(font_px, res)),
        Paging::Manual => manual_pages(text),
    };
    if pages.len() <= 1 {
        // One page is the text as written, exactly as without paging.
        return vec![text.to_string()];
    }
    pages
}

// ---- Timing ---------------------------------------------------------------------------------

/// Start of each page (ms) when `span_ms` from `from_ms` is shared by the pages' characters
/// ([`key_len`], at least 1 each), each page getting at least `min_ms` when `span_ms` allows it
/// (else equal shares). Integer arithmetic, so the renderer computes the same starts.
pub fn proportional_starts(pages: &[String], from_ms: u64, span_ms: u64, min_ms: u64) -> Vec<u64> {
    let n = pages.len() as u64;
    if n == 0 {
        return Vec::new();
    }
    let weights: Vec<u64> = pages.iter().map(|p| key_len(p).max(1)).collect();
    let total: u64 = weights.iter().sum();
    let mut out = Vec::with_capacity(pages.len());
    let mut cum = 0u64;
    for (k, w) in weights.iter().enumerate() {
        let k = k as u64;
        out.push(if min_ms * n <= span_ms {
            from_ms + min_ms * k + (span_ms - min_ms * n) * cum / total
        } else {
            from_ms + span_ms * k / n
        });
        cum += w;
    }
    out
}

/// Page starts from the narration's word timings: each page from [`PAGE_LEAD_MS`] before its
/// first word is spoken, but not before the word before it has been said (sentences spoken
/// without a pause flip as the last word ends). `words` are (word text, start ms, end ms, from
/// the scene start) of the spoken text, which is the scene text. `None` when a page's first word
/// is not among them.
pub fn word_starts(pages: &[String], words: &[(String, u64, u64)]) -> Option<Vec<u64>> {
    let mut out = Vec::with_capacity(pages.len());
    let mut page_key = 0u64;
    for (k, page) in pages.iter().enumerate() {
        if k == 0 {
            out.push(0);
        } else {
            let (mut word_key, mut before_end) = (0u64, 0u64);
            let mut found = None;
            for (w, at, end) in words {
                let len = key_len(w);
                if len > 0 && word_key >= page_key {
                    found = Some((*at, before_end));
                    break;
                }
                word_key += len;
                if len > 0 {
                    before_end = *end;
                }
            }
            let (at, before_end) = found?;
            out.push(at.saturating_sub(PAGE_LEAD_MS).max(before_end.min(at)));
        }
        page_key += key_len(page);
    }
    Some(monotonic(out))
}

/// Starts that never go backwards and keep [`MIN_PAGE_GAP_MS`] apart; the first page at 0.
pub fn monotonic(mut starts: Vec<u64>) -> Vec<u64> {
    for k in 0..starts.len() {
        starts[k] = if k == 0 {
            0
        } else {
            starts[k].max(starts[k - 1] + MIN_PAGE_GAP_MS)
        };
    }
    starts
}

/// What a scene's narration tells the page timing.
#[derive(Debug, Clone, PartialEq)]
pub enum PageClock<'a> {
    /// No narration of this text: the scene's duration.
    Scene { duration_ms: u64 },
    /// A narration clip of the scene text starting `offset_ms` into the scene, `clip_ms` long.
    Clip { offset_ms: u64, clip_ms: u64 },
    /// The clip's aligned words: (text, start ms, end ms, from the scene start).
    Words {
        words: &'a [(String, u64, u64)],
        offset_ms: u64,
        clip_ms: u64,
    },
}

/// When each page starts (ms from the scene start); see the module docs.
pub fn page_starts(pages: &[String], clock: &PageClock<'_>) -> Vec<u64> {
    let starts = match clock {
        PageClock::Scene { duration_ms } => proportional_starts(pages, 0, *duration_ms, MIN_PAGE_MS),
        PageClock::Clip { offset_ms, clip_ms } => proportional_starts(pages, *offset_ms, *clip_ms, 0),
        PageClock::Words {
            words,
            offset_ms,
            clip_ms,
        } => word_starts(pages, words)
            .unwrap_or_else(|| proportional_starts(pages, *offset_ms, *clip_ms, 0)),
    };
    monotonic(starts)
}

/// The pages with their starts.
pub fn timed_pages(pages: Vec<String>, clock: &PageClock<'_>) -> Vec<TextPage> {
    let starts = page_starts(&pages, clock);
    pages
        .into_iter()
        .zip(starts)
        .map(|(text, start_ms)| TextPage { text, start_ms })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAND: Resolution = Resolution {
        width: 1920,
        height: 1080,
    };
    const PORT: Resolution = Resolution {
        width: 1080,
        height: 1920,
    };

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn absent_layout_round_trips_and_resolves_to_the_old_look() {
        let l: TextLayout = serde_json::from_str("{}").unwrap();
        assert!(l.is_empty());
        assert_eq!(serde_json::to_string(&l).unwrap(), "{}");
        let r = l.resolved();
        assert_eq!(r.size, 92.0);
        assert_eq!(r.position, TextPosition::Bottom);
        assert_eq!(r.paging, Paging::Off);
        assert_eq!(r.font_px(LAND), 92.0);
        assert_eq!(r.font_px(PORT), 92.0);
        let json = r#"{"size":120.0,"position":"top","paging":"auto"}"#;
        let l: TextLayout = serde_json::from_str(json).unwrap();
        assert_eq!(serde_json::to_string(&l).unwrap(), json);
    }

    #[test]
    fn each_part_falls_back_to_the_brand() {
        let scene = TextLayout {
            size: Some(120.0),
            ..Default::default()
        };
        let brand = TextLayout {
            size: Some(60.0),
            position: Some(TextPosition::Middle),
            paging: None,
        };
        let r = scene.or(Some(&brand)).resolved();
        assert_eq!(
            (r.size, r.position, r.paging),
            (120.0, TextPosition::Middle, Paging::Off)
        );
    }

    #[test]
    fn size_range_and_unknown_names_are_clear_errors() {
        for bad in [10.0, 39.9, 160.5, f32::NAN] {
            let l = TextLayout {
                size: Some(bad),
                ..Default::default()
            };
            let p = l.problems("text_layout.");
            assert_eq!(p.len(), 1, "{bad}");
            assert!(p[0].starts_with("text_layout.size"), "{}", p[0]);
            assert!(p[0].contains("between 40 and 160"), "{}", p[0]);
        }
        for good in [40.0, 92.0, 160.0] {
            let l = TextLayout {
                size: Some(good),
                ..Default::default()
            };
            assert!(l.problems("").is_empty());
        }
        let e = serde_json::from_str::<TextLayout>(r#"{"position":"left"}"#)
            .unwrap_err()
            .to_string();
        assert!(e.contains("unknown variant `left`") && e.contains("`top`, `middle`, `bottom`"), "{e}");
        let e = serde_json::from_str::<TextLayout>(r#"{"paging":"sentences"}"#)
            .unwrap_err()
            .to_string();
        assert!(e.contains("`off`, `auto`, `manual`"), "{e}");
    }

    #[test]
    fn chinese_sentences_are_pages() {
        let t = "咖啡豆要新鲜研磨，香气才会完整地保留下来。水温控制在九十度左右最合适。最后慢慢注水，耐心等待萃取完成！";
        let p = auto_pages(t, page_cap_em(92.0, LAND));
        assert_eq!(
            p,
            s(&[
                "咖啡豆要新鲜研磨，香气才会完整地保留下来。",
                "水温控制在九十度左右最合适。",
                "最后慢慢注水，耐心等待萃取完成！"
            ])
        );
        assert_eq!(p.concat(), t);
    }

    #[test]
    fn english_sentences_keep_decimals_and_closing_quotes() {
        let t = "Brew at 92.5 degrees for the best taste. She said \"Wait four minutes.\" Then pour it slowly over the grounds and enjoy the cup!";
        let p = auto_pages(t, page_cap_em(92.0, LAND));
        assert_eq!(
            p,
            s(&[
                "Brew at 92.5 degrees for the best taste.",
                "She said \"Wait four minutes.\"",
                "Then pour it slowly over the grounds and enjoy the cup!"
            ])
        );
    }

    #[test]
    fn japanese_and_line_breaks() {
        let t = "コーヒーは挽きたてが一番です。\nお湯の温度にも気をつけましょう\nゆっくり注いでください。";
        let p = auto_pages(t, page_cap_em(92.0, LAND));
        assert_eq!(
            p,
            s(&[
                "コーヒーは挽きたてが一番です。",
                "お湯の温度にも気をつけましょう",
                "ゆっくり注いでください。"
            ])
        );
    }

    #[test]
    fn very_short_sentences_merge_with_a_neighbour() {
        let cap = page_cap_em(92.0, LAND);
        let p = auto_pages("好。我们开始吧，今天要做一杯手冲咖啡。对！", cap);
        assert_eq!(p, s(&["好。我们开始吧，今天要做一杯手冲咖啡。对！"]));
        let p = auto_pages(
            "Hi. This is a much longer sentence about pouring water slowly. That one is long as well, it goes on for a while too. OK.",
            cap,
        );
        assert_eq!(
            p,
            s(&[
                "Hi. This is a much longer sentence about pouring water slowly.",
                "That one is long as well, it goes on for a while too. OK."
            ])
        );
    }

    #[test]
    fn long_sentences_split_at_commas_then_words_within_the_cap() {
        let cap = page_cap_em(92.0, PORT);
        assert!((cap - 22.1).abs() < 0.1, "{cap}");
        let t = "我们今天要一起学习怎样在家里冲出一杯好喝的咖啡，从选豆子开始讲起，一直讲到磨豆、称重、控制水温和注水的节奏，最后再聊聊怎样清洗器具才能让它们用得更久";
        let p = auto_pages(t, cap);
        assert!(p.len() >= 4, "{p:?}");
        assert!(p.iter().all(|x| text_em(x) <= cap * PAGE_SLACK), "{p:?}");
        assert_eq!(p.concat(), t);
        let words = "word ".repeat(60);
        let p = auto_pages(words.trim(), cap);
        assert!(p.len() > 1 && p.iter().all(|x| text_em(x) <= cap * PAGE_SLACK), "{p:?}");
        assert_eq!(p.join(" "), words.trim());
        // A bigger font holds fewer characters per page.
        assert!(page_cap_em(140.0, LAND) < page_cap_em(92.0, LAND));
        assert!(page_cap_em(60.0, LAND) > page_cap_em(92.0, LAND));
    }

    #[test]
    fn manual_pages_are_paragraphs() {
        assert_eq!(
            manual_pages("First page\nstill first\n\n  \nSecond page\n\n\nThird"),
            s(&["First page\nstill first", "Second page", "Third"])
        );
        assert_eq!(manual_pages("no breaks here"), s(&["no breaks here"]));
        let one = split_pages("only one\n", Paging::Manual, 92.0, LAND);
        assert_eq!(one, s(&["only one\n"]), "one page is the text as written");
        assert_eq!(split_pages("a. b.\n\nc", Paging::Off, 92.0, LAND), s(&["a. b.\n\nc"]));
        assert!(split_pages("  ", Paging::Auto, 92.0, LAND).is_empty());
    }

    #[test]
    fn proportional_timing_with_a_minimum_per_page() {
        let pages = s(&["aaaa", "aa", "aaaaaa"]);
        assert_eq!(proportional_starts(&pages, 0, 12_000, 0), vec![0, 4000, 6000]);
        // 1200 each, the remaining 6400 by characters.
        assert_eq!(
            proportional_starts(&pages, 0, 10_000, 1_200),
            vec![0, 1200 + 2133, 2400 + 3200]
        );
        // Too short for the minimum: equal shares.
        assert_eq!(proportional_starts(&pages, 0, 3_000, 1_200), vec![0, 1000, 2000]);
        assert_eq!(
            page_starts(&pages, &PageClock::Clip { offset_ms: 500, clip_ms: 6_000 }),
            vec![0, 500 + 2000, 500 + 3000]
        );
    }

    #[test]
    fn word_timing_lands_pages_on_their_first_spoken_word() {
        let w = |v: &[(&str, u64, u64)]| -> Vec<(String, u64, u64)> {
            v.iter().map(|(t, a, b)| (t.to_string(), *a, *b)).collect()
        };
        let pages = s(&["一二。", "三四五。", "六。"]);
        let words = w(&[
            ("一", 300, 480),
            ("二。", 500, 700),
            ("三", 1_200, 1_380),
            ("四", 1_400, 1_580),
            ("五。", 1_600, 2_450),
            ("六。", 2_500, 2_700),
        ]);
        let clock = PageClock::Words {
            words: &words,
            offset_ms: 300,
            clip_ms: 2_600,
        };
        // 1200 - 150 after a pause; 六 follows 五 without one: it flips as 五 ends (2450).
        assert_eq!(page_starts(&pages, &clock), vec![0, 1_050, 2_450]);
        // English words carry their spaces and punctuation.
        let pages = s(&["Hello there.", "How are you?"]);
        let words = w(&[
            ("Hello ", 0, 350),
            ("there. ", 400, 700),
            ("How ", 1_000, 1_150),
            ("are ", 1_200, 1_350),
            ("you?", 1_400, 1_700),
        ]);
        assert_eq!(word_starts(&pages, &words), Some(vec![0, 850]));
        // Missing words: proportional over the clip.
        let clock = PageClock::Words {
            words: &words[..2],
            offset_ms: 0,
            clip_ms: 2_000,
        };
        assert_eq!(page_starts(&pages, &clock), vec![0, 2_000 * 10 / 19]);
    }

    #[test]
    fn starts_never_go_backwards() {
        assert_eq!(monotonic(vec![50, 0, 0, 500]), vec![0, 100, 200, 500]);
    }

    #[test]
    fn splitting_is_deterministic() {
        let t = "一杯好咖啡。需要耐心。还需要好的豆子，新鲜的水，以及合适的温度。Enjoy it slowly. 好吗？";
        let a = auto_pages(t, 20.0);
        for _ in 0..5 {
            assert_eq!(auto_pages(t, 20.0), a);
        }
    }

    #[test]
    fn top_inset_clears_a_top_logo() {
        let brand: Brand = serde_json::from_str(
            r#"{"logo":{"src":"l.png","corner":"top_right","size":0.1,"margin":0.05,"hide_on":["x"]}}"#,
        )
        .unwrap();
        let pad = 1920.0 * 0.08;
        assert_eq!(top_inset_px(TextPosition::Bottom, Some(&brand), "a", LAND), pad);
        assert_eq!(top_inset_px(TextPosition::Top, None, "a", LAND), pad);
        assert!((top_inset_px(TextPosition::Top, Some(&brand), "a", LAND) - 0.17 * 1080.0).abs() < 1e-3);
        assert_eq!(top_inset_px(TextPosition::Middle, Some(&brand), "x", LAND), pad);
        let low: Brand =
            serde_json::from_str(r#"{"logo":{"src":"l.png","corner":"bottom_left"}}"#).unwrap();
        assert_eq!(top_inset_px(TextPosition::Top, Some(&low), "a", LAND), pad);
    }
}
