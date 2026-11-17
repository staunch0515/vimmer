//! The brand logo over text and other layers.
//!
//! The renderer draws the logo above every scene layer and the scene text, below the subtitles
//! (`compositions/Timeline.tsx`, `components/Brand.tsx`), so wherever its box meets text the
//! text is covered. Its box is `brand.ts` `logoBox`: height `size` of the frame height, width from
//! the image's aspect ratio, `margin` of the frame width from the left/right edge and of the frame
//! height from the top/bottom edge (as `brand::logo_rect`). Text boxes are [`crate::textbox`].
//!
//! * Text (scene text, text layers, lower thirds, counters, charts and rings, whose labels are
//!   text): a warning, `brand.logo_overlap`.
//! * Image, icon, shape and video layers: info. A layer that fills a quarter of the frame or
//!   more is a backdrop and the logo is meant to sit on it: not reported.
//! * Subtitles are `brand.logo_subtitles` (in `brand`).
//!
//! Both must be visible at the same time: the layer's `start_ms..end_ms` on a scene the logo is
//! not hidden on. A layer with keyframes counts at every keyframe position. Boxes meeting by 2 px
//! or less are not an overlap. Without the logo file the logo is taken as square (and said so).
//! The fix is the first corner where the logo covers no text anywhere, else hiding it on the scene.

use std::path::Path;

use serde_json::json;
use vimmer_dsl::{Brand, Corner, Layer, LayerContent, Resolution, VideoTimeline};

use crate::brand::{aspect, logo_rect};
use crate::fonts::Fonts;
use crate::layers::{poses, Rect};
use crate::layout::SubtitleBand;
use crate::{scene_starts_ms, secs, textbox, Finding, Severity, Suggestion};

/// Layers at least this share of the frame are backdrops (not reported under the logo).
pub const BACKDROP_FRAC: f64 = 0.25;
/// Overlaps this thin or thinner (px, either direction) are ignored.
pub const MIN_OVERLAP_PX: f64 = 2.0;

/// (left, top, right, bottom).
pub(crate) type Edges = (f64, f64, f64, f64);

pub(crate) fn overlaps(a: Edges, r: Rect) -> bool {
    let (l, t, w, h) = r;
    let ow = a.2.min(l + w) - a.0.max(l);
    let oh = a.3.min(t + h) - a.1.max(t);
    ow > MIN_OVERLAP_PX && oh > MIN_OVERLAP_PX
}

/// Whether the layer shows text.
fn is_text(layer: &Layer) -> bool {
    matches!(
        layer.content,
        LayerContent::Text(_)
            | LayerContent::LowerThird(_)
            | LayerContent::Counter(_)
            | LayerContent::BarChart(_)
            | LayerContent::ProgressRing(_)
    )
}

/// Something the logo may cover on one scene.
pub(crate) struct Item {
    pub(crate) scene: usize,
    /// `None`: the scene text.
    layer: Option<usize>,
    /// Which page of the scene text (index, count); (0, 1) for anything else.
    page: (usize, usize),
    pub(crate) text: bool,
    pub(crate) boxes: Vec<Rect>,
    from_ms: u64,
    to_ms: u64,
}

pub(crate) fn items(t: &VideoTimeline, fonts: &Fonts) -> Vec<Item> {
    let res = t.resolution;
    let frame_area = f64::from(res.width) * f64::from(res.height);
    let mut out = Vec::new();
    for (si, scene) in t.scenes.iter().enumerate() {
        let pages = textbox::scene_pages(t, si, fonts);
        let n = pages.len();
        for (k, (b, page)) in pages.into_iter().enumerate() {
            out.push(Item {
                scene: si,
                layer: None,
                page: (k, n),
                text: true,
                boxes: vec![b],
                from_ms: page.from_ms,
                to_ms: page.to_ms,
            });
        }
        for (li, layer) in scene.layers.iter().enumerate() {
            let to_ms = layer.end_ms_in(scene.duration_ms);
            if layer.opacity <= 0.0 && layer.keyframes.iter().all(|k| k.opacity <= 0.0) {
                continue;
            }
            if to_ms <= layer.start_ms {
                continue;
            }
            let rest = textbox::layer_box(layer, res, fonts, t.brand.as_ref());
            let text = is_text(layer);
            if !text && rest.2 * rest.3 >= BACKDROP_FRAC * frame_area {
                continue;
            }
            out.push(Item {
                scene: si,
                layer: Some(li),
                page: (0, 1),
                text,
                boxes: poses(layer, rest, res),
                from_ms: layer.start_ms,
                to_ms,
            });
        }
    }
    out
}

