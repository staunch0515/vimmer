//! The narrator avatar (数字人讲解人).
//!
//! Only timelines where some scene shows an avatar get any of these findings; every other
//! timeline returns nothing here, exactly as before.
//!
//! * `avatar.photo_missing` (error): the photo file is not in the public dir.
//! * `avatar.env_missing` (warning): this machine cannot generate clips (not an Apple-silicon
//!   Mac, or the engine is not installed). The video still renders, without the inset.
//! * `avatar.not_generated` (warning): a clip is not generated yet, with a time estimate. Read
//!   from the status `vimmer avatar status|generate` and renders leave in
//!   `generated/avatar/status.json` under the timeline's plan signature; with no entry the status
//!   is unknown and the estimate comes from the speaking scenes' lengths.
//! * `avatar.over_text` (warning): the inset box covers scene text or a text layer of a scene it
//!   is shown on. The fix is the first free corner for that scene.
//! * `avatar.over_subtitles` (warning): the inset meets a subtitle cue's estimated box while
//!   it is shown.
//!
//! The inset box is `remotion-renderer/src/avatar.ts` `avatarBox`: a square `size` of the frame
//! height, `margin` of the width from the left/right edge and of the height from the top/bottom.
//! A face probe runs Python for seconds, so it is left to `vimmer avatar status`.

use std::path::Path;

use serde_json::json;
use vimmer_dsl::{avatar_plan_signature, Corner, ResolvedAvatar, Resolution, VideoTimeline};
use vimmer_media::{avatar_estimate_seconds, describe_estimate, AvatarCapability as Capability};

use crate::fonts::Fonts;
use crate::layout::SubtitleBand;
use crate::logo::{corner_name, items, overlaps, Edges};
use crate::{scene_starts_ms, Finding, Severity, Suggestion};

/// Where `vimmer avatar` keeps clip status, relative to the public dir.
pub const STATUS_FILE: &str = "generated/avatar/status.json";

const CORNERS: [Corner; 4] = [
    Corner::BottomRight,
    Corner::BottomLeft,
    Corner::TopRight,
    Corner::TopLeft,
];

/// The inset box (left, top, right, bottom) in pixels.
pub fn inset_rect(a: &ResolvedAvatar, corner: Corner, res: Resolution) -> Edges {
    let (w, h) = (f64::from(res.width), f64::from(res.height));
    let size = (f64::from(a.size).clamp(0.05, 1.0) * h).round();
    let (mx, my) = (f64::from(a.margin) * w, f64::from(a.margin) * h);
    let left = match corner {
        Corner::TopLeft | Corner::BottomLeft => mx,
        _ => w - mx - size,
    };
    let top = match corner {
        Corner::TopLeft | Corner::TopRight => my,
        _ => h - my - size,
    };
    // Rounded like the renderer's box.
    let (left, top) = (left.round(), top.round());
    (left, top, left + size, top + size)
}

/// Everything about the avatar. `capability` is passed in so tests can pick it.
pub fn check(
    t: &VideoTimeline,
    public_dir: Option<&Path>,
    fonts: &Fonts,
    capability: &dyn Fn() -> Capability,
) -> Vec<Finding> {
    let shown: Vec<(usize, ResolvedAvatar)> = (0..t.scenes.len())
        .filter_map(|i| t.scene_avatar(i).map(|a| (i, a)))
        .collect();
    if shown.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    out.extend(photos(t, &shown, public_dir));
    let cap = capability();
    let ready = matches!(cap, Capability::Ready { .. });
    if !ready {
        out.push(Finding::new(
            Severity::Warning,
            "avatar.env_missing",
            format!(
                "the narrator avatar (数字人) cannot be generated on this machine: {}. Clips that are already generated still show; others are left out.",
                cap.message()
            ),
            Suggestion::text("generate the avatar on the Apple-silicon Mac that renders videos"),
        ));
    }
    if let Some(dir) = public_dir {
        out.extend(generated(t, &shown, dir));
    }
    out.extend(over_text(t, &shown, fonts));
    out.extend(over_subtitles(t, &shown));
    out
}

fn photo_path(t: &VideoTimeline, i: usize) -> &'static str {
    if t.scenes[i].avatar.as_ref().and_then(|a| a.photo.as_ref()).is_some() {
        return "scene";
    }
    if t.avatar.is_some() {
        "/avatar/photo"
    } else {
        "/brand/avatar/photo"
    }
}

