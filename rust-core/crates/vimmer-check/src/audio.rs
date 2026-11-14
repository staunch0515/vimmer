//! Audio levels, measured with ffmpeg: sample peak, clipped samples and integrated loudness.
//!
//! Two ffmpeg runs per file:
//! 1. `ebur128` (EBU R128 / ITU-R BS.1770) for integrated loudness in LUFS, plus the stream
//!    listing for the duration and channel count.
//! 2. A decode to 16-bit PCM read straight from ffmpeg's stdout. A *clip* is a run of at least
//!    [`CLIP_RUN`] consecutive samples at full scale in one channel: a flat top that a clean
//!    full-scale wave (which touches full scale for one sample) does not produce.
//!
//! Thresholds for referenced files (`scene.audio`, `music`):
//! * any clip: warning `audio.clipping`
//! * sample peak at or above [`HOT_PEAK_DBFS`] (-0.1 dBFS) without clips: warning `audio.hot_peak`
//! * integrated loudness below [`QUIET_LUFS`] (-35 LUFS): warning `audio.quiet`; a file with no
//!   signal above -60 dBFS: warning `audio.silent`
//! * music below [`MUSIC_INAUDIBLE_LUFS`] after its volume: warning; above
//!   [`MUSIC_LOUD_LUFS`] after its volume while scenes have speech: warning (it can mask the voice).

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::json;
use vimmer_dsl::VideoTimeline;
use vimmer_media::{parse_ffmpeg_duration_ms, run_limited, Limits};

use crate::{is_url, scene_path, scene_starts_ms, Finding, Severity, Suggestion};

pub const CLIP_RUN: usize = 3;
pub const HOT_PEAK_DBFS: f64 = -0.1;
pub const QUIET_LUFS: f64 = -35.0;
pub const SILENT_PEAK_DBFS: f64 = -60.0;
pub const MUSIC_INAUDIBLE_LUFS: f64 = -45.0;
pub const MUSIC_LOUD_LUFS: f64 = -22.0;
/// Where music sits under speech after the suggested volume change.
pub const MUSIC_TARGET_LUFS: f64 = -30.0;
/// Pause after the clip before the scene may end (the narration step uses the same 300 ms).
pub const TAIL_MS: u64 = 300;
/// A cut shorter than this is a warning, longer an error.
pub const CUT_ERROR_MS: u64 = 250;

const ANALYZE_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, PartialEq)]
pub struct AudioStats {
    pub duration_ms: Option<u64>,
    pub channels: usize,
    /// Sample peak in dBFS; `-inf` for digital silence.
    pub peak_dbfs: f64,
    /// Number of clipped runs (see the module docs) and the samples in them.
    pub clip_events: u64,
    pub clipped_samples: u64,
    pub samples: u64,
    /// Integrated loudness; `None` when ffmpeg reports none (very short input).
    pub integrated_lufs: Option<f64>,
}

/// Parses `I: -23.0 LUFS` from ffmpeg's ebur128 summary.
pub fn parse_integrated(stderr: &str) -> Option<f64> {
    let summary = stderr.rsplit("Integrated loudness:").next()?;
    let line = summary.lines().find(|l| l.trim_start().starts_with("I:"))?;
    let value = line.trim_start().trim_start_matches("I:").trim();
    let value = value.trim_end_matches("LUFS").trim();
    value.parse::<f64>().ok()
}

/// Channel count of the first audio stream in ffmpeg's listing (2 when unclear).
pub fn parse_channels(stderr: &str) -> usize {
    let Some(line) = stderr
        .lines()
        .find(|l| l.contains("Stream #") && l.contains("Audio:"))
    else {
        return 2;
    };
    if line.contains(" mono") {
        1
    } else if line.contains(" stereo") {
        2
    } else if line.contains("5.1") {
        6
    } else if line.contains("7.1") {
        8
    } else {
        line.split(',')
            .find_map(|p| {
                p.trim()
                    .strip_suffix(" channels")
                    .and_then(|n| n.trim().parse().ok())
            })
            .unwrap_or(2)
    }
}

