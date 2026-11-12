//! Forced alignment of a known script to speech-recognition output.
//!
//! The narration text is known exactly, so subtitles show the script, not what the recognizer
//! heard. Recognition only supplies timing: the script is cut into units (an English word, or
//! one Chinese/Japanese character), the recognition into words the same way, the two sequences
//! are aligned with Levenshtein edit distance (pairing two different words costs their character
//! distance), and every script unit takes the time span of the recognized word it pairs with. Units the recognizer
//! missed get times interpolated from their neighbours, weighted by length.
//!
//! Everything here is pure, so it is tested with fixed recognition fixtures, no engine needed.

/// One recognized piece of speech, with times relative to the start of the audio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecognizedToken {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

impl RecognizedToken {
    pub fn new(text: impl Into<String>, start_ms: u64, end_ms: u64) -> Self {
        Self {
            text: text.into(),
            start_ms,
            end_ms,
        }
    }
}

/// One display unit of the script with its spoken time span, relative to the audio start.
/// Concatenating the `text` of all units gives back the script exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlignedWord {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// Chinese, Japanese and Korean script characters. Each is its own unit, since these
/// languages do not separate words with spaces.
pub fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{3040}'..='\u{309F}'   // Hiragana
        | '\u{30A0}'..='\u{30FF}' // Katakana
        | '\u{31F0}'..='\u{31FF}' // Katakana phonetic extensions
        | '\u{3400}'..='\u{4DBF}' // CJK extension A
        | '\u{4E00}'..='\u{9FFF}' // CJK unified ideographs
        | '\u{F900}'..='\u{FAFF}' // CJK compatibility ideographs
        | '\u{AC00}'..='\u{D7AF}' // Hangul syllables
        | '\u{FF66}'..='\u{FF9F}' // Half-width katakana
        | '\u{20000}'..='\u{2FA1F}')
}

/// Small kana and the long-vowel mark belong to the mora before them, so they share its unit.
fn is_kana_modifier(c: char) -> bool {
    "ぁぃぅぇぉっゃゅょゎァィゥェォッャュョヮヵヶー々".contains(c)
}

/// Opening punctuation attaches to the unit after it instead of the one before.
fn is_opening(c: char) -> bool {
    "([{（［｛「『《〈【〔“‘\"'¿¡".contains(c)
}

/// Characters that take part in alignment. Everything else (spaces, punctuation) only rides along.
fn is_key_char(c: char) -> bool {
    c.is_alphanumeric()
}

/// Lowercased comparable characters of a string.
fn key_chars(s: &str) -> Vec<char> {
    s.chars()
        .filter(|&c| is_key_char(c))
        .flat_map(char::to_lowercase)
        .collect()
}

/// Splits a script into display units: an English (or other spaced-language) word, or a single
/// CJK character. Punctuation and whitespace are attached to a neighbouring unit, so the units
/// concatenate back to exactly `text`. Apostrophes and hyphens inside a word keep it whole
/// ("don't", "state-of-the-art"). Returns no units for text without letters or digits.
pub fn split_units(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut units: Vec<String> = Vec::new();
    // Text waiting to be prefixed to the next unit (leading punctuation, opening quotes).
    let mut pending = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if is_cjk(c) && !(is_kana_modifier(c) && !units.is_empty() && pending.is_empty()) {
            let mut unit = std::mem::take(&mut pending);
            unit.push(c);
            i += 1;
            while i < chars.len() && is_kana_modifier(chars[i]) {
                unit.push(chars[i]);
                i += 1;
            }
            units.push(unit);
        } else if is_cjk(c) {
            // A modifier directly after a finished unit (e.g. after a closing bracket).
            units.last_mut().expect("checked non-empty").push(c);
            i += 1;
        } else if is_key_char(c) {
            let mut unit = std::mem::take(&mut pending);
            while i < chars.len() {
                let d = chars[i];
                let joins = (d == '\'' || d == '’' || d == '-')
                    && i + 1 < chars.len()
                    && is_key_char(chars[i + 1])
                    && !is_cjk(chars[i + 1]);
                if (is_key_char(d) && !is_cjk(d)) || joins {
                    unit.push(d);
                    i += 1;
                } else {
                    break;
                }
            }
            units.push(unit);
        } else {
            // Whitespace or punctuation.
            // Straight quotes open only at the start or after a space; otherwise they close.
            let opens = is_opening(c)
                && (!matches!(c, '"' | '\'')
                    || i == 0
                    || chars[i - 1].is_whitespace()
                    || is_opening(chars[i - 1]));
            let attach_forward = units.is_empty() || opens || !pending.is_empty();
            if attach_forward {
                pending.push(c);
            } else {
                units.last_mut().expect("checked non-empty").push(c);
            }
            i += 1;
        }
    }
    if !pending.is_empty() {
        match units.last_mut() {
            Some(last) => last.push_str(&pending),
            None => return Vec::new(),
        }
    }
    units
}

