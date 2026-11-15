//! Text over a scene's image or video: the WCAG contrast of the text color against the actual
//! pixels under the text.
//!
//! # What is sampled
//!
//! The scene background is rebuilt the way the renderer draws it, at a quarter of the output
//! size ([`SAMPLE_SCALE`]):
//!
//! * an image is letterboxed onto a frame-sized canvas (`vimmer_media::letterbox`: the picture
//!   contained, padded with a blurred, 0.45-brightness cover copy), then `KenBurnsImage.tsx` moves
//!   it by the scene's `motion` or `keyframes` (`translate(x%, y%) rotate(r) scale(s)` about the
//!   center); where it no longer covers the frame, the blurred 1.2x copy behind it shows;
//! * a video frame is taken with ffmpeg at the clip time the scene time maps to (trim, speed,
//!   freeze/loop/stretch, as `motion/video.ts`), fitted `cover` or `contain` (padded with
//!   `#09090b`), and moved the same way.
//!
//! Over that come, as in `AnimatedText.tsx`: the dark scrim the scene text adds on pictures
//! (`linear-gradient(to top, rgba(9,9,11,.62) 0%, .45 30%, 0 62%)`, drawn above layers with z < 0
//! and below the rest), and the text's own translucent background (text layers, lower thirds). The
//! scene text's shadow (`0 4px 32px rgba(0,0,0,.6)`) is not counted: in a render of
//! `fixtures/check-contrast-media.json` the gaps between its glyphs are within 1 level of the
//! scrim alone (the 32 px blur spreads it too thin), and the scrim matched to within 4 levels.
//!
//! Text styles (`vimmer_dsl::text_style`: the element's, else the brand's): a box is composited
//! over the background like any translucent background (an opaque one is all the text is
//! compared with); an outline at least [`min_readable_outline_px`] wide (2 px, or 2 % of the font
//! size) replaces the picture as what the text is read against, so the text is compared with the
//! outline color (composited over the picture when it is translucent). Shadows are not counted.
//!
//! The text box is [`crate::textbox`]; a grid of at most [`GRID_POINTS`] points inside it is
//! sampled at up to [`TIMES`] moments while the text shows (one when nothing moves). At each moment
//! the contrast of each point is computed, and its 20th percentile ([`PERCENTILE`]) is the
//! moment's value: at least 80 % of the box has that contrast or better. (A lower percentile
//! flags light text over busy dark textures, where a few bright specks do not hurt reading.) The
//! lowest moment is reported.
//!
//! # Thresholds
//!
//! WCAG 2 as in [`crate::brand`]: below 4.5:1 a warning for body text, below 3:1 for large text,
//! "large" being at least 6 % of the frame height, or 4.5 % at weight 700+ (`brand::is_large`).
//! The scene text (92 px, weight 600) is large on a 1080-high frame (8.5 %) and body text on a
//! 1920-high one (4.8 %).
//!
//! # What is not checked
//!
//! Text over an image or video *layer* (only the scene background is rebuilt), text layers with
//! an opaque background or a shape under it (`brand.contrast` checks that), subtitles (their band is
//! checked by `brand.contrast` against the frame background), remote or missing files (reported
//! by the asset checks), and keyframe opacity. ffmpeg runs at most [`MAX_VIDEO_FRAMES`] times.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use image::{imageops, imageops::FilterType, RgbImage};
use serde_json::json;
use vimmer_dsl::brand::{contrast_ratio, over, DEFAULT_LOWER_THIRD_SUBTITLE};
use vimmer_dsl::{
    style_scale, Brand, FitTime, ImageFit, Layer, LayerContent, MotionType, Resolution, Scene,
    TextStyle, VideoClip, VideoTimeline,
};

use crate::brand::{behind, hex, is_large, rgb, BODY_MIN_RATIO, LARGE_MIN_RATIO};
use crate::fonts::Fonts;
use crate::layers::{place, Rect};
use crate::layout::SCENE_FONT_PX;
use crate::motion::{sample, Pose};
use crate::{is_url, scene_starts_ms, secs, textbox, Finding, Severity, Suggestion};

/// Resolution of the rebuilt background, as a fraction of the output.
pub const SAMPLE_SCALE: f64 = 0.25;
/// Most points sampled inside a text box.
pub const GRID_POINTS: usize = 160;
/// Most moments sampled per text.
pub const TIMES: usize = 3;
/// Most video frames decoded per check.
pub const MAX_VIDEO_FRAMES: usize = 12;
/// Percentile of the per-point contrasts that stands for a moment.
pub const PERCENTILE: f64 = 0.20;

