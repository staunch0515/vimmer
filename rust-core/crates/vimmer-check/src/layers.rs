//! Scene layers (`scene.layers`): text layers get the scene-text checks (overflow, title-safe
//! area, reading speed), image and icon layers the asset checks, and a layer that never shows
//! inside the frame is a warning.
//!
//! Geometry mirrors `remotion-renderer/src/motion/layers.ts` and `components/LayerKinds.tsx`:
//! the box is `x`/`width` of the frame width and `y`/`height` of the frame height, placed by its
//! top-left corner or (with `anchor: center`) its center. Text is set like the scene text (word
//! spans with a 0.25em gap, letter spacing -0.02em) at `font_size` of the frame height, inside
//! the box minus the background padding. Rotation is ignored; keyframe offsets and scales are
//! taken into account only for "is it ever on screen".

use std::path::Path;

use serde_json::{json, Value};
use vimmer_dsl::{
    typing_units, Anchor, ImageFit, Layer, LayerContent, LowerThirdLayer, Resolution, Scene,
    TextLayer, TextReveal, VideoTimeline,
};

use crate::assets::{UPSCALE_INFO, UPSCALE_WARN};
use crate::layout::{title_safe, SceneTextArea, SCENE_LETTER_SPACING_EM, SCENE_WORD_GAP_EM};
use crate::text::{capacity_chars, cps_limit, reading_chars, scene_text_lines, Lang, OPTIMISTIC};
use crate::{is_url, scene_starts_ms, Finding, Severity, Suggestion};

fn layer_path(scene: usize, layer: usize, field: &str) -> String {
    format!("/scenes/{scene}/layers/{layer}/{field}")
}

/// (left, top, width, height) of a box in px, before rotation.
type Rect = (f64, f64, f64, f64);

fn place(layer: &Layer, res: Resolution, height_px: f64) -> Rect {
    let (w, h) = (f64::from(res.width), f64::from(res.height));
    let bw = f64::from(layer.width) * w;
    let (mut left, mut top) = (f64::from(layer.x) * w, f64::from(layer.y) * h);
    if layer.anchor == Anchor::Center {
        left -= bw / 2.0;
        top -= height_px / 2.0;
    }
    (left, top, bw, height_px)
}

/// Where a text layer's words go: the line box and how many lines fit (`u32::MAX` without a
/// fixed height).
fn text_area(t: &TextLayer, layer: &Layer, res: Resolution) -> (SceneTextArea, f64) {
    let h = f64::from(res.height);
    let font_px = f64::from(t.font_size) * h;
    let pad = t
        .background
        .as_ref()
        .map_or(0.0, |b| f64::from(b.padding) * font_px);
    let line_height_px = font_px * f64::from(t.line_height);
    let line_width = (f64::from(layer.width) * f64::from(res.width) - 2.0 * pad).max(1.0);
    let max_lines = match layer.height {
        Some(bh) => ((f64::from(bh) * h - 2.0 * pad) / line_height_px)
            .floor()
            .max(0.0) as u32,
        None => u32::MAX,
    };
    (
        SceneTextArea {
            font_px,
            letter_spacing_em: SCENE_LETTER_SPACING_EM,
            word_gap_em: SCENE_WORD_GAP_EM,
            line_height_px,
            line_width,
            max_lines,
            bottom_y: 0.0,
        },
        pad,
    )
}

/// Every box the layer takes at its keyframes (or its one box without keyframes).
fn poses(layer: &Layer, rect: Rect, res: Resolution) -> Vec<Rect> {
    if layer.keyframes.is_empty() {
        return vec![rect];
    }
    let (w, h) = (f64::from(res.width), f64::from(res.height));
    let (l, t, bw, bh) = rect;
    let (cx, cy) = (l + bw / 2.0, t + bh / 2.0);
    layer
        .keyframes
        .iter()
        .map(|k| {
            let s = f64::from(k.scale);
            let (nw, nh) = (bw * s, bh * s);
            let (nx, ny) = (
                cx + f64::from(k.x) / 100.0 * w,
                cy + f64::from(k.y) / 100.0 * h,
            );
            (nx - nw / 2.0, ny - nh / 2.0, nw, nh)
        })
        .collect()
}

fn on_screen(r: Rect, res: Resolution) -> bool {
    let (l, t, w, h) = r;
    // A line has no area; give it a pixel so it counts when it crosses the frame.
    let (w, h) = (w.max(1.0), h.max(1.0));
    l < f64::from(res.width) && t < f64::from(res.height) && l + w > 0.0 && t + h > 0.0
}