/// Measures `path` (any file ffmpeg can decode; for a video, its first audio stream).
pub fn analyze(ffmpeg: &Path, path: &Path) -> Result<AudioStats, String> {
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-nostats", "-nostdin", "-i"])
        .arg(path)
        .args([
            "-map",
            "0:a:0",
            "-af",
            "ebur128=framelog=quiet",
            "-f",
            "null",
            "-",
        ]);
    let limits = Limits::none().with_timeout(ANALYZE_TIMEOUT);
    let output = run_limited(cmd, &limits).map_err(|e| e.to_string())?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        let last = stderr.lines().last().unwrap_or("").trim().to_string();
        return Err(format!("ffmpeg could not read the audio: {last}"));
    }
    let channels = parse_channels(&stderr).max(1);
    let duration_ms = parse_ffmpeg_duration_ms(&stderr);
    let integrated_lufs = parse_integrated(&stderr);

    let mut child = Command::new(ffmpeg)
        .args(["-hide_banner", "-nostdin", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-map", "0:a:0", "-f", "s16le", "-acodec", "pcm_s16le", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot run ffmpeg: {e}"))?;
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut scan = ClipScan::new(channels);
    let mut buf = vec![0u8; 1 << 16];
    let mut carry: Option<u8> = None;
    loop {
        let n = stdout.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        let mut bytes = &buf[..n];
        if let Some(lo) = carry.take() {
            scan.push(i16::from_le_bytes([lo, bytes[0]]));
            bytes = &bytes[1..];
        }
        let (pairs, rest) = bytes.as_chunks::<2>();
        for p in pairs {
            scan.push(i16::from_le_bytes(*p));
        }
        carry = rest.first().copied();
    }
    let _ = child.wait();
    Ok(scan.finish(duration_ms, integrated_lufs))
}

/// Streaming peak and clip counter over interleaved 16-bit samples.
pub struct ClipScan {
    channels: usize,
    index: usize,
    runs: Vec<usize>,
    peak: u16,
    clip_events: u64,
    clipped_samples: u64,
    samples: u64,
}

impl ClipScan {
    pub fn new(channels: usize) -> Self {
        Self {
            channels,
            index: 0,
            runs: vec![0; channels],
            peak: 0,
            clip_events: 0,
            clipped_samples: 0,
            samples: 0,
        }
    }

    pub fn push(&mut self, s: i16) {
        let mag = s.unsigned_abs();
        self.peak = self.peak.max(mag);
        let ch = self.index % self.channels;
        self.index += 1;
        self.samples += 1;
        if mag >= 32767 {
            self.runs[ch] += 1;
        } else {
            self.end_run(ch);
        }
    }

    fn end_run(&mut self, ch: usize) {
        let run = std::mem::take(&mut self.runs[ch]);
        if run >= CLIP_RUN {
            self.clip_events += 1;
            self.clipped_samples += run as u64;
        }
    }

    pub fn finish(mut self, duration_ms: Option<u64>, integrated_lufs: Option<f64>) -> AudioStats {
        for ch in 0..self.channels {
            self.end_run(ch);
        }
        let peak_dbfs = if self.peak == 0 {
            f64::NEG_INFINITY
        } else {
            20.0 * (f64::from(self.peak) / 32768.0).log10()
        };
        AudioStats {
            duration_ms,
            channels: self.channels,
            peak_dbfs,
            clip_events: self.clip_events,
            clipped_samples: self.clipped_samples,
            samples: self.samples,
            integrated_lufs,
        }
    }
}

