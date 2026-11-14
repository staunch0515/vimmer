//! Beat sync: moving scene cuts onto the beats of the background music.
//!
//! Runs once the scene lengths are final (after narration). Cuts are visited in order; each one
//! moves to a beat of the grid within `max_shift_ms` (the nearest one, or the nearest later one),
//! trading time between the two scenes it separates, so the total length never changes. A move
//! is refused when either scene would get shorter than it needs (see [`scene_floor`] and the
//! caller's own needs, such as narration length) or the timeline would no longer validate; the
//! next candidate beat is tried, and a cut with none left stays where it is. Cuts land on whole
//! frames: the frame nearest the beat.
//!
//! The grid is in video time: the music starts with the video and loops, so the file's beats
//! repeat every file length.

use crate::model::{BeatGrid, BeatSync, Scene, SnapMode, VideoTimeline};
use crate::validation::validate;

/// No scene is made shorter than this by snapping.
pub const MIN_SCENE_MS: u64 = 500;

/// Beat times in a file from a hand-written grid: `offset_ms`, then every `60000 / bpm` ms
/// until the end of the file.
pub fn manual_beats(grid: &BeatGrid, file_ms: f64) -> Vec<f64> {
    let period = 60_000.0 / f64::from(grid.bpm);
    let mut out = Vec::new();
    let mut t = grid.offset_ms as f64;
    while t < file_ms && period > 0.0 {
        out.push(t);
        t += period;
    }
    out
}

/// The beats cuts may snap to, in ms from the video start, up to `total_ms`: the file's beats
/// (`downbeat` is the index of a bar's first beat), every `every`-th one counted from the
/// downbeat, repeated each time the music loops.
pub fn beat_grid(
    file_beats_ms: &[f64],
    downbeat: usize,
    every: u32,
    file_ms: f64,
    total_ms: f64,
) -> Vec<f64> {
    let every = every.max(1) as usize;
    let step: Vec<f64> = file_beats_ms
        .iter()
        .enumerate()
        .filter(|(i, _)| (*i as isize - downbeat as isize).rem_euclid(every as isize) == 0)
        .map(|(_, &b)| b)
        .collect();
    let mut out = Vec::new();
    if step.is_empty() || file_ms <= 0.0 {
        return out;
    }
    let mut base = 0.0;
    while base <= total_ms {
        for &b in &step {
            let t = base + b;
            if t > total_ms {
                return out;
            }
            out.push(t);
        }
        base += file_ms;
    }
    out
}

/// The shortest a scene may become, and why: [`MIN_SCENE_MS`], its transition, and everything
/// timed inside it (image keyframes, layer times and keyframes, sound effects).
pub fn scene_floor(scene: &Scene) -> (u64, String) {
    let mut floor = (MIN_SCENE_MS, "the minimum scene length".to_string());
    let mut need = |ms: u64, what: String| {
        if ms > floor.0 {
            floor = (ms, what);
        }
    };
    if let Some(t) = &scene.transition_in {
        need(t.duration_ms, "its transition".into());
    }
    if let Some(kf) = scene.keyframes.last() {
        need(kf.at_ms, "its keyframes".into());
    }
    for layer in &scene.layers {
        let what = || format!("layer `{}`", layer.id);
        need(layer.start_ms + 1, what());
        if let Some(end) = layer.end_ms {
            need(end, what());
        }
        if let Some(kf) = layer.keyframes.iter().map(|k| k.at_ms).max() {
            need(kf, what());
        }
    }
    for (i, fx) in scene.sfx.iter().enumerate() {
        if let Some(at) = fx.time_in(scene) {
            need(at + 1, format!("sound effect {i}"));
        }
    }
    floor
}

/// What happened to one cut (the start of scene `scene`).
#[derive(Debug, Clone, PartialEq)]
pub struct CutSnap {
    pub scene: usize,
    pub scene_id: String,
    /// Where the cut was and is now, in frames from the video start.
    pub from_frame: u32,
    pub to_frame: u32,
    /// The beat it snapped to (ms), or `None` when it stayed.
    pub beat_ms: Option<f64>,
    /// Why it did not snap.
    pub reason: Option<String>,
}

