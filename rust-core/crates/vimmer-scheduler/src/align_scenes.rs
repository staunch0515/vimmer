//! 按旁白对齐场景 (align scene cuts to the voice-over).
//!
//! A voice-over is recorded at its own pace, so the scene cuts drawn before it rarely fall where
//! the speaker moves on. This proposes new scene lengths from the voice-over itself:
//!
//! 1. The clip is recognized with the local whisper (as for word subtitles) and aligned to the
//!    scripts of the scenes it plays over (each scene's spoken line, else its text), joined in
//!    order. Every script unit gets a time, so each scene gets the span from its first to its
//!    last spoken word.
//! 2. Each cut between two covered scenes moves into the pause between them: the middle of the
//!    pause, but at least [`LEAD_MS`] before the next scene's first word when the pause allows it.
//!    So a cut lands at the sentence boundary, never inside a sentence.
//! 3. The first covered scene keeps its start (the scenes before it are not touched); the last
//!    one keeps its end, or ends [`TAIL_MS`] after the last word when that is later. Scenes after
//!    the covered range keep their lengths, so they only move when the last one grows.
//! 4. No scene gets shorter than it needs ([`vimmer_dsl::scene_floor`]); a scene whose words were
//!    not found keeps its length.
//!
//! [`propose`] is pure; [`align_to_voiceover`] runs the recognition.

use std::sync::Arc;

use serde::Serialize;
use vimmer_dsl::{scene_floor, VideoTimeline};
use vimmer_media::{
    align_words, guess_language, split_units, AudioProbe, Limits, TranscribeRequest, Transcriber,
    WhisperCli,
};

use crate::control::{Progress, RenderContext};
use crate::run::SchedulerConfig;
use crate::wordsubs::{script_of, voiceover_clips, ClipKind};

/// A cut lands at least this long before the next scene's first word, when the pause allows.
pub const LEAD_MS: u64 = 150;
/// The last covered scene lasts at least this long after the last word.
pub const TAIL_MS: u64 = 300;

/// One scene's proposed change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SceneChange {
    pub index: usize,
    pub id: String,
    pub old_ms: u64,
    pub new_ms: u64,
    /// When the scene's words are spoken, in ms from the video start (`None`: not found).
    pub said_from_ms: Option<u64>,
    pub said_to_ms: Option<u64>,
}

/// What [`align_to_voiceover`] proposes for one voice-over clip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Alignment {
    pub clip: String,
    /// Every covered scene, in order (unchanged ones included, `old_ms == new_ms`).
    pub changes: Vec<SceneChange>,
    /// Every scene's length after the change (what to apply).
    pub durations: Vec<u64>,
    pub notes: Vec<String>,
}

fn round10(ms: u64) -> u64 {
    (ms + 5) / 10 * 10
}

/// New lengths for the covered scenes. `spans[k]` is scene `covered[k]`'s spoken span (absolute
/// ms), `None` when its words were not found. `covered` must be consecutive scene indices.
pub fn propose(
    timeline: &VideoTimeline,
    covered: &[usize],
    spans: &[Option<(u64, u64)>],
) -> (Vec<SceneChange>, Vec<u64>) {
    let mut durations: Vec<u64> = timeline.scenes.iter().map(|s| s.duration_ms).collect();
    if covered.is_empty() {
        return (Vec::new(), durations);
    }
    let start_of = |i: usize| -> u64 { timeline.scenes[..i].iter().map(|s| s.duration_ms).sum() };
    let first = covered[0];
    let last = *covered.last().expect("non-empty");
    let floor = |i: usize| scene_floor(&timeline.scenes[i]).0;
    // Old cut positions (absolute): the end of each covered scene.
    let old_end = |i: usize| start_of(i) + timeline.scenes[i].duration_ms;

    // Cuts after each covered scene but the last.
    let mut cuts: Vec<u64> = Vec::with_capacity(covered.len());
    let mut prev_cut = start_of(first);
    for k in 0..covered.len() - 1 {
        let i = covered[k];
        let cut = match (spans[k], spans[k + 1]) {
            (Some((_, said_end)), Some((next_from, _))) if next_from >= said_end => {
                let mid = said_end + (next_from - said_end) / 2;
                let lead = next_from.saturating_sub(LEAD_MS).max(said_end);
                round10(mid.min(lead))
            }
            // Words overlap or are missing: the cut stays where it was.
            _ => old_end(i),
        };
        // Never shorter than the scene needs.
        let cut = cut.max(prev_cut + floor(i));
        cuts.push(cut);
        prev_cut = cut;
    }
    let speech_end = spans
        .last()
        .copied()
        .flatten()
        .map(|(_, e)| round10(e + TAIL_MS));
    let last_end = old_end(last)
        .max(speech_end.unwrap_or(0))
        .max(prev_cut + floor(last));
    cuts.push(last_end);

    let mut changes = Vec::with_capacity(covered.len());
    let mut from = start_of(first);
    for (k, &i) in covered.iter().enumerate() {
        let new_ms = cuts[k] - from;
        from = cuts[k];
        changes.push(SceneChange {
            index: i,
            id: timeline.scenes[i].id.clone(),
            old_ms: timeline.scenes[i].duration_ms,
            new_ms,
            said_from_ms: spans[k].map(|s| s.0),
            said_to_ms: spans[k].map(|s| s.1),
        });
        durations[i] = new_ms;
    }
    (changes, durations)
}

