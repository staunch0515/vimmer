//! Video clips (`scene.video` and `video` layers): files that are missing or cannot be decoded,
//! clips enlarged so much they look soft, clips that run out and freeze, very high speeds, and
//! timelines whose clips are expensive to render.
//!
//! * `video.missing` / `video.undecodable` (error): the render would fail.
//! * `video.trim_past_end` (error): `trim_start_ms` is at or after the end of the file.
//! * `assets.low_resolution` (warning above 2.0x, info above 1.5x): the clip's pixels are enlarged
//!   that much to fill its box (the frame, for a background), as for images.
//! * `video.freezes` (info): with `fit_time: freeze`, the clip (trimmed, at its speed) ends before
//!   its scene or layer, so its last frame holds for the rest.
//! * `video.fast` (warning): the clip plays faster than [`FAST_SPEED`] (its own speed or, with
//!   `stretch`, the fitted one): motion turns into a blur of jumps.
//! * `video.heavy` (warning): more than [`HEAVY_CONCURRENT`] clips on screen at once in a scene, or
//!   more decoded clip frames than [`HEAVY_FRAMES_RATIO`] times the video's frames. Each clip frame
//!   is extracted and composited for every output frame, which dominates render time on a CPU.

use std::path::Path;

use vimmer_dsl::{clip_frames, FitTime, ImageFit, LayerContent, VideoClip, VideoTimeline};
use vimmer_media::probe_video;

use crate::assets::{UPSCALE_INFO, UPSCALE_WARN};
use crate::{is_url, scene_path, scene_starts_ms, secs, Finding, Severity, Suggestion};

/// Speeds above this are a warning.
pub const FAST_SPEED: f64 = 3.0;
/// More clips than this visible at once in one scene is a warning.
pub const HEAVY_CONCURRENT: usize = 2;
/// Decoded clip frames above this many times the output frames is a warning.
pub const HEAVY_FRAMES_RATIO: f64 = 2.0;

struct Place<'a> {
    scene: usize,
    layer: Option<usize>,
    clip: &'a VideoClip,
    /// Start and end of the clip's time in the scene, in frames.
    from: u64,
    to: u64,
    /// Box size in px.
    box_w: f64,
    box_h: f64,
    zoom: f64,
}

fn places(timeline: &VideoTimeline) -> Vec<Place<'_>> {
    let fps = timeline.fps;
    let frames = |ms: u64| (ms * u64::from(fps) + 500) / 1000;
    let (w, h) = (
        f64::from(timeline.resolution.width),
        f64::from(timeline.resolution.height),
    );
    let mut out = Vec::new();
    for (si, scene) in timeline.scenes.iter().enumerate() {
        let scene_frames = u64::from(scene.duration_frames(fps));
        if let Some(clip) = &scene.video {
            let zoom = if scene.keyframes.is_empty() {
                match scene.motion {
                    vimmer_dsl::MotionType::Static => 1.0,
                    vimmer_dsl::MotionType::ZoomIn | vimmer_dsl::MotionType::ZoomOut => 1.12,
                    _ => 1.08,
                }
            } else {
                scene
                    .keyframes
                    .iter()
                    .map(|k| f64::from(k.scale))
                    .fold(1.0, f64::max)
            };
            out.push(Place {
                scene: si,
                layer: None,
                clip,
                from: 0,
                to: scene_frames,
                box_w: w,
                box_h: h,
                zoom,
            });
        }
        for (li, layer) in scene.layers.iter().enumerate() {
            if let LayerContent::Video(clip) = &layer.content {
                let to = layer
                    .end_ms
                    .map_or(scene_frames, |e| frames(e).min(scene_frames));
                out.push(Place {
                    scene: si,
                    layer: Some(li),
                    clip,
                    from: frames(layer.start_ms),
                    to,
                    box_w: f64::from(layer.width) * w,
                    box_h: f64::from(layer.height.unwrap_or(0.0)) * h,
                    zoom: layer
                        .keyframes
                        .iter()
                        .map(|k| f64::from(k.scale))
                        .fold(1.0, f64::max),
                });
            }
        }
    }
    out
}