const SCRIM: [u8; 3] = [9, 9, 11];
const PADDING: [u8; 3] = [9, 9, 11];

/// Scrim opacity at `y` of a frame `h` tall (`IMAGE_SCRIM`).
pub fn scrim_alpha(y: f64, h: f64) -> f64 {
    let u = ((h - y) / h).clamp(0.0, 1.0);
    if u <= 0.30 {
        0.62 + (0.45 - 0.62) * u / 0.30
    } else if u <= 0.62 {
        0.45 * (1.0 - (u - 0.30) / 0.32)
    } else {
        0.0
    }
}

fn mix(top: [u8; 3], alpha: f64, under: [u8; 3]) -> [u8; 3] {
    over(
        [top[0], top[1], top[2], (alpha * 255.0).round() as u8],
        under,
    )
}

/// The scene background as the renderer draws it, before the pose.
struct Canvas {
    img: RgbImage,
    /// What shows where the moved picture leaves the frame (same size as `img`).
    backdrop: Option<RgbImage>,
}

impl Canvas {
    fn backdrop_at(&self, x: f64, y: f64) -> [u8; 3] {
        match &self.backdrop {
            Some(b) => pixel(b, x, y),
            None => PADDING,
        }
    }
}

fn pixel(img: &RgbImage, x: f64, y: f64) -> [u8; 3] {
    let px = (x.floor().max(0.0) as u32).min(img.width() - 1);
    let py = (y.floor().max(0.0) as u32).min(img.height() - 1);
    img.get_pixel(px, py).0
}

fn sample_size(res: Resolution) -> (u32, u32) {
    (
        ((f64::from(res.width) * SAMPLE_SCALE).round() as u32).max(1),
        ((f64::from(res.height) * SAMPLE_SCALE).round() as u32).max(1),
    )
}

/// `vimmer_media::letterbox` at the sampling size; the blur is a cheap downscale and upscale.
fn letterbox(src: &RgbImage, tw: u32, th: u32) -> Canvas {
    let (sw, sh) = (f64::from(src.width()), f64::from(src.height()));
    let cover = (f64::from(tw) / sw).max(f64::from(th) / sh);
    let (cw, ch) = (
        ((sw * cover).ceil() as u32).max(tw),
        ((sh * cover).ceil() as u32).max(th),
    );
    let covered = imageops::resize(src, cw, ch, FilterType::Triangle);
    let covered = imageops::crop_imm(&covered, (cw - tw) / 2, (ch - th) / 2, tw, th).to_image();
    let blurred = blur(&covered, 0.45);
    let contain = (f64::from(tw) / sw).min(f64::from(th) / sh);
    let (fw, fh) = (
        ((sw * contain).round() as u32).clamp(1, tw),
        ((sh * contain).round() as u32).clamp(1, th),
    );
    let fg = imageops::resize(src, fw, fh, FilterType::Triangle);
    let mut img = blurred;
    imageops::overlay(
        &mut img,
        &fg,
        i64::from((tw - fw) / 2),
        i64::from((th - fh) / 2),
    );
    // KenBurnsImage's backdrop: the letterboxed file again, 1.2x, blurred, 0.45 bright.
    let backdrop = blur(&img, 0.45);
    Canvas {
        img,
        backdrop: Some(backdrop),
    }
}

/// A heavy blur (down to 1/16 and back) times `brightness`.
fn blur(img: &RgbImage, brightness: f64) -> RgbImage {
    let (w, h) = img.dimensions();
    let small = imageops::resize(img, (w / 16).max(1), (h / 16).max(1), FilterType::Triangle);
    let mut out = imageops::resize(&small, w, h, FilterType::Triangle);
    for p in out.pixels_mut() {
        for c in p.0.iter_mut() {
            *c = (f64::from(*c) * brightness) as u8;
        }
    }
    out
}

/// The Ken Burns pose at `time_ms` of a scene `scene_ms` long (`KenBurnsImage.tsx`).
fn pose_at(scene: &Scene, time_ms: f64, fps: u32) -> Pose {
    if !scene.keyframes.is_empty() {
        return sample(&scene.keyframes, time_ms).0;
    }
    let frames = scene.duration_frames(fps).max(2);
    let frame = (time_ms * f64::from(fps) / 1000.0).floor();
    let p = (frame / f64::from(frames - 1)).clamp(0.0, 1.0);
    let lerp = |a: f64, b: f64| a + (b - a) * p;
    let (scale, x) = match scene.motion {
        MotionType::Static => (1.0, 0.0),
        MotionType::ZoomIn => (lerp(1.0, 1.12), 0.0),
        MotionType::ZoomOut => (lerp(1.12, 1.0), 0.0),
        MotionType::PanLeft => (1.08, lerp(3.0, -3.0)),
        MotionType::PanRight => (1.08, lerp(-3.0, 3.0)),
    };
    Pose {
        scale,
        x,
        y: 0.0,
        rotation: 0.0,
    }
}

