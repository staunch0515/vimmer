//! Music ducking as a volume envelope computed from the timeline, not from the signal.
//!
//! The scheduler knows exactly when every scene's audio plays, so the envelope is built ahead of
//! time: the music is fully down when speech starts (the attack ramp ends at the onset, so the
//! first syllable is never masked), stays down while it plays, and comes back up over the
//! release ramp after it ends. The ramps are linear in dB. The same timeline always gives the
//! same envelope, unlike a compressor keyed on the voice, which reacts after the onset and
//! depends on how loud each clip is.

use vimmer_dsl::Duck;

/// Stretches of time, in milliseconds on the output timeline, during which the music is held
/// fully down. Speech intervals closer together than `attack + release` are merged, so the
/// music does not bob up for a moment between two lines.
pub fn duck_segments(speech: &[(f64, f64)], duck: &Duck) -> Vec<(f64, f64)> {
    if !duck.enabled || duck.depth_db == 0.0 {
        return Vec::new();
    }
    let bridge = (duck.attack_ms + duck.release_ms) as f64;
    let mut spans: Vec<(f64, f64)> = speech.iter().copied().filter(|(s, e)| e > s).collect();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (start, end) in spans {
        match merged.last_mut() {
            Some(last) if start - last.1 < bridge => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

/// How far down the music is at `t_ms`, from 0 (full level) to 1 (fully ducked).
/// Because [`duck_segments`] leaves at least `attack + release` between segments, the ramps of
/// neighbouring segments never overlap, so the amounts can simply be added.
pub fn duck_amount(segments: &[(f64, f64)], duck: &Duck, t_ms: f64) -> f64 {
    let (attack, release) = (duck.attack_ms as f64, duck.release_ms as f64);
    segments
        .iter()
        .map(|&(s, e)| {
            ((t_ms - (s - attack)) / attack)
                .min((e + release - t_ms) / release)
                .clamp(0.0, 1.0)
        })
        .sum::<f64>()
        .min(1.0)
}

/// Music gain change from ducking at `t_ms`, in dB (0 when not ducked, `depth_db` when fully down).
pub fn duck_gain_db(segments: &[(f64, f64)], duck: &Duck, t_ms: f64) -> f64 {
    f64::from(duck.depth_db) * duck_amount(segments, duck, t_ms)
}

/// The envelope as an ffmpeg `volume` filter expression of `t` (seconds). It computes exactly
/// [`duck_gain_db`] turned into a linear gain. `None` when nothing is ducked.
pub fn volume_expr(segments: &[(f64, f64)], duck: &Duck) -> Option<String> {
    if segments.is_empty() {
        return None;
    }
    let (attack, release) = (
        duck.attack_ms as f64 / 1000.0,
        duck.release_ms as f64 / 1000.0,
    );
    let terms: Vec<String> = segments
        .iter()
        .map(|&(s, e)| {
            let (s, e) = (s / 1000.0, e / 1000.0);
            format!(
                "clip(min((t-{:.6})/{attack:.6},({:.6}-t)/{release:.6}),0,1)",
                s - attack,
                e + release
            )
        })
        .collect();
    Some(format!(
        "pow(10,{:.4}*min(1,{})/20)",
        duck.depth_db,
        terms.join("+")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn duck() -> Duck {
        Duck::default()
    }

    #[test]
    fn close_speech_is_merged_and_far_speech_is_not() {
        // 600 ms bridge (200 attack + 400 release).
        let segs = duck_segments(
            &[(5_000.0, 6_000.0), (1_000.0, 2_000.0), (2_500.0, 3_000.0)],
            &duck(),
        );
        assert_eq!(segs, vec![(1_000.0, 3_000.0), (5_000.0, 6_000.0)]);
        // Exactly the bridge apart: the music just touches full level, so they stay apart.
        let segs = duck_segments(&[(0.0, 1_000.0), (1_600.0, 2_000.0)], &duck());
        assert_eq!(segs.len(), 2);
    }

    #[test]
    fn disabled_or_zero_depth_or_no_speech_ducks_nothing() {
        let speech = [(1_000.0, 2_000.0)];
        assert!(duck_segments(
            &speech,
            &Duck {
                enabled: false,
                ..duck()
            }
        )
        .is_empty());
        assert!(duck_segments(
            &speech,
            &Duck {
                depth_db: 0.0,
                ..duck()
            }
        )
        .is_empty());
        assert!(duck_segments(&[], &duck()).is_empty());
        assert!(duck_segments(&[(1_000.0, 1_000.0)], &duck()).is_empty());
        assert_eq!(volume_expr(&[], &duck()), None);
    }

    #[test]
    fn envelope_is_down_at_the_onset_and_back_up_after_the_release() {
        let d = duck();
        let segs = duck_segments(&[(1_000.0, 2_000.0)], &d);
        let at = |t: f64| duck_gain_db(&segs, &d, t);
        assert_eq!(at(0.0), 0.0);
        assert_eq!(at(800.0), 0.0, "attack starts 200 ms before the onset");
        assert!(
            (at(900.0) - -6.0).abs() < 1e-9,
            "halfway down, linear in dB"
        );
        assert_eq!(at(1_000.0), -12.0, "fully down at the onset");
        assert_eq!(at(1_500.0), -12.0);
        assert_eq!(at(2_000.0), -12.0);
        assert!((at(2_200.0) - -6.0).abs() < 1e-9, "halfway back up");
        assert_eq!(at(2_400.0), 0.0);
        assert_eq!(at(9_000.0), 0.0);
    }

    #[test]
    fn envelope_is_monotonic_on_each_ramp_and_never_below_depth() {
        let d = Duck {
            depth_db: -18.0,
            attack_ms: 150,
            release_ms: 700,
            ..duck()
        };
        let segs = duck_segments(
            &[(300.0, 900.0), (1_100.0, 1_400.0), (4_000.0, 4_100.0)],
            &d,
        );
        let mut prev = 0.0;
        let mut t = 0.0;
        while t < 6_000.0 {
            let g = duck_gain_db(&segs, &d, t);
            assert!((-18.0..=0.0).contains(&g), "{t}: {g}");
            if (4_100.0..5_000.0).contains(&t) {
                assert!(g >= prev - 1e-9, "release never goes back down");
            }
            prev = g;
            t += 5.0;
        }
        // Speech starting at t=0 is already partly ducked at the first sample, not a jump later.
        let segs = duck_segments(&[(0.0, 500.0)], &d);
        assert_eq!(duck_gain_db(&segs, &d, 0.0), -18.0);
    }

    #[test]
    fn expression_encodes_the_segments() {
        let d = duck();
        let segs = duck_segments(&[(1_000.0, 2_000.0), (5_000.0, 5_500.0)], &d);
        assert_eq!(
            volume_expr(&segs, &d).unwrap(),
            "pow(10,-12.0000*min(1,\
             clip(min((t-0.800000)/0.200000,(2.400000-t)/0.400000),0,1)+\
             clip(min((t-4.800000)/0.200000,(5.900000-t)/0.400000),0,1))/20)"
        );
    }
}