fn photos(t: &VideoTimeline, shown: &[(usize, ResolvedAvatar)], public_dir: Option<&Path>) -> Vec<Finding> {
    let Some(dir) = public_dir else { return Vec::new() };
    let mut seen: Vec<&str> = Vec::new();
    let mut out = Vec::new();
    for (i, a) in shown {
        if seen.contains(&a.photo.as_str()) {
            continue;
        }
        seen.push(&a.photo);
        let p = Path::new(&a.photo);
        if dir.join(p).is_file() || (p.is_absolute() && p.is_file()) {
            continue;
        }
        let path = match photo_path(t, *i) {
            "scene" => format!("/scenes/{i}/avatar/photo"),
            other => other.to_string(),
        };
        out.push(
            Finding::new(
                Severity::Error,
                "avatar.photo_missing",
                format!("the narrator photo `{}` was not found", a.photo),
                Suggestion::field("pick a face photo (png/jpg/webp) that exists", path),
            )
            .scene(&t.scenes[*i].id),
        );
    }
    out
}

/// Whether a scene has speech an avatar can lip-sync to (before the render measures it).
fn speaks(t: &VideoTimeline, i: usize) -> bool {
    let s = &t.scenes[i];
    s.spoken_text().is_some_and(|x| !x.trim().is_empty()) || s.audio.is_some() || !t.voiceover.is_empty()
}

fn generated(t: &VideoTimeline, shown: &[(usize, ResolvedAvatar)], dir: &Path) -> Vec<Finding> {
    let how = "run `vimmer avatar generate <timeline>` on the Mac that renders (it caches the clips; renders then reuse them)";
    let status: Option<Vec<serde_json::Value>> = avatar_plan_signature(t).and_then(|sig| {
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(STATUS_FILE)).ok()?).ok()?;
        doc.get(&sig)?.as_array().cloned()
    });
    match status {
        Some(runs) => {
            let missing: Vec<&serde_json::Value> = runs
                .iter()
                .filter(|r| {
                    let ready = r["ready"].as_bool().unwrap_or(false);
                    let file = r["clip"].as_str().is_some_and(|c| dir.join(c).is_file());
                    !(ready && file)
                })
                .collect();
            if missing.is_empty() {
                return Vec::new();
            }
            let secs: u64 = missing.iter().filter_map(|r| r["estimate_s"].as_u64()).sum();
            let scenes: Vec<&str> = missing
                .iter()
                .flat_map(|r| r["scenes"].as_array().into_iter().flatten())
                .filter_map(|s| s.as_str())
                .collect();
            let first = scenes.first().copied().unwrap_or_default();
            vec![Finding::new(
                Severity::Warning,
                "avatar.not_generated",
                format!(
                    "{} of {} narrator avatar clip(s) are not generated yet (scenes {}); generating takes {}. Until then the video renders without them.",
                    missing.len(),
                    runs.len(),
                    scenes.join(", "),
                    describe_estimate(secs)
                ),
                Suggestion::text(how),
            )
            .scene(first)]
        }
        None => {
            let speaking: Vec<usize> = shown.iter().map(|(i, _)| *i).filter(|&i| speaks(t, i)).collect();
            if speaking.is_empty() {
                return Vec::new();
            }
            let ms: u64 = speaking.iter().map(|&i| t.scenes[i].duration_ms).sum();
            vec![Finding::new(
                Severity::Warning,
                "avatar.not_generated",
                format!(
                    "the narrator avatar clips of this timeline have not been generated (or the timeline changed since); generating takes {}. Until then the video renders without them.",
                    describe_estimate(avatar_estimate_seconds(ms))
                ),
                Suggestion::text(how),
            )
            .scene(&t.scenes[speaking[0]].id)]
        }
    }
}

/// The JSON-patch fix that puts scene `i`'s avatar in `corner`.
fn corner_fix(t: &VideoTimeline, i: usize, corner: Corner, text: String) -> Suggestion {
    if t.scenes[i].avatar.is_some() {
        Suggestion::set(text, format!("/scenes/{i}/avatar/corner"), json!(corner_name(corner)))
    } else {
        Suggestion::set(text, format!("/scenes/{i}/avatar"), json!({ "corner": corner_name(corner) }))
    }
}

