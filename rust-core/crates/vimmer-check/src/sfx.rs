//! Sound effects and beat sync.
//!
//! Sound effects (needs the public dir):
//! * `sfx.missing` (error): the file does not exist; `sfx.unreadable` (error): ffmpeg cannot
//!   decode it.
//! * `sfx.past_video_end` (warning): the effect is still playing when the video ends and is cut.
//! * `sfx.clipping` (warning): two or more effects overlap and their peaks (times volume), plus
//!   the music's peak (times its volume) and the scene audio's peak, add up above full scale.
//!   The render's limiter keeps the mix from clipping, but squashes it audibly. The sum of peaks
//!   is a worst case: peaks rarely line up exactly, so this can warn about a mix that would
//!   have stayed under (it never misses one that clips).
//!
//! Effects timed outside their scene, on a missing layer or with a bad file type are validation
//! errors (`dsl.invalid`).
//!
//! Beat sync (when `music.beat_sync` is on):
//! * `beats.low_confidence` (warning): detection (cached in `public/generated/beats`, the same
//!   cache as the render) is unsure of the beat, so the render will not snap; suggests setting
//!   `music.beats` from the detected tempo and first beat.
//! * `beats.unsnapped` (warning): a cut that cannot reach a beat, with the reason. Computed on the
//!   timeline as written: narration that is synthesized at render time can lengthen scenes and
//!   so move cuts (run with `--synth` to measure it first; scenes are then stretched as the
//!   render would).
//! * `beats.summary` (info): how many cuts will be on a beat.

use std::path::Path;

use serde_json::json;
use vimmer_dsl::{beat_grid, manual_beats, snap_cuts, SnapReport, VideoTimeline};
use vimmer_media::{detect_beats_cached, LOW_CONFIDENCE};

use crate::audio::{analyze, AudioStats, TAIL_MS};
use crate::{scene_path, scene_starts_ms, Finding, Severity, Suggestion};

fn peak_linear(stats: &AudioStats) -> f64 {
    if stats.peak_dbfs.is_finite() {
        10f64.powf(stats.peak_dbfs / 20.0)
    } else {
        0.0
    }
}

/// Checks every sound effect's file, its end and overlapping peaks.
pub fn check(timeline: &VideoTimeline, public_dir: &Path, ffmpeg: &Path) -> Vec<Finding> {
    let mut out = Vec::new();
    let starts = scene_starts_ms(timeline);
    let total = timeline.total_duration_ms();
    // (scene index, sfx index, start ms, end ms, peak after volume)
    let mut placed: Vec<(usize, usize, u64, u64, f64)> = Vec::new();
    for (i, scene) in timeline.scenes.iter().enumerate() {
        for (k, fx) in scene.sfx.iter().enumerate() {
            let Some(at) = fx.time_in(scene) else {
                continue;
            };
            let at = starts[i] + at;
            let path = public_dir.join(&fx.src);
            let field = scene_path(i, &format!("sfx/{k}/src"));
            if !path.is_file() {
                out.push(
                    Finding::new(
                        Severity::Error,
                        "sfx.missing",
                        format!(
                            "sound effect `{}` does not exist ({})",
                            fx.src,
                            path.display()
                        ),
                        Suggestion::field(
                            "pick a file that exists, e.g. one of assets/sfx/*.wav",
                            field,
                        ),
                    )
                    .scene(&scene.id)
                    .at(at),
                );
                continue;
            }
            let stats = match analyze(ffmpeg, &path) {
                Ok(s) => s,
                Err(e) => {
                    out.push(
                        Finding::new(
                            Severity::Error,
                            "sfx.unreadable",
                            format!("sound effect `{}`: {e}", fx.src),
                            Suggestion::field("replace it with a WAV, MP3 or M4A file", field),
                        )
                        .scene(&scene.id)
                        .at(at),
                    );
                    continue;
                }
            };
            let len = stats.duration_ms.unwrap_or(0);
            if at + len > total {
                out.push(
                    Finding::new(
                        Severity::Warning,
                        "sfx.past_video_end",
                        format!(
                            "sound effect `{}` is {len} ms long but the video ends {} ms after it starts: its last {} ms are cut",
                            fx.src,
                            total - at,
                            at + len - total
                        ),
                        Suggestion::text("start it earlier, or lengthen the last scene"),
                    )
                    .scene(&scene.id)
                    .at(at),
                );
            }
            placed.push((
                i,
                k,
                at,
                at + len,
                peak_linear(&stats) * f64::from(fx.volume),
            ));
        }
    }
    if placed.len() < 2 {
        return out;
    }
    let music_peak = timeline
        .music
        .as_ref()
        .filter(|m| !crate::is_url(m.source_path()))
        .and_then(|m| {
            analyze(ffmpeg, &public_dir.join(m.source_path()))
                .ok()
                .map(|s| peak_linear(&s) * f64::from(m.volume))
        })
        .unwrap_or(0.0);
    let mut audio_peaks = vec![0.0; timeline.scenes.len()];
    for (i, scene) in timeline.scenes.iter().enumerate() {
        if let Some(a) = scene.audio.as_ref().filter(|a| !crate::is_url(&a.source)) {
            if let Ok(s) = analyze(ffmpeg, &public_dir.join(&a.source)) {
                audio_peaks[i] = peak_linear(&s);
            }
        }
    }
    for &(i, k, at, _, _) in &placed {
        let overlapping: Vec<&(usize, usize, u64, u64, f64)> =
            placed.iter().filter(|p| p.2 <= at && at < p.3).collect();
        if overlapping.len() < 2 {
            continue;
        }
        // Only report at the later effect's start (the first one starts alone).
        if overlapping.iter().any(|p| p.2 == at && (p.0, p.1) > (i, k)) {
            continue;
        }
        let sum_fx: f64 = overlapping.iter().map(|p| p.4).sum();
        let sum = sum_fx + music_peak + audio_peaks[i];
        if sum <= 1.0 {
            continue;
        }
        let this = placed.iter().find(|p| (p.0, p.1) == (i, k)).unwrap().4;
        let fx = &timeline.scenes[i].sfx[k];
        let room = (1.0 - (sum - this)).max(0.05);
        let volume = (f64::from(fx.volume) * room / this.max(1e-9)).clamp(0.05, 1.0);
        let volume = (volume * 100.0).floor() / 100.0;
        out.push(
            Finding::new(
                Severity::Warning,
                "sfx.clipping",
                format!(
                    "{} sound effects overlap here; with the music and scene audio their peaks add up to {:+.1} dBFS, so the limiter will squash them",
                    overlapping.len(),
                    20.0 * sum.log10()
                ),
                Suggestion::set(
                    format!("lower `{}` to volume {volume}", fx.src),
                    scene_path(i, &format!("sfx/{k}/volume")),
                    json!(volume),
                ),
            )
            .scene(&timeline.scenes[i].id)
            .at(at),
        );
    }
    out
}