impl CutSnap {
    pub fn snapped(&self) -> bool {
        self.beat_ms.is_some()
    }
    /// Signed move in ms (positive = later).
    pub fn shift_ms(&self, fps: u32) -> f64 {
        (f64::from(self.to_frame) - f64::from(self.from_frame)) * 1000.0 / f64::from(fps)
    }
}

/// The result of [`snap_cuts`] for every cut, in order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SnapReport {
    pub cuts: Vec<CutSnap>,
}

impl SnapReport {
    pub fn snapped(&self) -> usize {
        self.cuts.iter().filter(|c| c.snapped()).count()
    }
    /// Mean absolute shift of the snapped cuts, in ms.
    pub fn mean_shift_ms(&self, fps: u32) -> f64 {
        let moved: Vec<f64> = self
            .cuts
            .iter()
            .filter(|c| c.snapped())
            .map(|c| c.shift_ms(fps).abs())
            .collect();
        if moved.is_empty() {
            0.0
        } else {
            moved.iter().sum::<f64>() / moved.len() as f64
        }
    }
    /// One line for the render log.
    pub fn summary(&self, fps: u32) -> String {
        format!(
            "beat sync: {} of {} cuts on a beat, mean shift {:.0} ms",
            self.snapped(),
            self.cuts.len(),
            self.mean_shift_ms(fps)
        )
    }
    /// One line per cut: where it was, where it is, and the beat (or why it stayed).
    pub fn lines(&self, fps: u32) -> Vec<String> {
        self.cuts
            .iter()
            .map(|c| match c.beat_ms {
                Some(beat) => format!(
                    "  cut into `{}`: {} -> {} ms (beat {beat:.0} ms, {:+.0} ms)",
                    c.scene_id,
                    frame_ms(c.from_frame, fps),
                    frame_ms(c.to_frame, fps),
                    c.shift_ms(fps)
                ),
                None => format!(
                    "  cut into `{}`: {} ms stays ({})",
                    c.scene_id,
                    frame_ms(c.from_frame, fps),
                    c.reason.as_deref().unwrap_or("no beat")
                ),
            })
            .collect()
    }
    /// One warning per cut that stayed.
    pub fn warnings(&self, fps: u32) -> Vec<String> {
        self.cuts
            .iter()
            .filter(|c| !c.snapped())
            .map(|c| {
                format!(
                    "beat sync: the cut into scene `{}` at {} ms stays off the beat: {}",
                    c.scene_id,
                    frame_ms(c.from_frame, fps),
                    c.reason.as_deref().unwrap_or("no beat")
                )
            })
            .collect()
    }
}

fn frame_ms(frame: u32, fps: u32) -> u64 {
    (u64::from(frame) * 1000 + u64::from(fps) / 2) / u64::from(fps)
}

fn ms_frame(ms: f64, fps: u32) -> i64 {
    (ms * f64::from(fps) / 1000.0).round() as i64
}

