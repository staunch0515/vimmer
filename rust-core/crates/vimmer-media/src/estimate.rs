//! Narration length without synthesizing: how long the narration step's audio file will be.
//!
//! One estimator for storyboard generation (scene durations) and `vimmer check`
//! (`narration.too_long`). It predicts the length the scheduler measures: the whole synthesized
//! file, leading and trailing silence included, before the narration step's 300 ms tail.
//!
//! The model is linear in a few character classes, per engine and language:
//!
//! | feature   | counts                                                                  |
//! |-----------|-------------------------------------------------------------------------|
//! | `han`     | CJK ideographs (and 々)                                                  |
//! | `kana`    | hiragana and katakana, the long mark ー included                        |
//! | `letters` | ASCII letters                                                           |
//! | `words`   | runs of ASCII letters (an English word, a Latin name inside CJK)         |
//! | `number`  | spoken length of ASCII numbers (with `$` and `%`): moras (ja), syllables (zh) or words (en) of how the number is read (see [`number_units`]) |
//! | `minor`   | short pauses: `, ; :` `、，；：` and dashes (a comma between digits is not) |
//! | `major`   | sentence pauses: `. ! ?` `。！？…` and line breaks; a run counts once (a dot between digits is not) |
//! | `base`    | once per clip: the engine's lead-in and trailing silence                 |
//!
//! The costs come from `calibration/`: a corpus of zh / ja / en sentences synthesized with
//! each engine's default voice for the language (`say`: Tingting, Kyoko, Samantha; Kokoro:
//! zf_001, jf_alpha, af_heart), measured exactly as the render measures them, and fitted by
//! `calibration/fit.py`. Re-run both after a voice or model update (see
//! `calibration/README.md`). Other voices of the same engine can be ±20% off.

use crate::kokoro::{TtsEngine, ENV_TTS};
use crate::transcribe::guess_language;

/// Narration language as the engines pick it: kana means Japanese, other CJK Chinese,
/// anything else English.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpeechLang {
    Zh,
    Ja,
    En,
}

impl SpeechLang {
    /// The language a speech engine reads `text` in (same rule as the voice choice).
    pub fn of(text: &str) -> Self {
        match guess_language(text) {
            Some("ja") => Self::Ja,
            Some("zh") | Some("ko") => Self::Zh,
            _ => Self::En,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::Zh => "zh",
            Self::Ja => "ja",
            Self::En => "en",
        }
    }
}

impl TtsEngine {
    /// The engine a render uses when not told: `$VIMMER_TTS` (`say` or `kokoro`), else Kokoro.
    /// An unknown value means Kokoro, as in the render. Scenes that name a `say` voice are still
    /// spoken by `say` (see [`crate::engine_for_voice`]).
    pub fn from_env() -> Self {
        std::env::var(ENV_TTS)
            .ok()
            .and_then(|v| Self::parse(&v))
            .unwrap_or(Self::Kokoro)
    }
}

/// Character-class counts of a text (see the module docs).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpeechFeatures {
    pub han: u32,
    pub kana: u32,
    pub letters: u32,
    pub words: u32,
    pub number: u32,
    pub minor: u32,
    pub major: u32,
}

/// Cost of each feature in ms, plus the per-clip `base`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeechRates {
    pub han: u32,
    pub kana: u32,
    pub letters: u32,
    pub words: u32,
    pub number: u32,
    pub minor: u32,
    pub major: u32,
    pub base: u32,
}

fn is_han(c: char) -> bool {
    matches!(c, '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}' | '々')
}

fn is_kana(c: char) -> bool {
    matches!(c, '\u{3041}'..='\u{3096}' | '\u{30A1}'..='\u{30FA}' | 'ー')
}

fn is_major(c: char) -> bool {
    matches!(c, '.' | '!' | '?' | '。' | '！' | '？' | '…' | '\n')
}

fn is_minor(c: char) -> bool {
    matches!(c, ',' | ';' | ':' | '、' | '，' | '；' | '：' | '—' | '–')
}

/// Japanese mora count of each digit read alone (ぜろ, いち, に, さん, よん, ご, ろく, なな, はち, きゅう).
const JA_DIGIT_MORAS: [u32; 10] = [2, 2, 1, 2, 2, 1, 2, 2, 2, 2];