pub fn check(timeline: &VideoTimeline) -> Vec<Finding> {
    let mut out = Vec::new();
    let res = timeline.resolution;
    let starts = scene_starts_ms(timeline);
    for (si, scene) in timeline.scenes.iter().enumerate() {
        for (li, layer) in scene.layers.iter().enumerate() {
            let at = starts[si] + layer.start_ms;
            let name = format!("layer `{}`", layer.id);
            let fixed_h = layer.height.map(|v| f64::from(v) * f64::from(res.height));
            let mut box_h = fixed_h.unwrap_or(0.0);

            if let LayerContent::Text(t) = &layer.content {
                let (area, pad) = text_area(t, layer, res);
                let nominal = scene_text_lines(&t.text, &area, 1.0);
                let optimistic = scene_text_lines(&t.text, &area, OPTIMISTIC);
                if fixed_h.is_none() {
                    box_h = f64::from(nominal) * area.line_height_px + 2.0 * pad;
                }
                if nominal > area.max_lines {
                    let severity = if optimistic > area.max_lines {
                        Severity::Error
                    } else {
                        Severity::Warning
                    };
                    let cap =
                        capacity_chars(&t.text, area.line_width, area.font_px, area.max_lines);
                    out.push(
                        Finding::new(
                            severity,
                            "text.overflow",
                            format!(
                                "{name} needs about {nominal} lines at {:.0} px but its box fits {}; the text runs out of the box",
                                area.font_px, area.max_lines
                            ),
                            Suggestion::field(
                                format!("shorten the text to about {cap} characters, make the box taller or the font smaller"),
                                layer_path(si, li, "text"),
                            ),
                        )
                        .scene(&scene.id)
                        .at(at),
                    );
                }
                if let Some(f) = outside_safe_area(&name, layer, res, box_h, si, li) {
                    out.push(f.scene(&scene.id).at(at));
                }
                reading_speed(&mut out, scene, si, li, layer, t, at);
                typing_time(&mut out, scene, si, li, layer, t, at);
            }

            if let LayerContent::LowerThird(lt) = &layer.content {
                let (needed, optimistic) = lower_third_height(lt, layer, res);
                match fixed_h {
                    Some(fixed) if needed > fixed + 0.5 => {
                        let severity = if optimistic > fixed + 0.5 {
                            Severity::Error
                        } else {
                            Severity::Warning
                        };
                        out.push(
                            Finding::new(
                                severity,
                                "text.overflow",
                                format!(
                                    "{name} needs about {needed:.0} px for its title and subtitle but its box is {fixed:.0} px tall"
                                ),
                                Suggestion::set(
                                    "leave out `height` so the box fits its text",
                                    layer_path(si, li, "height"),
                                    Value::Null,
                                ),
                            )
                            .scene(&scene.id)
                            .at(at),
                        );
                    }
                    Some(_) => {}
                    None => box_h = needed,
                }
                if let Some(f) = outside_safe_area(&name, layer, res, box_h, si, li) {
                    out.push(f.scene(&scene.id).at(at));
                }
            }

            let rect = place(layer, res, box_h);
            if !poses(layer, rect, res)
                .into_iter()
                .any(|r| on_screen(r, res))
            {
                out.push(
                    Finding::new(
                        Severity::Warning,
                        "layer.off_frame",
                        format!(
                            "{name} ({}) lies entirely outside the frame, so it is never seen",
                            layer.content.kind()
                        ),
                        Suggestion::set(
                            "move it into the frame",
                            layer_path(si, li, "x"),
                            json!(0.1),
                        ),
                    )
                    .scene(&scene.id)
                    .at(at),
                );
            }
        }
    }
    out
}

/// A box (before rotation, at rest) that leaves the title-safe area.
fn outside_safe_area(
    name: &str,
    layer: &Layer,
    res: Resolution,
    box_h: f64,
    si: usize,
    li: usize,
) -> Option<Finding> {
    let (safe_l, safe_t, safe_r, safe_b) = title_safe(res);
    let (l, top, w, h) = place(layer, res, box_h);
    if l >= safe_l - 0.5 && top >= safe_t - 0.5 && l + w <= safe_r + 0.5 && top + h <= safe_b + 0.5
    {
        return None;
    }
    Some(Finding::new(
        Severity::Warning,
        "safe_area.text",
        format!(
            "{name} spans x {l:.0}..{:.0}, y {top:.0}..{:.0} px; the title-safe area is x {safe_l:.0}..{safe_r:.0}, y {safe_t:.0}..{safe_b:.0}",
            l + w,
            top + h
        ),
        Suggestion::field(
            "move or shrink the layer so its text stays 5% inside every edge",
            layer_path(si, li, "x"),
        ),
    ))
}