fn over_text(t: &VideoTimeline, shown: &[(usize, ResolvedAvatar)], fonts: &Fonts) -> Vec<Finding> {
    let all = items(t, fonts);
    let covers = |i: usize, rect: Edges| {
        all.iter()
            .any(|it| it.scene == i && it.text && it.boxes.iter().any(|b| overlaps(rect, *b)))
    };
    let mut out = Vec::new();
    for (i, a) in shown {
        if !covers(*i, inset_rect(a, a.corner, t.resolution)) {
            continue;
        }
        let free = CORNERS
            .into_iter()
            .find(|&c| c != a.corner && !covers(*i, inset_rect(a, c, t.resolution)));
        let suggestion = match free {
            Some(c) => corner_fix(t, *i, c, format!("move the avatar to {} on this scene", corner_name(c))),
            None => Suggestion::set(
                "every corner covers text here: turn the avatar off on this scene (or make it smaller)",
                format!("/scenes/{i}/avatar"),
                json!({ "enabled": false }),
            ),
        };
        out.push(
            Finding::new(
                Severity::Warning,
                "avatar.over_text",
                format!(
                    "the narrator avatar in the {} corner covers text on this scene",
                    corner_name(a.corner)
                ),
                suggestion,
            )
            .scene(&t.scenes[*i].id),
        );
    }
    out
}

/// A cue's box: centered in the band, as wide as its text (≈0.55 em per Latin character, 1 em per
/// CJK one), two lines when it wraps.
fn cue_rect(band: &SubtitleBand, text: &str) -> (f64, f64, f64, f64) {
    let em: f64 = text
        .chars()
        .map(|c| if (c as u32) >= 0x2E80 { 1.0 } else { 0.55 })
        .sum();
    let width = (em * band.font_px).min(band.text_width);
    let lines = if em * band.font_px > band.text_width { 2 } else { 1 };
    let center = (band.left_x + band.right_x) / 2.0;
    let pad = (band.right_x - band.left_x - band.text_width) / 2.0;
    let half = width / 2.0 + pad;
    let top = band.top_y(lines);
    (center - half, top, 2.0 * half, band.bottom_y - top)
}