/// Spoken length of one ASCII number: its integer digits (no separators), the digits after the
/// decimal point (empty when none), and whether a `年` follows. The unit is the language's
/// natural counting unit: moras for ja, syllables (characters) for zh, words for en.
///
/// Integers are read by place value (`3776` = 三千七百七十六 / さんぜんななひゃくななじゅうろく /
/// "three thousand seven hundred seventy-six"): a digit, then its place word 十百千, with 万億兆
/// every four digits (en: thousand, million, ... every three); zero digits are silent, except
/// that zh says 零 once for a gap of zeros between non-zero digits. A leading 1 is dropped before
/// 十百千 in ja and before a leading 十 in zh. Digit by digit instead: numbers with a leading
/// zero or more than 16 digits (ids, phone numbers), zh years (`2024年`), and everything after
/// the decimal point. English years 1100-1999 and 2010-2099 are read as two pairs (twenty
/// twenty-four).
pub fn number_units(int: &str, frac: &str, year: bool, lang: SpeechLang) -> u32 {
    let d: Vec<u32> = int.bytes().map(|b| u32::from(b - b'0')).collect();
    let single = |x: u32| match lang {
        SpeechLang::Ja => JA_DIGIT_MORAS[x as usize],
        _ => 1,
    };
    let by_digit = |ds: &[u32]| ds.iter().map(|&x| single(x)).sum::<u32>();
    let n = d.len();
    let value: u64 = if n <= 16 { int.parse().unwrap_or(0) } else { 0 };
    let int_units = if n > 16 || (n > 1 && d[0] == 0) || (lang == SpeechLang::Zh && year && n == 4)
    {
        by_digit(&d)
    } else if lang == SpeechLang::En {
        let words = |v: u64| -> u32 {
            // 0..=99
            match v {
                0 => 0,
                1..=19 => 1,
                _ => 1 + u32::from(!v.is_multiple_of(10)),
            }
        };
        let below_1000 = |v: u64| -> u32 { (if v >= 100 { 2 } else { 0 }) + words(v % 100) };
        if (1100..=1999).contains(&value) || (2010..=2099).contains(&value) {
            let pair = |v: u64| {
                if v == 0 {
                    1
                } else if v < 10 {
                    2
                } else {
                    words(v)
                }
            };
            pair(value / 100) + pair(value % 100)
        } else if value == 0 {
            1
        } else {
            let mut total = 0;
            let mut rest = value;
            let mut scale = 0;
            while rest > 0 {
                let g = rest % 1000;
                if g > 0 {
                    total += below_1000(g) + u32::from(scale > 0);
                }
                rest /= 1000;
                scale += 1;
            }
            total
        }
    } else if value == 0 {
        single(0)
    } else {
        // zh / ja place-value reading.
        let mut total = 0;
        let mut pending_zero = false;
        let mut started = false;
        let mut group_nonzero = false;
        for (i, &x) in d.iter().enumerate() {
            let r = n - 1 - i; // digits to the right
            let (k, g) = (r % 4, r / 4);
            if x == 0 {
                pending_zero = started;
            } else {
                if lang == SpeechLang::Zh && pending_zero {
                    total += 1; // 零
                }
                pending_zero = false;
                started = true;
                group_nonzero = true;
                let drop_one = x == 1
                    && match lang {
                        SpeechLang::Ja => k > 0,
                        _ => k == 1 && i == 0,
                    };
                if !drop_one {
                    total += single(x);
                }
                if k > 0 {
                    // 十 百 千: じゅう ひゃく せん are two moras each.
                    total += if lang == SpeechLang::Ja { 2 } else { 1 };
                }
            }
            if k == 0 {
                if g > 0 && group_nonzero {
                    total += if lang == SpeechLang::Ja { 2 } else { 1 }; // 万 億 兆
                }
                group_nonzero = false;
            }
        }
        total
    };
    let point = if frac.is_empty() {
        0
    } else {
        // 点 / てん / "point", then the digits one by one.
        let fd: Vec<u32> = frac.bytes().map(|b| u32::from(b - b'0')).collect();
        let point = if lang == SpeechLang::Ja { 2 } else { 1 };
        point + by_digit(&fd)
    };
    int_units + point
}

