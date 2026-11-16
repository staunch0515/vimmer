//! The voice-over track (旁白音轨): files, lengths and levels.
//!
//! * a missing or unreadable file: error `voiceover.missing` / `voiceover.unreadable`
//! * two clips that play at the same time: error `voiceover.overlap`
//! * a clip that runs past the end of the video: warning `voiceover.past_end` (its tail is cut)
//! * a clip longer than the whole video: error `voiceover.longer_than_video`
//! * levels after the clip's gain: warning `voiceover.clipping` when the gain pushes the peak to
//!   full scale, plus the usual file levels (`audio.clipping`, `audio.hot_peak`, `audio.quiet`,
//!   `audio.silent`) of [`crate::audio::level_findings`].
//!
//! Clips starting at or after the end, bad gains and duplicate ids are validation errors
//! (`dsl.invalid`).

use std::path::Path;

use serde_json::json;
use vimmer_dsl::{overlapping_clips, VideoTimeline};

use crate::audio::{analyze, level_findings, AudioStats};
use crate::{is_url, Finding, Severity, Suggestion};

fn clip_path(index: usize, field: &str) -> String {
    format!("/voiceover/{index}/{field}")
}

/// Checks every voice-over clip against its file. `analyze` measures a file (ffmpeg in use;
/// injectable for tests).
pub fn check(timeline: &VideoTimeline, public_dir: &Path, ffmpeg: &Path) -> Vec<Finding> {
    check_with(timeline, public_dir, &|path| analyze(ffmpeg, path))
}