fn over_subtitles(t: &VideoTimeline, shown: &[(usize, ResolvedAvatar)]) -> Vec<Finding> {
    if t.subtitles.is_empty() {
        return Vec::new();
    }
    let band = SubtitleBand::for_timeline(t);
    let starts = scene_starts_ms(t);
    for (i, a) in shown {
        let (start, end) = (starts[*i], starts[*i] + t.scenes[*i].duration_ms);
        let rect = inset_rect(a, a.corner, t.resolution);
        let hit = t
            .subtitles
            .iter()
            .find(|c| c.start_ms < end && c.end_ms > start && overlaps(rect, cue_rect(&band, &c.text)));
        if let Some(cue) = hit {
            let top = if matches!(a.corner, Corner::BottomLeft | Corner::TopLeft) {
                Corner::TopLeft
            } else {
                Corner::TopRight
            };
            return vec![Finding::new(
                Severity::Warning,
                "avatar.over_subtitles",
                format!(
                    "the narrator avatar in the {} corner covers part of the subtitles",
                    corner_name(a.corner)
                ),
                corner_fix(t, *i, top, format!("move the avatar to {} (or make it smaller)", corner_name(top))),
            )
            .scene(&t.scenes[*i].id)
            .at(cue.start_ms)];
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use vimmer_dsl::Avatar;

    fn timeline(subs: bool) -> VideoTimeline {
        let mut t: VideoTimeline = serde_json::from_value(json!({
            "fps": 30,
            "resolution": {"width": 1920, "height": 1080},
            "scenes": [
                {"id": "a", "duration_ms": 4000, "text": "Hello there", "narration": {"voice": "zf_001"}},
                {"id": "b", "duration_ms": 4000, "text": null}
            ],
            "subtitles": []
        }))
        .unwrap();
        t.avatar = Some(Avatar::with_photo("me.png"));
        if subs {
            t.subtitles = serde_json::from_value(json!([
                {"start_ms": 0, "end_ms": 3000, "text": "这是一句比较长的字幕，它会一直延伸到画面的右下角那里去吧"}
            ]))
            .unwrap();
        }
        t
    }

    fn ready() -> Capability {
        Capability::Ready { home: "/x".into(), python: "/x/venv/bin/python".into() }
    }

    fn codes(f: &[Finding]) -> Vec<&'static str> {
        f.iter().map(|f| f.code).collect()
    }

    #[test]
    fn no_avatar_no_findings() {
        let mut t = timeline(true);
        t.avatar = None;
        let fonts = Fonts::load(None, None);
        let cap = || panic!("not asked");
        assert!(check(&t, Some(Path::new("/nonexistent")), &fonts, &cap).is_empty());
    }

    #[test]
    fn missing_photo_env_and_subtitles() {
        let t = timeline(true);
        let fonts = Fonts::load(None, None);
        let unsupported = || Capability::Unsupported { reason: "Intel".into() };
        let dir = std::env::temp_dir().join("vimmer-check-avatar-none");
        let f = check(&t, Some(&dir), &fonts, &unsupported);
        let c = codes(&f);
        assert!(c.contains(&"avatar.photo_missing"), "{c:?}");
        assert!(c.contains(&"avatar.env_missing"), "{c:?}");
        assert!(c.contains(&"avatar.over_subtitles"), "{c:?}");
        let sub = f.iter().find(|f| f.code == "avatar.over_subtitles").unwrap();
        assert_eq!(sub.suggestion.path.as_deref(), Some("/scenes/0/avatar"));
        assert_eq!(sub.suggestion.value, Some(json!({"corner": "top_right"})));
        // A short cue in the middle leaves the corner free.
        let mut short = t.clone();
        short.subtitles[0].text = "你好".into();
        assert!(!codes(&check(&short, Some(&dir), &fonts, &ready)).contains(&"avatar.over_subtitles"));
    }

    #[test]
    fn not_generated_reads_the_status_file() {
        let t = timeline(false);
        let fonts = Fonts::load(None, None);
        let dir = std::env::temp_dir().join(format!("vimmer-check-avatar-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("generated/avatar")).unwrap();
        std::fs::write(dir.join("me.png"), b"x").unwrap();
        // No status: unknown, estimate from the speaking scene (4 s → 25 + 44 s).
        let f = check(&t, Some(&dir), &fonts, &ready);
        assert_eq!(codes(&f), vec!["avatar.not_generated"]);
        assert!(f[0].message.contains(&describe_estimate(69)), "{}", f[0].message);
        // Status says ready and the clip exists: nothing.
        let sig = avatar_plan_signature(&t).unwrap();
        let runs = json!([{"scenes": ["a"], "photo": "me.png", "clip": "generated/avatar/k.mp4", "ready": true, "duration_ms": 4000, "estimate_s": 69}]);
        std::fs::write(dir.join(STATUS_FILE), json!({ sig.clone(): runs }).to_string()).unwrap();
        // Ready in the status but the file is gone: still not generated.
        assert_eq!(codes(&check(&t, Some(&dir), &fonts, &ready)), vec!["avatar.not_generated"]);
        std::fs::write(dir.join("generated/avatar/k.mp4"), b"x").unwrap();
        assert!(check(&t, Some(&dir), &fonts, &ready).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn inset_over_text_suggests_a_free_corner() {
        let mut t = timeline(false);
        t.scenes[1].layers = serde_json::from_value(json!([
            {"id": "t", "kind": "text", "text": "Covered", "x": 0.75, "y": 0.75, "width": 0.2, "font_size": 0.07}
        ]))
        .unwrap();
        let fonts = Fonts::load(None, None);
        let f = over_text(&t, &[(1, t.scene_avatar(1).unwrap())], &fonts);
        assert_eq!(codes(&f), vec!["avatar.over_text"]);
        assert_eq!(f[0].suggestion.path.as_deref(), Some("/scenes/1/avatar"));
        assert_eq!(f[0].suggestion.value, Some(json!({"corner": "bottom_left"})));
    }

    #[test]
    fn box_matches_the_renderer() {
        let t = timeline(false);
        let r = t.scene_avatar(0).unwrap();
        let res = Resolution { width: 1920, height: 1080 };
        assert_eq!(inset_rect(&r, Corner::BottomRight, res), (1519.0, 713.0, 1843.0, 1037.0));
        assert_eq!(inset_rect(&r, Corner::TopLeft, res).0, 77.0);
    }
}
