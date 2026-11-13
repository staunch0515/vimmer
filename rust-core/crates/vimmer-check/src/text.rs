//! Text length checks: scene text that overflows its area, subtitle cues longer than the band
//! allows, and reading speed.
//!
//! # Width heuristic
//!
//! The renderer loads no web font, so text is set in Inter when installed and otherwise in the
//! system UI font (SF Pro on macOS), with PingFang/Hiragino for CJK. Widths are estimated per
//! character class in em, from Inter SemiBold's advances (SF Pro is a few percent narrower):
//!
//! | class                                   | em    |
//! |-----------------------------------------|-------|
//! | CJK ideographs, kana, hangul, full-width forms and CJK punctuation | 1.00 |
//! | Latin lowercase                         | 0.55  |
//! | Latin uppercase                         | 0.68  |
//! | digits                                  | 0.62  |
//! | narrow punctuation `.,:;'!|` and space  | 0.28  |
//! | other ASCII punctuation                 | 0.45  |
//! | other letters (accented, Cyrillic, …)   | 0.58  |
//! | emoji and other symbols above U+1F000   | 1.20  |
//!
//! Lines are filled greedily the way the browser breaks them: scene text word by word (each
//! word is an inline-block followed by a 0.25em margin, a word wider than the line wraps inside
//! itself); subtitles at spaces and between CJK characters, with closing CJK punctuation kept on
//! the previous line. `text-wrap: balance` evens lines out but does not change their number.
//!
//! **Error margin.** Latin widths are within about ±12% of the browser's (font fallback, kerning,
//! the exact letters used), CJK widths within about ±3%. A finding is therefore an error only
//! when the text still overflows with every width 12% narrower ([`OPTIMISTIC`]); when only the
//! nominal estimate overflows, it is a warning.

use serde_json::json;
use vimmer_dsl::{SubtitleCue, VideoTimeline};

use crate::layout::{SceneTextArea, SubtitleBand};
use crate::{cue_path, scene_path, scene_starts_ms, Finding, Severity, Suggestion};

/// Width factor of the optimistic estimate (see the module docs).
pub const OPTIMISTIC: f64 = 0.88;

/// Most lines a subtitle cue should take. Two is the usual subtitle limit, and the renderer's
/// bottom inset for scene text (260 px) only leaves room for a two-line band.
pub const MAX_CUE_LINES: u32 = 2;

