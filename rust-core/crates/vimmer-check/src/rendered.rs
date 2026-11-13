//! Checks of a rendered MP4: streams, frame size, duration and the final audio mix.
//!
//! * Missing video stream, or a frame size other than the timeline's: error.
//! * Missing audio stream while the timeline has narration, scene audio or music: error.
//! * Duration off from the timeline by more than [`DURATION_TOLERANCE_MS`]: error, or a warning
//!   when narration may have stretched scenes and `--synth` did not measure it.
//! * Mix: clipping is an error; a sample peak at or above -0.1 dBFS a warning; integrated
//!   loudness below [`RENDER_QUIET_LUFS`] (-30) or above [`RENDER_LOUD_LUFS`] (-9) a warning.
//!   Streaming platforms play back at about -14 LUFS, so a mix far from that gets turned up
//!   (with its noise) or down.

use std::path::Path;
use std::process::Command;

use vimmer_dsl::VideoTimeline;
use vimmer_media::parse_ffmpeg_duration_ms;

use crate::audio::{analyze, level_findings, TAIL_MS};
use crate::{Finding, Severity, Suggestion};

pub const DURATION_TOLERANCE_MS: u64 = 200;
pub const RENDER_QUIET_LUFS: f64 = -30.0;
pub const RENDER_LOUD_LUFS: f64 = -9.0;

/// What ffmpeg's stream listing says about a file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Streams {
    pub duration_ms: Option<u64>,
    pub video: bool,
    pub audio: bool,
    pub frame_size: Option<(u32, u32)>,
}

pub fn parse_streams(stderr: &str) -> Streams {
    let mut s = Streams {
        duration_ms: parse_ffmpeg_duration_ms(stderr),
        ..Streams::default()
    };
    for line in stderr.lines().filter(|l| l.contains("Stream #")) {
        if line.contains("Audio:") {
            s.audio = true;
        }
        if line.contains("Video:") && !s.video {
            s.video = true;
            s.frame_size = line.split([',', ' ']).find_map(|tok| {
                let (w, h) = tok.split_once('x')?;
                Some((w.parse().ok()?, h.parse().ok()?))
            });
        }
    }
    s
}