pub(crate) fn corner_name(c: Corner) -> &'static str {
    match c {
        Corner::TopLeft => "top_left",
        Corner::TopRight => "top_right",
        Corner::BottomLeft => "bottom_left",
        Corner::BottomRight => "bottom_right",
    }
}

/// The logo box with the brand's logo moved to `corner`.
fn rect_at(brand: &Brand, corner: Corner, res: Resolution, ratio: f64) -> Option<Edges> {
    let mut b = brand.clone();
    b.logo.as_mut()?.corner = corner;
    logo_rect(&b, res, ratio)
}

/// Whether the logo at `rect` covers any text of a scene it shows on (or the subtitle band).
fn covers_text(t: &VideoTimeline, items: &[Item], rect: Edges, hidden: &[String]) -> bool {
    let shown = |si: usize| !hidden.contains(&t.scenes[si].id);
    let on_text = items
        .iter()
        .any(|i| i.text && shown(i.scene) && i.boxes.iter().any(|b| overlaps(rect, *b)));
    let band = SubtitleBand::for_timeline(t);
    let band_rect = (
        band.left_x,
        band.top_y(2),
        band.right_x - band.left_x,
        band.bottom_y - band.top_y(2),
    );
    on_text || (!t.subtitles.is_empty() && overlaps(rect, band_rect))
}