/// A text area of `line_width` px at `font_px` with line height `lh` (a multiple of the font).
fn area(font_px: f64, lh: f64, line_width: f64) -> SceneTextArea {
    SceneTextArea {
        font_px,
        letter_spacing_em: SCENE_LETTER_SPACING_EM,
        word_gap_em: SCENE_WORD_GAP_EM,
        line_height_px: font_px * lh,
        line_width: line_width.max(1.0),
        max_lines: u32::MAX,
        bottom_y: 0.0,
    }
}

/// Lower-third layout, in em of the title font (`components/LayerKinds.tsx` `LowerThirdView`).
const LT_BAR_EM: f64 = 0.16;
const LT_PAD_V_EM: f64 = 0.38;
const LT_PAD_H_EM: f64 = 0.7;
const LT_TITLE_LH: f64 = 1.15;
const LT_SUB_SCALE: f64 = 0.62;
const LT_SUB_LH: f64 = 1.3;
const LT_SUB_GAP_EM: f64 = 0.25;

/// Height in px a lower third needs for its text: (nominal, optimistic).
fn lower_third_height(lt: &LowerThirdLayer, layer: &Layer, res: Resolution) -> (f64, f64) {
    let f = f64::from(lt.font_size) * f64::from(res.height);
    let line_width =
        f64::from(layer.width) * f64::from(res.width) - (LT_BAR_EM + 2.0 * LT_PAD_H_EM) * f;
    let title = area(f, LT_TITLE_LH, line_width);
    let sub = area(f * LT_SUB_SCALE, LT_SUB_LH, line_width);
    let at = |factor: f64| {
        let mut h = 2.0 * LT_PAD_V_EM * f
            + f64::from(scene_text_lines(&lt.title, &title, factor)) * title.line_height_px;
        if let Some(s) = lt.subtitle.as_deref().filter(|s| !s.trim().is_empty()) {
            h += LT_SUB_GAP_EM * sub.font_px
                + f64::from(scene_text_lines(s, &sub, factor)) * sub.line_height_px;
        }
        h
    };
    (at(1.0), at(OPTIMISTIC))
}

fn reading_speed(
    out: &mut Vec<Finding>,
    scene: &Scene,
    si: usize,
    li: usize,
    layer: &Layer,
    t: &TextLayer,
    at: u64,
) {
    let shown = layer
        .end_ms_in(scene.duration_ms)
        .saturating_sub(layer.start_ms);
    if shown == 0 {
        return;
    }
    let lang = Lang::of(&t.text);
    let chars = reading_chars(&t.text, lang);
    let limit = cps_limit(lang);
    let cps = chars as f64 * 1000.0 / shown as f64;
    if cps > limit {
        out.push(
            Finding::new(
                Severity::Warning,
                "text.reading_speed",
                format!(
                    "layer `{}` shows {chars} characters for {} ({cps:.1} chars/s); the {} limit is {limit} chars/s",
                    layer.id,
                    crate::secs(shown),
                    lang.code()
                ),
                Suggestion::field(
                    format!(
                        "show it for at least {} ms or shorten the text",
                        ((chars as f64 / limit) * 1000.0).ceil() as u64
                    ),
                    layer_path(si, li, "end_ms"),
                ),
            )
            .scene(&scene.id)
            .at(at),
        );
    }
}

/// A typewriter that is still typing when its layer starts to leave.
fn typing_time(
    out: &mut Vec<Finding>,
    scene: &Scene,
    si: usize,
    li: usize,
    layer: &Layer,
    t: &TextLayer,
    at: u64,
) {
    if t.reveal != TextReveal::Typewriter {
        return;
    }
    let shown = layer
        .end_ms_in(scene.duration_ms)
        .saturating_sub(layer.start_ms);
    let until_exit = shown.saturating_sub(layer.exit.map_or(0, |e| e.duration_ms));
    let typing = t.typing_ms();
    if typing <= until_exit || until_exit == 0 {
        return;
    }
    let units = typing_units(&t.text).len();
    let needed = (units as f64 * 1000.0 / until_exit as f64).ceil();
    out.push(
        Finding::new(
            Severity::Warning,
            "text.typing_unfinished",
            format!(
                "layer `{}` types {units} characters in {} but leaves after {}; the end of the text is never shown",
                layer.id,
                crate::secs(typing),
                crate::secs(until_exit)
            ),
            Suggestion::set(
                format!("type faster (typing_cps {needed}) or show the layer longer"),
                layer_path(si, li, "typing_cps"),
                json!(needed),
            ),
        )
        .scene(&scene.id)
        .at(at),
    );
}