/// Snaps the cuts of `timeline` to `grid_ms` (from [`beat_grid`]). `needs[i]` is the extra
/// shortest length of scene `i` and its reason (narration, a clip), on top of [`scene_floor`].
/// Scene durations are rewritten to whole frames where a cut moved; user subtitle cues move
/// with the start of the scene they begin in.
pub fn snap_cuts(
    timeline: &mut VideoTimeline,
    grid_ms: &[f64],
    sync: &BeatSync,
    needs: &[(u64, String)],
) -> SnapReport {
    let fps = timeline.fps;
    let mut report = SnapReport::default();
    let old_starts = timeline.scene_start_frames();
    for i in 1..timeline.scenes.len() {
        let starts = timeline.scene_start_frames();
        let (prev_start, cut) = (i64::from(starts[i - 1]), i64::from(starts[i]));
        let end = cut + i64::from(timeline.scenes[i].duration_frames(fps));
        let cut_ms = cut as f64 * 1000.0 / f64::from(fps);
        let mut candidates: Vec<f64> = grid_ms
            .iter()
            .copied()
            .filter(|&b| (b - cut_ms).abs() <= sync.max_shift_ms as f64)
            .filter(|&b| sync.mode == SnapMode::Nearest || ms_frame(b, fps) >= cut)
            .collect();
        candidates.sort_by(|a, b| {
            (a - cut_ms)
                .abs()
                .total_cmp(&(b - cut_ms).abs())
                .then(b.total_cmp(a))
        });
        let id = timeline.scenes[i].id.clone();
        let mut entry = CutSnap {
            scene: i,
            scene_id: id,
            from_frame: cut as u32,
            to_frame: cut as u32,
            beat_ms: None,
            reason: None,
        };
        if candidates.is_empty() {
            entry.reason = Some(format!("no beat within {} ms", sync.max_shift_ms));
        }
        let need = |k: usize, scene: &Scene| {
            let floor = scene_floor(scene);
            match needs.get(k) {
                Some((ms, what)) if *ms > floor.0 => (*ms, what.clone()),
                _ => floor,
            }
        };
        for beat in candidates {
            let target = ms_frame(beat, fps);
            if target == cut {
                entry.beat_ms = Some(beat);
                entry.reason = None;
                break;
            }
            let left_ms = frames_to_ms(target - prev_start, fps);
            let right_ms = frames_to_ms(end - target, fps);
            let (left_need, left_what) = need(i - 1, &timeline.scenes[i - 1]);
            let (right_need, right_what) = need(i, &timeline.scenes[i]);
            let short = if target <= prev_start || left_ms < left_need {
                Some((i - 1, left_ms, left_need, left_what))
            } else if target >= end || right_ms < right_need {
                Some((i, right_ms, right_need, right_what))
            } else {
                None
            };
            if let Some((k, got, need, what)) = short {
                entry.reason = Some(format!(
                    "the beat at {beat:.0} ms would leave scene `{}` {got} ms, shorter than {what} needs ({need} ms)",
                    timeline.scenes[k].id
                ));
                continue;
            }
            let before = (
                timeline.scenes[i - 1].duration_ms,
                timeline.scenes[i].duration_ms,
            );
            timeline.scenes[i - 1].duration_ms = left_ms;
            timeline.scenes[i].duration_ms = right_ms;
            if let Err(errors) = validate(timeline) {
                timeline.scenes[i - 1].duration_ms = before.0;
                timeline.scenes[i].duration_ms = before.1;
                entry.reason = Some(format!(
                    "the beat at {beat:.0} ms would make the timeline invalid: {}",
                    errors[0]
                ));
                continue;
            }
            entry.to_frame = target as u32;
            entry.beat_ms = Some(beat);
            entry.reason = None;
            break;
        }
        report.cuts.push(entry);
    }
    if report.cuts.iter().any(|c| c.from_frame != c.to_frame) {
        move_subtitles(timeline, &old_starts);
    }
    report
}

fn frames_to_ms(frames: i64, fps: u32) -> u64 {
    if frames <= 0 {
        return 0;
    }
    (frames as u64 * 1000 + u64::from(fps) / 2) / u64::from(fps)
}