/// Logo over text (warning) and over small decorative layers (info).
pub fn overlap(t: &VideoTimeline, public_dir: Option<&Path>, fonts: &Fonts) -> Vec<Finding> {
    let mut out = Vec::new();
    let Some(brand) = &t.brand else {
        return out;
    };
    let Some(logo) = &brand.logo else {
        return out;
    };
    let measured = public_dir.and_then(|d| aspect(&d.join(&logo.src)));
    let ratio = measured.unwrap_or(1.0);
    let Some(rect) = logo_rect(brand, t.resolution, ratio) else {
        return out;
    };
    let items = items(t, fonts);
    let starts = scene_starts_ms(t);
    let free = [
        Corner::TopRight,
        Corner::TopLeft,
        Corner::BottomRight,
        Corner::BottomLeft,
    ]
    .into_iter()
    .filter(|&c| c != logo.corner)
    .find(|&c| {
        rect_at(brand, c, t.resolution, ratio)
            .is_some_and(|r| !covers_text(t, &items, r, &logo.hide_on))
    });
    let assumed = if measured.is_none() {
        " (its width assumed equal to its height: the logo file was not read)"
    } else {
        ""
    };
    let (l, top, r, b) = rect;
    for item in &items {
        let scene = &t.scenes[item.scene];
        if logo.hide_on.contains(&scene.id) || !item.boxes.iter().any(|b| overlaps(rect, *b)) {
            continue;
        }
        let what = match item.layer {
            None if item.page.1 > 1 => crate::pages::page_name(item.page.0, item.page.1),
            None => "the scene text".to_string(),
            Some(li) => {
                let layer = &scene.layers[li];
                format!("layer `{}` ({})", layer.id, layer.content.kind())
            }
        };
        let (severity, effect) = if item.text {
            (Severity::Warning, "covers part of")
        } else {
            (Severity::Info, "is drawn over")
        };
        let window = if item.from_ms == 0 && item.to_ms == scene.duration_ms {
            "for the whole scene".to_string()
        } else {
            format!(
                "from {} to {} into the scene",
                secs(item.from_ms),
                secs(item.to_ms)
            )
        };
        let suggestion = match free {
            Some(c) => Suggestion::set(
                format!(
                    "move the logo to the {} corner, where it covers no text",
                    corner_name(c).replace('_', " ")
                ),
                "/brand/logo/corner",
                json!(corner_name(c)),
            ),
            None => {
                let mut hide = logo.hide_on.clone();
                hide.push(scene.id.clone());
                Suggestion::set(
                    format!("hide the logo on scene `{}`, or move the text", scene.id),
                    "/brand/logo/hide_on",
                    json!(hide),
                )
            }
        };
        out.push(
            Finding::new(
                severity,
                "brand.logo_overlap",
                format!(
                    "the logo (x {l:.0}..{r:.0}, y {top:.0}..{b:.0} px{assumed}) {effect} {what} {window}"
                ),
                suggestion,
            )
            .scene(&scene.id)
            .at(starts[item.scene] + item.from_ms),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timeline(logo: &str, layers: &str, text: &str) -> VideoTimeline {
        serde_json::from_str(&format!(
            r#"{{"fps":30,"resolution":{{"width":1920,"height":1080}},
            "brand":{{"logo":{logo}}},
            "scenes":[{{"id":"a","duration_ms":3000{text},"layers":{layers}}}]}}"#
        ))
        .unwrap()
    }

    const TOP_RIGHT: &str = r#"{"src":"l.png","corner":"top_right","size":0.1,"margin":0.05}"#;

    #[test]
    fn text_layer_under_the_logo_is_a_warning_with_a_free_corner() {
        // Logo: x 1692..1824, y 54..162. The title runs across the top right.
        let t = timeline(
            TOP_RIGHT,
            r#"[{"id":"title","kind":"text","x":0.5,"y":0.06,"width":0.45,"text":"Quarterly results","align":"right","font_size":0.07}]"#,
            "",
        );
        let f = overlap(&t, None, &Fonts::none());
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].severity, Severity::Warning);
        assert_eq!(f[0].code, "brand.logo_overlap");
        assert!(
            f[0].message.contains("layer `title` (text)"),
            "{}",
            f[0].message
        );
        assert!(f[0].message.contains("assumed"), "{}", f[0].message);
        assert_eq!(f[0].suggestion.path.as_deref(), Some("/brand/logo/corner"));
        assert_eq!(f[0].suggestion.value, Some(json!("top_left")));
    }

    #[test]
    fn left_aligned_short_text_in_a_wide_box_is_clear_of_the_logo() {
        let t = timeline(
            TOP_RIGHT,
            r#"[{"id":"title","kind":"text","x":0.05,"y":0.06,"width":0.9,"text":"Hi","align":"left","font_size":0.07}]"#,
            "",
        );
        assert!(overlap(&t, None, &Fonts::none()).is_empty());
    }

    #[test]
    fn layer_shown_only_when_the_logo_is_hidden_is_fine() {
        let logo =
            r#"{"src":"l.png","corner":"top_right","size":0.1,"margin":0.05,"hide_on":["a"]}"#;
        let t = timeline(
            logo,
            r#"[{"id":"title","kind":"text","x":0.5,"y":0.06,"width":0.45,"text":"Quarterly results","align":"right","font_size":0.07}]"#,
            "",
        );
        assert!(overlap(&t, None, &Fonts::none()).is_empty());
    }

    #[test]
    fn small_icon_is_info_and_backdrop_is_ignored() {
        let t = timeline(
            TOP_RIGHT,
            r##"[{"id":"bg","kind":"shape","x":0,"y":0,"width":1,"height":1,"fill":"#111111","z":-1},
                 {"id":"star","kind":"icon","x":0.9,"y":0.08,"width":0.05,"height":0.08,"src":"assets/icons/star.svg"}]"##,
            "",
        );
        let f = overlap(&t, None, &Fonts::none());
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].severity, Severity::Info);
        assert!(f[0].message.contains("layer `star` (icon)"));
    }

    #[test]
    fn wide_scene_text_meets_a_bottom_logo_but_a_layer_after_it_ends_does_not() {
        let logo = r#"{"src":"l.png","corner":"bottom_left","size":0.2,"margin":0.08}"#;
        // A scene text of 3 full lines spans x 230..1690, y 489..820; the logo 154..370, 778..994.
        let text = r#","text":"This is a long scene text that wraps across three full lines of the frame so that it reaches far to the left side""#;
        let t = timeline(
            logo,
            r#"[{"id":"late","kind":"text","x":0.08,"y":0.75,"width":0.3,"text":"note","start_ms":0,"end_ms":0}]"#,
            text,
        );
        let f = overlap(&t, None, &Fonts::none());
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].message.contains("the scene text"), "{}", f[0].message);
    }
}
