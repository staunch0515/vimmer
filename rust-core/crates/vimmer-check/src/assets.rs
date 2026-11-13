//! Image assets: missing or unreadable files, aspect ratios far from the canvas, and images so
//! small they are upscaled blurry.
//!
//! The renderer fits the whole image into the frame with blurred padding (never crops it), so
//! a mismatched aspect ratio shows as a small picture on a large blurred background.
//!
//! * Coverage (share of the frame the picture itself fills): below 40% a warning, below 60% info.
//!   A square image on 16:9 covers 56%, a 16:9 image on 9:16 covers 32%.
//! * Upscale (fit factor times the largest zoom of the scene's motion or keyframes): above 2.0x a
//!   warning, above 1.5x info. Above about 1.5x soft detail starts to show at 1080p.

use std::path::Path;

use serde_json::json;
use vimmer_dsl::{MotionType, Scene, VideoTimeline};

use crate::{is_url, scene_path, scene_starts_ms, Finding, Severity, Suggestion};

pub const COVERAGE_WARN: f64 = 0.40;
pub const COVERAGE_INFO: f64 = 0.60;
pub const UPSCALE_WARN: f64 = 2.0;
pub const UPSCALE_INFO: f64 = 1.5;

/// Largest zoom the scene applies to its image layer.
fn max_zoom(scene: &Scene) -> f64 {
    if !scene.keyframes.is_empty() {
        return scene
            .keyframes
            .iter()
            .map(|k| f64::from(k.scale))
            .fold(1.0, f64::max);
    }
    match scene.motion {
        MotionType::Static => 1.0,
        MotionType::ZoomIn | MotionType::ZoomOut => 1.12,
        MotionType::PanLeft | MotionType::PanRight => 1.08,
    }
}

pub fn check(timeline: &VideoTimeline, public_dir: &Path) -> Vec<Finding> {
    let mut out = Vec::new();
    let starts = scene_starts_ms(timeline);
    let (cw, ch) = (
        f64::from(timeline.resolution.width),
        f64::from(timeline.resolution.height),
    );
    for (i, scene) in timeline.scenes.iter().enumerate() {
        let Some(rel) = scene.image.as_deref() else {
            continue;
        };
        if is_url(rel) {
            continue;
        }
        let path = public_dir.join(rel);
        let at = starts[i];
        if !path.is_file() {
            out.push(
                Finding::new(
                    Severity::Error,
                    "assets.missing_image",
                    format!("image `{rel}` does not exist ({})", path.display()),
                    Suggestion::field(
                        format!(
                            "add the file under {} or point `image` at an existing one",
                            public_dir.display()
                        ),
                        scene_path(i, "image"),
                    ),
                )
                .scene(&scene.id)
                .at(at),
            );
            continue;
        }
        let (w, h) = match image::image_dimensions(&path) {
            Ok((w, h)) if w > 0 && h > 0 => (f64::from(w), f64::from(h)),
            Ok(_) | Err(_) => {
                out.push(
                    Finding::new(
                        Severity::Error,
                        "assets.unreadable_image",
                        format!("image `{rel}` is not a PNG or JPEG the renderer can normalize"),
                        Suggestion::field("convert it to PNG or JPEG", scene_path(i, "image")),
                    )
                    .scene(&scene.id)
                    .at(at),
                );
                continue;
            }
        };
        let fit = (cw / w).min(ch / h);
        let coverage = (w * fit) * (h * fit) / (cw * ch);
        let cover_scale = ((cw / (w * fit)).max(ch / (h * fit)) * 100.0).ceil() / 100.0;
        let severity = if coverage < COVERAGE_WARN {
            Some(Severity::Warning)
        } else if coverage < COVERAGE_INFO {
            Some(Severity::Info)
        } else {
            None
        };
        if let Some(severity) = severity {
            let suggestion = if scene.keyframes.is_empty() {
                Suggestion::set(
                    format!("crop the image to {}:{}, or zoom it to fill the frame (scale {cover_scale}, crops its edges)", timeline.resolution.width, timeline.resolution.height),
                    scene_path(i, "keyframes"),
                    json!([{ "at_ms": 0, "scale": cover_scale }]),
                )
            } else {
                Suggestion::text(format!(
                    "crop the image to the frame's aspect ratio, or keep keyframe scales at {cover_scale} or more"
                ))
            };
            out.push(
                Finding::new(
                    severity,
                    "assets.aspect_ratio",
                    format!(
                        "image `{rel}` is {w}x{h} (aspect {:.2}) on a {cw}x{ch} frame (aspect {:.2}): the picture fills {:.0}% of the frame, the rest is blurred padding",
                        w / h,
                        cw / ch,
                        coverage * 100.0
                    ),
                    suggestion,
                )
                .scene(&scene.id)
                .at(at),
            );
        }
        let upscale = fit * max_zoom(scene);
        let severity = if upscale > UPSCALE_WARN {
            Some(Severity::Warning)
        } else if upscale > UPSCALE_INFO {
            Some(Severity::Info)
        } else {
            None
        };
        if let Some(severity) = severity {
            // Shown pixel for pixel at the largest zoom.
            let min_w = (w * upscale).ceil();
            let min_h = (h * upscale).ceil();
            out.push(
                Finding::new(
                    severity,
                    "assets.low_resolution",
                    format!("image `{rel}` is {w}x{h} and is enlarged {upscale:.1}x on screen, so it will look soft"),
                    Suggestion::field(
                        format!("use a version of at least {min_w}x{min_h} px"),
                        scene_path(i, "image"),
                    ),
                )
                .scene(&scene.id)
                .at(at),
            );
        }
    }
    out
}
