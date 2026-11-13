//! Narration: turn text into a WAV file with a local speech engine.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::MediaError;

/// Synthesizes speech to a stereo 48 kHz WAV file.
pub trait Speech: Send + Sync {
    fn synthesize(&self, text: &str, voice: &str, out_wav: &Path) -> Result<(), MediaError>;
}

/// The voice used when a scene does not name one. Any kana selects a Japanese voice; otherwise
/// any CJK ideograph selects a Chinese voice.
pub fn default_voice_for(text: &str) -> &'static str {
    let kana = text.chars().any(|c| ('\u{3040}'..='\u{30ff}').contains(&c));
    let cjk = text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c));
    if kana {
        "Kyoko"
    } else if cjk {
        "Tingting"
    } else {
        "Samantha"
    }
}

/// macOS `say` engine. Writes AIFF, then converts to WAV with ffmpeg.
#[derive(Debug, Clone)]
pub struct SaySpeech {
    pub say: PathBuf,
    pub ffmpeg: PathBuf,
}

impl Default for SaySpeech {
    fn default() -> Self {
        Self {
            say: PathBuf::from("say"),
            ffmpeg: PathBuf::from("ffmpeg"),
        }
    }
}

impl Speech for SaySpeech {
    fn synthesize(&self, text: &str, voice: &str, out_wav: &Path) -> Result<(), MediaError> {
        if let Some(dir) = out_wav.parent() {
            fs::create_dir_all(dir)?;
        }
        let aiff = out_wav.with_extension("aiff");
        // Text goes through a file, so no quoting or leading-dash problems reach the command line.
        let text_file = out_wav.with_extension("txt");
        fs::write(&text_file, text)?;

        let said = Command::new(&self.say)
            .arg("-v")
            .arg(voice)
            .arg("-f")
            .arg(&text_file)
            .arg("-o")
            .arg(&aiff)
            .output();
        let _ = fs::remove_file(&text_file);
        let said = said?;
        if !said.status.success() {
            return Err(MediaError::Speech(
                String::from_utf8_lossy(&said.stderr).trim().to_string(),
            ));
        }

        let converted = Command::new(&self.ffmpeg)
            .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
            .arg(&aiff)
            .args(["-ac", "2", "-ar", "48000"])
            .arg(out_wav)
            .output();
        let _ = fs::remove_file(&aiff);
        let converted = converted?;
        if !converted.status.success() {
            return Err(MediaError::Speech(
                String::from_utf8_lossy(&converted.stderr)
                    .trim()
                    .to_string(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_chinese_voice_for_cjk_text() {
        assert_eq!(default_voice_for("西安钟楼"), "Tingting");
        assert_eq!(default_voice_for("Hello world"), "Samantha");
        assert_eq!(default_voice_for("mixed 中文 text"), "Tingting");
        assert_eq!(default_voice_for("今日はいい天気"), "Kyoko");
    }
}