/// Reading-speed limits in characters per second.
///
/// * `en` 20 cps: Netflix's limit for adult English programs (counting spaces).
/// * `zh` 9 cps: Netflix's limit for Simplified Chinese (punctuation not counted here).
/// * `ja` 4 cps: Netflix's limit for Japanese (punctuation not counted here).
///
/// These are limits for text read *without* matching speech. Text that is spoken as it shows
/// is not checked against them: scene text with `narration` (its length is checked against the
/// speech instead, see `timing`), cues with word timing and cues over a narrated scene (the
/// render times those from the speech). Text over
/// the limit is a warning.
pub fn cps_limit(lang: Lang) -> f64 {
    match lang {
        Lang::En => 20.0,
        Lang::Zh => 9.0,
        Lang::Ja => 4.0,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Zh,
    Ja,
}

impl Lang {
    /// Kana means Japanese, other CJK (or hangul) Chinese-like, everything else English-like.
    pub fn of(text: &str) -> Lang {
        match vimmer_media::guess_language(text) {
            Some("ja") => Lang::Ja,
            Some("zh") | Some("ko") => Lang::Zh,
            _ => Lang::En,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Zh => "zh",
            Lang::Ja => "ja",
        }
    }
}

/// Characters counted for reading speed: for `en` every character including inner spaces; for
/// CJK every character that is neither whitespace nor punctuation.
pub fn reading_chars(text: &str, lang: Lang) -> usize {
    match lang {
        Lang::En => text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .count(),
        Lang::Zh | Lang::Ja => text
            .chars()
            .filter(|c| !c.is_whitespace() && !is_punct(*c))
            .count(),
    }
}

fn is_punct(c: char) -> bool {
    c.is_ascii_punctuation()
        || ('\u{3000}'..='\u{303F}').contains(&c)
        || ('\u{FF01}'..='\u{FF0F}').contains(&c)
        || ('\u{FF1A}'..='\u{FF20}').contains(&c)
        || ('\u{FF3B}'..='\u{FF40}').contains(&c)
        || ('\u{FF5B}'..='\u{FF65}').contains(&c)
        || matches!(c, '…' | '—' | '–' | '“' | '”' | '‘' | '’' | '·' | '・')
}

/// True for characters set full width.
pub fn is_wide(c: char) -> bool {
    vimmer_media::is_cjk(c)
        || ('\u{3000}'..='\u{303F}').contains(&c)
        || ('\u{FF00}'..='\u{FF60}').contains(&c)
        || ('\u{FFE0}'..='\u{FFE6}').contains(&c)
        || ('\u{AC00}'..='\u{D7AF}').contains(&c)
        || ('\u{2E80}'..='\u{2FDF}').contains(&c)
        || matches!(c, '…' | '—' | '“' | '”' | '‘' | '’' | '・')
}

/// Advance width of `c` in em (see the module docs).
pub fn char_em(c: char) -> f64 {
    if c.is_whitespace() {
        0.28
    } else if is_wide(c) {
        1.0
    } else if c.is_ascii_lowercase() {
        0.55
    } else if c.is_ascii_uppercase() {
        0.68
    } else if c.is_ascii_digit() {
        0.62
    } else if matches!(c, '.' | ',' | ':' | ';' | '\'' | '!' | '|') {
        0.28
    } else if c.is_ascii_punctuation() {
        0.45
    } else if u32::from(c) >= 0x1F000 {
        1.2
    } else {
        0.58
    }
}

/// Closing punctuation that may not start a line in CJK text.
fn no_break_before(c: char) -> bool {
    matches!(
        c,
        '，' | '。'
            | '、'
            | '！'
            | '？'
            | '：'
            | '；'
            | '）'
            | '」'
            | '』'
            | '】'
            | '》'
            | '〉'
            | '…'
            | 'ー'
            | 'ゃ'
            | 'ゅ'
            | 'ょ'
            | 'っ'
            | '’'
            | '”'
            | ','
            | '.'
            | '!'
            | '?'
    )
}

/// Width in px of `s` at `font_px`, with `letter_spacing_em` after each character.
fn width(s: &str, font_px: f64, letter_spacing_em: f64) -> f64 {
    s.chars()
        .map(|c| (char_em(c) + letter_spacing_em) * font_px)
        .sum()
}

/// Estimated line count of scene text in [`SceneTextArea`] with widths scaled by `factor`.
pub fn scene_text_lines(text: &str, area: &SceneTextArea, factor: f64) -> u32 {
    let gap = area.word_gap_em * area.font_px;
    let mut lines = 0u32;
    let mut cur = 0.0f64;
    for word in text.split_whitespace() {
        let w = width(word, area.font_px, area.letter_spacing_em) * factor;
        let boxed = w + gap;
        if w > area.line_width {
            // An inline-block wider than the line takes whole lines of its own.
            if cur > 0.0 {
                lines += 1;
            }
            let inner = (w / area.line_width).ceil() as u32;
            lines += inner - 1;
            cur = (w - f64::from(inner - 1) * area.line_width + gap).min(area.line_width);
            continue;
        }
        if cur > 0.0 && cur + boxed > area.line_width + 0.5 {
            lines += 1;
            cur = 0.0;
        }
        cur += boxed;
    }
    if cur > 0.0 {
        lines += 1;
    }
    lines
}

/// Estimated line count of a subtitle cue in `band` with widths scaled by `factor`.
pub fn cue_lines(text: &str, band: &SubtitleBand, factor: f64) -> u32 {
    let segments = break_segments(text);
    let space = char_em(' ') * band.font_px * factor;
    let max = band.text_width;
    let mut lines = 0u32;
    let mut cur = 0.0f64;
    for (spaced, seg) in segments {
        let w = width(&seg, band.font_px, 0.0) * factor;
        let add = if spaced && cur > 0.0 { space + w } else { w };
        if cur > 0.0 && cur + add > max + 0.5 {
            lines += 1;
            cur = 0.0;
        }
        if cur == 0.0 && w > max {
            let inner = (w / max).ceil() as u32;
            lines += inner - 1;
            cur = w - f64::from(inner - 1) * max;
        } else if cur == 0.0 {
            cur = w;
        } else {
            cur += add;
        }
    }
    if cur > 0.0 {
        lines += 1;
    }
    lines
}

/// Splits text at its line-break opportunities: spaces, and around every CJK character except
/// before closing punctuation. Each segment carries whether a space preceded it.
fn break_segments(text: &str) -> Vec<(bool, String)> {
    let mut out: Vec<(bool, String)> = Vec::new();
    let mut spaced = false;
    let mut cur = String::new();
    let mut cur_spaced = false;
    let flush = |out: &mut Vec<(bool, String)>, cur: &mut String, sp: bool| {
        if !cur.is_empty() {
            out.push((sp, std::mem::take(cur)));
        }
    };
    for c in text.chars() {
        if c.is_whitespace() {
            flush(&mut out, &mut cur, cur_spaced);
            spaced = true;
            continue;
        }
        if no_break_before(c) && !spaced {
            if cur.is_empty() {
                if let Some(last) = out.last_mut() {
                    last.1.push(c);
                    continue;
                }
            }
            cur.push(c);
            continue;
        }
        if is_wide(c) {
            flush(&mut out, &mut cur, cur_spaced);
            cur.push(c);
            cur_spaced = spaced;
            flush(&mut out, &mut cur, cur_spaced);
        } else {
            if cur.is_empty() {
                cur_spaced = spaced;
            } else if cur.chars().last().is_some_and(is_wide) {
                flush(&mut out, &mut cur, cur_spaced);
                cur_spaced = spaced;
            }
            cur.push(c);
        }
        spaced = false;
    }
    flush(&mut out, &mut cur, cur_spaced);
    out
}

/// Characters of `text` that fit in `lines` lines, at the average width of its own characters.
fn capacity_chars(text: &str, per_line_px: f64, font_px: f64, lines: u32) -> usize {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return 0;
    }
    let avg = chars.iter().map(|c| char_em(*c)).sum::<f64>() / chars.len() as f64 * font_px;
    ((per_line_px * f64::from(lines) / avg) * 0.9).floor() as usize
}