fn moves(scene: &Scene) -> bool {
    scene.keyframes.len() > 1 || scene.motion != MotionType::Static
}

/// The background color at frame point (`x`, `y`) in px.
fn background_at(canvas: &Canvas, pose: &Pose, res: Resolution, x: f64, y: f64) -> [u8; 3] {
    let (w, h) = (f64::from(res.width), f64::from(res.height));
    let (cx, cy) = (w / 2.0, h / 2.0);
    let (dx, dy) = (x - cx - pose.x / 100.0 * w, y - cy - pose.y / 100.0 * h);
    let (sin, cos) = pose.rotation.to_radians().sin_cos();
    let s = pose.scale.max(1e-6);
    let qx = (dx * cos + dy * sin) / s + cx;
    let qy = (-dx * sin + dy * cos) / s + cy;
    if (0.0..w).contains(&qx) && (0.0..h).contains(&qy) {
        pixel(&canvas.img, qx * SAMPLE_SCALE, qy * SAMPLE_SCALE)
    } else {
        let bx = cx + (x - cx) / 1.2;
        let by = cy + (y - cy) / 1.2;
        canvas.backdrop_at(bx * SAMPLE_SCALE, by * SAMPLE_SCALE)
    }
}

/// Where in the clip file scene time `t_ms` falls (`motion/video.ts` `clipTimeAt`).
fn clip_file_ms(clip: &VideoClip, t_ms: u64, span_ms: u64, file_ms: u64) -> u64 {
    let end = clip.trim_end_ms.unwrap_or(file_ms).min(file_ms);
    let source = end.saturating_sub(clip.trim_start_ms).max(1);
    let speed = clip.effective_speed(source, span_ms.max(1));
    let into = (t_ms as f64 * speed) as u64;
    let into = match clip.fit_time {
        FitTime::Loop => into % source,
        FitTime::Freeze | FitTime::Stretch => into.min(source.saturating_sub(40)),
    };
    clip.trim_start_ms + into
}

/// One frame of `path` at `ms`, fitted to `tw` x `th`, as RGB.
fn video_frame(
    ffmpeg: &Path,
    path: &Path,
    ms: u64,
    fit: ImageFit,
    tw: u32,
    th: u32,
) -> Option<RgbImage> {
    let vf = match fit {
        ImageFit::Cover => {
            format!("scale={tw}:{th}:force_original_aspect_ratio=increase,crop={tw}:{th}")
        }
        ImageFit::Contain => format!(
            "scale={tw}:{th}:force_original_aspect_ratio=decrease,pad={tw}:{th}:(ow-iw)/2:(oh-ih)/2:color=0x09090b"
        ),
    };
    let out = Command::new(ffmpeg)
        .args(["-hide_banner", "-nostdin", "-loglevel", "error", "-ss"])
        .arg(format!("{:.3}", ms as f64 / 1000.0))
        .arg("-i")
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-vf",
            &vf,
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "-",
        ])
        .output()
        .ok()?;
    let need = (tw * th * 3) as usize;
    if !out.status.success() || out.stdout.len() < need {
        return None;
    }
    RgbImage::from_raw(tw, th, out.stdout[..need].to_vec())
}

/// The scene's picture, decoded on demand.
enum Source {
    Image(Canvas),
    Video {
        path: PathBuf,
        file_ms: u64,
        frames: HashMap<u64, Option<Canvas>>,
    },
}

struct Sampler<'a> {
    ffmpeg: &'a Path,
    decoded: usize,
}

impl Sampler<'_> {
    fn canvas<'s>(
        &mut self,
        src: &'s mut Source,
        scene: &Scene,
        t_ms: u64,
        res: Resolution,
    ) -> Option<&'s Canvas> {
        match src {
            Source::Image(c) => Some(c),
            Source::Video {
                path,
                file_ms,
                frames,
            } => {
                let clip = scene.video.as_ref()?;
                let at = clip_file_ms(clip, t_ms, scene.duration_ms, *file_ms);
                if let std::collections::hash_map::Entry::Vacant(slot) = frames.entry(at) {
                    if self.decoded >= MAX_VIDEO_FRAMES {
                        return None;
                    }
                    self.decoded += 1;
                    let (tw, th) = sample_size(res);
                    slot.insert(
                        video_frame(self.ffmpeg, path, at, clip.fit, tw, th).map(|img| Canvas {
                            img,
                            backdrop: None,
                        }),
                    );
                }
                frames.get(&at)?.as_ref()
            }
        }
    }
}

