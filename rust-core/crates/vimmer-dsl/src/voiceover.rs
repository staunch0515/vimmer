//! The voice-over track (旁白音轨): recorded or imported speech placed on the timeline at an
//! absolute time, independent of scene cuts.
//!
//! Each clip plays its file from `trim_start_ms` to `trim_end_ms` before the file's end, starting
//! at `start_ms` from the video start, with a linear `gain`. Clips never overlap. Music ducks
//! under the voice-over exactly like it ducks under narration.
//!
//! A clip's length depends on its file, so the rules that need it (overlap, running past the
//! end, a clip longer than the whole timeline) are checked where the file can be measured
//! (`vimmer check`, the render); [`VoiceoverClip::problems`] covers the rest.
//!
//! The track is `skip_serializing_if` empty: a timeline without it serializes exactly as before.

use serde::{Deserialize, Serialize};

/// Most clips one timeline may have.
pub const MAX_VOICEOVER_CLIPS: usize = 200;
/// Highest gain (linear; 1.0 = as recorded).
pub const MAX_VOICEOVER_GAIN: f32 = 4.0;

fn unit_gain() -> f32 {
    1.0
}

fn is_unit(g: &f32) -> bool {
    *g == 1.0
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

/// One clip of the voice-over track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoiceoverClip {
    /// Unique among the clips (names the clip in messages and in the editor).
    pub id: String,
    /// The audio file, like `scene.audio.source` (project asset or public-relative path).
    pub source: String,
    /// Where the clip starts, in ms from the start of the video.
    #[serde(default)]
    pub start_ms: u64,
    /// Skipped at the start of the file.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub trim_start_ms: u64,
    /// Cut off the end of the file.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub trim_end_ms: u64,
    /// Linear gain, 0 to [`MAX_VOICEOVER_GAIN`]; 1.0 (the default) plays the file as it is.
    #[serde(default = "unit_gain", skip_serializing_if = "is_unit")]
    pub gain: f32,
}

impl VoiceoverClip {
    /// How long the clip plays, for a file `file_ms` long.
    pub fn played_ms(&self, file_ms: u64) -> u64 {
        file_ms.saturating_sub(self.trim_start_ms.saturating_add(self.trim_end_ms))
    }

    /// Where the clip ends on the timeline, for a file `file_ms` long.
    pub fn end_ms(&self, file_ms: u64) -> u64 {
        self.start_ms + self.played_ms(file_ms)
    }

    /// Problems that need no file: id, source, gain, a start past the end of the video.
    pub fn problems(&self, total_ms: u64) -> Vec<String> {
        let mut out = Vec::new();
        if self.id.trim().is_empty() {
            out.push("needs an id".to_string());
        }
        if self.source.trim().is_empty() {
            out.push("needs an audio file (source)".to_string());
        }
        if !self.gain.is_finite() || !(0.0..=MAX_VOICEOVER_GAIN).contains(&self.gain) {
            out.push(format!(
                "gain {} must be between 0 and {MAX_VOICEOVER_GAIN} (1 = as recorded)",
                self.gain
            ));
        }
        if self.start_ms >= total_ms {
            out.push(format!(
                "starts at {} ms, at or after the end of the video ({total_ms} ms)",
                self.start_ms
            ));
        }
        out
    }
}

/// Pairs of clips (by index, earlier first in time) that overlap, given each clip's file length.
/// Clips that only touch (one ends where the next starts) do not overlap.
pub fn overlapping_clips(clips: &[VoiceoverClip], file_ms: &[u64]) -> Vec<(usize, usize)> {
    let mut order: Vec<usize> = (0..clips.len().min(file_ms.len())).collect();
    order.sort_by_key(|&i| (clips[i].start_ms, i));
    let mut out = Vec::new();
    for (k, &a) in order.iter().enumerate() {
        let end = clips[a].end_ms(file_ms[a]);
        for &b in &order[k + 1..] {
            if clips[b].start_ms >= end {
                break;
            }
            if clips[b].played_ms(file_ms[b]) > 0 && clips[a].played_ms(file_ms[a]) > 0 {
                out.push((a, b));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(id: &str, start: u64) -> VoiceoverClip {
        VoiceoverClip {
            id: id.into(),
            source: "rec.wav".into(),
            start_ms: start,
            trim_start_ms: 0,
            trim_end_ms: 0,
            gain: 1.0,
        }
    }

    #[test]
    fn defaults_are_not_written() {
        let c: VoiceoverClip =
            serde_json::from_str(r#"{"id":"a","source":"x.wav","start_ms":100}"#).unwrap();
        assert_eq!(c.gain, 1.0);
        assert_eq!((c.trim_start_ms, c.trim_end_ms), (0, 0));
        assert_eq!(
            serde_json::to_string(&c).unwrap(),
            r#"{"id":"a","source":"x.wav","start_ms":100}"#
        );
        let full = r#"{"id":"a","source":"x.wav","start_ms":100,"trim_start_ms":5,"trim_end_ms":7,"gain":0.5}"#;
        let c: VoiceoverClip = serde_json::from_str(full).unwrap();
        assert_eq!(serde_json::to_string(&c).unwrap(), full);
    }

    #[test]
    fn played_length_follows_the_trims() {
        let mut c = clip("a", 1000);
        c.trim_start_ms = 200;
        c.trim_end_ms = 300;
        assert_eq!(c.played_ms(2000), 1500);
        assert_eq!(c.end_ms(2000), 2500);
        assert_eq!(c.played_ms(400), 0, "trims longer than the file leave nothing");
    }

    #[test]
    fn problems_name_what_is_wrong() {
        let mut c = clip(" ", 9000);
        c.source = String::new();
        c.gain = 5.0;
        let p = c.problems(8000);
        assert_eq!(p.len(), 4, "{p:?}");
        assert!(p[3].contains("after the end of the video (8000 ms)"), "{p:?}");
        assert!(clip("a", 0).problems(8000).is_empty());
    }

    #[test]
    fn overlaps_are_found_in_time_order() {
        let clips = [clip("a", 0), clip("b", 1000), clip("c", 2500)];
        // a: 0..1000 touches b; b: 1000..3000 overlaps c.
        assert_eq!(overlapping_clips(&clips, &[1000, 2000, 500]), vec![(1, 2)]);
        assert!(overlapping_clips(&clips, &[1000, 1500, 500]).is_empty());
        // Listed order does not matter.
        let clips = [clip("late", 500), clip("early", 0)];
        assert_eq!(overlapping_clips(&clips, &[100, 1000]), vec![(1, 0)]);
    }

    #[test]
    fn validation_reports_the_track() {
        let mut t: crate::VideoTimeline = serde_json::from_value(serde_json::json!({
            "fps": 30, "resolution": {"width": 1920, "height": 1080},
            "scenes": [{"id": "a", "duration_ms": 2000, "text": "Hi"}],
        }))
        .unwrap();
        assert!(crate::validate(&t).is_ok());
        assert!(!serde_json::to_string(&t).unwrap().contains("voiceover"));
        t.voiceover = vec![clip("v", 0), clip("v", 2000)];
        let errors: Vec<String> = crate::validate(&t)
            .unwrap_err()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            errors,
            vec![
                "voiceover: duplicate clip id `v`".to_string(),
                "voiceover clip `v`: starts at 2000 ms, at or after the end of the video (2000 ms)"
                    .to_string(),
            ]
        );
        t.voiceover = vec![clip("v", 1999)];
        assert!(crate::validate(&t).is_ok());
    }
}
