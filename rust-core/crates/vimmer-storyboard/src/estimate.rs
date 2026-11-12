use crate::model::Language;

/// Lower bound for an estimated scene length.
pub const MIN_ESTIMATED_MS: u64 = 2_000;
const PADDING_MS: u64 = 600;

/// Estimates how long a scene's text takes to read aloud.
///
/// * `zh`: 230 ms per CJK character; `ja`: 200 ms per kana/kanji character.
///   Punctuation and whitespace are free. A run of ASCII letters/digits counts as 2 characters.
/// * `en`: 400 ms per word (about 150 words/minute); words are whitespace-separated
///   tokens containing at least one letter or digit.
///
/// Then 600 ms of padding is added and the result is at least [`MIN_ESTIMATED_MS`].
pub fn estimate_duration_ms(text: &str, lang: Language) -> u64 {
    let speech = match lang {
        Language::En => 400 * count_words(text),
        Language::Zh => 230 * count_cjk_units(text),
        Language::Ja => 200 * count_cjk_units(text),
    };
    (speech + PADDING_MS).max(MIN_ESTIMATED_MS)
}

fn count_words(text: &str) -> u64 {
    text.split_whitespace()
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .count() as u64
}

fn count_cjk_units(text: &str) -> u64 {
    let mut units = 0;
    let mut in_ascii_run = false;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            if !in_ascii_run {
                units += 2;
                in_ascii_run = true;
            }
        } else {
            in_ascii_run = false;
            if c.is_alphanumeric() {
                units += 1;
            }
        }
    }
    units
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_counts_characters_not_punctuation() {
        assert_eq!(estimate_duration_ms("西安钟楼，始建于明代。", Language::Zh), 230 * 9 + 600);
    }

    #[test]
    fn japanese_counts_kana_and_kanji() {
        assert_eq!(estimate_duration_ms("こんにちは、世界。", Language::Ja), 200 * 7 + 600);
    }

    #[test]
    fn english_counts_words() {
        assert_eq!(
            estimate_duration_ms("The quick brown fox jumps over the lazy dog", Language::En),
            400 * 9 + 600
        );
        assert_eq!(estimate_duration_ms("a - b", Language::En), MIN_ESTIMATED_MS);
    }

    #[test]
    fn short_text_hits_the_floor() {
        assert_eq!(estimate_duration_ms("好", Language::Zh), MIN_ESTIMATED_MS);
    }

    #[test]
    fn ascii_run_in_cjk_text_counts_as_two() {
        assert_eq!(count_cjk_units("用AI做视频"), 1 + 2 + 3);
    }
}
