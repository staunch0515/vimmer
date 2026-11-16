//! Recording (录音): saving what the editor captured from the microphone.
//!
//! Capture happens in the webview (`getUserMedia` + Web Audio, see the editor's useRecorder.ts):
//! WKWebView asks macOS for the microphone the first time, using `NSMicrophoneUsageDescription`
//! from `Info.plist` (Tauri embeds that file into the binary in `tauri dev`, and into the bundle
//! when building the app), and wry grants WebKit's own media-capture request. This command only
//! stores the take: a WAV, 48 kHz mono 16-bit, under `assets/recordings/<scene>-<n>.wav` of the
//! open project (or of the public folder without one). A take in another WAV format is converted
//! with ffmpeg first.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::render::{find_in_path, user_path};
use crate::studio::{blocking, studio};

/// Where takes go, relative to the project (or public) folder.
pub const RECORDINGS_DIR: &str = "assets/recordings";
/// Format every saved take has.
pub const RATE: u32 = 48_000;
/// Largest take accepted: 200 s of 48 kHz stereo 32-bit, with room to spare.
pub const MAX_BYTES: usize = 80 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveArgs {
    /// The open project's folder; `None`: the public folder.
    pub dir: Option<String>,
    /// Scene id or `voiceover`: the start of the file name.
    pub stem: String,
    /// The WAV file, base64.
    pub wav_base64: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedRecording {
    /// The path the timeline names the file by (`assets/recordings/intro-1.wav`).
    pub path: String,
    /// Absolute path.
    pub file: String,
    pub duration_ms: u64,
    /// The take was converted to 48 kHz mono.
    pub converted: bool,
}

/// The format of a PCM WAV file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavFormat {
    pub format: u16,
    pub channels: u16,
    pub rate: u32,
    pub bits: u16,
    pub data_bytes: u32,
}

impl WavFormat {
    pub fn duration_ms(&self) -> u64 {
        let frame = u64::from(self.channels) * u64::from(self.bits / 8);
        if frame == 0 || self.rate == 0 {
            return 0;
        }
        u64::from(self.data_bytes) / frame * 1000 / u64::from(self.rate)
    }

    /// Exactly what the render and whisper want: PCM, mono, 48 kHz, 16-bit.
    pub fn is_target(&self) -> bool {
        self.format == 1 && self.channels == 1 && self.rate == RATE && self.bits == 16
    }
}

/// Reads the `fmt ` and `data` chunks of a WAV file.
pub fn wav_format(bytes: &[u8]) -> Result<WavFormat, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("The recording is not a WAV file.".into());
    }
    let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    let mut at = 12;
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32_at(at + 4) as usize;
        let body = at + 8;
        if id == b"fmt " && body + 16 <= bytes.len() {
            fmt = Some((u16_at(body), u16_at(body + 2), u32_at(body + 4), u16_at(body + 14)));
        } else if id == b"data" {
            let (format, channels, rate, bits) = fmt.ok_or("The WAV file has no format chunk.")?;
            let data_bytes = size.min(bytes.len() - body) as u32;
            return Ok(WavFormat { format, channels, rate, bits, data_bytes });
        }
        at = body + size + (size & 1);
    }
    Err("The WAV file has no audio data.".into())
}

/// A file-name stem: ASCII letters, digits, `-` and `_`; anything else becomes `-`.
pub fn safe_stem(stem: &str) -> String {
    let s: String = stem
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    let s = s.trim_matches('-');
    let s: String = s.chars().take(40).collect();
    if s.is_empty() {
        "take".into()
    } else {
        s
    }
}

/// The first free `<stem>-<n>.wav` in `folder`, n from 1.
pub fn next_name(folder: &Path, stem: &str) -> String {
    (1..)
        .map(|n| format!("{stem}-{n}.wav"))
        .find(|name| !folder.join(name).exists())
        .expect("a free name")
}

