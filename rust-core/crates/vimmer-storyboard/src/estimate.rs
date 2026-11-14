use vimmer_media::{estimate_speech_ms, SpeechLang, TtsEngine};

use crate::model::Language;

/// Lower bound for an estimated scene length.
pub const MIN_ESTIMATED_MS: u64 = 2_000;
const PADDING_MS: u64 = 600;

/// Estimates how long a scene must be for `engine` to read its text aloud.
///
/// The speech length comes from the shared, measured estimator
/// ([`vimmer_media::estimate_speech_ms`]: per-engine, per-language costs for characters,
/// Latin words, digits and pauses, fitted on the default voices). Then 600 ms of padding is
/// added (the render's 300 ms narration tail plus 300 ms of slack) and the result is at least
/// [`MIN_ESTIMATED_MS`].
pub fn estimate_duration_ms(text: &str, lang: Language, engine: TtsEngine) -> u64 {
    let lang = match lang {
        Language::Zh => SpeechLang::Zh,
        Language::Ja => SpeechLang::Ja,
        Language::En => SpeechLang::En,
    };
    (estimate_speech_ms(text, lang, engine) + PADDING_MS).max(MIN_ESTIMATED_MS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_padding_to_the_shared_speech_estimate() {
        for (text, lang, speech_lang) in [
            (
                "西安钟楼，始建于明代，是中国现存最大的钟楼。",
                Language::Zh,
                SpeechLang::Zh,
            ),
            (
                "京都には千六百以上のお寺があります。",
                Language::Ja,
                SpeechLang::Ja,
            ),
            (
                "The quick brown fox jumps over the lazy dog.",
                Language::En,
                SpeechLang::En,
            ),
        ] {
            for engine in [TtsEngine::Say, TtsEngine::Kokoro] {
                let speech = estimate_speech_ms(text, speech_lang, engine);
                assert!(speech + PADDING_MS > MIN_ESTIMATED_MS, "{text}");
                assert_eq!(
                    estimate_duration_ms(text, lang, engine),
                    speech + PADDING_MS
                );
            }
        }
    }

    #[test]
    fn short_text_hits_the_floor() {
        assert_eq!(
            estimate_duration_ms("好", Language::Zh, TtsEngine::Say),
            MIN_ESTIMATED_MS
        );
        assert_eq!(
            estimate_duration_ms("a - b", Language::En, TtsEngine::Kokoro),
            MIN_ESTIMATED_MS
        );
    }

    #[test]
    fn the_storyboard_language_decides_how_text_is_read() {
        // Japanese written without kana is still estimated as Japanese (it gets `Kyoko`).
        let text = "東京都庁舎展望室";
        assert_eq!(
            estimate_duration_ms(text, Language::Ja, TtsEngine::Say),
            (estimate_speech_ms(text, SpeechLang::Ja, TtsEngine::Say) + PADDING_MS)
                .max(MIN_ESTIMATED_MS)
        );
    }
}