/// Splits aligned script units back into scenes: `counts[k]` units belong to scene k. Returns
/// each scene's (first start, last end), or `None` for a scene without units.
pub fn scene_spans(units: &[(u64, u64)], counts: &[usize]) -> Vec<Option<(u64, u64)>> {
    let mut out = Vec::with_capacity(counts.len());
    let mut at = 0;
    for &n in counts {
        let part = &units[at.min(units.len())..(at + n).min(units.len())];
        out.push(match (part.first(), part.last()) {
            (Some(a), Some(b)) => Some((a.0, b.1.max(a.0))),
            _ => None,
        });
        at += n;
    }
    out
}

/// Recognizes voice-over clip `clip_id` (the first clip when `None`) and proposes scene lengths
/// that put the cuts at its sentence boundaries. The timeline's paths must already be relative
/// to `cfg.public_dir` (staged).
pub async fn align_to_voiceover(
    timeline: &VideoTimeline,
    clip_id: Option<&str>,
    cfg: &SchedulerConfig,
    probe: Arc<dyn AudioProbe>,
    ctx: &RenderContext,
) -> Result<Alignment, String> {
    let wanted = match clip_id {
        Some(id) => timeline
            .voiceover
            .iter()
            .find(|c| c.id == id)
            .ok_or_else(|| format!("There is no voice-over clip `{id}`."))?,
        None => timeline
            .voiceover
            .first()
            .ok_or("There is no voice-over to align to: record or add one first.")?,
    };
    let mut only = timeline.clone();
    only.voiceover = vec![wanted.clone()];
    let clips = voiceover_clips(&only, cfg, probe.clone(), ctx)
        .await
        .map_err(|e| format!("The voice-over could not be prepared: {e}"))?;
    let clip = clips
        .into_iter()
        .find(|c| matches!(&c.kind, ClipKind::Voiceover(id) if *id == wanted.id))
        .ok_or("The voice-over clip plays nothing (it is trimmed away, or starts after the end).")?;
    // The scenes it plays over, and their scripts.
    let starts: Vec<u64> = timeline
        .scenes
        .iter()
        .scan(0u64, |acc, s| {
            let from = *acc;
            *acc += s.duration_ms;
            Some(from)
        })
        .collect();
    let covered: Vec<usize> = (0..timeline.scenes.len())
        .filter(|&i| {
            starts[i] < clip.scene_end_ms
                && starts[i] + timeline.scenes[i].duration_ms > clip.start_ms
        })
        .filter(|&i| !script_of(&timeline.scenes[i]).is_empty())
        .collect();
    if covered.is_empty() {
        return Err("The scenes under the voice-over have no text to match it with.".into());
    }
    if covered.windows(2).any(|w| w[1] != w[0] + 1) {
        return Err("A scene without text sits between the scenes the voice-over speaks; give it text (or a spoken line) first.".into());
    }
    let scripts: Vec<&str> = covered.iter().map(|&i| script_of(&timeline.scenes[i])).collect();
    let script = crate::wordsubs::join_scripts(scripts.iter().copied());
    let counts: Vec<usize> = scripts.iter().map(|s| split_units(s).len()).collect();

    let ws = &cfg.word_subtitles;
    let engine = WhisperCli::locate(
        ws.whisper_cli.as_deref(),
        ws.model.as_deref(),
        cfg.child_path.as_deref(),
    )
    .map(|mut e| {
        e.ffmpeg = cfg.ffmpeg.clone();
        e
    })
    .map_err(|e| format!("Aligning needs the local speech recognizer (whisper): {e}"))?;
    let wav = clip.wav.clone();
    let p = probe.clone();
    let audio_ms = tokio::task::spawn_blocking(move || p.duration_ms(&wav))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    ctx.emit(Progress::Stage {
        name: "Listening to the voice-over".into(),
    });
    let cancel = ctx.cancel.clone();
    let limits = Limits {
        timeout: Some(crate::wordsubs::recognition_timeout(audio_ms)),
        cancelled: Arc::new(move || cancel.is_cancelled()),
    };
    let request = TranscribeRequest {
        language: match ws.language.as_deref() {
            Some("auto") => None,
            Some(code) => Some(code.to_string()),
            None => guess_language(&script).map(String::from),
        },
        prompt: Some(script.clone()),
    };
    let wav = clip.wav.clone();
    let tokens = tokio::task::spawn_blocking(move || engine.transcribe(&wav, &request, &limits))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("The voice-over could not be recognized: {e}"))?;
    let mut notes = Vec::new();
    if tokens.is_empty() {
        return Err("Speech recognition heard nothing in the voice-over.".into());
    }
    let units: Vec<(u64, u64)> = align_words(&script, &tokens, audio_ms)
        .iter()
        .map(|w| (w.start_ms + clip.start_ms, w.end_ms + clip.start_ms))
        .collect();
    if units.len() != counts.iter().sum::<usize>() {
        notes.push("the script's words could not be counted per scene exactly; cuts are approximate".into());
    }
    let spans = scene_spans(&units, &counts);
    let (changes, durations) = propose(timeline, &covered, &spans);
    let moved = changes.iter().filter(|c| c.new_ms != c.old_ms).count();
    notes.push(format!(
        "{moved} of {} scene(s) under `{}` change length",
        changes.len(),
        wanted.id
    ));
    Ok(Alignment {
        clip: wanted.id.clone(),
        changes,
        durations,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timeline(lengths: &[u64]) -> VideoTimeline {
        let scenes: Vec<serde_json::Value> = lengths
            .iter()
            .enumerate()
            .map(|(i, ms)| serde_json::json!({"id": format!("s{i}"), "duration_ms": ms, "text": "x"}))
            .collect();
        serde_json::from_value(serde_json::json!({
            "fps": 30, "resolution": {"width": 1920, "height": 1080}, "scenes": scenes
        }))
        .unwrap()
    }

    #[test]
    fn cuts_move_into_the_pauses_and_the_rest_is_untouched() {
        // Scenes: 0..2000 (intro, not covered), 2000..5000, 5000..8000, 8000..11500, 11500..14000.
        let t = timeline(&[2000, 3000, 3000, 3500, 2500]);
        // Speech: s1 2100-4400, s2 4900-7600, s3 8100-10900.
        let spans = [Some((2100, 4400)), Some((4900, 7600)), Some((8100, 10900))];
        let (changes, durations) = propose(&t, &[1, 2, 3], &spans);
        // Cut 1: pause 4400..4900, middle 4650, lead 4750 -> 4650. Cut 2: 7600..8100 -> 7850.
        // Last: keeps its end 11500 (speech ends at 10900 + 300 = 11200, earlier).
        assert_eq!(durations, vec![2000, 2650, 3200, 3650, 2500]);
        assert_eq!(changes.iter().map(|c| (c.index, c.old_ms, c.new_ms)).collect::<Vec<_>>(),
            vec![(1, 3000, 2650), (2, 3000, 3200), (3, 3500, 3650)]);
        assert_eq!(durations.iter().sum::<u64>(), 14_000, "nothing after the range moves");
        assert_eq!(changes[0].said_from_ms, Some(2100));
    }

    #[test]
    fn a_short_pause_cuts_before_the_next_word_and_the_last_scene_grows() {
        let t = timeline(&[3000, 3000]);
        // A long pause: 1000..2600, middle 1800, lead 2450 -> 1800. Speech runs to 6500.
        let (_, d) = propose(&t, &[0, 1], &[Some((100, 1000)), Some((2600, 6500))]);
        assert_eq!(d, vec![1800, 6500 + TAIL_MS - 1800]);
        // A tiny pause 1000..1100: middle 1050, lead floor 1000 -> 1000 (rounded 1000).
        let (_, d) = propose(&t, &[0, 1], &[Some((100, 1000)), Some((1100, 2000))]);
        assert_eq!(d[0], 1000);
    }

    #[test]
    fn missing_words_keep_the_cut_and_floors_are_kept() {
        let t = timeline(&[3000, 3000, 3000]);
        let (_, d) = propose(&t, &[0, 1, 2], &[Some((0, 1000)), None, Some((6500, 8000))]);
        assert_eq!(d, vec![3000, 3000, 3000], "around a scene with no words, cuts stay");
        // A pause that would leave a scene under 500 ms: the floor wins.
        let (_, d) = propose(&t, &[0, 1], &[Some((0, 100)), Some((200, 2000))]);
        assert_eq!(d[0], 500);
        assert_eq!(d[0] + d[1], 6000, "the last covered scene keeps its end");
    }

    #[test]
    fn units_are_split_back_into_scenes() {
        let units = [(0, 100), (100, 200), (300, 400), (500, 600), (600, 700)];
        assert_eq!(
            scene_spans(&units, &[2, 0, 3]),
            vec![Some((0, 200)), None, Some((300, 700))]
        );
        assert_eq!(scene_spans(&units[..1], &[1, 2]), vec![Some((0, 100)), None]);
    }
}