fn open_source(scene: &Scene, dir: &Path, res: Resolution, ffmpeg: &Path) -> Option<Source> {
    if let Some(rel) = scene.image.as_deref() {
        if is_url(rel) {
            return None;
        }
        let src = image::open(dir.join(rel)).ok()?.to_rgb8();
        if src.width() == 0 || src.height() == 0 {
            return None;
        }
        let (tw, th) = sample_size(res);
        return Some(Source::Image(letterbox(&src, tw, th)));
    }
    let clip = scene.video.as_ref()?;
    if is_url(&clip.src) {
        return None;
    }
    let path = dir.join(&clip.src);
    let file_ms = vimmer_media::probe_video(ffmpeg, &path).ok()?.duration_ms;
    Some(Source::Video {
        path,
        file_ms,
        frames: HashMap::new(),
    })
}

/// A shape drawn between the picture and a text.
#[derive(Debug, Clone)]
struct Underlay {
    /// (left, top, right, bottom) in px.
    edges: (f64, f64, f64, f64),
    ellipse: bool,
    color: [u8; 4],
    /// Drawn before the scrim (z < 0).
    under_scrim: bool,
}

impl Underlay {
    fn contains(&self, x: f64, y: f64) -> bool {
        let (l, t, r, b) = self.edges;
        if !(l..r).contains(&x) || !(t..b).contains(&y) {
            return false;
        }
        if !self.ellipse {
            return true;
        }
        let (cx, cy, rx, ry) = ((l + r) / 2.0, (t + b) / 2.0, (r - l) / 2.0, (b - t) / 2.0);
        ((x - cx) / rx).powi(2) + ((y - cy) / ry).powi(2) <= 1.0
    }
}

/// Filled shapes drawn before layer `index` (or, with `None`, before the scene text), in draw
/// order. Rotation, keyframes and entrance animations are ignored (the box at rest).
fn underlays(scene: &Scene, index: Option<usize>, res: Resolution) -> Vec<Underlay> {
    let mut order: Vec<usize> = (0..scene.layers.len()).collect();
    order.sort_by_key(|&i| (scene.layers[i].z, i));
    let before = |i: usize| match index {
        Some(j) => (scene.layers[i].z, i) < (scene.layers[j].z, j),
        None => scene.layers[i].z < 0,
    };
    order
        .into_iter()
        .filter(|&i| before(i))
        .filter_map(|i| {
            let l = &scene.layers[i];
            let LayerContent::Shape(sh) = &l.content else {
                return None;
            };
            if sh.shape == vimmer_dsl::ShapeKind::Line {
                return None;
            }
            let mut c = rgb(sh.fill.as_deref()?)?;
            c[3] = (f64::from(c[3]) * f64::from(l.opacity).clamp(0.0, 1.0)).round() as u8;
            let (x, y, w, h) = place(
                l,
                res,
                f64::from(l.height.unwrap_or(0.0)) * f64::from(res.height),
            );
            Some(Underlay {
                edges: (x, y, x + w, y + h),
                ellipse: sh.shape == vimmer_dsl::ShapeKind::Ellipse,
                color: c,
                under_scrim: l.z < 0,
            })
        })
        .collect()
}

/// The thinnest outline (px on the frame) that the text is read against instead of the picture:
/// 2 px, or 2 % of the font size for bigger text.
pub fn min_readable_outline_px(font_px: f64) -> f64 {
    (font_px * 0.02).max(2.0)
}

/// The outline color of `style`, when its outline is wide enough for text of `font_px`.
fn reads_on_outline(style: &TextStyle, font_px: f64, res: Resolution) -> Option<[u8; 4]> {
    let o = style.drawn_outline()?;
    let px = f64::from(o.width) * style_scale(res.width, res.height);
    (px >= min_readable_outline_px(font_px)).then(|| rgb(&o.color)).flatten()
}