fn src_path(p: &Place) -> String {
    match p.layer {
        None => scene_path(p.scene, "video/src"),
        Some(li) => format!("/scenes/{}/layers/{li}/src", p.scene),
    }
}

fn field_path(p: &Place, field: &str) -> String {
    match p.layer {
        None => scene_path(p.scene, &format!("video/{field}")),
        Some(li) => format!("/scenes/{}/layers/{li}/{field}", p.scene),
    }
}

fn name(timeline: &VideoTimeline, p: &Place) -> String {
    match p.layer {
        None => "background video".into(),
        Some(li) => format!("video layer `{}`", timeline.scenes[p.scene].layers[li].id),
    }
}

/// Checks that need the files: existence, decoding, trim against the file length, resolution and
/// freezing clips.
pub fn check(timeline: &VideoTimeline, public_dir: &Path, ffmpeg: &Path) -> Vec<Finding> {
    let mut out = Vec::new();
    let starts = scene_starts_ms(timeline);
    let fps = timeline.fps;
    for p in places(timeline) {
        let scene = &timeline.scenes[p.scene];
        let rel = p.clip.src.as_str();
        if is_url(rel) {
            continue;
        }
        let at = starts[p.scene] + p.from * 1000 / u64::from(fps);
        let what = name(timeline, &p);
        let finding = |sev, code, msg: String, sug: Suggestion| {
            Finding::new(sev, code, format!("{what}: {msg}"), sug)
                .scene(&scene.id)
                .at(at)
        };
        let path = public_dir.join(rel);
        if !path.is_file() {
            out.push(finding(
                Severity::Error,
                "video.missing",
                format!("`{rel}` does not exist ({})", path.display()),
                Suggestion::field(
                    format!(
                        "add the file under {} or point `src` at an existing clip",
                        public_dir.display()
                    ),
                    src_path(&p),
                ),
            ));
            continue;
        }
        let info = match probe_video(ffmpeg, &path) {
            Ok(info) => info,
            Err(e) => {
                out.push(finding(
                    Severity::Error,
                    "video.undecodable",
                    format!("`{rel}` cannot be used: {e}"),
                    Suggestion::field(
                        "convert it to an H.264 .mp4 (ffmpeg -i in -c:v libx264 -pix_fmt yuv420p out.mp4)",
                        src_path(&p),
                    ),
                ));
                continue;
            }
        };
        if p.clip.trim_start_ms >= info.duration_ms {
            out.push(finding(
                Severity::Error,
                "video.trim_past_end",
                format!(
                    "trim_start_ms ({}) is at or after the end of `{rel}` ({})",
                    p.clip.trim_start_ms,
                    secs(info.duration_ms)
                ),
                Suggestion::field("start the clip earlier", field_path(&p, "trim_start_ms")),
            ));
            continue;
        }

        // Resolution against the box.
        let (iw, ih) = (f64::from(info.width), f64::from(info.height));
        if p.box_w > 0.0 && p.box_h > 0.0 {
            let fit = match p.clip.fit {
                ImageFit::Cover => (p.box_w / iw).max(p.box_h / ih),
                ImageFit::Contain => (p.box_w / iw).min(p.box_h / ih),
            };
            let upscale = fit * p.zoom;
            let severity = if upscale > UPSCALE_WARN {
                Some(Severity::Warning)
            } else if upscale > UPSCALE_INFO {
                Some(Severity::Info)
            } else {
                None
            };
            if let Some(sev) = severity {
                out.push(finding(
                    sev,
                    "assets.low_resolution",
                    format!(
                        "`{rel}` is {iw}x{ih} and is enlarged {upscale:.1}x in its {:.0}x{:.0} px box, so it will look soft",
                        p.box_w, p.box_h
                    ),
                    Suggestion::field(
                        format!(
                            "use a version of at least {}x{} px, or make the box smaller",
                            (iw * upscale).ceil(),
                            (ih * upscale).ceil()
                        ),
                        src_path(&p),
                    ),
                ));
            }
        }

        // Length against its time.
        let end = p
            .clip
            .trim_end_ms
            .unwrap_or(info.duration_ms)
            .min(info.duration_ms);
        let source_ms = end.saturating_sub(p.clip.trim_start_ms);
        let span = p.to.saturating_sub(p.from).max(1);
        let span_ms = span * 1000 / u64::from(fps);
        let speed = p.clip.effective_speed(source_ms, span_ms);
        let frames = clip_frames(source_ms, speed, fps);
        if p.clip.fit_time == FitTime::Freeze && frames < span {
            let clip_ms = frames * 1000 / u64::from(fps);
            out.push(
                finding(
                    Severity::Info,
                    "video.freezes",
                    format!(
                        "the clip lasts {} (trimmed, at {speed}x) but is shown {}, so its last frame holds for {}",
                        secs(clip_ms),
                        secs(span_ms),
                        secs(span_ms - clip_ms)
                    ),
                    Suggestion::set(
                        "loop it, stretch it to fit, or shorten the scene/layer",
                        field_path(&p, "fit_time"),
                        serde_json::json!("loop"),
                    ),
                )
                .at(at + clip_ms),
            );
        }
        if p.clip.fit_time == FitTime::Stretch && speed > FAST_SPEED {
            out.push(finding(
                Severity::Warning,
                "video.fast",
                format!(
                    "stretching {} of clip into {} plays it at {speed:.1}x",
                    secs(source_ms),
                    secs(span_ms)
                ),
                Suggestion::field(
                    "trim the clip shorter, or use fit_time: freeze or loop",
                    field_path(&p, "trim_end_ms"),
                ),
            ));
        }
    }
    out
}

