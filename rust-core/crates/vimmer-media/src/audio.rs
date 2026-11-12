use std::path::{Path, PathBuf};

use crate::MediaError;

/// Reports the duration of an audio asset. Abstracted so the scheduler can be tested
/// without ffmpeg installed.
pub trait AudioProbe: Send + Sync {
    fn duration_ms(&self, path: &Path) -> Result<u64, MediaError>;
}

/// Probes with the `ffmpeg` binary: `ffmpeg -i <file>` prints `Duration:` to stderr
/// and exits non-zero when no output is given, which is expected here.
#[derive(Debug, Clone)]
pub struct FfmpegProbe {
    pub binary: PathBuf,
}

impl Default for FfmpegProbe {
    fn default() -> Self {
        Self {
            binary: PathBuf::from("ffmpeg"),
        }
    }
}

impl AudioProbe for FfmpegProbe {
    fn duration_ms(&self, path: &Path) -> Result<u64, MediaError> {
        let output = std::process::Command::new(&self.binary)
            .args(["-hide_banner", "-nostdin", "-i"])
            .arg(path)
            .output()?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        parse_ffmpeg_duration_ms(&stderr).ok_or_else(|| MediaError::Probe {
            path: path.display().to_string(),
            reason: "no Duration line in ffmpeg output".into(),
        })
    }
}

/// True when the file has at least one audio stream, judged from ffmpeg's stream listing.
pub fn has_audio_stream(ffmpeg: &Path, path: &Path) -> bool {
    std::process::Command::new(ffmpeg)
        .args(["-hide_banner", "-nostdin", "-i"])
        .arg(path)
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stderr)
                .lines()
                .any(|l| l.contains("Stream #") && l.contains("Audio:"))
        })
        .unwrap_or(false)
}

/// Extracts `Duration: HH:MM:SS.ff` from ffmpeg's banner and returns milliseconds.
pub fn parse_ffmpeg_duration_ms(text: &str) -> Option<u64> {
    let rest = text.split("Duration:").nth(1)?.trim_start();
    let token = rest.split(',').next()?.trim();
    if token == "N/A" {
        return None;
    }
    let mut parts = token.split(':');
    let hours: u64 = parts.next()?.parse().ok()?;
    let minutes: u64 = parts.next()?.parse().ok()?;
    let seconds: f64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    let total = (hours * 3600 + minutes * 60) as f64 + seconds;
    Some((total * 1000.0).round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typical_banner() {
        let banner = "  Input #0, mp3, from 'a.mp3':\n  Duration: 00:01:02.50, start: 0.025000, bitrate: 128 kb/s";
        assert_eq!(parse_ffmpeg_duration_ms(banner), Some(62_500));
    }

    #[test]
    fn parses_hours_and_fraction() {
        assert_eq!(
            parse_ffmpeg_duration_ms("Duration: 01:00:00.04, start"),
            Some(3_600_040)
        );
    }

    #[test]
    fn rejects_missing_or_na() {
        assert_eq!(parse_ffmpeg_duration_ms("no duration here"), None);
        assert_eq!(parse_ffmpeg_duration_ms("Duration: N/A, bitrate"), None);
    }
}