/// Beat sync: detection confidence and the cuts that will not snap.
pub fn check_beats(
    timeline: &VideoTimeline,
    public_dir: &Path,
    ffmpeg: &Path,
    narration_ms: Option<&[Option<u64>]>,
) -> Vec<Finding> {
    let mut out = Vec::new();
    let Some(music) = &timeline.music else {
        return out;
    };
    let Some(sync) = music.beat_sync_on() else {
        return out;
    };
    let path = public_dir.join(music.source_path());
    if !path.is_file() || timeline.scenes.len() < 2 {
        return out;
    }
    let (beats, downbeat, file_ms) = match &music.beats {
        Some(grid) => {
            let Ok(stats) = analyze(ffmpeg, &path) else {
                return out;
            };
            let file_ms = stats.duration_ms.unwrap_or(0) as f64;
            (manual_beats(grid, file_ms), 0, file_ms)
        }
        None => {
            let cache = public_dir.join("generated").join("beats");
            let a = match detect_beats_cached(ffmpeg, &path, &cache) {
                Ok(a) => a,
                Err(e) => {
                    out.push(Finding::new(
                        Severity::Warning,
                        "beats.undetected",
                        format!(
                            "could not detect the beats of `{}`: {e}",
                            music.source_path()
                        ),
                        Suggestion::field("set the tempo by hand", "/music/beats"),
                    ));
                    return out;
                }
            };
            if a.confidence < LOW_CONFIDENCE {
                let first = a.beats_ms.first().copied().unwrap_or(0.0).round() as u64;
                out.push(Finding::new(
                    Severity::Warning,
                    "beats.low_confidence",
                    format!(
                        "beat detection is unsure of `{}` (confidence {:.2}, best guess {:.1} BPM), so the render will not snap cuts",
                        music.source_path(), a.confidence, a.bpm
                    ),
                    Suggestion::set(
                        "if the music does have a steady beat, give it by hand (check the tempo by ear)",
                        "/music/beats",
                        json!({"bpm": (a.bpm * 10.0).round() / 10.0, "offset_ms": first}),
                    ),
                ));
                return out;
            }
            (a.beats_ms, a.downbeat, a.duration_ms)
        }
    };
    // Scenes as the render will have them before snapping: stretched to measured narration.
    let mut t = timeline.clone();
    let mut needs = vec![(0, String::new()); t.scenes.len()];
    for (i, scene) in t.scenes.iter_mut().enumerate() {
        if let Some(a) = scene.audio.as_ref().filter(|a| !crate::is_url(&a.source)) {
            if let Ok(s) = analyze(ffmpeg, &public_dir.join(&a.source)) {
                needs[i] = (a.offset_ms + s.duration_ms.unwrap_or(0), "its audio".into());
            }
        } else if let Some(Some(ms)) = narration_ms.and_then(|m| m.get(i)) {
            let need = ms + TAIL_MS;
            scene.duration_ms = scene.duration_ms.max(need);
            needs[i] = (need, "its narration".into());
        }
    }
    let total_ms = t.total_frames() as f64 * 1000.0 / f64::from(t.fps);
    let grid = beat_grid(&beats, downbeat, sync.every, file_ms, total_ms);
    let report: SnapReport = snap_cuts(&mut t, &grid, sync, &needs);
    let fps = t.fps;
    for cut in report.cuts.iter().filter(|c| !c.snapped()) {
        let at = (u64::from(cut.from_frame) * 1000 + u64::from(fps) / 2) / u64::from(fps);
        out.push(
            Finding::new(
                Severity::Warning,
                "beats.unsnapped",
                format!(
                    "the cut into this scene at {at} ms stays off the beat: {}",
                    cut.reason.as_deref().unwrap_or("no beat")
                ),
                Suggestion::set(
                    "allow longer moves (or change the scene lengths)",
                    "/music/beat_sync/max_shift_ms",
                    json!((sync.max_shift_ms * 2).min(1_000)),
                ),
            )
            .scene(&cut.scene_id)
            .at(at),
        );
    }
    out.push(Finding::new(
        Severity::Info,
        "beats.summary",
        report.summary(fps),
        Suggestion::text("nothing to do"),
    ));
    out
}