/// Level findings shared by source files and the rendered mix: (severity, code, message).
/// The rendered check maps them to its own codes and severities.
pub fn level_findings(stats: &AudioStats, what: &str) -> Vec<(Severity, &'static str, String)> {
    let mut out = Vec::new();
    if stats.clip_events > 0 {
        out.push((
            Severity::Warning,
            "audio.clipping",
            format!(
                "{what} clips: {} flat-topped run(s), {} samples at full scale",
                stats.clip_events, stats.clipped_samples
            ),
        ));
    } else if stats.peak_dbfs >= HOT_PEAK_DBFS {
        out.push((
            Severity::Warning,
            "audio.hot_peak",
            format!(
                "{what} peaks at {:.2} dBFS, with no headroom left",
                stats.peak_dbfs
            ),
        ));
    }
    if stats.peak_dbfs < SILENT_PEAK_DBFS {
        out.push((
            Severity::Warning,
            "audio.silent",
            format!("{what} is silent (peak {:.1} dBFS)", stats.peak_dbfs),
        ));
    } else if let Some(i) = stats.integrated_lufs.filter(|i| *i < QUIET_LUFS) {
        out.push((
            Severity::Warning,
            "audio.quiet",
            format!("{what} is very quiet: {i:.1} LUFS integrated (below {QUIET_LUFS} LUFS)"),
        ));
    }
    out
}

const NORMALIZE_HINT: &str =
    "re-export the file normalized, e.g. ffmpeg -i in.wav -af loudnorm=I=-16:TP=-1.5 out.wav";