pub fn check(
    timeline: &VideoTimeline,
    mp4: &Path,
    measured: Option<&[Option<u64>]>,
    ffmpeg: &Path,
) -> Vec<Finding> {
    let mut out = Vec::new();
    let what = mp4.display().to_string();
    if !mp4.is_file() {
        out.push(Finding::new(
            Severity::Error,
            "rendered.missing",
            format!("rendered file {what} does not exist"),
            Suggestion::text("render the timeline first, or fix the --rendered path"),
        ));
        return out;
    }
    let listing = Command::new(ffmpeg)
        .args(["-hide_banner", "-nostdin", "-i"])
        .arg(mp4)
        .output();
    let streams = match listing {
        Ok(o) => parse_streams(&String::from_utf8_lossy(&o.stderr)),
        Err(e) => {
            out.push(Finding::new(
                Severity::Error,
                "rendered.unreadable",
                format!("cannot run ffmpeg on {what}: {e}"),
                Suggestion::text("install ffmpeg or pass --ffmpeg"),
            ));
            return out;
        }
    };
    if !streams.video {
        out.push(Finding::new(
            Severity::Error,
            "rendered.no_video",
            format!("{what} has no video stream"),
            Suggestion::text("render again; the file is incomplete or not a video"),
        ));
    }
    let res = timeline.resolution;
    if let Some((w, h)) = streams.frame_size {
        if (w, h) != (res.width, res.height) {
            out.push(Finding::new(
                Severity::Error,
                "rendered.frame_size",
                format!(
                    "{what} is {w}x{h}, the timeline is {}x{}",
                    res.width, res.height
                ),
                Suggestion::text("render this timeline again"),
            ));
        }
    }

    // Expected length: the timeline, with measured narration stretches when known.
    let narrated: Vec<usize> = timeline
        .scenes
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            s.narration.is_some()
                && s.audio.is_none()
                && s.text.as_deref().is_some_and(|t| !t.trim().is_empty())
        })
        .map(|(i, _)| i)
        .collect();
    let mut expected = 0;
    let mut unknown_stretch = false;
    for (i, scene) in timeline.scenes.iter().enumerate() {
        let mut d = scene.duration_ms;
        if narrated.contains(&i) {
            match measured.and_then(|m| m.get(i).copied().flatten()) {
                Some(speech) => d = d.max(speech + TAIL_MS),
                None => unknown_stretch = true,
            }
        }
        expected += d;
    }
    match streams.duration_ms {
        Some(actual) => {
            let diff = actual.abs_diff(expected);
            if diff > DURATION_TOLERANCE_MS {
                let (severity, note) = if unknown_stretch && actual > expected {
                    (
                        Severity::Warning,
                        "; narration probably stretched scenes (run with --synth to measure)",
                    )
                } else {
                    (Severity::Error, "")
                };
                out.push(Finding::new(
                    severity,
                    "rendered.duration",
                    format!("{what} is {actual} ms, the timeline is {expected} ms ({diff} ms off){note}"),
                    Suggestion::text("render the current timeline again"),
                ));
            }
        }
        None => out.push(Finding::new(
            Severity::Error,
            "rendered.duration",
            format!("ffmpeg reports no duration for {what}"),
            Suggestion::text("render again; the file looks truncated"),
        )),
    }

    let expects_audio = timeline.music.is_some()
        || timeline.scenes.iter().any(|s| s.audio.is_some())
        || !narrated.is_empty();
    if !streams.audio {
        if expects_audio {
            out.push(Finding::new(
                Severity::Error,
                "rendered.no_audio",
                format!(
                    "{what} has no audio stream, but the timeline has narration, audio or music"
                ),
                Suggestion::text("render again and check the narration and music warnings"),
            ));
        }
        return out;
    }
    match analyze(ffmpeg, mp4) {
        Ok(stats) => {
            for (severity, code, message) in level_findings(&stats, "the final mix") {
                let (severity, code) = match code {
                    "audio.clipping" => (Severity::Error, "rendered.clipping"),
                    "audio.hot_peak" => (severity, "rendered.hot_peak"),
                    "audio.silent" if expects_audio => (Severity::Error, "rendered.silent"),
                    "audio.silent" => continue,
                    "audio.quiet" => continue, // judged against the render thresholds below
                    _ => (severity, code),
                };
                out.push(Finding::new(
                    severity,
                    code,
                    message,
                    Suggestion::text(MIX_HINT),
                ));
            }
            if let Some(i) = stats.integrated_lufs.filter(|_| stats.peak_dbfs >= -60.0) {
                if i < RENDER_QUIET_LUFS && expects_audio {
                    out.push(Finding::new(
                        Severity::Warning,
                        "rendered.quiet",
                        format!("the final mix is {i:.1} LUFS integrated, below {RENDER_QUIET_LUFS} LUFS (platforms play about -14)"),
                        Suggestion::text(MIX_HINT),
                    ));
                } else if i > RENDER_LOUD_LUFS {
                    out.push(Finding::new(
                        Severity::Warning,
                        "rendered.loud",
                        format!("the final mix is {i:.1} LUFS integrated, above {RENDER_LOUD_LUFS} LUFS"),
                        Suggestion::text(MIX_HINT),
                    ));
                }
            }
        }
        Err(e) => out.push(Finding::new(
            Severity::Error,
            "rendered.unreadable",
            format!("cannot measure the audio of {what}: {e}"),
            Suggestion::text("render again"),
        )),
    }
    out
}

const MIX_HINT: &str = "adjust the source levels (scene audio files, music volume) and render again, or normalize the result: ffmpeg -i out.mp4 -c:v copy -af loudnorm=I=-14:TP=-1.5 fixed.mp4";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mp4_listing() {
        let s = "Input #0, mov,mp4, from 'out.mp4':\n  Duration: 00:00:15.02, start: 0.000000, bitrate: 1200 kb/s\n  Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), yuv420p(tv, bt709, progressive), 1920x1080 [SAR 1:1 DAR 16:9], 1000 kb/s, 30 fps\n  Stream #0:1[0x2](und): Audio: aac (LC) (mp4a / 0x6134706D), 48000 Hz, stereo, fltp, 128 kb/s\n";
        let st = parse_streams(s);
        assert_eq!(st.duration_ms, Some(15_020));
        assert!(st.video && st.audio);
        assert_eq!(st.frame_size, Some((1920, 1080)));
    }

    #[test]
    fn video_only_file() {
        let st = parse_streams("  Duration: 00:00:01.00, start\n  Stream #0:0: Video: h264, yuv420p, 1080x1920, 30 fps\n");
        assert!(st.video && !st.audio);
        assert_eq!(st.frame_size, Some((1080, 1920)));
    }
}