/// Cost of leaving a unit or a recognized word unpaired. Pairing two different words costs
/// their character edit distance scaled to `0..=GAP`, so a misheard word ("Wimmer" for "Vimmer")
/// is cheaper to pair than to skip, and two unrelated words cost no less than skipping both.
const GAP: u32 = 100;

fn char_distance(a: &[char], b: &[char]) -> usize {
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut diag = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let next = (diag + usize::from(ca != cb))
                .min(row[j] + 1)
                .min(row[j + 1] + 1);
            diag = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}

fn pair_cost(a: &[char], b: &[char]) -> u32 {
    let longest = a.len().max(b.len()).max(1);
    (char_distance(a, b) as u32 * GAP).div_ceil(longest as u32)
}

/// Levenshtein alignment of script units to recognized words. For each unit, the recognized
/// word it is paired with, or `None` when the unit was not recognized at all. Ties prefer
/// pairing, then skipping the unit, so pairs stay as early as the costs allow.
fn align_sequences(reference: &[Vec<char>], hypothesis: &[Vec<char>]) -> Vec<Option<usize>> {
    let n = reference.len();
    let m = hypothesis.len();
    let width = m + 1;
    let mut cost = vec![0u32; (n + 1) * width];
    for (j, slot) in cost.iter_mut().take(width).enumerate() {
        *slot = j as u32 * GAP;
    }
    let mut pair = vec![0u32; (n + 1) * width];
    for i in 1..=n {
        cost[i * width] = i as u32 * GAP;
        for j in 1..=m {
            let p = pair_cost(&reference[i - 1], &hypothesis[j - 1]);
            pair[i * width + j] = p;
            let diag = cost[(i - 1) * width + j - 1] + p;
            let up = cost[(i - 1) * width + j] + GAP;
            let left = cost[i * width + j - 1] + GAP;
            cost[i * width + j] = diag.min(up).min(left);
        }
    }

    let mut map = vec![None; n];
    let (mut i, mut j) = (n, m);
    while i > 0 && j > 0 {
        let here = cost[i * width + j];
        if here == cost[(i - 1) * width + j - 1] + pair[i * width + j] {
            map[i - 1] = Some(j - 1);
            i -= 1;
            j -= 1;
        } else if here == cost[(i - 1) * width + j] + GAP {
            i -= 1; // unit not recognized
        } else {
            j -= 1; // extra recognized word
        }
    }
    map
}

/// Recognized words with their time spans. Whitespace separates words and every CJK character
/// is a word of its own, matching [`split_units`]; tokens without a leading space continue the
/// word before them. A token's span is shared evenly by its characters. Token times are first
/// made monotonic and clamped to the audio.
fn timed_hypothesis(
    tokens: &[RecognizedToken],
    audio_ms: u64,
) -> (Vec<Vec<char>>, Vec<(u64, u64)>) {
    let mut words: Vec<Vec<char>> = Vec::new();
    let mut spans: Vec<(u64, u64)> = Vec::new();
    let mut open = false; // whether the last word can still grow
    let mut floor = 0u64;
    for token in tokens {
        let start = token.start_ms.max(floor).min(audio_ms);
        let end = token.end_ms.max(start).min(audio_ms);
        floor = start;
        let chars: Vec<char> = token.text.chars().collect();
        let count = chars.iter().filter(|&&c| is_key_char(c)).count() as u64;
        let len = end - start;
        let mut k = 0u64;
        for c in chars {
            if c.is_whitespace() {
                open = false;
                continue;
            }
            if !is_key_char(c) {
                continue;
            }
            let span = (start + len * k / count, start + len * (k + 1) / count);
            k += 1;
            let lower: Vec<char> = c.to_lowercase().collect();
            if is_cjk(c) {
                words.push(lower);
                spans.push(span);
                open = false;
            } else if open {
                words.last_mut().expect("open word exists").extend(lower);
                spans.last_mut().expect("open word exists").1 = span.1;
            } else {
                words.push(lower);
                spans.push(span);
                open = true;
            }
        }
    }
    (words, spans)
}