/// Checks every referenced audio file: existence, length against its scene, and levels.
pub fn check_sources(timeline: &VideoTimeline, public_dir: &Path, ffmpeg: &Path) -> Vec<Finding> {
    let mut out = Vec::new();
    let starts = scene_starts_ms(timeline);
    let has_speech = timeline
        .scenes
        .iter()
        .any(|s| s.audio.is_some() || s.narration.is_some());
    for (i, scene) in timeline.scenes.iter().enumerate() {
        let Some(audio) = &scene.audio else { continue };
        if audio.source.trim().is_empty() || is_url(&audio.source) {
            continue;
        }
        let path = public_dir.join(&audio.source);
        let at = starts[i] + audio.offset_ms;
        if !path.is_file() {
            out.push(
                Finding::new(
                    Severity::Error,
                    "audio.missing",
                    format!("audio `{}` does not exist ({})", audio.source, path.display()),
                    Suggestion::set(
                        "add the file, or remove the audio (narration, if set, is then synthesized)",
                        scene_path(i, "audio"),
                        serde_json::Value::Null,
                    ),
                )
                .scene(&scene.id)
                .at(starts[i]),
            );
            continue;
        }
        let stats = match analyze(ffmpeg, &path) {
            Ok(s) => s,
            Err(e) => {
                out.push(
                    Finding::new(
                        Severity::Error,
                        "audio.unreadable",
                        format!("audio `{}`: {e}", audio.source),
                        Suggestion::field(
                            "replace it with a WAV, MP3 or M4A file",
                            scene_path(i, "audio/source"),
                        ),
                    )
                    .scene(&scene.id)
                    .at(starts[i]),
                );
                continue;
            }
        };
        if let Some(dur) = stats.duration_ms {
            let available = scene.duration_ms.saturating_sub(audio.offset_ms);
            if audio.offset_ms < scene.duration_ms && dur > available {
                let cut = dur - available;
                let needed = audio.offset_ms + dur + TAIL_MS;
                out.push(
                    Finding::new(
                        if cut > CUT_ERROR_MS { Severity::Error } else { Severity::Warning },
                        "audio.cut_off",
                        format!(
                            "audio `{}` is {dur} ms but only {available} ms fit before the scene ends: the last {cut} ms are cut",
                            audio.source
                        ),
                        Suggestion::set(
                            format!("lengthen the scene to {needed} ms"),
                            scene_path(i, "duration_ms"),
                            json!(needed),
                        ),
                    )
                    .scene(&scene.id)
                    .at(starts[i] + scene.duration_ms),
                );
            }
        }
        for (severity, code, message) in
            level_findings(&stats, &format!("audio `{}`", audio.source))
        {
            out.push(
                Finding::new(
                    severity,
                    code,
                    message,
                    Suggestion::field(NORMALIZE_HINT, scene_path(i, "audio/source")),
                )
                .scene(&scene.id)
                .at(at),
            );
        }
    }

    if let Some(music) = &timeline.music {
        if music.source_path().trim().is_empty() || is_url(music.source_path()) {
            return out;
        }
        let path = public_dir.join(music.source_path());
        if !path.is_file() {
            out.push(Finding::new(
                Severity::Error,
                "audio.missing",
                format!(
                    "music `{}` does not exist ({})",
                    music.source_path(),
                    path.display()
                ),
                Suggestion::set(
                    "add the file or remove the music",
                    "/music",
                    serde_json::Value::Null,
                ),
            ));
            return out;
        }
        let stats = match analyze(ffmpeg, &path) {
            Ok(s) => s,
            Err(e) => {
                out.push(Finding::new(
                    Severity::Error,
                    "audio.unreadable",
                    format!("music `{}`: {e}", music.source_path()),
                    Suggestion::field("replace it with a WAV, MP3 or M4A file", "/music/source"),
                ));
                return out;
            }
        };
        for (severity, code, message) in
            level_findings(&stats, &format!("music `{}`", music.source_path()))
        {
            out.push(Finding::new(
                severity,
                code,
                message,
                Suggestion::field(NORMALIZE_HINT, "/music/source"),
            ));
        }
        let volume = f64::from(music.volume);
        if let (Some(i), true) = (
            stats.integrated_lufs,
            volume > 0.0 && stats.peak_dbfs > SILENT_PEAK_DBFS,
        ) {
            let effective = i + 20.0 * volume.log10();
            let target_volume = |target: f64| {
                let v = volume * 10f64.powf((target - effective) / 20.0);
                (v.clamp(0.01, 1.0) * 100.0).round() / 100.0
            };
            if effective < MUSIC_INAUDIBLE_LUFS {
                out.push(Finding::new(
                    Severity::Warning,
                    "audio.music_inaudible",
                    format!("music plays at about {effective:.1} LUFS after volume {volume}: barely audible"),
                    Suggestion::set("raise the music volume", "/music/volume", json!(target_volume(MUSIC_TARGET_LUFS))),
                ));
            } else if has_speech && effective > MUSIC_LOUD_LUFS {
                out.push(Finding::new(
                    Severity::Warning,
                    "audio.music_too_loud",
                    format!("music plays at about {effective:.1} LUFS after volume {volume}, loud enough to mask speech"),
                    Suggestion::set("lower the music volume", "/music/volume", json!(target_volume(MUSIC_TARGET_LUFS))),
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ebur128_summary() {
        let s = "[Parsed_ebur128_0 @ 0x1] Summary:\n\n  Integrated loudness:\n    I:         -15.7 LUFS\n    Threshold: -25.7 LUFS\n";
        assert_eq!(parse_integrated(s), Some(-15.7));
        assert_eq!(parse_integrated("nothing"), None);
    }

    #[test]
    fn parses_channel_layouts() {
        assert_eq!(
            parse_channels("  Stream #0:0: Audio: pcm_s16le, 48000 Hz, mono, s16"),
            1
        );
        assert_eq!(
            parse_channels("  Stream #0:1(und): Audio: aac (LC), 48000 Hz, stereo, fltp"),
            2
        );
        assert_eq!(
            parse_channels("  Stream #0:0: Audio: pcm_s16le, 48000 Hz, 3 channels, s16"),
            3
        );
    }

    #[test]
    fn clip_runs_are_counted_per_channel() {
        let mut scan = ClipScan::new(2);
        // Left channel holds full scale for 4 samples, right channel touches it once.
        for (l, r) in [(32767, 0), (32767, 32767), (-32768, 0), (32767, 0), (0, 0)] {
            scan.push(l);
            scan.push(r);
        }
        let stats = scan.finish(None, None);
        assert_eq!(stats.clip_events, 1);
        assert_eq!(stats.clipped_samples, 4);
        assert!(stats.peak_dbfs > -0.01);
    }

    #[test]
    fn silence_has_no_peak() {
        let mut scan = ClipScan::new(1);
        scan.push(0);
        assert_eq!(scan.finish(None, None).peak_dbfs, f64::NEG_INFINITY);
    }
}