/// Decodes standard base64 (with or without padding; whitespace ignored).
pub fn decode_base64(text: &str) -> Result<Vec<u8>, String> {
    let value = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let clean: Vec<u8> = text.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=').collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.chunks(4) {
        let mut acc = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            acc |= value(c).ok_or("The recording data is damaged (not base64).")? << (18 - 6 * i);
        }
        let n = chunk.len();
        if n == 1 {
            return Err("The recording data is damaged (base64 length).".into());
        }
        out.push((acc >> 16) as u8);
        if n > 2 {
            out.push((acc >> 8) as u8);
        }
        if n > 3 {
            out.push(acc as u8);
        }
    }
    Ok(out)
}

/// Stores a take in `<base>/assets/recordings/`. A WAV that is not 48 kHz mono 16-bit is
/// converted with `ffmpeg`. Written to a temporary name first, so a half-written file never
/// carries the final name.
pub fn save_into(base: &Path, stem: &str, wav: &[u8], ffmpeg: &Path) -> Result<SavedRecording, String> {
    if wav.len() > MAX_BYTES {
        return Err("The recording is too long to save (more than 200 seconds).".into());
    }
    let format = wav_format(wav)?;
    if format.data_bytes == 0 {
        return Err("The recording is empty.".into());
    }
    let folder = base.join(RECORDINGS_DIR);
    fs::create_dir_all(&folder).map_err(|e| format!("Could not create {}: {e}", folder.display()))?;
    let name = next_name(&folder, &safe_stem(stem));
    let target = folder.join(&name);
    let part = folder.join(format!(".{name}.part"));
    let converted = !format.is_target();
    if converted {
        let raw = folder.join(format!(".{name}.in.wav"));
        fs::write(&raw, wav).map_err(|e| format!("Could not write {}: {e}", raw.display()))?;
        let out = Command::new(ffmpeg)
            .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
            .arg(&raw)
            .args(["-ac", "1", "-ar", "48000", "-c:a", "pcm_s16le", "-f", "wav"])
            .arg(&part)
            .output();
        let _ = fs::remove_file(&raw);
        let out = out.map_err(|e| format!("ffmpeg could not run to convert the recording: {e}"))?;
        if !out.status.success() {
            let _ = fs::remove_file(&part);
            return Err(format!(
                "ffmpeg could not convert the recording: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    } else {
        fs::write(&part, wav).map_err(|e| format!("Could not write {}: {e}", part.display()))?;
    }
    fs::rename(&part, &target).map_err(|e| format!("Could not save {}: {e}", target.display()))?;
    let duration_ms = fs::read(&target)
        .ok()
        .and_then(|b| wav_format(&b).ok())
        .map_or(format.duration_ms(), |f| f.duration_ms());
    Ok(SavedRecording {
        path: format!("{RECORDINGS_DIR}/{name}"),
        file: target.display().to_string(),
        duration_ms,
        converted,
    })
}

/// Saves a take into the open project (checked to be one of the studio's projects), or into the
/// public folder without one.
#[tauri::command]
pub async fn save_recording(app: AppHandle, args: SaveArgs) -> Result<SavedRecording, String> {
    let base: PathBuf = match &args.dir {
        Some(dir) => {
            let studio = studio(&app)?;
            let dir = dir.clone();
            blocking(move || studio.project(&dir)).await?
        }
        None => crate::assets::public_dir(),
    };
    let ffmpeg = find_in_path("ffmpeg", &user_path());
    blocking(move || {
        let wav = decode_base64(&args.wav_base64)?;
        save_into(&base, &args.stem, &wav, &ffmpeg)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 16-bit PCM WAV of `n` samples per channel.
    fn wav(rate: u32, channels: u16, n: usize) -> Vec<u8> {
        let data = n * usize::from(channels) * 2;
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data as u32).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&channels.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
        b.extend_from_slice(&(channels * 2).to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(data as u32).to_le_bytes());
        for i in 0..n * usize::from(channels) {
            let s = ((i as f64 * 0.05).sin() * 8000.0) as i16;
            b.extend_from_slice(&s.to_le_bytes());
        }
        b
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vimmer-rec-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn encode(bytes: &[u8]) -> String {
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut s = String::new();
        for c in bytes.chunks(3) {
            let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
            for i in 0..4 {
                if i <= c.len() {
                    s.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
                } else {
                    s.push('=');
                }
            }
        }
        s
    }

    #[test]
    fn base64_round_trips() {
        for len in [0, 1, 2, 3, 4, 5, 100, 1001] {
            let bytes: Vec<u8> = (0..len).map(|i| (i * 37 % 256) as u8).collect();
            assert_eq!(decode_base64(&encode(&bytes)).unwrap(), bytes, "{len}");
        }
        assert!(decode_base64("ab$d").is_err());
        assert!(decode_base64("abcde").is_err());
    }

    #[test]
    fn wav_headers_are_read() {
        let f = wav_format(&wav(48_000, 1, 48_000)).unwrap();
        assert!(f.is_target());
        assert_eq!(f.duration_ms(), 1000);
        let f = wav_format(&wav(44_100, 2, 22_050)).unwrap();
        assert!(!f.is_target());
        assert_eq!((f.rate, f.channels, f.duration_ms()), (44_100, 2, 500));
        assert!(wav_format(b"not a wav").unwrap_err().contains("not a WAV"));
        assert!(wav_format(&wav(48_000, 1, 10)[..36]).unwrap_err().contains("no audio data"));
    }

    #[test]
    fn stems_are_safe_and_names_count_up() {
        assert_eq!(safe_stem("intro"), "intro");
        assert_eq!(safe_stem("../../etc/passwd"), "etc-passwd");
        assert_eq!(safe_stem("场景一"), "take");
        assert_eq!(safe_stem("scene 03"), "scene-03");
        let dir = temp("names");
        assert_eq!(next_name(&dir, "a"), "a-1.wav");
        fs::write(dir.join("a-1.wav"), b"x").unwrap();
        assert_eq!(next_name(&dir, "a"), "a-2.wav");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_target_take_is_stored_as_is_under_recordings() {
        let base = temp("store");
        let take = wav(48_000, 1, 24_000);
        let saved = save_into(&base, "intro", &take, Path::new("ffmpeg-not-needed")).unwrap();
        assert_eq!(saved.path, "assets/recordings/intro-1.wav");
        assert_eq!(saved.duration_ms, 500);
        assert!(!saved.converted);
        assert_eq!(fs::read(base.join(&saved.path)).unwrap(), take);
        let again = save_into(&base, "intro", &take, Path::new("ffmpeg-not-needed")).unwrap();
        assert_eq!(again.path, "assets/recordings/intro-2.wav", "a re-record never overwrites a take");
        // Nothing temporary is left behind.
        let names: Vec<String> = fs::read_dir(base.join(RECORDINGS_DIR))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.iter().all(|n| !n.starts_with('.')), "{names:?}");
        assert!(save_into(&base, "x", b"RIFF....WAVE", Path::new("ffmpeg")).is_err());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn another_format_is_converted_with_ffmpeg() {
        let ffmpeg = find_in_path("ffmpeg", &user_path());
        if Command::new(&ffmpeg).arg("-version").output().is_err() {
            eprintln!("ffmpeg not found; skipped");
            return;
        }
        let base = temp("convert");
        let saved = save_into(&base, "vo", &wav(44_100, 2, 44_100), &ffmpeg).unwrap();
        assert!(saved.converted);
        let f = wav_format(&fs::read(base.join(&saved.path)).unwrap()).unwrap();
        assert!(f.is_target(), "{f:?}");
        assert!((990..=1010).contains(&saved.duration_ms), "{}", saved.duration_ms);
        // A failing converter leaves nothing.
        let err = save_into(&base, "bad", &wav(44_100, 2, 100), Path::new("/nonexistent/ffmpeg")).unwrap_err();
        assert!(err.contains("ffmpeg could not run"), "{err}");
        assert!(!base.join(RECORDINGS_DIR).join("bad-1.wav").exists());
        let _ = fs::remove_dir_all(&base);
    }
}