/// One text to check.
struct Text {
    what: String,
    fg: String,
    rect: Rect,
    large: bool,
    /// Drawn below the scrim (a layer with z < 0).
    under_scrim: bool,
    /// The text's own translucent background.
    box_color: Option<[u8; 4]>,
    /// An outline wide enough to read the text against ([`reads_on_outline`]): the text is then
    /// compared with the outline color over what is under it.
    outline: Option<[u8; 4]>,
    underlays: Vec<Underlay>,
    from_ms: u64,
    to_ms: u64,
    fix: Suggestion,
}

fn texts(scene: &Scene, si: usize, t: &VideoTimeline, fonts: &Fonts) -> Vec<Text> {
    let brand = t.brand.as_ref();
    let res = t.resolution;
    let fh = f64::from(res.height);
    let text_color = Brand::text(brand).to_string();
    let brand_text = brand.and_then(|b| b.colors.text.as_ref()).is_some();
    let mut out = Vec::new();
    let style = scene.effective_text_style(brand);
    if let Some(rect) = scene
        .text
        .as_deref()
        .and_then(|s| textbox::scene_text(s, res, fonts, &style))
    {
        let outline = reads_on_outline(&style, SCENE_FONT_PX, res);
        let fix = if outline.is_some() {
            Suggestion::field(
                "use an outline color that differs more from the text color",
                if scene.text_style.as_ref().is_some_and(|s| s.outline.is_some()) {
                    format!("/scenes/{si}/text_style/outline/color")
                } else {
                    "/brand/text_style/outline/color".into()
                },
            )
        } else {
            let outline = json!({"color": "#000000", "width": 4});
            match &scene.text_style {
                None => Suggestion::set(
                    "give the scene text a black outline (or use a darker, calmer picture behind it)",
                    format!("/scenes/{si}/text_style"),
                    json!({ "outline": outline }),
                ),
                Some(ts) => {
                    let mut ts = serde_json::to_value(ts).unwrap_or_else(|_| json!({}));
                    ts["outline"] = outline;
                    Suggestion::set(
                        "give the scene text a black outline (or use a darker, calmer picture behind it)",
                        format!("/scenes/{si}/text_style"),
                        ts,
                    )
                }
            }
        };
        out.push(Text {
            what: "the scene text".into(),
            fg: text_color.clone(),
            rect,
            large: is_large(SCENE_FONT_PX / fh, 600),
            under_scrim: false,
            box_color: style.background.as_ref().and_then(|b| rgb(&b.color)),
            outline,
            underlays: underlays(scene, None, res),
            from_ms: 0,
            to_ms: scene.duration_ms,
            fix,
        });
    }
    for (li, layer) in scene.layers.iter().enumerate() {
        // An opaque box or a covering shape is checked by `brand.contrast`.
        if behind(scene, li, None).is_some() || over_media_layer(scene, li) {
            continue;
        }
        let lp = |f: &str| format!("/scenes/{si}/layers/{li}/{f}");
        let name = format!("layer `{}`", layer.id);
        let rect = textbox::layer_box(layer, res, fonts, brand);
        let to_ms = layer.end_ms_in(scene.duration_ms);
        let base = |what: String, fg: String, large: bool, box_color: Option<[u8; 4]>, fix| Text {
            what,
            fg,
            rect,
            large,
            under_scrim: layer.z < 0,
            box_color,
            outline: None,
            underlays: underlays(scene, Some(li), res),
            from_ms: layer.start_ms,
            to_ms,
            fix,
        };
        match &layer.content {
            LayerContent::Text(tl) => {
                let style = tl.effective_style(brand);
                let box_color = style.background.as_ref().and_then(|b| rgb(&b.color));
                if box_color.is_some_and(|c| c[3] == 255) {
                    continue;
                }
                let outline = reads_on_outline(&style, f64::from(tl.font_size) * fh, res);
                let fix = if outline.is_some() {
                    Suggestion::field(
                        "use an outline color that differs more from the text color",
                        if tl.outline.is_some() {
                            lp("outline/color")
                        } else {
                            "/brand/text_style/outline/color".into()
                        },
                    )
                } else {
                    Suggestion::set(
                        "give the text a dark background box",
                        lp("background"),
                        json!({"color": "#09090bcc"}),
                    )
                };
                let mut item = base(
                    name,
                    tl.color.clone().unwrap_or_else(|| text_color.clone()),
                    is_large(f64::from(tl.font_size), tl.weight),
                    box_color,
                    fix,
                );
                item.outline = outline;
                out.push(item);
            }
            LayerContent::Counter(c) => out.push(base(
                name,
                c.color.clone().unwrap_or_else(|| text_color.clone()),
                is_large(f64::from(c.font_size), c.weight),
                None,
                Suggestion::field(
                    "change the counter's color or put a shape behind it",
                    if c.color.is_some() || !brand_text {
                        lp("color")
                    } else {
                        "/brand/colors/text".into()
                    },
                ),
            )),
            LayerContent::LowerThird(lt) => {
                // `undefined` draws #09090bd9; `null` (no box) cannot be told apart here.
                let bg = lt.background_color.as_deref().unwrap_or("#09090bd9");
                let box_color = rgb(bg);
                if box_color.is_some_and(|c| c[3] == 255) {
                    continue;
                }
                let fix = || {
                    Suggestion::set(
                        "make the lower third's background opaque",
                        lp("background_color"),
                        json!("#09090b"),
                    )
                };
                out.push(base(
                    format!("{name} title"),
                    lt.color.clone().unwrap_or_else(|| text_color.clone()),
                    is_large(f64::from(lt.font_size), 700),
                    box_color,
                    fix(),
                ));
                if lt.subtitle.is_some() {
                    out.push(base(
                        format!("{name} subtitle"),
                        lt.subtitle_color.clone().unwrap_or_else(|| {
                            Brand::muted_text(brand, DEFAULT_LOWER_THIRD_SUBTITLE).to_string()
                        }),
                        is_large(f64::from(lt.font_size) * 0.62, 500),
                        box_color,
                        fix(),
                    ));
                }
            }
            _ => {}
        }
    }
    out
}

