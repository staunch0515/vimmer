//! Keyframes that move, shrink or rotate the image so it no longer covers the frame.
//!
//! Before rendering, every image is letterboxed onto a canvas of the output size (blurred
//! padding, `vimmer_media::letterbox`), so at scale 1 with no offset or rotation the image layer
//! covers the frame exactly. `KenBurnsImage.tsx` applies
//! `translate(x%, y%) rotate(r) scale(s)` about the centre, with the percentages relative to the
//! frame. Where the layer does not cover the frame, the blurred backdrop copy shows: an image
//! edge, plain dark at the corners when rotated.
//!
//! The pose is sampled at every frame with the renderer's easing (`motion/keyframes.ts`). For a
//! pose the smallest covering scale has a closed form: every frame corner, mapped back into the
//! layer's coordinates, must lie inside the scaled layer.

use serde_json::json;
use vimmer_dsl::{Easing, Keyframe, Resolution, VideoTimeline};

use crate::{scene_path, scene_starts_ms, Finding, Severity, Suggestion};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub scale: f64,
    pub x: f64,
    pub y: f64,
    pub rotation: f64,
}

fn ease(e: Easing, t: f64) -> f64 {
    match e {
        Easing::Linear => t,
        Easing::EaseIn => t * t,
        Easing::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
        Easing::EaseInOut => {
            if t < 0.5 {
                2.0 * t * t
            } else {
                1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
            }
        }
    }
}

fn pose_of(k: &Keyframe) -> Pose {
    Pose {
        scale: f64::from(k.scale),
        x: f64::from(k.x),
        y: f64::from(k.y),
        rotation: f64::from(k.rotation),
    }
}

/// Pose at `time_ms` and the index of the segment's first keyframe (as `sampleKeyframes`).
pub fn sample(keyframes: &[Keyframe], time_ms: f64) -> (Pose, usize, usize) {
    let first = &keyframes[0];
    if time_ms <= first.at_ms as f64 {
        return (pose_of(first), 0, 0);
    }
    let last_i = keyframes.len() - 1;
    if time_ms >= keyframes[last_i].at_ms as f64 {
        return (pose_of(&keyframes[last_i]), last_i, last_i);
    }
    let mut i = 0;
    while keyframes[i + 1].at_ms as f64 <= time_ms {
        i += 1;
    }
    let (a, b) = (&keyframes[i], &keyframes[i + 1]);
    let t = ease(
        a.ease,
        (time_ms - a.at_ms as f64) / (b.at_ms - a.at_ms) as f64,
    );
    let (pa, pb) = (pose_of(a), pose_of(b));
    let lerp = |x: f64, y: f64| x + (y - x) * t;
    (
        Pose {
            scale: lerp(pa.scale, pb.scale),
            x: lerp(pa.x, pb.x),
            y: lerp(pa.y, pb.y),
            rotation: lerp(pa.rotation, pb.rotation),
        },
        i,
        i + 1,
    )
}

/// Smallest scale at which a layer with this offset and rotation covers the whole frame.
pub fn covering_scale(pose: &Pose, res: Resolution) -> f64 {
    let (w, h) = (f64::from(res.width), f64::from(res.height));
    let (tx, ty) = (pose.x / 100.0 * w, pose.y / 100.0 * h);
    let (sin, cos) = pose.rotation.to_radians().sin_cos();
    let mut need: f64 = 0.0;
    for (cx, cy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        let dx = cx * w / 2.0 - tx;
        let dy = cy * h / 2.0 - ty;
        // Undo the rotation: q = R(-r) d.
        let qx = dx * cos + dy * sin;
        let qy = -dx * sin + dy * cos;
        need = need.max(2.0 * qx.abs() / w).max(2.0 * qy.abs() / h);
    }
    need
}

/// A keyframe as JSON with its `f32` fields rounded to 4 decimals (no `0.4000000059604645`).
fn keyframe_json(k: &Keyframe) -> serde_json::Value {
    let r = |v: f32| (f64::from(v) * 10_000.0).round() / 10_000.0;
    let mut v = serde_json::to_value(k).unwrap_or_default();
    for (field, value) in [
        ("scale", k.scale),
        ("x", k.x),
        ("y", k.y),
        ("opacity", k.opacity),
        ("rotation", k.rotation),
    ] {
        v[field] = json!(r(value));
    }
    v
}