/// Aligns the script `text` to recognized `tokens` of an `audio_ms`-long clip.
///
/// Returns one entry per unit of [`split_units`], in order, with times inside `0..=audio_ms`
/// that never go backwards and never overlap. Never fails: recognition errors become
/// substitutions, missed words are interpolated between their neighbours, extra recognized
/// words are ignored, and with no usable recognition at all the units are spread over the clip
/// in proportion to their length.
pub fn align_words(text: &str, tokens: &[RecognizedToken], audio_ms: u64) -> Vec<AlignedWord> {
    let units = split_units(text);
    if units.is_empty() {
        return Vec::new();
    }
    let unit_keys: Vec<Vec<char>> = units.iter().map(|u| key_chars(u)).collect();
    let (hypothesis, spans) = timed_hypothesis(tokens, audio_ms);
    let map = align_sequences(&unit_keys, &hypothesis);
    let known: Vec<Option<(u64, u64)>> = map.iter().map(|m| m.map(|j| spans[j])).collect();

    // Fill runs of unknown units between known neighbours, weighted by character count.
    let weight = |i: usize| unit_keys[i].len().max(1) as u64;
    let mut times: Vec<(u64, u64)> = vec![(0, 0); units.len()];
    let mut i = 0;
    let mut prev_end = 0u64;
    while i < units.len() {
        if let Some(span) = known[i] {
            times[i] = span;
            prev_end = span.1;
            i += 1;
            continue;
        }
        let run_start = i;
        while i < units.len() && known[i].is_none() {
            i += 1;
        }
        let right = if i < units.len() {
            known[i].expect("loop stopped at a known unit").0
        } else {
            audio_ms
        };
        let left = prev_end.min(right);
        let total: u64 = (run_start..i).map(weight).sum();
        let mut acc = 0;
        for (k, slot) in times.iter_mut().enumerate().take(i).skip(run_start) {
            let s = left + (right - left) * acc / total;
            acc += weight(k);
            let e = left + (right - left) * acc / total;
            *slot = (s, e);
        }
        prev_end = right;
    }

    // Monotonic and non-overlapping, inside the clip.
    let mut floor = 0u64;
    units
        .into_iter()
        .zip(times)
        .map(|(text, (s, e))| {
            let start = s.max(floor).min(audio_ms);
            let end = e.max(start).min(audio_ms);
            floor = end;
            AlignedWord {
                text,
                start_ms: start,
                end_ms: end,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(text: &str, start_ms: u64, end_ms: u64) -> RecognizedToken {
        RecognizedToken::new(text, start_ms, end_ms)
    }

    fn texts(words: &[AlignedWord]) -> Vec<&str> {
        words.iter().map(|w| w.text.as_str()).collect()
    }

    fn spans(words: &[AlignedWord]) -> Vec<(u64, u64)> {
        words.iter().map(|w| (w.start_ms, w.end_ms)).collect()
    }

    fn assert_well_formed(text: &str, words: &[AlignedWord], audio_ms: u64) {
        assert_eq!(
            words.iter().map(|w| w.text.as_str()).collect::<String>(),
            text
        );
        let mut floor = 0;
        for w in words {
            assert!(w.start_ms >= floor, "{words:?}");
            assert!(w.end_ms >= w.start_ms, "{words:?}");
            assert!(w.end_ms <= audio_ms, "{words:?}");
            floor = w.end_ms;
        }
    }

    #[test]
    fn splits_english_into_words_with_trailing_punctuation_and_space() {
        assert_eq!(
            split_units("Hello, world! Don't stop state-of-the-art."),
            vec!["Hello, ", "world! ", "Don't ", "stop ", "state-of-the-art."]
        );
    }

    #[test]
    fn splits_chinese_into_characters() {
        assert_eq!(
            split_units("你好，世界。"),
            vec!["你", "好，", "世", "界。"]
        );
    }

    #[test]
    fn splits_japanese_by_mora_keeping_small_kana_and_long_marks() {
        assert_eq!(
            split_units("「きょうは」コーヒー。"),
            vec!["「きょ", "う", "は」", "コー", "ヒー。"]
        );
    }

    #[test]
    fn mixed_scripts_numbers_and_leading_punctuation() {
        assert_eq!(
            split_units("  “Vimmer 在 2026 年发布"),
            vec!["  “Vimmer ", "在 ", "2026 ", "年", "发", "布"]
        );
        assert_eq!(
            split_units("say \"hi\" to “them”"),
            vec!["say ", "\"hi\" ", "to ", "“them”"]
        );
        assert!(split_units("。。。 ").is_empty());
        assert!(split_units("").is_empty());
    }

    #[test]
    fn exact_english_recognition_takes_token_times() {
        let text = "Turn any idea into video.";
        let tokens = [
            tok(" Turn", 100, 400),
            tok(" any", 400, 600),
            tok(" idea", 600, 1000),
            tok(" into", 1000, 1200),
            tok(" video", 1200, 1700),
            tok(".", 1700, 1750),
        ];
        let words = align_words(text, &tokens, 2000);
        assert_well_formed(text, &words, 2000);
        assert_eq!(
            spans(&words),
            vec![
                (100, 400),
                (400, 600),
                (600, 1000),
                (1000, 1200),
                (1200, 1700)
            ]
        );
    }

    #[test]
    fn subword_tokens_merge_into_one_word() {
        let words = align_words(
            "Remotion renders",
            &[
                tok(" Rem", 0, 200),
                tok("otion", 200, 500),
                tok(" renders", 600, 900),
            ],
            1000,
        );
        assert_eq!(spans(&words), vec![(0, 500), (600, 900)]);
    }

    #[test]
    fn misrecognized_word_still_gets_its_slot() {
        // "Vimmer" heard as "Wimmer": a substitution keeps the timing.
        let words = align_words(
            "Vimmer makes videos",
            &[
                tok(" Wimmer", 0, 500),
                tok(" makes", 500, 800),
                tok(" videos", 800, 1300),
            ],
            1500,
        );
        assert_eq!(texts(&words), vec!["Vimmer ", "makes ", "videos"]);
        assert_eq!(spans(&words), vec![(0, 500), (500, 800), (800, 1300)]);
    }

    #[test]
    fn dropped_words_are_interpolated_between_neighbours() {
        // "quick brown" was not recognized at all.
        let text = "the quick brown fox";
        let words = align_words(text, &[tok(" the", 0, 200), tok(" fox", 1200, 1500)], 1600);
        assert_well_formed(text, &words, 1600);
        assert_eq!(words[0].end_ms, 200);
        // 5 + 5 letters share 200..1200 evenly.
        assert_eq!(spans(&words)[1..3], [(200, 700), (700, 1200)]);
        assert_eq!(words[3].start_ms, 1200);
    }

    #[test]
    fn extra_recognized_words_are_ignored() {
        let words = align_words(
            "hello world",
            &[
                tok(" hello", 0, 300),
                tok(" um", 300, 500),
                tok(" uh", 500, 700),
                tok(" world", 700, 1000),
            ],
            1000,
        );
        assert_eq!(spans(&words), vec![(0, 300), (700, 1000)]);
    }

    #[test]
    fn missing_tail_is_spread_up_to_the_end_of_the_audio() {
        let words = align_words("one two three", &[tok(" one", 0, 300)], 1300);
        assert_eq!(words[0].end_ms, 300);
        assert_eq!(words[2].end_ms, 1300);
        assert_well_formed("one two three", &words, 1300);
    }

    #[test]
    fn no_recognition_spreads_by_length() {
        let words = align_words("ab abcd", &[], 600);
        assert_eq!(spans(&words), vec![(0, 200), (200, 600)]);
        let words = align_words("你好", &[tok("", 0, 0)], 1000);
        assert_eq!(spans(&words), vec![(0, 500), (500, 1000)]);
    }

    #[test]
    fn chinese_multi_character_tokens_are_split_per_character() {
        // Fixture modelled on whisper output for a Chinese clip: tokens carry 1-3 characters,
        // punctuation comes as its own token, and one character is a homophone error (在→再).
        let text = "我们在西安，看钟楼。";
        let tokens = [
            tok("我们", 0, 400),
            tok("再", 400, 600),
            tok("西安", 600, 1000),
            tok("，", 1000, 1000),
            tok("看", 1100, 1300),
            tok("钟楼", 1300, 1900),
            tok("。", 1900, 1900),
        ];
        let words = align_words(text, &tokens, 2100);
        assert_well_formed(text, &words, 2100);
        assert_eq!(
            texts(&words),
            vec!["我", "们", "在", "西", "安，", "看", "钟", "楼。"]
        );
        assert_eq!(
            spans(&words),
            vec![
                (0, 200),
                (200, 400),
                (400, 600),
                (600, 800),
                (800, 1000),
                (1100, 1300),
                (1300, 1600),
                (1600, 1900)
            ]
        );
    }

    #[test]
    fn traditional_characters_from_the_recognizer_still_align() {
        // Whisper sometimes answers Mandarin in traditional script: every character differs.
        let words = align_words("钟楼", &[tok("鐘樓", 100, 500)], 600);
        assert_eq!(spans(&words), vec![(100, 300), (300, 500)]);
    }

    #[test]
    fn japanese_with_kanji_kana_mismatch() {
        // Script uses kana where the recognizer wrote kanji (きょう → 今日): fewer recognized
        // characters, so part of the script is interpolated, and timing stays monotonic.
        let text = "きょうはいい天気です。";
        let tokens = [
            tok("今日", 0, 400),
            tok("は", 400, 500),
            tok("いい", 500, 800),
            tok("天気", 800, 1300),
            tok("です", 1300, 1700),
            tok("。", 1700, 1700),
        ];
        let words = align_words(text, &tokens, 1800);
        assert_well_formed(text, &words, 1800);
        assert_eq!(
            texts(&words),
            vec!["きょ", "う", "は", "い", "い", "天", "気", "で", "す。"]
        );
        assert_eq!(words[2].start_ms, 400, "は matches exactly");
        assert_eq!((words[5].start_ms, words[6].end_ms), (800, 1300));
        assert_eq!(words[8].end_ms, 1700);
    }

    #[test]
    fn out_of_order_and_out_of_range_token_times_are_sanitized() {
        let text = "a b c";
        let words = align_words(
            text,
            &[tok("a", 500, 400), tok("b", 300, 900), tok("c", 800, 5000)],
            1000,
        );
        assert_well_formed(text, &words, 1000);
        assert_eq!(words[2].end_ms, 1000);
    }

    #[test]
    fn mixed_language_script() {
        let text = "用 Vimmer 做视频";
        let tokens = [
            tok("用", 0, 200),
            tok(" Vimmer", 200, 700),
            tok("做", 700, 900),
            tok("视频", 900, 1300),
        ];
        let words = align_words(text, &tokens, 1400);
        assert_eq!(texts(&words), vec!["用 ", "Vimmer ", "做", "视", "频"]);
        assert_eq!(words[1].start_ms, 200);
        assert_eq!(words[1].end_ms, 700);
    }

    #[test]
    fn completely_wrong_recognition_does_not_panic() {
        let text = "完全不同的句子";
        let words = align_words(text, &[tok(" totally unrelated words here", 0, 900)], 1000);
        assert_well_formed(text, &words, 1000);
        let words = align_words("x", &[tok("a b c d e f g h", 0, 900)], 1000);
        assert_well_formed("x", &words, 1000);
        assert!(align_words("", &[tok("a", 0, 1)], 10).is_empty());
        assert_well_formed("a b", &align_words("a b", &[], 0), 0);
    }
}