/// Counts the features of `text` read in `lang` (only numbers depend on it). Must match
/// `features()` in `calibration/fit.py`.
pub fn speech_features(text: &str, lang: SpeechLang) -> SpeechFeatures {
    let mut f = SpeechFeatures::default();
    let chars: Vec<char> = text.chars().collect();
    let mut in_word = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let at = i;
        i += 1;
        let prev = at.checked_sub(1).map(|p| chars[p]);
        let next = chars.get(at + 1).copied();
        let between_digits =
            prev.is_some_and(|p| p.is_ascii_digit()) && next.is_some_and(|n| n.is_ascii_digit());
        if c.is_ascii_alphabetic() {
            f.letters += 1;
            if !in_word {
                f.words += 1;
            }
            in_word = true;
            continue;
        }
        if c == '\'' && in_word {
            continue;
        }
        in_word = false;
        if c.is_ascii_digit() {
            // One number: digits, thousands groups (`8,848`) and one decimal part (`.86`).
            let mut int = String::new();
            let mut frac = String::new();
            let mut j = at;
            let digits_at = |j: usize| chars.get(j).is_some_and(|c| c.is_ascii_digit());
            while digits_at(j) {
                int.push(chars[j]);
                j += 1;
                if chars.get(j) == Some(&',')
                    && (1..=3).all(|k| digits_at(j + k))
                    && !digits_at(j + 4)
                {
                    j += 1;
                }
            }
            if chars.get(j) == Some(&'.') && digits_at(j + 1) {
                j += 1;
                while digits_at(j) {
                    frac.push(chars[j]);
                    j += 1;
                }
            }
            f.number += number_units(&int, &frac, chars.get(j) == Some(&'年'), lang);
            // "$5" is said "five dollars", "5%" "five percent" / 百分之五 / ごパーセント.
            if lang == SpeechLang::En && at > 0 && chars[at - 1] == '$' {
                f.number += 1;
            }
            if chars.get(j) == Some(&'%') {
                f.number += match lang {
                    SpeechLang::En => 1,
                    SpeechLang::Zh => 3,
                    SpeechLang::Ja => 5,
                };
            }
            i = j;
        } else if is_han(c) {
            f.han += 1;
        } else if is_kana(c) {
            f.kana += 1;
        } else if is_major(c) {
            if !(c == '.' && between_digits) && !prev.is_some_and(is_major) {
                f.major += 1;
            }
        } else if is_minor(c) && !(c == ',' && between_digits) {
            f.minor += 1;
        }
    }
    f
}

/// Fitted rates per engine and language (see `calibration/`).
pub fn speech_rates(engine: TtsEngine, lang: SpeechLang) -> SpeechRates {
    use SpeechLang::*;
    use TtsEngine::*;
    let r = |han, kana, letters, words, number, minor, major, base| SpeechRates {
        han,
        kana,
        letters,
        words,
        number,
        minor,
        major,
        base,
    };
    // FITTED-RATES-BEGIN (from calibration/fit.py; r(han, kana, letters, words, number, minor, major, base))
    match (engine, lang) {
        (Say, Zh) => r(212, 0, 207, 0, 224, 444, 117, 0),
        (Say, Ja) => r(273, 123, 109, 0, 152, 152, 0, 169),
        (Say, En) => r(0, 0, 40, 82, 310, 335, 106, 143),
        (Kokoro, Zh) => r(193, 0, 0, 482, 195, 122, 207, 354),
        (Kokoro, Ja) => r(256, 117, 0, 400, 140, 18, 53, 211),
        (Kokoro, En) => r(0, 0, 40, 93, 325, 104, 68, 326),
    }
    // FITTED-RATES-END
}

