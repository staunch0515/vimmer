//! Minimal SubRip (.srt) parser, so timed text from TTS or external tools can become cues.

use thiserror::Error;

use crate::model::SubtitleCue;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SrtError {
    #[error("cue {cue}: missing `-->` timing line")]
    MissingTiming { cue: usize },
    #[error("cue {cue}: invalid timestamp `{value}`")]
    BadTimestamp { cue: usize, value: String },
    #[error("cue {cue}: no text")]
    EmptyText { cue: usize },
}

/// Parses SubRip text. Accepts `,` or `.` as the millisecond separator, CRLF line endings,
/// a leading BOM, and optional numeric cue indices.
pub fn parse_srt(input: &str) -> Result<Vec<SubtitleCue>, SrtError> {
    let input = input.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let mut cues = Vec::new();

    for (n, block) in input
        .split("\n\n")
        .filter(|b| !b.trim().is_empty())
        .enumerate()
    {
        let number = n + 1;
        let lines: Vec<&str> = block
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        let timing_at = lines
            .iter()
            .position(|l| l.contains("-->"))
            .ok_or(SrtError::MissingTiming { cue: number })?;
        let (start, end) = parse_timing(lines[timing_at], number)?;
        let text = lines[timing_at + 1..].join(" ");
        if text.trim().is_empty() {
            return Err(SrtError::EmptyText { cue: number });
        }
        cues.push(SubtitleCue::new(start, end, text));
    }
    Ok(cues)
}

fn parse_timing(line: &str, cue: usize) -> Result<(u64, u64), SrtError> {
    let (a, b) = line
        .split_once("-->")
        .ok_or(SrtError::MissingTiming { cue })?;
    Ok((
        parse_timestamp(a.trim(), cue)?,
        parse_timestamp(b.trim(), cue)?,
    ))
}

fn parse_timestamp(value: &str, cue: usize) -> Result<u64, SrtError> {
    let bad = || SrtError::BadTimestamp {
        cue,
        value: value.to_string(),
    };
    let (hms, ms) = value.split_once([',', '.']).ok_or_else(bad)?;
    let mut parts = hms.split(':');
    let h: u64 = parts.next().and_then(|v| v.parse().ok()).ok_or_else(bad)?;
    let m: u64 = parts.next().and_then(|v| v.parse().ok()).ok_or_else(bad)?;
    let s: u64 = parts.next().and_then(|v| v.parse().ok()).ok_or_else(bad)?;
    if parts.next().is_some() || m > 59 || s > 59 || ms.len() != 3 {
        return Err(bad());
    }
    let millis: u64 = ms.parse().map_err(|_| bad())?;
    Ok(((h * 60 + m) * 60 + s) * 1000 + millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_cues() {
        let srt = "1\n00:00:01,000 --> 00:00:03,500\nHello there\n\n2\n00:00:03,500 --> 00:00:05,000\nLine one\nline two\n";
        let cues = parse_srt(srt).unwrap();
        assert_eq!(cues.len(), 2);
        assert_eq!(
            cues[0],
            SubtitleCue {
                start_ms: 1000,
                end_ms: 3500,
                text: "Hello there".into(),
                words: Vec::new(),
            }
        );
        assert_eq!(cues[1].text, "Line one line two");
    }

    #[test]
    fn accepts_crlf_bom_and_dot_separator() {
        let srt = "\u{feff}1\r\n01:02:03.004 --> 01:02:04.500\r\nHi\r\n";
        let cues = parse_srt(srt).unwrap();
        assert_eq!(cues[0].start_ms, 3_723_004);
        assert_eq!(cues[0].end_ms, 3_724_500);
    }

    #[test]
    fn reports_bad_input() {
        assert_eq!(
            parse_srt("1\nnot a timing\nHi"),
            Err(SrtError::MissingTiming { cue: 1 })
        );
        assert!(matches!(
            parse_srt("1\n00:00:01 --> 00:00:02,000\nHi"),
            Err(SrtError::BadTimestamp { cue: 1, .. })
        ));
        assert_eq!(
            parse_srt("1\n00:00:01,000 --> 00:00:02,000\n"),
            Err(SrtError::EmptyText { cue: 1 })
        );
    }
}