/// Image and icon files: missing, unreadable, too small for their box; icons without a viewBox.
pub fn check_assets(timeline: &VideoTimeline, public_dir: &Path) -> Vec<Finding> {
    let mut out = Vec::new();
    let res = timeline.resolution;
    let starts = scene_starts_ms(timeline);
    for (si, scene) in timeline.scenes.iter().enumerate() {
        for (li, layer) in scene.layers.iter().enumerate() {
            let Some(rel) = layer.content.asset() else {
                continue;
            };
            if is_url(rel) {
                continue;
            }
            let at = starts[si] + layer.start_ms;
            let path = public_dir.join(rel);
            let src = layer_path(si, li, "src");
            let finding = |sev, code, msg: String, fix: String| {
                Finding::new(sev, code, msg, Suggestion::field(fix, src.clone()))
                    .scene(&scene.id)
                    .at(at)
            };
            if !path.is_file() {
                let code = if matches!(layer.content, LayerContent::Icon(_)) {
                    "assets.missing_icon"
                } else {
                    "assets.missing_image"
                };
                out.push(finding(
                    Severity::Error,
                    code,
                    format!(
                        "layer `{}`: `{rel}` does not exist ({})",
                        layer.id,
                        path.display()
                    ),
                    format!(
                        "add the file under {} or point `src` at an existing one",
                        public_dir.display()
                    ),
                ));
                continue;
            }
            match &layer.content {
                LayerContent::Image(img) => {
                    let Ok((iw, ih)) = image::image_dimensions(&path) else {
                        out.push(finding(
                            Severity::Error,
                            "assets.unreadable_image",
                            format!(
                                "layer `{}`: `{rel}` is not an image the renderer can read",
                                layer.id
                            ),
                            "convert it to PNG or JPEG".into(),
                        ));
                        continue;
                    };
                    let (iw, ih) = (f64::from(iw.max(1)), f64::from(ih.max(1)));
                    let bw = f64::from(layer.width) * f64::from(res.width);
                    let bh = f64::from(layer.height.unwrap_or(0.0)) * f64::from(res.height);
                    let fit = match img.fit {
                        ImageFit::Cover => (bw / iw).max(bh / ih),
                        ImageFit::Contain => (bw / iw).min(bh / ih),
                    };
                    let zoom = layer
                        .keyframes
                        .iter()
                        .map(|k| f64::from(k.scale))
                        .fold(1.0, f64::max);
                    let upscale = fit * zoom;
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
                                "layer `{}`: `{rel}` is {iw}x{ih} and is enlarged {upscale:.1}x in its {bw:.0}x{bh:.0} px box, so it will look soft",
                                layer.id
                            ),
                            format!(
                                "use a version of at least {}x{} px, or make the layer smaller",
                                (iw * upscale).ceil(),
                                (ih * upscale).ceil()
                            ),
                        ));
                    }
                }
                LayerContent::Icon(_) => {
                    let text = std::fs::read(&path)
                        .map(|b| String::from_utf8_lossy(&b[..b.len().min(64 * 1024)]).into_owned())
                        .unwrap_or_default();
                    let Some(tag_start) = text.find("<svg") else {
                        out.push(finding(
                            Severity::Error,
                            "assets.unreadable_icon",
                            format!("layer `{}`: `{rel}` is not an SVG file", layer.id),
                            "use an .svg file with an <svg> root".into(),
                        ));
                        continue;
                    };
                    // Vector art has no resolution; without a viewBox, though, the drawing keeps
                    // its own size instead of scaling to the box (cropped or tiny on screen).
                    let tag_end = text[tag_start..]
                        .find('>')
                        .map_or(text.len(), |e| tag_start + e);
                    if !text[tag_start..tag_end].contains("viewBox") {
                        out.push(finding(
                            Severity::Warning,
                            "assets.icon_no_viewbox",
                            format!("layer `{}`: `{rel}` has no viewBox, so it does not scale to the layer's box", layer.id),
                            "add viewBox=\"0 0 W H\" to the <svg> element (W, H = its drawing size)".into(),
                        ));
                    }
                }
                _ => {}
            }
        }
    }
    out
}