/// Checks from the timeline alone: very high speeds and the render cost of all clips.
pub fn check_cost(timeline: &VideoTimeline) -> Vec<Finding> {
    let mut out = Vec::new();
    let starts = scene_starts_ms(timeline);
    let all = places(timeline);
    for p in &all {
        let speed = f64::from(p.clip.speed);
        if p.clip.fit_time != FitTime::Stretch && speed > FAST_SPEED {
            out.push(
                Finding::new(
                    Severity::Warning,
                    "video.fast",
                    format!(
                        "{} plays at {speed}x; above {FAST_SPEED}x motion turns into jumps",
                        name(timeline, p)
                    ),
                    Suggestion::set(
                        format!("use a speed of at most {FAST_SPEED}"),
                        field_path(p, "speed"),
                        serde_json::json!(2.0),
                    ),
                )
                .scene(&timeline.scenes[p.scene].id)
                .at(starts[p.scene]),
            );
        }
    }
    for (si, scene) in timeline.scenes.iter().enumerate() {
        let here: Vec<&Place> = all.iter().filter(|p| p.scene == si).collect();
        // Most clips visible on one frame.
        let peak = here
            .iter()
            .map(|a| {
                here.iter()
                    .filter(|b| b.from <= a.from && a.from < b.to)
                    .count()
            })
            .max()
            .unwrap_or(0);
        if peak > HEAVY_CONCURRENT {
            out.push(
                Finding::new(
                    Severity::Warning,
                    "video.heavy",
                    format!(
                        "{peak} video clips are on screen at once; each one is decoded for every frame, which slows the render a lot"
                    ),
                    Suggestion::text("show fewer clips at the same time, or use a still image for some"),
                )
                .scene(&scene.id)
                .at(starts[si]),
            );
        }
    }
    let decoded: u64 = all.iter().map(|p| p.to.saturating_sub(p.from)).sum();
    let total = u64::from(timeline.total_frames()).max(1);
    if decoded as f64 > HEAVY_FRAMES_RATIO * total as f64 {
        out.push(Finding::new(
            Severity::Warning,
            "video.heavy",
            format!(
                "the clips add up to {decoded} decoded frames for a {total}-frame video ({:.1}x); expect a long render",
                decoded as f64 / total as f64
            ),
            Suggestion::text("shorten or remove some video layers"),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn public() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../remotion-renderer/public")
    }

    fn timeline(scenes: serde_json::Value) -> VideoTimeline {
        serde_json::from_value(serde_json::json!({
            "fps": 30, "resolution": {"width": 1920, "height": 1080}, "subtitles": [], "scenes": scenes
        }))
        .unwrap()
    }

    fn codes(f: &[Finding]) -> Vec<&str> {
        f.iter().map(|f| f.code).collect()
    }

    #[test]
    fn missing_undecodable_and_freezing_clips() {
        let t = timeline(serde_json::json!([
            {"id": "a", "duration_ms": 3000, "video": {"src": "assets/clips/nope.mp4"}},
            {"id": "b", "duration_ms": 3000, "video": {"src": "assets/sample.png"}},
            {"id": "c", "duration_ms": 6000, "video": {"src": "assets/clips/counter-6s.mp4", "trim_end_ms": 2000, "speed": 2}},
            {"id": "d", "duration_ms": 3000, "video": {"src": "assets/clips/counter-6s.mp4", "trim_start_ms": 9000}}
        ]));
        let f = check(&t, &public(), Path::new("ffmpeg"));
        assert_eq!(
            codes(&f),
            [
                "video.missing",
                "video.undecodable",
                "assets.low_resolution",
                "video.freezes",
                "video.trim_past_end"
            ],
            "{f:#?}"
        );
        // 2 s at 2x = 1 s of a 6 s scene.
        assert!(f[1].message.contains("not a video"), "{}", f[1].message);
        assert!(f[3].message.contains("lasts 1.00 s"), "{}", f[3].message);
        assert_eq!(f[3].severity, Severity::Info);
    }

    #[test]
    fn small_clip_in_a_big_box_is_soft_and_loops_do_not_freeze() {
        // life-tone is 320x180: 6x in the full frame.
        let t = timeline(serde_json::json!([
            {"id": "a", "duration_ms": 3000, "video": {"src": "assets/clips/life-tone-4s.mp4", "fit_time": "loop"},
             "layers": [{"id": "pip", "kind": "video", "x": 0.6, "y": 0.6, "width": 0.3, "height": 0.3,
                         "src": "assets/clips/life-tone-4s.mp4"}]}
        ]));
        let f = check(&t, &public(), Path::new("ffmpeg"));
        assert_eq!(
            codes(&f),
            ["assets.low_resolution", "assets.low_resolution"],
            "{f:#?}"
        );
        assert_eq!(f[0].severity, Severity::Warning);
        assert_eq!(
            f[1].severity,
            Severity::Info,
            "480x270 box from 320x180: 1.5x"
        );
        assert!(f[1].message.contains("video layer `pip`"));
    }

    #[test]
    fn speed_and_cost() {
        let layer = |id: &str| {
            serde_json::json!({"id": id, "kind": "video", "x": 0, "y": 0, "width": 0.2, "height": 0.2,
                               "src": "assets/clips/counter-6s.mp4", "speed": 4})
        };
        let t = timeline(serde_json::json!([
            {"id": "a", "duration_ms": 2000, "video": {"src": "assets/clips/counter-6s.mp4", "fit_time": "stretch"},
             "layers": [layer("v1"), layer("v2")]}
        ]));
        let f = check_cost(&t);
        assert_eq!(
            codes(&f),
            ["video.fast", "video.fast", "video.heavy", "video.heavy"],
            "{f:#?}"
        );
        // Stretching 6 s into 2 s is 3x: not above the limit. Into 1 s it would be.
        let f = check(&t, &public(), Path::new("ffmpeg"));
        assert!(!codes(&f).contains(&"video.fast"), "{f:#?}");
        let t = timeline(serde_json::json!([
            {"id": "a", "duration_ms": 1000, "video": {"src": "assets/clips/counter-6s.mp4", "fit_time": "stretch"}}
        ]));
        assert_eq!(
            codes(&check(&t, &public(), Path::new("ffmpeg"))),
            ["assets.low_resolution", "video.fast"]
        );
        assert!(check_cost(&t).is_empty());
    }
}