/// Whether an image or video layer drawn before layer `index` meets its box (the background
/// under the text is then not the scene's picture: not checked).
fn over_media_layer(scene: &Scene, index: usize) -> bool {
    let target = &scene.layers[index];
    let key = |i: usize| (scene.layers[i].z, i);
    scene.layers.iter().enumerate().any(|(i, l)| {
        matches!(l.content, LayerContent::Image(_) | LayerContent::Video(_))
            && key(i) < key(index)
            && frac_overlap(l, target)
    })
}

fn frac_overlap(a: &Layer, b: &Layer) -> bool {
    let bx = |l: &Layer| {
        let h = l.height.unwrap_or(1.0 - l.y);
        let (mut x, mut y) = (l.x, l.y);
        if l.anchor == vimmer_dsl::Anchor::Center {
            x -= l.width / 2.0;
            y -= h / 2.0;
        }
        (x, y, x + l.width, y + h)
    };
    let (a, b) = (bx(a), bx(b));
    a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3
}

/// The moments sampled in `from..to`.
fn times(from: u64, to: u64, moving: bool) -> Vec<u64> {
    let span = to.saturating_sub(from);
    if !moving || span < 300 {
        return vec![from + span / 2];
    }
    // After the words have faded in (12 frames), the middle and near the end.
    let mut v = vec![
        from + (span / 10).max(400).min(span / 2),
        from + span / 2,
        to - span / 20 - 1,
    ];
    v.dedup();
    v.truncate(TIMES);
    v
}

/// Grid points inside `rect` clipped to the frame.
fn grid(rect: Rect, res: Resolution) -> Vec<(f64, f64)> {
    let (w, h) = (f64::from(res.width), f64::from(res.height));
    let (l, t) = (rect.0.max(0.0), rect.1.max(0.0));
    let (r, b) = ((rect.0 + rect.2).min(w), (rect.1 + rect.3).min(h));
    if r - l < 1.0 || b - t < 1.0 {
        return Vec::new();
    }
    let aspect = (r - l) / (b - t);
    let rows = ((GRID_POINTS as f64 / aspect).sqrt().round() as usize).clamp(1, GRID_POINTS);
    let cols = (GRID_POINTS / rows).max(1);
    let mut out = Vec::with_capacity(rows * cols);
    for i in 0..rows {
        for j in 0..cols {
            out.push((
                l + (r - l) * (j as f64 + 0.5) / cols as f64,
                t + (b - t) * (i as f64 + 0.5) / rows as f64,
            ));
        }
    }
    out
}

fn percentile(mut v: Vec<f64>, p: f64) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    Some(v[((v.len() - 1) as f64 * p).round() as usize])
}