/// Estimated length in ms of the narration file `engine` makes from `text` read in `lang`,
/// as the render measures it (silence included, without the narration step's tail).
/// Empty or blank text is 0.
pub fn estimate_speech_ms(text: &str, lang: SpeechLang, engine: TtsEngine) -> u64 {
    if text.trim().is_empty() {
        return 0;
    }
    let f = speech_features(text, lang);
    let r = speech_rates(engine, lang);
    [
        (f.han, r.han),
        (f.kana, r.kana),
        (f.letters, r.letters),
        (f.words, r.words),
        (f.number, r.number),
        (f.minor, r.minor),
        (f.major, r.major),
        (1, r.base),
    ]
    .iter()
    .map(|&(n, ms)| u64::from(n) * u64::from(ms))
    .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn features_count_character_classes() {
        let f = speech_features("用AI做视频，2024年！真的吗？！", SpeechLang::Zh);
        assert_eq!(
            f,
            SpeechFeatures {
                han: 8,
                kana: 0,
                letters: 2,
                words: 1,
                number: 4,
                minor: 1,
                major: 2
            }
        );
        let f = speech_features("こんにちは、東京スカイツリー。", SpeechLang::Ja);
        assert_eq!((f.han, f.kana, f.minor, f.major), (2, 11, 1, 1));
        let f = speech_features("It's 8,848.86 meters... really.", SpeechLang::En);
        assert_eq!(
            (f.words, f.letters, f.number, f.minor, f.major),
            (3, 15, 9, 0, 2)
        );
    }

    #[test]
    fn numbers_are_counted_as_they_are_read() {
        let u = |n: &str, l| number_units(n, "", false, l);
        // さんぜんななひゃくななじゅうろく: 7 digit/place pairs of 2 moras each
        assert_eq!(u("3776", SpeechLang::Ja), 14);
        // 三千七百七十六
        assert_eq!(u("3776", SpeechLang::Zh), 7);
        assert_eq!(u("3776", SpeechLang::En), 6); // three thousand seven hundred seventy six
                                                  // 一千零五; ja drops the leading いち: せんご
        assert_eq!(u("1005", SpeechLang::Zh), 4);
        assert_eq!(u("1005", SpeechLang::Ja), 3);
        assert_eq!(u("12", SpeechLang::Zh), 2); // 十二
        assert_eq!(u("20000", SpeechLang::Zh), 2); // 两万
        assert_eq!(u("007", SpeechLang::Zh), 3); // digit by digit
        assert_eq!(number_units("2024", "", true, SpeechLang::Zh), 4); // 二零二四年
        assert_eq!(number_units("2024", "", true, SpeechLang::En), 3); // twenty twenty-four
        assert_eq!(number_units("3", "14", false, SpeechLang::En), 4); // three point one four
                                                                       // Longer numbers never read shorter than their shorter prefix of the same magnitude.
        assert!(u("1000000", SpeechLang::En) < u("1234567", SpeechLang::En));
    }

    #[test]
    fn language_follows_the_text() {
        assert_eq!(SpeechLang::of("今日はいい天気"), SpeechLang::Ja);
        assert_eq!(SpeechLang::of("西安钟楼"), SpeechLang::Zh);
        assert_eq!(SpeechLang::of("Hello 123"), SpeechLang::En);
        assert_eq!(SpeechLang::of("123"), SpeechLang::En);
    }

    #[test]
    fn estimate_is_the_weighted_sum_and_blank_text_is_free() {
        let r = speech_rates(TtsEngine::Say, SpeechLang::Zh);
        assert_eq!(
            estimate_speech_ms("西安钟楼，始建于明代。", SpeechLang::Zh, TtsEngine::Say),
            u64::from(9 * r.han + r.minor + r.major + r.base)
        );
        assert_eq!(
            estimate_speech_ms("  ", SpeechLang::En, TtsEngine::Kokoro),
            0
        );
        // Longer text never gets a shorter estimate.
        for engine in [TtsEngine::Say, TtsEngine::Kokoro] {
            let short = estimate_speech_ms("The fox jumps.", SpeechLang::En, engine);
            let long = estimate_speech_ms("The quick brown fox jumps.", SpeechLang::En, engine);
            assert!(long > short);
        }
    }

    /// Re-scores the committed calibration measurements: catches rates that were not updated
    /// after `calibration/measured.csv` was re-measured, and a `speech_features` that drifted
    /// from `calibration/fit.py`. Bounds: mean absolute error per engine x language.
    #[test]
    fn calibration_corpus_error() {
        let csv = include_str!("../calibration/measured.csv");
        let mut errs: std::collections::BTreeMap<(String, String), Vec<f64>> = Default::default();
        for line in csv.lines().skip(1) {
            let mut cols = line.splitn(6, ',');
            let (_id, lang, engine, _voice, ms) = (
                cols.next().unwrap(),
                cols.next().unwrap(),
                cols.next().unwrap(),
                cols.next().unwrap(),
                cols.next().unwrap().parse::<f64>().unwrap(),
            );
            let text = cols.next().unwrap();
            let text = text[1..text.len() - 1].replace("\"\"", "\"");
            let lang_v = match lang {
                "zh" => SpeechLang::Zh,
                "ja" => SpeechLang::Ja,
                _ => SpeechLang::En,
            };
            let engine_v = TtsEngine::parse(engine).unwrap();
            let est = estimate_speech_ms(&text, lang_v, engine_v) as f64;
            errs.entry((engine.into(), lang.into()))
                .or_default()
                .push((est - ms).abs() / ms);
        }
        assert_eq!(errs.len(), 6, "every engine x language is measured");
        for ((engine, lang), e) in errs {
            let mean = e.iter().sum::<f64>() / e.len() as f64;
            let bound = if engine == "say" { 0.06 } else { 0.08 };
            assert!(
                mean <= bound,
                "{engine} {lang}: mean error {:.1}%",
                mean * 100.0
            );
        }
    }
}