pub fn check(timeline: &VideoTimeline) -> Vec<Finding> {
    let mut out = Vec::new();
    let starts = scene_starts_ms(timeline);
    let fps = f64::from(timeline.fps.max(1));
    for (i, scene) in timeline.scenes.iter().enumerate() {
        if scene.image.is_none() || scene.keyframes.is_empty() {
            continue;
        }
        let kfs = &scene.keyframes;
        if kfs.windows(2).any(|w| w[1].at_ms <= w[0].at_ms) {
            continue; // invalid order; validation reports it
        }
        let frames = scene.duration_frames(timeline.fps);
        let mut first_bad: Option<(f64, f64, f64)> = None; // (time, scale, needed)
        let mut worst_gap: f64 = 0.0;
        let mut needed = vec![0.0f64; kfs.len()];
        for f in 0..frames {
            let t = f64::from(f) * 1000.0 / fps;
            let (pose, a, b) = sample(kfs, t);
            let need = covering_scale(&pose, timeline.resolution);
            if pose.scale + 1e-3 < need {
                if first_bad.is_none() {
                    first_bad = Some((t, pose.scale, need));
                }
                worst_gap = worst_gap.max(need - pose.scale);
                needed[a] = needed[a].max(need);
                needed[b] = needed[b].max(need);
            }
        }
        let Some((t, scale, need)) = first_bad else {
            continue;
        };
        let fixed: Vec<serde_json::Value> = kfs
            .iter()
            .zip(&needed)
            .map(|(k, n)| {
                let mut k = *k;
                if f64::from(k.scale) < *n {
                    k.scale = ((n * 100.0).ceil() / 100.0) as f32;
                }
                keyframe_json(&k)
            })
            .collect();
        out.push(
            Finding::new(
                Severity::Warning,
                "motion.uncovered_edge",
                format!(
                    "keyframes leave part of the frame uncovered from {:.2} s into the scene (scale {scale:.3}, needs {need:.3}; worst shortfall {worst_gap:.3}): the image edge and the dark backdrop show",
                    t / 1000.0
                ),
                Suggestion::set(
                    "raise the scale of the keyframes around that time",
                    scene_path(i, "keyframes"),
                    json!(fixed),
                ),
            )
            .scene(&scene.id)
            .at(starts[i] + t as u64),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const RES: Resolution = Resolution {
        width: 1920,
        height: 1080,
    };

    fn pose(scale: f64, x: f64, y: f64, rotation: f64) -> Pose {
        Pose {
            scale,
            x,
            y,
            rotation,
        }
    }

    #[test]
    fn identity_needs_scale_one() {
        assert!((covering_scale(&pose(1.0, 0.0, 0.0, 0.0), RES) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn offset_needs_twice_the_offset_extra() {
        // 3% right: the left edge needs 6% more width.
        assert!((covering_scale(&pose(1.0, 3.0, 0.0, 0.0), RES) - 1.06).abs() < 1e-9);
        // The built-in pan_left motion (scale 1.08, x ±3%) is covered.
        assert!(covering_scale(&pose(1.08, -3.0, 0.0, 0.0), RES) <= 1.08);
    }

    #[test]
    fn rotation_needs_more_scale() {
        let need = covering_scale(&pose(1.0, 0.0, 0.0, 5.0), RES);
        assert!(need > 1.1 && need < 1.2, "{need}");
    }

    #[test]
    fn sampling_follows_easing() {
        let a = Keyframe {
            at_ms: 0,
            scale: 1.0,
            x: 0.0,
            y: 0.0,
            opacity: 1.0,
            rotation: 0.0,
            ease: Easing::Linear,
        };
        let b = Keyframe {
            at_ms: 1000,
            scale: 2.0,
            ..a
        };
        let (p, _, _) = sample(&[a, b], 500.0);
        assert!((p.scale - 1.5).abs() < 1e-9);
        let a = Keyframe {
            ease: Easing::EaseIn,
            ..a
        };
        let (p, _, _) = sample(&[a, b], 500.0);
        assert!((p.scale - 1.25).abs() < 1e-9);
    }
}