/// Contrast of `text` at one moment, and the median background color under it.
fn contrast_at(
    text: &Text,
    scene_text: bool,
    canvas: &Canvas,
    pose: &Pose,
    res: Resolution,
) -> Option<(f64, [u8; 3])> {
    let fg = rgb(&text.fg)?;
    let h = f64::from(res.height);
    let mut ratios = Vec::new();
    let mut lum = Vec::new();
    for (x, y) in grid(text.rect, res) {
        let mut bg = background_at(canvas, pose, res, x, y);
        let scrim = if scene_text { scrim_alpha(y, h) } else { 0.0 };
        for u in text.underlays.iter().filter(|u| u.under_scrim) {
            if u.contains(x, y) {
                bg = over(u.color, bg);
            }
        }
        if !text.under_scrim {
            bg = mix(SCRIM, scrim, bg);
        }
        for u in text.underlays.iter().filter(|u| !u.under_scrim) {
            if u.contains(x, y) {
                bg = over(u.color, bg);
            }
        }
        if let Some(c) = text.box_color {
            bg = over(c, bg);
        }
        if let Some(c) = text.outline {
            bg = over(c, bg);
        }
        let mut f = over(fg, bg);
        if text.under_scrim {
            bg = mix(SCRIM, scrim, bg);
            f = mix(SCRIM, scrim, f);
        }
        ratios.push(contrast_ratio(f, bg));
        lum.push((vimmer_dsl::brand::luminance(bg), bg));
    }
    let ratio = percentile(ratios, PERCENTILE)?;
    lum.sort_by(|a, b| a.0.total_cmp(&b.0));
    Some((ratio, lum[lum.len() / 2].1))
}