pub fn check_with(
    timeline: &VideoTimeline,
    public_dir: &Path,
    analyze: &dyn Fn(&Path) -> Result<AudioStats, String>,
) -> Vec<Finding> {
    let mut out = Vec::new();
    let clips = &timeline.voiceover;
    let total = timeline.total_duration_ms();
    // Lengths of the clips whose file could be measured (others are left out of the overlap
    // check).
    let mut measured: Vec<(usize, u64)> = Vec::new();
    for (i, clip) in clips.iter().enumerate() {
        if clip.source.trim().is_empty() || is_url(&clip.source) {
            continue;
        }
        let path = public_dir.join(&clip.source);
        if !path.is_file() {
            out.push(
                Finding::new(
                    Severity::Error,
                    "voiceover.missing",
                    format!(
                        "voice-over clip `{}`: `{}` does not exist ({})",
                        clip.id,
                        clip.source,
                        path.display()
                    ),
                    Suggestion::field("record it again or pick another file", clip_path(i, "source")),
                )
                .at(clip.start_ms),
            );
            continue;
        }
        let stats = match analyze(&path) {
            Ok(s) => s,
            Err(e) => {
                out.push(
                    Finding::new(
                        Severity::Error,
                        "voiceover.unreadable",
                        format!("voice-over clip `{}` (`{}`): {e}", clip.id, clip.source),
                        Suggestion::field("replace it with a WAV, MP3 or M4A file", clip_path(i, "source")),
                    )
                    .at(clip.start_ms),
                );
                continue;
            }
        };
        if let Some(file_ms) = stats.duration_ms {
            measured.push((i, file_ms));
            let played = clip.played_ms(file_ms);
            let end = clip.end_ms(file_ms);
            if played > total {
                let trim = clip.trim_end_ms + (played - total);
                out.push(
                    Finding::new(
                        Severity::Error,
                        "voiceover.longer_than_video",
                        format!(
                            "voice-over clip `{}` plays {played} ms, longer than the whole video ({total} ms)",
                            clip.id
                        ),
                        Suggestion::set(
                            format!("trim {trim} ms off its end, or make the scenes longer"),
                            clip_path(i, "trim_end_ms"),
                            json!(trim),
                        ),
                    )
                    .at(clip.start_ms),
                );
            } else if clip.start_ms < total && end > total {
                let start = total - played;
                out.push(
                    Finding::new(
                        Severity::Warning,
                        "voiceover.past_end",
                        format!(
                            "voice-over clip `{}` runs to {end} ms, past the end of the video ({total} ms): the last {} ms are cut",
                            clip.id,
                            end - total
                        ),
                        Suggestion::set(
                            format!("start it at {start} ms, or lengthen the last scene"),
                            clip_path(i, "start_ms"),
                            json!(start),
                        ),
                    )
                    .at(clip.start_ms),
                );
            }
        }
        let gain = f64::from(clip.gain);
        if gain > 1.0 && stats.clip_events == 0 && stats.peak_dbfs.is_finite() {
            let peak = stats.peak_dbfs + 20.0 * gain.log10();
            if peak >= 0.0 {
                let safe = (10f64.powf(-(stats.peak_dbfs + 1.0) / 20.0) * 100.0).floor() / 100.0;
                out.push(
                    Finding::new(
                        Severity::Warning,
                        "voiceover.clipping",
                        format!(
                            "voice-over clip `{}` peaks at {:.2} dBFS; with gain {} it would reach {peak:+.1} dBFS and distort",
                            clip.id, stats.peak_dbfs, clip.gain
                        ),
                        Suggestion::set("lower its gain", clip_path(i, "gain"), json!(safe.max(1.0))),
                    )
                    .at(clip.start_ms),
                );
            }
        }
        for (severity, code, message) in
            level_findings(&stats, &format!("voice-over clip `{}`", clip.id))
        {
            out.push(
                Finding::new(
                    severity,
                    code,
                    message,
                    Suggestion::field(
                        "record again closer to the microphone, or re-export the file normalized",
                        clip_path(i, "source"),
                    ),
                )
                .at(clip.start_ms),
            );
        }
    }

    // Overlaps among the measured clips, reported once per pair.
    let subset: Vec<vimmer_dsl::VoiceoverClip> = measured.iter().map(|&(i, _)| clips[i].clone()).collect();
    let lengths: Vec<u64> = measured.iter().map(|&(_, ms)| ms).collect();
    for (a, b) in overlapping_clips(&subset, &lengths) {
        let (ia, ib) = (measured[a].0, measured[b].0);
        let end = clips[ia].end_ms(lengths[a]);
        out.push(
            Finding::new(
                Severity::Error,
                "voiceover.overlap",
                format!(
                    "voice-over clips `{}` and `{}` overlap: `{}` still plays until {end} ms when `{}` starts at {} ms",
                    clips[ia].id, clips[ib].id, clips[ia].id, clips[ib].id, clips[ib].start_ms
                ),
                Suggestion::set(
                    format!("move `{}` to start at {end} ms", clips[ib].id),
                    clip_path(ib, "start_ms"),
                    json!(end),
                ),
            )
            .at(clips[ib].start_ms),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(ms: u64, peak: f64) -> AudioStats {
        AudioStats {
            duration_ms: Some(ms),
            channels: 1,
            peak_dbfs: peak,
            clip_events: 0,
            clipped_samples: 0,
            samples: ms * 48,
            integrated_lufs: Some(-20.0),
        }
    }

    fn codes(f: &[Finding]) -> Vec<&str> {
        f.iter().map(|f| f.code).collect()
    }

    #[test]
    fn files_lengths_overlaps_and_gain() {
        let dir = std::env::temp_dir().join(format!("vimmer-check-vo-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("rec")).unwrap();
        for name in ["a", "b", "c"] {
            std::fs::write(dir.join(format!("rec/{name}.wav")), b"wav").unwrap();
        }
        let t: VideoTimeline = serde_json::from_value(json!({
            "fps": 30, "resolution": {"width": 1920, "height": 1080},
            "scenes": [{"id": "s", "duration_ms": 4000, "text": "x"}],
            "voiceover": [
                {"id": "a", "source": "rec/a.wav", "start_ms": 0, "gain": 2.0},
                {"id": "b", "source": "rec/b.wav", "start_ms": 1000},
                {"id": "c", "source": "rec/c.wav", "start_ms": 3500},
                {"id": "gone", "source": "rec/gone.wav", "start_ms": 100}
            ]
        }))
        .unwrap();
        let lengths = |p: &Path| -> Result<AudioStats, String> {
            let name = p.file_stem().unwrap().to_string_lossy().to_string();
            Ok(match name.as_str() {
                "a" => stats(1500, -3.0),
                "b" => stats(2000, -12.0),
                _ => stats(1000, -12.0),
            })
        };
        let f = check_with(&t, &dir, &lengths);
        assert_eq!(
            codes(&f),
            vec!["voiceover.clipping", "voiceover.past_end", "voiceover.missing", "voiceover.overlap"],
            "{f:#?}"
        );
        assert!(f[1].message.contains("runs to 4500 ms"), "{}", f[1].message);
        assert!(f[3].message.contains("`a` and `b` overlap"), "{}", f[3].message);
        assert_eq!(f[3].suggestion.value, Some(json!(1500)));
        assert_eq!(f[0].suggestion.value, Some(json!(1.25)), "peaks at -1 dBFS");

        // A clip longer than the whole video.
        let long = |_: &Path| -> Result<AudioStats, String> { Ok(stats(9000, -12.0)) };
        let mut one = t.clone();
        one.voiceover.truncate(1);
        one.voiceover[0].gain = 1.0;
        let f = check_with(&one, &dir, &long);
        assert_eq!(codes(&f), vec!["voiceover.longer_than_video"]);
        assert_eq!(f[0].suggestion.value, Some(json!(5000)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