pub fn check(timeline: &VideoTimeline) -> Vec<Finding> {
    let mut out = Vec::new();
    let res = timeline.resolution;
    let area = SceneTextArea::new(res);
    let starts = scene_starts_ms(timeline);
    for (i, scene) in timeline.scenes.iter().enumerate() {
        let Some(text) = scene.text.as_deref().filter(|t| !t.trim().is_empty()) else {
            continue;
        };
        let nominal = scene_text_lines(text, &area, 1.0);
        let optimistic = scene_text_lines(text, &area, OPTIMISTIC);
        if nominal > area.max_lines {
            let severity = if optimistic > area.max_lines {
                Severity::Error
            } else {
                Severity::Warning
            };
            let cap = capacity_chars(text, area.line_width, area.font_px, area.max_lines);
            out.push(
                Finding::new(
                    severity,
                    "text.overflow",
                    format!(
                        "scene text needs about {nominal} lines at {} px but only {} fit above the subtitle area ({}x{}); the top lines run off the frame",
                        area.font_px, area.max_lines, res.width, res.height
                    ),
                    Suggestion::field(
                        format!("shorten the text to about {cap} characters, or split the scene in two"),
                        scene_path(i, "text"),
                    ),
                )
                .scene(&scene.id)
                .at(starts[i]),
            );
        }
        if scene.narration.is_some() {
            continue;
        }
        let lang = Lang::of(text);
        let chars = reading_chars(text, lang);
        let limit = cps_limit(lang);
        let cps = chars as f64 * 1000.0 / scene.duration_ms.max(1) as f64;
        if cps > limit && scene.duration_ms > 0 {
            let needed = ((chars as f64 / limit) * 1000.0 / 100.0).ceil() as u64 * 100;
            out.push(
                Finding::new(
                    Severity::Warning,
                    "text.reading_speed",
                    format!(
                        "scene text is {chars} characters in {} ({cps:.1} chars/s); the {} limit is {limit} chars/s",
                        crate::secs(scene.duration_ms),
                        lang.code()
                    ),
                    Suggestion::set(
                        format!("lengthen the scene to {needed} ms or shorten the text"),
                        scene_path(i, "duration_ms"),
                        json!(needed),
                    ),
                )
                .scene(&scene.id)
                .at(starts[i]),
            );
        }
    }

    let band = SubtitleBand::new(res);
    for (i, cue) in timeline.subtitles.iter().enumerate() {
        let text = shown_text(cue);
        if text.trim().is_empty() {
            continue;
        }
        let nominal = cue_lines(&text, &band, 1.0);
        let optimistic = cue_lines(&text, &band, OPTIMISTIC);
        if nominal > MAX_CUE_LINES {
            let severity = if optimistic > MAX_CUE_LINES + 1 {
                Severity::Error
            } else {
                Severity::Warning
            };
            let cap = capacity_chars(&text, band.text_width, band.font_px, MAX_CUE_LINES);
            out.push(
                Finding::new(
                    severity,
                    "subtitle.too_long",
                    format!(
                        "cue needs about {nominal} lines in the subtitle band (max {MAX_CUE_LINES}, about {cap} characters); a taller band covers the scene text",
                    ),
                    split_suggestion(i, cue, cap),
                )
                .cue(i)
                .at(cue.start_ms),
            );
        }
        // Spoken as it shows: word-timed, or over a narrated scene (timed at render).
        let over_narration = timeline.scenes.iter().zip(&starts).any(|(sc, st)| {
            sc.narration.is_some() && cue.start_ms < st + sc.duration_ms && cue.end_ms > *st
        });
        if !cue.words.is_empty() || over_narration {
            continue;
        }
        let lang = Lang::of(&text);
        let chars = reading_chars(&text, lang);
        let limit = cps_limit(lang);
        let dur = cue.end_ms.saturating_sub(cue.start_ms);
        if dur == 0 {
            continue;
        }
        let cps = chars as f64 * 1000.0 / dur as f64;
        if cps > limit {
            let needed = ((chars as f64 / limit) * 1000.0).ceil() as u64;
            let next_start = timeline
                .subtitles
                .get(i + 1)
                .map(|n| n.start_ms)
                .unwrap_or(timeline.total_duration_ms());
            let end = cue.start_ms + needed;
            let suggestion = if end <= next_start {
                Suggestion::set(
                    format!("end the cue at {end} ms, or shorten the text"),
                    cue_path(i, "end_ms"),
                    json!(end),
                )
            } else {
                Suggestion::field(
                    format!(
                        "shorten the text to about {} characters (no room to lengthen the cue)",
                        (limit * dur as f64 / 1000.0).floor()
                    ),
                    cue_path(i, "text"),
                )
            };
            out.push(
                Finding::new(
                    Severity::Warning,
                    "subtitle.reading_speed",
                    format!(
                        "cue is {chars} characters in {} ({cps:.1} chars/s); the {} limit is {limit} chars/s",
                        crate::secs(dur),
                        lang.code()
                    ),
                    suggestion,
                )
                .cue(i)
                .at(cue.start_ms),
            );
        }
    }
    out
}