/// Shifts each subtitle cue (and its words) by how far the start of the scene it begins in
/// moved, keeping it inside the timeline.
fn move_subtitles(timeline: &mut VideoTimeline, old_starts: &[u32]) {
    let fps = timeline.fps;
    let new_starts = timeline.scene_start_frames();
    let total = timeline.total_duration_ms();
    let old_ms: Vec<i64> = old_starts
        .iter()
        .map(|&f| frame_ms(f, fps) as i64)
        .collect();
    let new_ms: Vec<i64> = new_starts
        .iter()
        .map(|&f| frame_ms(f, fps) as i64)
        .collect();
    for cue in &mut timeline.subtitles {
        let k = old_ms
            .iter()
            .rposition(|&s| s <= cue.start_ms as i64)
            .unwrap_or(0);
        let delta = new_ms[k] - old_ms[k];
        if delta == 0 {
            continue;
        }
        let shift = |t: u64| (t as i64 + delta).clamp(0, total as i64) as u64;
        cue.start_ms = shift(cue.start_ms);
        cue.end_ms = shift(cue.end_ms);
        for w in &mut cue.words {
            w.start_ms = shift(w.start_ms);
            w.end_ms = shift(w.end_ms);
        }
        if cue.end_ms <= cue.start_ms {
            cue.start_ms = cue.end_ms.saturating_sub(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        AudioMapping, LayerEvent, Resolution, SfxTrigger, SoundEffect, SubtitleCue, Transition,
        TransitionKind,
    };

    fn scene(id: &str, ms: u64) -> Scene {
        serde_json::from_value(serde_json::json!({"id": id, "duration_ms": ms, "text": id}))
            .unwrap()
    }

    fn timeline(lengths: &[u64]) -> VideoTimeline {
        VideoTimeline {
            fps: 30,
            resolution: Resolution {
                width: 1920,
                height: 1080,
            },
            scenes: lengths
                .iter()
                .enumerate()
                .map(|(i, &ms)| scene(&format!("s{i}"), ms))
                .collect(),
            subtitles: vec![],
            music: None,
            brand: None,
        }
    }

    fn cut_ms(t: &VideoTimeline) -> Vec<u64> {
        t.scene_start_frames()[1..]
            .iter()
            .map(|&f| frame_ms(f, 30))
            .collect()
    }

    #[test]
    fn grid_takes_every_nth_beat_from_the_downbeat_and_loops() {
        let beats: Vec<f64> = (0..8).map(|i| 250.0 + i as f64 * 500.0).collect();
        assert_eq!(
            beat_grid(&beats, 1, 2, 4_000.0, 9_000.0),
            vec![750.0, 1_750.0, 2_750.0, 3_750.0, 4_750.0, 5_750.0, 6_750.0, 7_750.0, 8_750.0]
        );
        assert_eq!(
            beat_grid(&beats, 0, 4, 4_000.0, 4_300.0),
            vec![250.0, 2_250.0, 4_250.0]
        );
        let manual = manual_beats(
            &BeatGrid {
                bpm: 96.0,
                offset_ms: 750,
            },
            3_000.0,
        );
        assert_eq!(manual, vec![750.0, 1_375.0, 2_000.0, 2_625.0]);
    }

    #[test]
    fn cuts_move_to_the_nearest_beat_and_the_total_stays() {
        let mut t = timeline(&[1_900, 2_150, 3_000, 2_000]);
        let total = t.total_frames();
        let grid: Vec<f64> = (0..20).map(|i| i as f64 * 500.0).collect();
        let r = snap_cuts(&mut t, &grid, &BeatSync::default(), &[]);
        assert_eq!(cut_ms(&t), vec![2_000, 4_000, 7_000]);
        assert_eq!(t.total_frames(), total);
        assert_eq!(r.snapped(), 3);
        // 1900 -> 2000; 2150 ms is 65 frames, so 4066.7 -> 4000 and 7066.7 -> 7000.
        assert!(
            (r.mean_shift_ms(30) - 77.8).abs() < 0.1,
            "{}",
            r.mean_shift_ms(30)
        );
    }

    #[test]
    fn later_mode_only_moves_cuts_later() {
        let mut t = timeline(&[2_100, 2_000]);
        let grid: Vec<f64> = (0..20).map(|i| i as f64 * 500.0).collect();
        let sync = BeatSync {
            mode: SnapMode::Later,
            max_shift_ms: 400,
            ..BeatSync::default()
        };
        let r = snap_cuts(&mut t, &grid, &sync, &[]);
        assert_eq!(cut_ms(&t), vec![2_500], "2000 is nearer but earlier");
        assert!(r.cuts[0].shift_ms(30) > 0.0);
    }

    #[test]
    fn a_cut_too_far_or_blocked_stays_with_a_reason() {
        let grid = vec![0.0, 3_000.0, 6_000.0];
        let mut t = timeline(&[2_000, 2_000]);
        let r = snap_cuts(&mut t, &grid, &BeatSync::default(), &[]);
        assert_eq!(cut_ms(&t), vec![2_000]);
        assert_eq!(r.cuts[0].reason.as_deref(), Some("no beat within 250 ms"));
        assert_eq!(r.warnings(30).len(), 1);

        // Narration needs 2000 ms in the first scene: the earlier beat is refused.
        let mut t = timeline(&[2_100, 2_000]);
        let needs = vec![(2_050, "its narration".to_string()), (0, String::new())];
        let r = snap_cuts(&mut t, &[1_950.0], &BeatSync::default(), &needs);
        assert_eq!(cut_ms(&t), vec![2_100]);
        assert!(
            r.cuts[0]
                .reason
                .as_deref()
                .unwrap()
                .contains("its narration needs (2050 ms)"),
            "{:?}",
            r.cuts[0].reason
        );
        // ... but a later beat in reach is taken instead.
        let r = snap_cuts(&mut t, &[1_950.0, 2_300.0], &BeatSync::default(), &needs);
        assert_eq!(cut_ms(&t), vec![2_300]);
        assert_eq!(r.snapped(), 1);
    }

    #[test]
    fn transitions_layers_and_sfx_keep_their_room() {
        let mut t = timeline(&[2_000, 1_000]);
        t.scenes[1].transition_in = Some(Transition {
            kind: TransitionKind::Fade,
            duration_ms: 900,
        });
        let r = snap_cuts(&mut t, &[2_200.0], &BeatSync::default(), &[]);
        assert_eq!(
            cut_ms(&t),
            vec![2_000],
            "only 800 ms would be left for a 900 ms fade"
        );
        assert!(r.cuts[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("its transition"));

        let mut t = timeline(&[2_000, 2_000]);
        t.scenes[0].sfx.push(SoundEffect {
            src: "assets/sfx/pop.wav".into(),
            at_ms: Some(1_900),
            on: None,
            volume: 0.8,
            duck_music: false,
        });
        snap_cuts(&mut t, &[1_850.0], &BeatSync::default(), &[]);
        assert_eq!(
            cut_ms(&t),
            vec![2_000],
            "the pop at 1900 ms must stay inside"
        );
        t.scenes[0].sfx[0] = SoundEffect {
            at_ms: None,
            on: Some(SfxTrigger {
                layer: "x".into(),
                event: LayerEvent::Enter,
            }),
            ..t.scenes[0].sfx[0].clone()
        };
        assert_eq!(
            scene_floor(&t.scenes[0]).0,
            MIN_SCENE_MS,
            "unknown layer: no time"
        );
    }

    #[test]
    fn subtitles_move_with_their_scene_and_audio_scenes_get_frames() {
        let mut t = timeline(&[1_900, 2_000, 2_000]);
        t.scenes[1].audio = Some(AudioMapping {
            source: "a.wav".into(),
            offset_ms: 0,
        });
        t.subtitles = vec![
            SubtitleCue::new(100, 1_500, "first"),
            SubtitleCue::new(1_900, 3_500, "second"),
        ];
        let grid: Vec<f64> = (0..20).map(|i| i as f64 * 1_000.0).collect();
        snap_cuts(&mut t, &grid, &BeatSync::default(), &[]);
        assert_eq!(cut_ms(&t), vec![2_000, 4_000]);
        assert_eq!(
            (t.subtitles[0].start_ms, t.subtitles[0].end_ms),
            (100, 1_500)
        );
        assert_eq!(
            (t.subtitles[1].start_ms, t.subtitles[1].end_ms),
            (2_000, 3_600)
        );
        assert!(validate(&t).is_ok());
    }
}
