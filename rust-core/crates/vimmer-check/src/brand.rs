//! Brand and template checks: text-versus-background contrast, the logo's place (title-safe
//! area, subtitles), brand files, and template text that does not fit.
//!
//! Contrast follows WCAG 2: the ratio of the relative luminances (+0.05) of the text color and
//! the color behind it. Translucent colors are composited over what is under them. Below 4.5:1 is
//! a warning for body text, below 3:1 for large text. "Large" here is relative to the frame (a
//! video is watched scaled down): a font of at least 6 % of the frame height, or 4.5 % when bold
//! (weight 700+). What is behind the text is only known when nothing but solid colors is there: a
//! text box, a shape layer that covers the text, or the frame background on a scene without an
//! image or video. Text over pictures is not checked (the scene text gets a dark scrim there).

use std::collections::HashSet;
use std::path::Path;

use vimmer_dsl::brand::{
    contrast_ratio, over, parse_hex, DEFAULT_CHART_LABEL, DEFAULT_LOWER_THIRD_SUBTITLE,
};
use vimmer_dsl::template::param_of_layer;
use vimmer_dsl::{
    Anchor, Brand, Corner, Layer, LayerContent, Resolution, Scene, SceneTemplate, VideoTimeline,
};

use crate::layout::{title_safe, SubtitleBand, SCENE_FONT_PX};
use crate::{scene_path, scene_starts_ms, Finding, Severity, Suggestion};

/// Body text needs this contrast ratio, large text [`LARGE_MIN_RATIO`].
pub const BODY_MIN_RATIO: f64 = 4.5;
pub const LARGE_MIN_RATIO: f64 = 3.0;
/// Large text: at least this fraction of the frame height, or [`LARGE_BOLD_FRAC`] at weight 700+.
pub const LARGE_FRAC: f64 = 0.06;
pub const LARGE_BOLD_FRAC: f64 = 0.045;

fn rgb(c: &str) -> Option<[u8; 4]> {
    parse_hex(c)
}

/// `color` (maybe translucent) over the opaque `under`.
fn flatten(color: &str, under: [u8; 3]) -> Option<[u8; 3]> {
    rgb(color).map(|c| over(c, under))
}

fn is_large(font_frac: f64, weight: u16) -> bool {
    font_frac >= LARGE_FRAC || (weight >= 700 && font_frac >= LARGE_BOLD_FRAC)
}

/// (left, top, right, bottom) in frame fractions of a layer at rest; `None` without a height.
fn frac_box(l: &Layer) -> Option<(f32, f32, f32, f32)> {
    let h = l.height?;
    let (mut x, mut y) = (l.x, l.y);
    if l.anchor == Anchor::Center {
        x -= l.width / 2.0;
        y -= h / 2.0;
    }
    Some((x, y, x + l.width, y + h))
}

/// One text element to check: where it is, its color and what is behind it.
struct Sample {
    scene: usize,
    what: String,
    fg: [u8; 3],
    bg: [u8; 3],
    large: bool,
    /// Where the text color is set, for the fix.
    path: String,
}

/// The solid color behind layer `index` of `scene` (shapes below it that cover its box, else the
/// frame background), or `None` when a picture may be behind it.
fn behind(scene: &Scene, index: usize, base: Option<[u8; 3]>) -> Option<[u8; 3]> {
    let layer = &scene.layers[index];
    let target = frac_box(layer).unwrap_or((layer.x, layer.y, layer.x + layer.width, layer.y));
    // Draw order: z, then list order. Layers drawn before this one that are opaque-ish shapes
    // covering it determine its background, the latest one winning.
    let mut order: Vec<usize> = (0..scene.layers.len()).collect();
    order.sort_by_key(|&i| (scene.layers[i].z, i));
    let mut color = base;
    for &i in order.iter().take_while(|&&i| i != index) {
        let l = &scene.layers[i];
        let LayerContent::Shape(sh) = &l.content else {
            continue;
        };
        if sh.shape != vimmer_dsl::ShapeKind::Rect || l.rotation != 0.0 || l.opacity < 1.0 {
            continue;
        }
        let (Some(fill), Some(b)) = (&sh.fill, frac_box(l)) else {
            continue;
        };
        let covers = b.0 <= target.0 + 1e-3
            && b.1 <= target.1 + 1e-3
            && b.2 >= target.2 - 1e-3
            && b.3 >= target.3 - 1e-3;
        if covers {
            let Some(c) = rgb(fill) else { continue };
            color = match color {
                Some(under) => Some(over(c, under)),
                None if c[3] == 255 => Some([c[0], c[1], c[2]]),
                None => None,
            };
        }
    }
    color
}