/// The text the renderer shows for a cue: the word texts when they spell the cue text.
pub fn shown_text(cue: &SubtitleCue) -> String {
    if cue.words.is_empty() {
        return cue.text.clone();
    }
    let joined: String = cue.words.iter().map(|w| w.text.as_str()).collect();
    let squash = |s: &str| s.split_whitespace().collect::<String>();
    if squash(&joined) == squash(&cue.text) {
        joined
    } else {
        cue.text.clone()
    }
}

/// Splits a too-long cue into two at the break nearest its middle, with times proportional to
/// the characters on each side. Cues with word timing get a text-only suggestion.
fn split_suggestion(index: usize, cue: &SubtitleCue, cap: usize) -> Suggestion {
    if !cue.words.is_empty() {
        return Suggestion::field(
            format!("shorten the text to about {cap} characters or split the cue in two (its word timing is redone on the next render)"),
            cue_path(index, "text"),
        );
    }
    let chars: Vec<char> = cue.text.trim().chars().collect();
    let n = chars.len();
    let mut best = None;
    for (i, c) in chars.iter().enumerate().skip(1) {
        let breakable = c.is_whitespace() || is_wide(*c) || is_wide(chars[i - 1]);
        if breakable && !no_break_before(*c) {
            let dist = (i as i64 - n as i64 / 2).abs();
            if best.is_none_or(|(_, d)| dist < d) {
                best = Some((i, dist));
            }
        }
    }
    let Some((at, _)) = best else {
        return Suggestion::field(
            format!("shorten the text to about {cap} characters"),
            cue_path(index, "text"),
        );
    };
    let first: String = chars[..at].iter().collect::<String>().trim().to_string();
    let second: String = chars[at..].iter().collect::<String>().trim().to_string();
    let dur = cue.end_ms - cue.start_ms;
    let split = cue.start_ms
        + dur * first.chars().count() as u64
            / (first.chars().count() + second.chars().count()).max(1) as u64;
    Suggestion::set(
        format!("split the cue in two at {split} ms"),
        cue_path(index, ""),
        json!([
            {"start_ms": cue.start_ms, "end_ms": split, "text": first},
            {"start_ms": split, "end_ms": cue.end_ms, "text": second},
        ]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use vimmer_dsl::Resolution;

    const LANDSCAPE: Resolution = Resolution {
        width: 1920,
        height: 1080,
    };
    const PORTRAIT: Resolution = Resolution {
        width: 1080,
        height: 1920,
    };

    #[test]
    fn short_english_scene_text_fits_in_two_lines() {
        let area = SceneTextArea::new(LANDSCAPE);
        assert_eq!(area.max_lines, 6);
        let n = scene_text_lines("Turn any idea into a polished video in minutes", &area, 1.0);
        assert_eq!(n, 2);
    }

    #[test]
    fn cjk_sentence_wraps_inside_its_inline_block() {
        let area = SceneTextArea::new(LANDSCAPE);
        // 1419 px per line at 92 px: 15 full-width characters fit, 16 do not.
        let fifteen = "一二三四五六七八九十一二三四五";
        assert_eq!(scene_text_lines(fifteen, &area, 1.0), 1);
        assert_eq!(scene_text_lines(&fifteen.repeat(3), &area, 1.0), 3);
        let portrait = SceneTextArea::new(PORTRAIT);
        assert_eq!(portrait.max_lines, 14);
        assert_eq!(scene_text_lines(fifteen, &portrait, 1.0), 2);
    }

    #[test]
    fn cue_lines_break_at_spaces_and_cjk() {
        let band = SubtitleBand::new(LANDSCAPE);
        assert_eq!(cue_lines("in just a few minutes.", &band, 1.0), 1);
        // 1587 px of text width at 54 px: 29 CJK characters per line.
        let cjk = "西安钟楼，古城的中心。".repeat(6);
        assert_eq!(cue_lines(&cjk, &band, 1.0), 3);
        let long = "word ".repeat(80);
        assert!(cue_lines(&long, &band, 1.0) >= 4);
    }

    #[test]
    fn closing_punctuation_stays_with_the_previous_character() {
        let segs = break_segments("中心。好");
        assert_eq!(
            segs.iter().map(|s| s.1.as_str()).collect::<Vec<_>>(),
            ["中", "心。", "好"]
        );
        let segs = break_segments("hello world");
        assert_eq!(
            segs,
            vec![(false, "hello".to_string()), (true, "world".to_string())]
        );
    }

    #[test]
    fn reading_chars_by_language() {
        assert_eq!(reading_chars("西安钟楼，古城的中心。", Lang::Zh), 9);
        assert_eq!(reading_chars("  in just  a few ", Lang::En), 13);
        assert_eq!(Lang::of("こんにちは"), Lang::Ja);
        assert_eq!(Lang::of("西安"), Lang::Zh);
        assert_eq!(Lang::of("Hello"), Lang::En);
    }

    #[test]
    fn split_suggestion_halves_the_cue() {
        let cue = SubtitleCue::new(0, 4000, "aaaa bbbb cccc dddd");
        let s = split_suggestion(3, &cue, 10);
        assert_eq!(s.path.as_deref(), Some("/subtitles/3"));
        let v = s.value.unwrap();
        assert_eq!(v[0]["text"], "aaaa bbbb");
        assert_eq!(v[1]["text"], "cccc dddd");
        assert_eq!(v[0]["end_ms"], v[1]["start_ms"]);
    }
}