/// Text over scene images and videos with too little contrast.
pub fn check(t: &VideoTimeline, public_dir: &Path, ffmpeg: &Path, fonts: &Fonts) -> Vec<Finding> {
    let mut out = Vec::new();
    let res = t.resolution;
    let starts = scene_starts_ms(t);
    let mut sampler = Sampler { ffmpeg, decoded: 0 };
    for (si, scene) in t.scenes.iter().enumerate() {
        if scene.image.is_none() && scene.video.is_none() {
            continue;
        }
        let items = texts(scene, si, t, fonts);
        if items.is_empty() {
            continue;
        }
        let Some(mut source) = open_source(scene, public_dir, res, ffmpeg) else {
            continue;
        };
        let has_scene_text = scene.text.as_deref().is_some_and(|s| !s.trim().is_empty());
        let moving = moves(scene) || scene.video.is_some();
        for text in &items {
            let min = if text.large {
                LARGE_MIN_RATIO
            } else {
                BODY_MIN_RATIO
            };
            let mut worst: Option<(f64, u64, [u8; 3])> = None;
            for at in times(text.from_ms, text.to_ms, moving) {
                let pose = pose_at(scene, at as f64, t.fps);
                let Some(canvas) = sampler.canvas(&mut source, scene, at, res) else {
                    continue;
                };
                let Some((ratio, bg)) = contrast_at(text, has_scene_text, canvas, &pose, res)
                else {
                    continue;
                };
                if worst.is_none_or(|w| ratio < w.0) {
                    worst = Some((ratio, at, bg));
                }
            }
            let Some((ratio, at, bg)) = worst else {
                continue;
            };
            if ratio >= min {
                continue;
            }
            let size = if text.large { "large" } else { "body" };
            let picture = match (&scene.image, &scene.video) {
                (Some(i), _) => format!("the image `{i}`"),
                (_, Some(v)) => format!("the video `{}`", v.src),
                _ => unreachable!(),
            };
            out.push(
                Finding::new(
                    Severity::Warning,
                    "text.contrast_media",
                    format!(
                        "{} ({}) over {picture}: contrast {ratio:.2}:1 at {} into the scene (the lowest {:.0}% of the text box; typical background {}), below {min}:1 for {size} text",
                        text.what,
                        text.fg,
                        secs(at),
                        PERCENTILE * 100.0,
                        hex(bg),
                    ),
                    text.fix.clone(),
                )
                .scene(&scene.id)
                .at(starts[si] + at),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    const RES: Resolution = Resolution {
        width: 1920,
        height: 1080,
    };

    fn solid(c: [u8; 3]) -> Canvas {
        let (w, h) = sample_size(RES);
        letterbox(&RgbImage::from_pixel(w, h, Rgb(c)), w, h)
    }

    fn still() -> Pose {
        Pose {
            scale: 1.0,
            x: 0.0,
            y: 0.0,
            rotation: 0.0,
        }
    }

    fn text(fg: &str, rect: Rect, large: bool) -> Text {
        Text {
            what: "t".into(),
            fg: fg.into(),
            rect,
            large,
            under_scrim: false,
            box_color: None,
            outline: None,
            underlays: Vec::new(),
            from_ms: 0,
            to_ms: 1000,
            fix: Suggestion::text(""),
        }
    }

    #[test]
    fn scrim_matches_the_gradient_stops() {
        assert!((scrim_alpha(1080.0, 1080.0) - 0.62).abs() < 1e-9);
        assert!((scrim_alpha(1080.0 * 0.7, 1080.0) - 0.45).abs() < 1e-9);
        assert!(scrim_alpha(1080.0 * 0.38, 1080.0).abs() < 1e-9);
        assert_eq!(scrim_alpha(10.0, 1080.0), 0.0);
    }

    #[test]
    fn white_on_white_fails_and_white_on_black_passes() {
        let r = (200.0, 100.0, 600.0, 100.0);
        let white = solid([255, 255, 255]);
        let (ratio, _) =
            contrast_at(&text("#ffffff", r, true), false, &white, &still(), RES).unwrap();
        assert!(ratio < 1.1, "{ratio}");
        let black = solid([0, 0, 0]);
        let (ratio, _) =
            contrast_at(&text("#ffffff", r, true), false, &black, &still(), RES).unwrap();
        assert!(ratio > 20.0, "{ratio}");
    }

    #[test]
    fn scrim_and_text_box_lift_contrast_near_the_bottom() {
        let pale = solid([235, 235, 235]);
        let low = (200.0, 900.0, 600.0, 100.0);
        let (plain, _) =
            contrast_at(&text("#fafafa", low, true), false, &pale, &still(), RES).unwrap();
        let (scrimmed, _) =
            contrast_at(&text("#fafafa", low, true), true, &pale, &still(), RES).unwrap();
        assert!(plain < 1.2 && scrimmed > 2.0, "{plain} {scrimmed}");
        let mut boxed = text("#fafafa", (200.0, 100.0, 600.0, 100.0), false);
        boxed.box_color = rgb("#09090bcc");
        let (ratio, _) = contrast_at(&boxed, false, &pale, &still(), RES).unwrap();
        assert!(ratio > BODY_MIN_RATIO, "{ratio}");
    }

    #[test]
    fn a_wide_outline_is_what_the_text_is_read_against() {
        let white = solid([255, 255, 255]);
        let r = (200.0, 100.0, 600.0, 100.0);
        let mut t = text("#fafafa", r, true);
        t.outline = rgb("#000000");
        let (ratio, _) = contrast_at(&t, false, &white, &still(), RES).unwrap();
        assert!(ratio > 18.0, "{ratio}");
        t.outline = rgb("#e4e4e7");
        let (ratio, _) = contrast_at(&t, false, &white, &still(), RES).unwrap();
        assert!(ratio < 1.5, "{ratio}");
        // Too thin to count: 1 px at 1080p on 92 px text.
        let thin: TextStyle = serde_json::from_str(r##"{"outline":{"color":"#000","width":1}}"##).unwrap();
        assert_eq!(reads_on_outline(&thin, 92.0, RES), None);
        let ok: TextStyle = serde_json::from_str(r##"{"outline":{"color":"#000","width":2}}"##).unwrap();
        assert_eq!(reads_on_outline(&ok, 92.0, RES), Some([0, 0, 0, 255]));
        assert!((min_readable_outline_px(200.0) - 4.0).abs() < 1e-9);
    }

    #[test]
    fn zoomed_out_picture_shows_the_dim_backdrop_at_the_edges() {
        let white = solid([255, 255, 255]);
        let pose = Pose {
            scale: 0.5,
            ..still()
        };
        assert_eq!(
            background_at(&white, &pose, RES, 960.0, 540.0),
            [255, 255, 255]
        );
        let edge = background_at(&white, &pose, RES, 10.0, 10.0);
        assert!(edge[0] < 130, "{edge:?}");
    }

    #[test]
    fn clip_time_follows_trim_speed_and_freeze() {
        let mut clip = VideoClip::new("c.mp4");
        clip.trim_start_ms = 1000;
        assert_eq!(clip_file_ms(&clip, 500, 4000, 10_000), 1500);
        assert_eq!(clip_file_ms(&clip, 9500, 10_000, 10_000), 1000 + 9000 - 40);
        clip.fit_time = FitTime::Loop;
        assert_eq!(clip_file_ms(&clip, 9500, 10_000, 3000), 1000 + 9500 % 2000);
    }

    #[test]
    fn grid_is_capped_and_clipped() {
        let g = grid((-100.0, 1000.0, 3000.0, 500.0), RES);
        assert!(!g.is_empty() && g.len() <= GRID_POINTS);
        assert!(g
            .iter()
            .all(|&(x, y)| (0.0..=1920.0).contains(&x) && (1000.0..=1080.0).contains(&y)));
    }
}