/// A text of a layer: (what, color, its field, the brand color it falls back to when the field
/// is unset (`None` when it is set), box color, font frac, weight).
type Item = (
    String,
    String,
    String,
    Option<&'static str>,
    Option<String>,
    f64,
    u16,
);

fn samples(t: &VideoTimeline) -> Vec<Sample> {
    let brand = t.brand.as_ref();
    let h = f64::from(t.resolution.height);
    let frame_bg = flatten(Brand::background(brand), [0, 0, 0]).unwrap_or([9, 9, 11]);
    let text = Brand::text(brand);
    let text_path = |own: bool, layer_path: String| {
        if own || brand.and_then(|b| b.colors.text.as_ref()).is_none() {
            layer_path
        } else {
            "/brand/colors/text".to_string()
        }
    };
    let mut out = Vec::new();
    for (si, scene) in t.scenes.iter().enumerate() {
        let picture = scene.image.is_some() || scene.video.is_some();
        let base = (!picture).then_some(frame_bg);
        if scene.text.as_deref().is_some_and(|s| !s.trim().is_empty()) && !picture {
            if let Some(fg) = flatten(text, frame_bg) {
                out.push(Sample {
                    scene: si,
                    what: "the scene text".into(),
                    fg,
                    bg: frame_bg,
                    large: is_large(SCENE_FONT_PX / h, 600),
                    path: text_path(false, scene_path(si, "text")),
                });
            }
        }
        for (li, layer) in scene.layers.iter().enumerate() {
            let lp = |f: &str| format!("/scenes/{si}/layers/{li}/{f}");
            let name = format!("layer `{}`", layer.id);
            let under = behind(scene, li, base);
            let mut items: Vec<Item> = Vec::new();
            match &layer.content {
                LayerContent::Text(tl) => items.push((
                    name.clone(),
                    tl.color.clone().unwrap_or_else(|| text.to_string()),
                    lp("color"),
                    tl.color.is_none().then_some("text"),
                    tl.background.as_ref().map(|b| b.color.clone()),
                    f64::from(tl.font_size),
                    tl.weight,
                )),
                LayerContent::Counter(c) => items.push((
                    name.clone(),
                    c.color.clone().unwrap_or_else(|| text.to_string()),
                    lp("color"),
                    c.color.is_none().then_some("text"),
                    None,
                    f64::from(c.font_size),
                    c.weight,
                )),
                LayerContent::LowerThird(lt) => {
                    let bx = lt.background_color.clone();
                    items.push((
                        format!("{name} title"),
                        lt.color.clone().unwrap_or_else(|| text.to_string()),
                        lp("color"),
                        lt.color.is_none().then_some("text"),
                        bx.clone(),
                        f64::from(lt.font_size),
                        700,
                    ));
                    if lt.subtitle.is_some() {
                        let muted = Brand::muted_text(brand, DEFAULT_LOWER_THIRD_SUBTITLE);
                        items.push((
                            format!("{name} subtitle"),
                            lt.subtitle_color
                                .clone()
                                .unwrap_or_else(|| muted.to_string()),
                            lp("subtitle_color"),
                            lt.subtitle_color.is_none().then_some("muted_text"),
                            bx,
                            f64::from(lt.font_size) * 0.62,
                            500,
                        ));
                    }
                }
                LayerContent::BarChart(ch) => {
                    let muted = Brand::muted_text(brand, DEFAULT_CHART_LABEL);
                    items.push((
                        format!("{name} labels"),
                        ch.label_color.clone().unwrap_or_else(|| muted.to_string()),
                        lp("label_color"),
                        ch.label_color.is_none().then_some("muted_text"),
                        None,
                        f64::from(ch.font_size),
                        400,
                    ));
                }
                _ => {}
            }
            for (what, color, path, fallback, box_color, frac, weight) in items {
                // Template layers carry the brand's colors explicitly: point at the brand color
                // they came from.
                let from_brand = fallback.or_else(|| {
                    let c = &brand?.colors;
                    let is_tpl = layer
                        .id
                        .starts_with(vimmer_dsl::template::TEMPLATE_LAYER_PREFIX);
                    [
                        ("text", &c.text),
                        ("muted_text", &c.muted_text),
                        ("primary", &c.primary),
                    ]
                    .into_iter()
                    .find(|(_, v)| is_tpl && v.as_deref() == Some(color.as_str()))
                    .map(|(k, _)| k)
                });
                let brand_has = |k: &str| {
                    let c = brand.map(|b| &b.colors);
                    match k {
                        "text" => c.and_then(|c| c.text.as_ref()).is_some(),
                        "muted_text" => c.and_then(|c| c.muted_text.as_ref()).is_some(),
                        "primary" => c.and_then(|c| c.primary.as_ref()).is_some(),
                        _ => false,
                    }
                };
                let path = match from_brand {
                    Some(k) if brand_has(k) => format!("/brand/colors/{k}"),
                    _ => path,
                };
                let bg = match (&box_color, under) {
                    (Some(b), Some(u)) => flatten(b, u),
                    (Some(b), None) => rgb(b).filter(|c| c[3] == 255).map(|c| [c[0], c[1], c[2]]),
                    (None, u) => u,
                };
                let (Some(bg), Some(c)) = (bg, rgb(&color)) else {
                    continue;
                };
                out.push(Sample {
                    scene: si,
                    what,
                    fg: over(c, bg),
                    bg,
                    large: is_large(frac, weight),
                    path,
                });
            }
        }
    }
    // Subtitles: their box over the frame background (the worst case on a picture is unknown).
    if !t.subtitles.is_empty() {
        let sub_bg = Brand::subtitle_background(brand);
        if let Some(bg) = flatten(sub_bg, frame_bg) {
            if let Some(fg) = flatten(text, bg) {
                let band = SubtitleBand::new(t.resolution);
                out.push(Sample {
                    scene: usize::MAX,
                    what: "the subtitles".into(),
                    fg,
                    bg,
                    large: is_large(band.font_px / h, 500),
                    path: if brand
                        .and_then(|b| b.colors.subtitle_background.as_ref())
                        .is_some()
                    {
                        "/brand/colors/subtitle_background".into()
                    } else {
                        "/brand/colors/text".into()
                    },
                });
            }
        }
    }
    out
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// Low-contrast text, one finding per color pair (naming how many elements share it).
pub fn contrast(t: &VideoTimeline) -> Vec<Finding> {
    let starts = scene_starts_ms(t);
    let mut groups: Vec<(Sample, usize, f64, f64)> = Vec::new();
    for s in samples(t) {
        let ratio = contrast_ratio(s.fg, s.bg);
        let min = if s.large {
            LARGE_MIN_RATIO
        } else {
            BODY_MIN_RATIO
        };
        if ratio >= min {
            continue;
        }
        match groups
            .iter_mut()
            .find(|g| g.0.fg == s.fg && g.0.bg == s.bg && g.3 == min)
        {
            Some(g) => g.1 += 1,
            None => groups.push((s, 0, ratio, min)),
        }
    }
    groups
        .into_iter()
        .map(|(s, more, ratio, min)| {
            let others = if more > 0 {
                format!(" (and {more} more with the same colors)")
            } else {
                String::new()
            };
            let size = if s.large { "large" } else { "body" };
            let mut f = Finding::new(
                Severity::Warning,
                "brand.contrast",
                format!(
                    "{} is {} on {}: contrast {ratio:.2}:1, below {min}:1 for {size} text{others}",
                    s.what,
                    hex(s.fg),
                    hex(s.bg),
                ),
                Suggestion::field(
                    "use a lighter text color on a dark background or a darker one on a light background",
                    s.path,
                ),
            );
            if let Some(&start) = starts.get(s.scene) {
                f = f.scene(&t.scenes[s.scene].id).at(start);
            }
            f
        })
        .collect()
}

/// Width over height of an image file, if it can be read (raster files and SVG `viewBox`).
fn aspect(path: &Path) -> Option<f64> {
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"))
    {
        let text = std::fs::read_to_string(path).ok()?;
        let at = text.find("viewBox")?;
        let rest = &text[at..];
        let q = rest.find(['"', '\''])?;
        let rest = &rest[q + 1..];
        let end = rest.find(['"', '\''])?;
        let nums: Vec<f64> = rest[..end]
            .split([' ', ','])
            .filter_map(|s| s.parse().ok())
            .collect();
        return (nums.len() == 4 && nums[3] > 0.0).then(|| nums[2] / nums[3]);
    }
    let (w, h) = image::image_dimensions(path).ok()?;
    (h > 0).then(|| f64::from(w) / f64::from(h))
}

/// The logo's box in px: (left, top, right, bottom).
fn logo_rect(brand: &Brand, res: Resolution, aspect: f64) -> Option<(f64, f64, f64, f64)> {
    let logo = brand.logo.as_ref()?;
    let (w, h) = (f64::from(res.width), f64::from(res.height));
    let lh = f64::from(logo.size) * h;
    let lw = lh * aspect;
    let (mx, my) = (f64::from(logo.margin) * w, f64::from(logo.margin) * h);
    let left = match logo.corner {
        Corner::TopLeft | Corner::BottomLeft => mx,
        _ => w - mx - lw,
    };
    let top = match logo.corner {
        Corner::TopLeft | Corner::TopRight => my,
        _ => h - my - lh,
    };
    Some((left, top, left + lw, top + lh))
}

/// The logo's place (title-safe area, subtitles), `hide_on` ids, and brand files.
pub fn check(t: &VideoTimeline, public_dir: Option<&Path>) -> Vec<Finding> {
    let mut out = Vec::new();
    let Some(brand) = &t.brand else {
        return out;
    };
    if let Some(dir) = public_dir {
        for (what, src) in brand.assets() {
            if !dir.join(src).is_file() {
                let code = if what == "logo" {
                    "assets.missing_logo"
                } else {
                    "assets.missing_font"
                };
                let field = match what {
                    "logo" => "/brand/logo/src",
                    "heading font" => "/brand/fonts/heading/src",
                    _ => "/brand/fonts/body/src",
                };
                out.push(Finding::new(
                    Severity::Error,
                    code,
                    format!(
                        "the brand {what} `{src}` is not in the public directory; the render stops"
                    ),
                    Suggestion::field("add the file to the public dir or fix the path", field),
                ));
            }
        }
    }
    let Some(logo) = &brand.logo else {
        return out;
    };
    let ids: HashSet<&str> = t.scenes.iter().map(|s| s.id.as_str()).collect();
    for id in &logo.hide_on {
        if !ids.contains(id.as_str()) {
            out.push(Finding::new(
                Severity::Warning,
                "brand.unknown_scene",
                format!("logo.hide_on names `{id}`, which is not a scene id, so it has no effect"),
                Suggestion::field("use an existing scene id", "/brand/logo/hide_on"),
            ));
        }
    }
    let ratio = public_dir
        .and_then(|d| aspect(&d.join(&logo.src)))
        .unwrap_or(1.0);
    let Some((l, top, r, b)) = logo_rect(brand, t.resolution, ratio) else {
        return out;
    };
    let (sl, st, sr, sb) = title_safe(t.resolution);
    if l < sl - 0.5 || top < st - 0.5 || r > sr + 0.5 || b > sb + 0.5 {
        out.push(Finding::new(
            Severity::Warning,
            "brand.logo_safe_area",
            format!(
                "the logo spans x {l:.0}..{r:.0}, y {top:.0}..{b:.0} px, outside the title-safe area (x {sl:.0}..{sr:.0}, y {st:.0}..{sb:.0}); it may be cut off on some screens"
            ),
            Suggestion::set(
                "move it in with a margin of at least 0.05",
                "/brand/logo/margin",
                serde_json::json!(0.05),
            ),
        ));
    }
    // Subtitles: a two-line band, where a cue shows while the logo is drawn.
    let band = SubtitleBand::new(t.resolution);
    let (bl, bt, br, bb) = (
        band.left_x,
        band.top_y(2),
        f64::from(t.resolution.width) - band.left_x,
        band.bottom_y,
    );
    if l < br && r > bl && top < bb && b > bt {
        let starts = scene_starts_ms(t);
        let hit = t.subtitles.iter().find(|cue| {
            t.scenes.iter().zip(&starts).any(|(s, &start)| {
                !logo.hide_on.contains(&s.id)
                    && cue.start_ms < start + s.duration_ms
                    && cue.end_ms > start
            })
        });
        if let Some(cue) = hit {
            out.push(
                Finding::new(
                    Severity::Warning,
                    "brand.logo_subtitles",
                    "the logo sits where two-line subtitles are drawn and covers part of them",
                    Suggestion::field("put the logo in a top corner", "/brand/logo/corner"),
                )
                .at(cue.start_ms),
            );
        }
    }
    out
}

/// Text overflow findings on template layers, renamed `template.text_overflow` and pointed at
/// the template param. `templates` are the scenes' templates before expansion.
pub fn template_findings(
    findings: Vec<Finding>,
    expanded: &VideoTimeline,
    templates: &[Option<SceneTemplate>],
) -> Vec<Finding> {
    findings
        .into_iter()
        .map(|mut f| {
            let Some(path) = f.suggestion.path.clone() else {
                return f;
            };
            // `/scenes/<si>/layers/<li>/<field>`
            let parts: Vec<&str> = path.split('/').collect();
            let (Some(si), Some(li)) = (
                parts.get(2).and_then(|s| s.parse::<usize>().ok()),
                parts.get(4).and_then(|s| s.parse::<usize>().ok()),
            ) else {
                return f;
            };
            if f.code == "brand.contrast"
                || parts.get(3) != Some(&"layers")
                || templates.get(si).is_none_or(Option::is_none)
            {
                return f;
            }
            let Some(layer) = expanded.scenes.get(si).and_then(|s| s.layers.get(li)) else {
                return f;
            };
            let Some(param) = param_of_layer(&layer.id) else {
                return f;
            };
            if f.code == "text.overflow" {
                f.code = "template.text_overflow";
                f.message = format!("template param `{param}`: {}", f.message);
                f.suggestion = Suggestion::field(
                    "shorten this template text",
                    format!("/scenes/{si}/template/params/{param}"),
                );
            } else {
                // Other findings about template layers point at the param, not at layers that
                // only exist after expansion.
                f.suggestion.path = Some(format!("/scenes/{si}/template/params/{param}"));
                f.suggestion.value = None;
            }
            f
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timeline(extra: &str) -> VideoTimeline {
        serde_json::from_str(&format!(
            r#"{{"fps":30,"resolution":{{"width":1920,"height":1080}},"scenes":[{{"id":"a","duration_ms":3000,"text":"Hello there"}}],
            "subtitles":[{{"start_ms":0,"end_ms":1000,"text":"hi"}}]{extra}}}"#
        ))
        .unwrap()
    }

    #[test]
    fn no_brand_has_no_contrast_findings() {
        assert!(contrast(&timeline("")).is_empty());
    }

    #[test]
    fn low_contrast_brand_is_a_warning() {
        let t = timeline(
            r##","brand":{"colors":{"text":"#777777","background":"#5a5a5a","subtitle_background":"#666666"}}"##,
        );
        let f = contrast(&t);
        assert!(!f.is_empty());
        assert!(f.iter().all(|f| f.code == "brand.contrast"));
        assert!(f[0].message.contains("below"), "{}", f[0].message);
        assert_eq!(f[0].suggestion.path.as_deref(), Some("/brand/colors/text"));
    }

    #[test]
    fn layer_text_is_checked_against_the_shape_under_it() {
        let mut t = timeline("");
        t.scenes[0].layers = serde_json::from_str(
            r##"[{"id":"box","kind":"shape","x":0,"y":0,"width":1,"height":1,"fill":"#ffffff","z":-1},
                 {"id":"t","kind":"text","x":0.1,"y":0.1,"width":0.5,"height":0.1,"text":"hi","font_size":0.03}]"##,
        )
        .unwrap();
        let f = contrast(&t);
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].message.starts_with("layer `t` is #fafafa on #ffffff"));
        assert_eq!(
            f[0].suggestion.path.as_deref(),
            Some("/scenes/0/layers/1/color")
        );
    }

    #[test]
    fn logo_place() {
        let t = timeline(
            r#","brand":{"logo":{"src":"l.png","corner":"bottom_right","margin":0.01,"size":0.3}}"#,
        );
        let codes: Vec<&str> = check(&t, None).iter().map(|f| f.code).collect();
        assert_eq!(codes, ["brand.logo_safe_area", "brand.logo_subtitles"]);
        let t = timeline(r#","brand":{"logo":{"src":"l.png","hide_on":["zzz"]}}"#);
        let codes: Vec<&str> = check(&t, None).iter().map(|f| f.code).collect();
        assert_eq!(codes, ["brand.unknown_scene"]);
    }

    #[test]
    fn template_text_that_does_not_fit_points_at_the_param() {
        let title = "WWWWWWW MMMMMMM WWWWWWW MMMMMMM WWWWWWW MMMMMMM WWWWWWW MMM";
        let t: VideoTimeline = serde_json::from_str(&format!(
            r#"{{"fps":30,"resolution":{{"width":1920,"height":1080}},"scenes":[{{"id":"l","duration_ms":6000,
            "template":{{"name":"list","params":{{"title":"{title}","items":["a"]}}}}}}]}}"#
        ))
        .unwrap();
        let f = crate::check_timeline(&t, &crate::CheckOptions::default());
        let over: Vec<_> = f
            .iter()
            .filter(|f| f.code == "template.text_overflow")
            .collect();
        assert_eq!(over.len(), 1, "{f:?}");
        assert_eq!(
            over[0].suggestion.path.as_deref(),
            Some("/scenes/0/template/params/title")
        );
        assert!(over[0].message.starts_with("template param `title`"));
    }
}
