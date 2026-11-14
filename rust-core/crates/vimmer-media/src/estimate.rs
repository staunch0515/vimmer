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
//! | `digits`  | ASCII digits (numbers are read out, so they cost more than one letter)  |
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
    /// The engine a render uses when not told: `$VIMMER_TTS` (`say` or `kokoro`), else `say`.
    /// An unknown value means `say`, as in the render.
    pub fn from_env() -> Self {
        std::env::var(ENV_TTS)
            .ok()
            .and_then(|v| Self::parse(&v))
            .unwrap_or(Self::Say)
    }
}

/// Character-class counts of a text (see the module docs).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpeechFeatures {
    pub han: u32,
    pub kana: u32,
    pub letters: u32,
    pub words: u32,
    pub digits: u32,
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
    pub digits: u32,
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

/// Counts the features of `text`. Must match `features()` in `calibration/fit.py`.
pub fn speech_features(text: &str) -> SpeechFeatures {
    let mut f = SpeechFeatures::default();
    let chars: Vec<char> = text.chars().collect();
    let mut in_word = false;
    for (i, &c) in chars.iter().enumerate() {
        let prev = i.checked_sub(1).map(|p| chars[p]);
        let next = chars.get(i + 1).copied();
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
            f.digits += 1;
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
    let r = |han, kana, letters, words, digits, minor, major, base| SpeechRates {
        han,
        kana,
        letters,
        words,
        digits,
        minor,
        major,
        base,
    };
    // FITTED-RATES-BEGIN (from calibration/fit.py; r(han, kana, letters, words, digits, minor, major, base))
    match (engine, lang) {
        (Say, Zh) => r(212, 0, 207, 0, 352, 427, 118, 0),
        (Say, Ja) => r(270, 125, 107, 10, 348, 133, 0, 167),
        (Say, En) => r(0, 0, 37, 98, 388, 313, 102, 151),
        (Kokoro, Zh) => r(191, 0, 0, 490, 294, 131, 217, 344),
        (Kokoro, Ja) => r(261, 113, 0, 402, 303, 0, 33, 259),
        (Kokoro, En) => r(0, 0, 41, 90, 402, 96, 65, 335),
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
    let f = speech_features(text);
    let r = speech_rates(engine, lang);
    [
        (f.han, r.han),
        (f.kana, r.kana),
        (f.letters, r.letters),
        (f.words, r.words),
        (f.digits, r.digits),
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
        let f = speech_features("用AI做视频，2024年！真的吗？！");
        assert_eq!(
            f,
            SpeechFeatures {
                han: 8,
                kana: 0,
                letters: 2,
                words: 1,
                digits: 4,
                minor: 1,
                major: 2
            }
        );
        let f = speech_features("こんにちは、東京スカイツリー。");
        assert_eq!((f.han, f.kana, f.minor, f.major), (2, 11, 1, 1));
        let f = speech_features("It's 8,848.86 meters... really.");
        assert_eq!(
            (f.words, f.letters, f.digits, f.minor, f.major),
            (3, 15, 6, 0, 2)
        );
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
