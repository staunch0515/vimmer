//! Built-in scene templates: a scene with `template: {name, params}` is turned into ordinary
//! layers (a background, text, shapes, the brand logo) by [`expand_templates`], before
//! validation and rendering. Expansion is a pure function of the scene, the brand and the
//! resolution, so the scheduler, `vimmer check` and the editor preview (which calls the same code
//! through the CLI or the desktop shell) all see the same layers.
//!
//! Rules every template follows:
//!
//! * Template layers get ids starting with `tpl-` (reserved on template scenes) and come first in
//!   the scene's layer list with one `z` below every explicit layer, so explicit layers on a
//!   template scene are drawn above the template. They are also below the scene `text`.
//! * A scene without an image or video gets an opaque background in the brand's `background`
//!   color; with one, a translucent scrim of that color keeps the text readable.
//! * Text uses the brand fonts (`heading` for titles) and colors. Text colors that equal the
//!   brand `text` color are left unset, so they follow the brand like any layer.
//! * `duration_ms` 0 (or left out) takes the template's default length. Animation times shrink
//!   with short scenes so they always fit.
//! * Unknown params, a missing required param, a wrong type and text longer than the param's
//!   limit are errors. The layouts are separate for landscape and portrait frames.
//!
//! [`CATALOG`] describes every template and its params; it is written to
//! `remotion-renderer/editor/src/template-catalog.json` (checked by a test) for the editor's params form.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::brand::{Brand, FontRole, DEFAULT_LOWER_THIRD_SUBTITLE};
use crate::layer::{
    Anchor, ImageFit, ImageLayer, Layer, LayerAnimation, LayerAnimationKind, LayerContent,
    ShapeKind, ShapeLayer, TextAlign, TextLayer, TextReveal,
};
use crate::model::{Resolution, Scene, VideoTimeline};

/// Prefix of the ids of layers a template makes.
pub const TEMPLATE_LAYER_PREFIX: &str = "tpl-";
/// Shortest template scene, in ms.
pub const MIN_TEMPLATE_MS: u64 = 1_000;

/// A scene made from a built-in template.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneTemplate {
    /// One of [`CATALOG`]'s names.
    pub name: String,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub params: Map<String, Value>,
}

/// The type of a template param.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ParamKind {
    /// A string (a number is accepted and written as text) of at most `max_chars` characters.
    Text { max_chars: usize },
    /// A list of `min_items..=max_items` strings of at most `max_chars` characters each.
    Lines {
        min_items: usize,
        max_items: usize,
        max_chars: usize,
    },
    /// `true` or `false`.
    Bool { default: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ParamSpec {
    pub name: &'static str,
    pub label: &'static str,
    pub required: bool,
    #[serde(flatten)]
    pub kind: ParamKind,
    pub help: &'static str,
}

/// One built-in template.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TemplateInfo {
    pub name: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub default_ms: u64,
    pub params: &'static [ParamSpec],
}

const fn text(
    name: &'static str,
    label: &'static str,
    required: bool,
    max_chars: usize,
    help: &'static str,
) -> ParamSpec {
    ParamSpec {
        name,
        label,
        required,
        kind: ParamKind::Text { max_chars },
        help,
    }
}

const LOGO_PARAM: ParamSpec = ParamSpec {
    name: "logo",
    label: "Show logo",
    required: false,
    kind: ParamKind::Bool { default: true },
    help: "Draw the brand logo large (when the brand has one).",
};

/// Every built-in template.
pub const CATALOG: &[TemplateInfo] = &[
    TemplateInfo {
        name: "intro",
        label: "Intro",
        description: "Logo, title and subtitle, animated in.",
        default_ms: 4_000,
        params: &[
            text("title", "Title", true, 60, "The video's title."),
            text(
                "subtitle",
                "Subtitle",
                false,
                100,
                "One line under the title.",
            ),
            LOGO_PARAM,
        ],
    },
    TemplateInfo {
        name: "outro",
        label: "Outro",
        description: "A call to action, the logo and up to 3 short lines (handles, a URL).",
        default_ms: 5_000,
        params: &[
            text(
                "cta",
                "Call to action",
                true,
                50,
                "For example \"Subscribe for more\".",
            ),
            ParamSpec {
                name: "lines",
                label: "Lines",
                required: false,
                kind: ParamKind::Lines {
                    min_items: 0,
                    max_items: 3,
                    max_chars: 40,
                },
                help: "Up to 3 short lines such as @handle or example.com.",
            },
            LOGO_PARAM,
        ],
    },
    TemplateInfo {
        name: "title_card",
        label: "Title card",
        description: "A section number and a title.",
        default_ms: 3_000,
        params: &[
            text("number", "Number", false, 4, "Section number, e.g. 01."),
            text("title", "Title", true, 60, "The section title."),
        ],
    },
    TemplateInfo {
        name: "quote",
        label: "Quote",
        description: "A quote, revealed word by word, and its author.",
        default_ms: 6_000,
        params: &[
            text(
                "quote",
                "Quote",
                true,
                220,
                "The quoted text, without quote marks.",
            ),
            text("author", "Author", false, 60, "Who said it."),
        ],
    },
    TemplateInfo {
        name: "list",
        label: "List",
        description: "A title and up to 5 bullet items revealed one by one.",
        default_ms: 6_000,
        params: &[
            text("title", "Title", true, 60, "The list's heading."),
            ParamSpec {
                name: "items",
                label: "Items",
                required: true,
                kind: ParamKind::Lines {
                    min_items: 1,
                    max_items: 5,
                    max_chars: 60,
                },
                help: "1 to 5 short items.",
            },
        ],
    },
];

/// The template called `name`.
pub fn template_info(name: &str) -> Option<&'static TemplateInfo> {
    CATALOG.iter().find(|t| t.name == name)
}

/// A template problem of one scene.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateError {
    pub scene: String,
    pub message: String,
}

impl std::fmt::Display for TemplateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "scene `{}` template: {}", self.scene, self.message)
    }
}

/// Checked param values.
enum Arg {
    Text(String),
    Lines(Vec<String>),
    Bool(bool),
}

/// Why the params do not fit the template, as messages; or the checked values.
fn check_params(
    info: &TemplateInfo,
    params: &Map<String, Value>,
) -> Result<Vec<(&'static str, Option<Arg>)>, Vec<String>> {
    let mut errors = Vec::new();
    for key in params.keys() {
        if !info.params.iter().any(|p| p.name == key) {
            let names: Vec<&str> = info.params.iter().map(|p| p.name).collect();
            errors.push(format!(
                "unknown param `{key}` for `{}`; its params are {}",
                info.name,
                names.join(", ")
            ));
        }
    }
    let mut out = Vec::new();
    for spec in info.params {
        let value = params.get(spec.name).filter(|v| !v.is_null());
        let arg = match (value, spec.kind) {
            (None, _) => None,
            (Some(v), ParamKind::Text { max_chars }) => {
                let s = match v {
                    Value::String(s) => Some(s.trim().to_string()),
                    Value::Number(n) => Some(n.to_string()),
                    _ => None,
                };
                match s {
                    None => {
                        errors.push(format!("`{}` must be text", spec.name));
                        None
                    }
                    Some(s) if s.is_empty() => None,
                    Some(s) => {
                        let n = s.chars().count();
                        if n > max_chars {
                            errors.push(format!(
                                "`{}` is {n} characters; the limit is {max_chars}",
                                spec.name
                            ));
                        }
                        if s.chars().any(|c| c.is_control() && c != '\n') {
                            errors.push(format!("`{}` has control characters", spec.name));
                        }
                        Some(Arg::Text(s))
                    }
                }
            }
            (
                Some(v),
                ParamKind::Lines {
                    max_items,
                    max_chars,
                    ..
                },
            ) => match v.as_array() {
                Some(items) if items.iter().all(Value::is_string) => {
                    let lines: Vec<String> = items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(|s| s.trim().to_string())
                        .collect();
                    if lines.len() > max_items {
                        errors.push(format!(
                            "`{}` has {} items; the limit is {max_items}",
                            spec.name,
                            lines.len()
                        ));
                    }
                    for (i, l) in lines.iter().enumerate() {
                        let n = l.chars().count();
                        if l.is_empty() {
                            errors.push(format!("`{}` item {} is empty", spec.name, i + 1));
                        } else if n > max_chars {
                            errors.push(format!(
                                "`{}` item {} is {n} characters; the limit is {max_chars}",
                                spec.name,
                                i + 1
                            ));
                        }
                    }
                    Some(Arg::Lines(lines))
                }
                _ => {
                    errors.push(format!("`{}` must be a list of strings", spec.name));
                    None
                }
            },
            (Some(v), ParamKind::Bool { .. }) => match v.as_bool() {
                Some(b) => Some(Arg::Bool(b)),
                None => {
                    errors.push(format!("`{}` must be true or false", spec.name));
                    None
                }
            },
        };
        let present = match &arg {
            None => false,
            Some(Arg::Lines(l)) => !l.is_empty(),
            Some(_) => true,
        };
        if let ParamKind::Lines { min_items, .. } = spec.kind {
            let n = match &arg {
                Some(Arg::Lines(l)) => l.len(),
                _ => 0,
            };
            if present && n < min_items {
                errors.push(format!("`{}` needs at least {min_items} items", spec.name));
            }
        }
        if spec.required && !present && value.is_none_or(|v| v.is_string() || v.is_array()) {
            errors.push(format!("`{}` is required", spec.name));
        }
        out.push((spec.name, arg));
    }
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors)
    }
}

/// The checked args, by name.
struct Args(Vec<(&'static str, Option<Arg>)>);

impl Args {
    fn text(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(n, _)| *n == name)
            .and_then(|(_, a)| match a {
                Some(Arg::Text(s)) => Some(s.as_str()),
                _ => None,
            })
    }
    fn lines(&self, name: &str) -> &[String] {
        self.0
            .iter()
            .find(|(n, _)| *n == name)
            .and_then(|(_, a)| match a {
                Some(Arg::Lines(l)) => Some(l.as_slice()),
                _ => None,
            })
            .unwrap_or(&[])
    }
    fn flag(&self, name: &str, default: bool) -> bool {
        self.0
            .iter()
            .find(|(n, _)| *n == name)
            .and_then(|(_, a)| match a {
                Some(Arg::Bool(b)) => Some(*b),
                _ => None,
            })
            .unwrap_or(default)
    }
}

/// Layout context of one template scene.
struct Ctx<'a> {
    brand: Option<&'a Brand>,
    portrait: bool,
    /// Frame height over frame width: turns a size in frame heights into frame widths.
    hw: f32,
    scene_ms: u64,
    out: Vec<Layer>,
}

/// A layer with the common fields at their defaults.
fn base(id: &str, x: f32, y: f32, width: f32, height: Option<f32>, content: LayerContent) -> Layer {
    Layer {
        id: format!("{TEMPLATE_LAYER_PREFIX}{id}"),
        x,
        y,
        width,
        height,
        anchor: Anchor::TopLeft,
        z: 0,
        start_ms: 0,
        end_ms: None,
        opacity: 1.0,
        rotation: 0.0,
        keyframes: Vec::new(),
        enter: None,
        exit: None,
        content,
    }
}

struct TextSpec<'a> {
    id: &'a str,
    text: &'a str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    size: f32,
    font: FontRole,
    weight: u16,
    align: TextAlign,
    color: Option<String>,
    line_height: f32,
}

impl Ctx<'_> {
    /// (start, duration) of an entrance planned at `at_ms` for `ms`, shrunk to fit the scene:
    /// starts within the first 40 % and lasts at most a third of what is left.
    fn timing(&self, at_ms: u64, ms: u64) -> (u64, u64) {
        let start = at_ms.min(self.scene_ms * 2 / 5);
        let dur = ms.min((self.scene_ms - start) / 3).max(1);
        (start, dur)
    }

    /// Gives the last layer an entrance (see [`Ctx::timing`]).
    fn anim(&mut self, kind: LayerAnimationKind, at_ms: u64, ms: u64) {
        let (start, dur) = self.timing(at_ms, ms);
        let layer = self.out.last_mut().expect("a layer to animate");
        layer.start_ms = start;
        layer.enter = Some(LayerAnimation {
            kind,
            duration_ms: dur,
            ease: None,
        });
    }

    /// Frame-height size `v` in frame widths.
    fn wide(&self, v: f32) -> f32 {
        v * self.hw
    }

    /// The muted text color for secondary lines.
    fn muted(&self) -> Option<String> {
        Some(Brand::muted_text(self.brand, DEFAULT_LOWER_THIRD_SUBTITLE).to_string())
    }

    fn primary(&self) -> String {
        Brand::primary(self.brand).to_string()
    }

    fn text(&mut self, t: TextSpec<'_>) {
        self.out.push(base(
            t.id,
            t.x,
            t.y,
            t.w,
            Some(t.h),
            LayerContent::Text(TextLayer {
                text: t.text.to_string(),
                font_size: t.size,
                color: t.color,
                font: Some(t.font),
                weight: t.weight,
                align: t.align,
                line_height: t.line_height,
                background: None,
                highlights: Vec::new(),
                reveal: TextReveal::None,
                typing_cps: None,
                cursor: false,
            }),
        ));
    }

    fn shape(&mut self, id: &str, shape: ShapeKind, rect: (f32, f32, f32, f32), fill: String) {
        let (x, y, w, h) = rect;
        self.out.push(base(
            id,
            x,
            y,
            w,
            Some(h),
            LayerContent::Shape(ShapeLayer {
                shape,
                fill: Some(fill),
                stroke: None,
                stroke_width: 0.0,
                corner_radius: if shape == ShapeKind::Rect {
                    h / 2.0
                } else {
                    0.0
                },
            }),
        ));
    }

    /// The brand logo, `h` frame heights tall, centered on `cy`, popping in at `at_ms`.
    fn logo(&mut self, cy: f32, h: f32, at_ms: u64) {
        let Some(logo) = self.brand.and_then(|b| b.logo.as_ref()) else {
            return;
        };
        let w = 0.6;
        self.out.push(base(
            "logo",
            0.5 - w / 2.0,
            cy - h / 2.0,
            w,
            Some(h),
            LayerContent::Image(ImageLayer {
                src: logo.src.clone(),
                fit: ImageFit::Contain,
                corner_radius: 0.0,
            }),
        ));
        self.anim(LayerAnimationKind::Pop, at_ms, 600);
    }

    /// Content column: (left, width).
    fn column(&self) -> (f32, f32) {
        if self.portrait {
            (0.08, 0.84)
        } else {
            (0.1, 0.8)
        }
    }

    /// A rule in the primary color at `y`, centered unless `x` is given.
    fn rule(&mut self, y: f32, w: f32, h: f32, x: Option<f32>) {
        let fill = self.primary();
        let x = x.unwrap_or(0.5 - w / 2.0);
        self.shape("rule", ShapeKind::Rect, (x, y, w, h), fill);
    }
}

/// Text color for a heading: unset (so it follows the brand `text` color).
const HEADING: Option<String> = None;

fn intro(c: &mut Ctx, a: &Args) {
    let p = c.portrait;
    let has_logo = a.flag("logo", true) && c.brand.is_some_and(|b| b.logo.is_some());
    let (x, w) = c.column();
    let (size, lines, sub_size) = if p {
        (0.056, 3.0, 0.03)
    } else {
        (0.1, 2.0, 0.042)
    };
    let mut y = match (has_logo, p) {
        (true, true) => 0.4,
        (true, false) => 0.42,
        (false, true) => 0.32,
        (false, false) => 0.26,
    };
    if has_logo {
        let (cy, h) = if p { (0.3, 0.1) } else { (0.22, 0.17) };
        c.logo(cy, h, 0);
    }
    let rule_h = if p { 0.005 } else { 0.008 };
    c.rule(y, 0.1, rule_h, None);
    c.anim(LayerAnimationKind::SlideRight, 150, 500);
    y += rule_h + 0.03;
    let title_h = size * 1.15 * lines;
    c.text(TextSpec {
        id: "title",
        text: a.text("title").unwrap_or_default(),
        x,
        y,
        w,
        h: title_h,
        size,
        font: FontRole::Heading,
        weight: 700,
        align: TextAlign::Center,
        color: HEADING,
        line_height: 1.15,
    });
    c.anim(LayerAnimationKind::SlideUp, 250, 700);
    y += title_h + 0.02;
    if let Some(sub) = a.text("subtitle") {
        let color = c.muted();
        c.text(TextSpec {
            id: "subtitle",
            text: sub,
            x,
            y,
            w,
            h: sub_size * 1.3 * if p { 3.0 } else { 2.0 },
            size: sub_size,
            font: FontRole::Body,
            weight: 500,
            align: TextAlign::Center,
            color,
            line_height: 1.3,
        });
        c.anim(LayerAnimationKind::Fade, 700, 700);
    }
}

fn outro(c: &mut Ctx, a: &Args) {
    let p = c.portrait;
    let has_logo = a.flag("logo", true) && c.brand.is_some_and(|b| b.logo.is_some());
    let (x, w) = c.column();
    let (size, cta_lines, line_size) = if p {
        (0.05, 3.0, 0.03)
    } else {
        (0.085, 2.0, 0.042)
    };
    let mut y = if p { 0.3 } else { 0.2 };
    if has_logo {
        let (cy, h) = if p { (0.24, 0.09) } else { (0.19, 0.15) };
        c.logo(cy, h, 0);
        y = if p { 0.33 } else { 0.32 };
    }
    let cta_h = size * 1.15 * cta_lines;
    c.text(TextSpec {
        id: "cta",
        text: a.text("cta").unwrap_or_default(),
        x,
        y,
        w,
        h: cta_h,
        size,
        font: FontRole::Heading,
        weight: 700,
        align: TextAlign::Center,
        color: HEADING,
        line_height: 1.15,
    });
    c.anim(LayerAnimationKind::SlideUp, 200, 600);
    y += cta_h + 0.03;
    let rule_h = if p { 0.004 } else { 0.006 };
    c.rule(y, 0.06, rule_h, None);
    c.anim(LayerAnimationKind::Fade, 450, 400);
    y += rule_h + 0.03;
    let slot = line_size * 1.3 * 1.25;
    for (i, line) in a.lines("lines").iter().enumerate() {
        let color = c.muted();
        c.text(TextSpec {
            id: &format!("line-{}", i + 1),
            text: line,
            x,
            y: y + slot * i as f32,
            w,
            h: slot,
            size: line_size,
            font: FontRole::Body,
            weight: 500,
            align: TextAlign::Center,
            color,
            line_height: 1.3,
        });
        c.anim(LayerAnimationKind::SlideUp, 700 + 180 * i as u64, 450);
    }
}

fn title_card(c: &mut Ctx, a: &Args) {
    let p = c.portrait;
    let (x, w) = c.column();
    let (num_size, size, lines) = if p {
        (0.12, 0.055, 3.0)
    } else {
        (0.22, 0.09, 2.0)
    };
    let mut y = if p { 0.3 } else { 0.2 };
    if let Some(n) = a.text("number") {
        let h = num_size * 1.1;
        let color = Some(c.primary());
        c.text(TextSpec {
            id: "number",
            text: n,
            x,
            y,
            w,
            h,
            size: num_size,
            font: FontRole::Heading,
            weight: 800,
            align: TextAlign::Left,
            color,
            line_height: 1.0,
        });
        c.anim(LayerAnimationKind::SlideRight, 0, 600);
        y += h + 0.02;
    } else {
        y += if p { 0.08 } else { 0.14 };
    }
    let (rule_w, rule_h) = if p { (0.2, 0.005) } else { (0.12, 0.008) };
    c.rule(y, rule_w, rule_h, Some(x));
    c.anim(LayerAnimationKind::SlideRight, 150, 500);
    y += rule_h + 0.03;
    c.text(TextSpec {
        id: "title",
        text: a.text("title").unwrap_or_default(),
        x,
        y,
        w,
        h: size * 1.15 * lines,
        size,
        font: FontRole::Heading,
        weight: 700,
        align: TextAlign::Left,
        color: HEADING,
        line_height: 1.15,
    });
    c.anim(LayerAnimationKind::SlideUp, 250, 600);
}

fn quote(c: &mut Ctx, a: &Args) {
    let p = c.portrait;
    let (mark_size, mark_y) = if p { (0.16, 0.17) } else { (0.3, 0.06) };
    let (x, w) = if p { (0.1, 0.8) } else { (0.14, 0.72) };
    let color = Some(c.primary());
    let mark_w = c.wide(mark_size * 1.3);
    c.text(TextSpec {
        id: "mark",
        text: "\u{201C}",
        x: 0.07,
        y: mark_y,
        w: mark_w,
        h: mark_size,
        size: mark_size,
        font: FontRole::Body,
        weight: 700,
        align: TextAlign::Left,
        color,
        line_height: 1.0,
    });
    c.anim(LayerAnimationKind::Pop, 0, 500);
    let (size, y, h) = if p {
        (0.036, 0.33, 0.4)
    } else {
        (0.058, 0.24, 0.47)
    };
    c.text(TextSpec {
        id: "quote",
        text: a.text("quote").unwrap_or_default(),
        x,
        y,
        w,
        h,
        size,
        font: FontRole::Heading,
        weight: 500,
        align: TextAlign::Left,
        color: HEADING,
        line_height: 1.3,
    });
    let (start, _) = c.timing(300, 0);
    let t = c.out.last_mut().expect("quote");
    if let LayerContent::Text(tl) = &mut t.content {
        tl.reveal = TextReveal::Words;
    }
    t.start_ms = start;
    if let Some(author) = a.text("author") {
        let color = c.muted();
        let size = if p { 0.026 } else { 0.038 };
        c.text(TextSpec {
            id: "author",
            text: &format!("\u{2014} {author}"),
            x,
            y: 0.76,
            w,
            h: size * 1.3 * 1.2,
            size,
            font: FontRole::Body,
            weight: 500,
            align: TextAlign::Left,
            color,
            line_height: 1.3,
        });
        c.anim(LayerAnimationKind::Fade, 1_200, 600);
    }
}

fn list(c: &mut Ctx, a: &Args) {
    let p = c.portrait;
    let (x, w) = c.column();
    let items = a.lines("items");
    let (size, title_lines, y0) = if p {
        (0.045, 3.0, 0.14)
    } else {
        (0.075, 2.0, 0.1)
    };
    let title_h = size * 1.2 * title_lines;
    c.text(TextSpec {
        id: "title",
        text: a.text("title").unwrap_or_default(),
        x,
        y: y0,
        w,
        h: title_h,
        size,
        font: FontRole::Heading,
        weight: 700,
        align: TextAlign::Left,
        color: HEADING,
        line_height: 1.2,
    });
    c.anim(LayerAnimationKind::SlideUp, 0, 500);
    let (item_size, item_lines) = if p { (0.028, 3.0) } else { (0.045, 2.0) };
    let slot = item_size * 1.25 * item_lines;
    let gap = 0.012;
    let top = y0 + title_h + 0.04;
    // Items come in one after another over the first 60 % of the scene.
    let n = items.len().max(1) as u64;
    let step = ((c.scene_ms * 3 / 5).saturating_sub(500) / n).clamp(1, 900);
    let dot = if p { 0.014 } else { 0.02 };
    let dot_w = c.wide(dot);
    let indent = dot_w + if p { 0.04 } else { 0.025 };
    let primary = c.primary();
    for (i, item) in items.iter().enumerate() {
        let y = top + (slot + gap) * i as f32;
        let at = 500 + step * i as u64;
        // The text is centered in its slot, and so is the bullet.
        c.shape(
            &format!("bullet-{}", i + 1),
            ShapeKind::Ellipse,
            (x, y + slot / 2.0 - dot / 2.0, dot_w, dot),
            primary.clone(),
        );
        c.anim(LayerAnimationKind::Pop, at, 400);
        c.text(TextSpec {
            id: &format!("item-{}", i + 1),
            text: item,
            x: x + indent,
            y,
            w: w - indent,
            h: slot,
            size: item_size,
            font: FontRole::Body,
            weight: 500,
            align: TextAlign::Left,
            color: HEADING,
            line_height: 1.25,
        });
        c.anim(LayerAnimationKind::SlideLeft, at, 450);
    }
}

/// The layers `template` makes for a scene of `scene_ms` with or without a background picture.
fn template_layers(
    info: &TemplateInfo,
    args: &Args,
    brand: Option<&Brand>,
    res: Resolution,
    scene_ms: u64,
    has_picture: bool,
) -> Vec<Layer> {
    let mut c = Ctx {
        brand,
        portrait: res.height > res.width,
        hw: res.height as f32 / res.width as f32,
        scene_ms,
        out: Vec::new(),
    };
    let bg = Brand::background(brand);
    let fill = if has_picture {
        // About 70 % opaque: the picture shows through, the text stays readable.
        match bg.len() {
            4 => {
                let b = bg.as_bytes();
                let d = |i: usize| (b[i] as char).to_string().repeat(2);
                format!("#{}{}{}b3", d(1), d(2), d(3))
            }
            7 => format!("{bg}b3"),
            _ => bg.to_string(),
        }
    } else {
        bg.to_string()
    };
    let mut back = base(
        "bg",
        0.0,
        0.0,
        1.0,
        Some(1.0),
        LayerContent::Shape(ShapeLayer {
            shape: ShapeKind::Rect,
            fill: Some(fill),
            stroke: None,
            stroke_width: 0.0,
            corner_radius: 0.0,
        }),
    );
    back.id = format!("{TEMPLATE_LAYER_PREFIX}bg");
    c.out.push(back);
    match info.name {
        "intro" => intro(&mut c, args),
        "outro" => outro(&mut c, args),
        "title_card" => title_card(&mut c, args),
        "quote" => quote(&mut c, args),
        "list" => list(&mut c, args),
        _ => unreachable!("every catalog entry has a layout"),
    }
    c.out
}

/// Expands one template scene in place: sets its default length when `duration_ms` is 0, puts
/// the template's layers under its own layers and clears `template`. A scene without a template
/// is left alone.
pub fn expand_scene(
    scene: &mut Scene,
    brand: Option<&Brand>,
    res: Resolution,
) -> Result<(), Vec<String>> {
    let Some(tpl) = &scene.template else {
        return Ok(());
    };
    let Some(info) = template_info(&tpl.name) else {
        let names: Vec<&str> = CATALOG.iter().map(|t| t.name).collect();
        return Err(vec![format!(
            "unknown template `{}`; the templates are {}",
            tpl.name,
            names.join(", ")
        )]);
    };
    let args = Args(check_params(info, &tpl.params)?);
    let mut errors = Vec::new();
    if scene.duration_ms == 0 {
        scene.duration_ms = info.default_ms;
    }
    if scene.duration_ms < MIN_TEMPLATE_MS {
        errors.push(format!(
            "a template scene must be at least {MIN_TEMPLATE_MS} ms, got {} ms",
            scene.duration_ms
        ));
    }
    for l in &scene.layers {
        if l.id.starts_with(TEMPLATE_LAYER_PREFIX) {
            errors.push(format!(
                "layer `{}`: ids starting with `{TEMPLATE_LAYER_PREFIX}` are reserved for the template's layers",
                l.id
            ));
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let has_picture = scene.image.is_some() || scene.video.is_some();
    let mut layers = template_layers(info, &args, brand, res, scene.duration_ms, has_picture);
    let z = scene.layers.iter().map(|l| l.z).min().unwrap_or(0).min(0) - 1;
    for l in &mut layers {
        l.z = z;
    }
    layers.append(&mut scene.layers);
    scene.layers = layers;
    scene.template = None;
    Ok(())
}

/// Expands every template scene of the timeline (see the module docs). On errors the timeline
/// may be partly expanded.
pub fn expand_templates(timeline: &mut VideoTimeline) -> Result<(), Vec<TemplateError>> {
    let mut errors = Vec::new();
    let brand = timeline.brand.clone();
    let res = timeline.resolution;
    for scene in &mut timeline.scenes {
        if let Err(msgs) = expand_scene(scene, brand.as_ref(), res) {
            errors.extend(msgs.into_iter().map(|message| TemplateError {
                scene: scene.id.clone(),
                message,
            }));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// The template param a template layer shows, as a pointer below `params` (`title`,
/// `items/2`), for messages and fixes.
pub fn param_of_layer(layer_id: &str) -> Option<String> {
    let rest = layer_id.strip_prefix(TEMPLATE_LAYER_PREFIX)?;
    let indexed = |prefix: &str, param: &str| {
        rest.strip_prefix(prefix)
            .and_then(|n| n.parse::<usize>().ok())
            .map(|n| format!("{param}/{}", n.saturating_sub(1)))
    };
    match rest {
        "title" | "subtitle" | "cta" | "number" | "quote" | "author" => Some(rest.to_string()),
        _ => indexed("item-", "items").or_else(|| indexed("line-", "lines")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation::validate;

    fn landscape() -> Resolution {
        Resolution {
            width: 1920,
            height: 1080,
        }
    }

    fn scene(json: &str) -> Scene {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn every_template_expands_and_validates_in_both_aspects() {
        let cases = [
            r#"{"name":"intro","params":{"title":"Hello","subtitle":"A short subtitle"}}"#,
            r#"{"name":"outro","params":{"cta":"Follow for more","lines":["@vimmer","vimmer.dev"]}}"#,
            r#"{"name":"title_card","params":{"number":"01","title":"Getting started"}}"#,
            r#"{"name":"title_card","params":{"number":2,"title":"No zero pad"}}"#,
            r#"{"name":"quote","params":{"quote":"Simplicity is prerequisite for reliability.","author":"Edsger Dijkstra"}}"#,
            r#"{"name":"list","params":{"title":"Three things","items":["One","Two","Three","Four","Five"]}}"#,
        ];
        let brand: Brand = serde_json::from_str(
            r##"{"colors":{"primary":"#ff5500"},"logo":{"src":"assets/logo.png"}}"##,
        )
        .unwrap();
        for res in [
            landscape(),
            Resolution {
                width: 1080,
                height: 1920,
            },
        ] {
            for case in cases {
                for b in [None, Some(&brand)] {
                    let mut s = scene(&format!(r#"{{"id":"s","template":{case}}}"#));
                    expand_scene(&mut s, b, res).unwrap();
                    assert!(s.template.is_none());
                    assert!(s.duration_ms >= 3000);
                    assert!(s.layers.iter().all(|l| l.id.starts_with("tpl-")));
                    let t = VideoTimeline {
                        fps: 30,
                        resolution: res,
                        scenes: vec![s],
                        subtitles: vec![],
                        music: None,
                        brand: b.cloned(),
                    };
                    validate(&t).unwrap_or_else(|e| panic!("{case}: {e:?}"));
                    // Everything stays inside the frame.
                    for l in &t.scenes[0].layers {
                        assert!(l.x >= 0.0 && l.x + l.width <= 1.0001, "{case} {}", l.id);
                        let h = l.height.unwrap_or(0.0);
                        assert!(
                            l.y >= 0.0 && l.y + h <= 1.0001,
                            "{case} {} {}",
                            l.id,
                            l.y + h
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn logo_only_with_a_brand_logo_and_the_flag() {
        let brand: Brand = serde_json::from_str(r#"{"logo":{"src":"assets/logo.png"}}"#).unwrap();
        let ids = |b: Option<&Brand>, extra: &str| {
            let mut s = scene(&format!(
                r#"{{"id":"s","template":{{"name":"intro","params":{{"title":"Hi"{extra}}}}}}}"#
            ));
            expand_scene(&mut s, b, landscape()).unwrap();
            s.layers.into_iter().map(|l| l.id).collect::<Vec<_>>()
        };
        assert!(ids(Some(&brand), "").contains(&"tpl-logo".to_string()));
        assert!(!ids(None, "").contains(&"tpl-logo".to_string()));
        assert!(!ids(Some(&brand), r#","logo":false"#).contains(&"tpl-logo".to_string()));
    }

    #[test]
    fn explicit_layers_stay_on_top_and_keep_their_order() {
        let mut s = scene(
            r##"{"id":"s","duration_ms":4000,"template":{"name":"title_card","params":{"title":"T"}},
            "layers":[{"id":"mine","kind":"shape","x":0,"y":0,"width":0.1,"height":0.1,"fill":"#fff","z":-3}]}"##,
        );
        expand_scene(&mut s, None, landscape()).unwrap();
        assert_eq!(s.duration_ms, 4000, "an explicit length is kept");
        let last = s.layers.last().unwrap();
        assert_eq!(last.id, "mine");
        assert!(s.layers[..s.layers.len() - 1].iter().all(|l| l.z == -4));
    }

    #[test]
    fn param_errors() {
        let err = |params: &str| {
            let mut s = scene(&format!(
                r#"{{"id":"s","template":{{"name":"list","params":{params}}}}}"#
            ));
            expand_scene(&mut s, None, landscape()).unwrap_err()
        };
        let e = err(r#"{"title":"T","items":["a"],"colour":"red"}"#);
        assert!(e[0].contains("unknown param `colour`"), "{e:?}");
        let e = err(r#"{"items":["a"]}"#);
        assert_eq!(e, ["`title` is required"]);
        let e = err(r#"{"title":"T","items":["a","b","c","d","e","f"]}"#);
        assert!(e[0].contains("6 items; the limit is 5"), "{e:?}");
        let long = "x".repeat(61);
        let e = err(&format!(r#"{{"title":"{long}","items":["a"]}}"#));
        assert!(e[0].contains("61 characters; the limit is 60"), "{e:?}");
        let e = err(r#"{"title":"T","items":"a"}"#);
        assert!(e[0].contains("must be a list"), "{e:?}");
        let e = err(r#"{"title":"T","items":[]}"#);
        assert_eq!(e, ["`items` is required"]);
        let mut s = scene(r#"{"id":"s","template":{"name":"slideshow"}}"#);
        let e = expand_scene(&mut s, None, landscape()).unwrap_err();
        assert!(e[0].starts_with("unknown template `slideshow`"));
        let mut s = scene(
            r#"{"id":"s","duration_ms":500,"template":{"name":"title_card","params":{"title":"T"}}}"#,
        );
        let e = expand_scene(&mut s, None, landscape()).unwrap_err();
        assert!(e[0].contains("at least 1000 ms"));
        let mut s = scene(
            r##"{"id":"s","template":{"name":"title_card","params":{"title":"T"}},"layers":[{"id":"tpl-x","kind":"shape","x":0,"y":0,"width":0.1,"height":0.1,"fill":"#fff"}]}"##,
        );
        assert!(expand_scene(&mut s, None, landscape()).unwrap_err()[0].contains("reserved"));
    }

    #[test]
    fn short_scenes_keep_animations_inside_the_layers() {
        for name in ["intro", "outro", "list", "quote"] {
            let params = match name {
                "intro" => r#"{"title":"T","subtitle":"S"}"#,
                "outro" => r#"{"cta":"C","lines":["a","b","c"]}"#,
                "list" => r#"{"title":"T","items":["a","b","c","d","e"]}"#,
                _ => r#"{"quote":"Q","author":"A"}"#,
            };
            let mut s = scene(&format!(
                r#"{{"id":"s","duration_ms":1000,"template":{{"name":"{name}","params":{params}}}}}"#
            ));
            expand_scene(&mut s, None, landscape()).unwrap();
            let t = VideoTimeline {
                fps: 30,
                resolution: landscape(),
                scenes: vec![s],
                subtitles: vec![],
                music: None,
                brand: None,
            };
            validate(&t).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        }
    }

    #[test]
    fn expansion_is_deterministic_and_idempotent() {
        let raw = r#"{"fps":30,"resolution":{"width":1920,"height":1080},"scenes":[{"id":"a","template":{"name":"quote","params":{"quote":"Q"}}}]}"#;
        let mut a: VideoTimeline = serde_json::from_str(raw).unwrap();
        let mut b: VideoTimeline = serde_json::from_str(raw).unwrap();
        expand_templates(&mut a).unwrap();
        expand_templates(&mut b).unwrap();
        assert_eq!(a, b);
        let once = a.clone();
        expand_templates(&mut a).unwrap();
        assert_eq!(a, once);
    }

    #[test]
    fn params_of_layers() {
        assert_eq!(param_of_layer("tpl-title").as_deref(), Some("title"));
        assert_eq!(param_of_layer("tpl-item-3").as_deref(), Some("items/2"));
        assert_eq!(param_of_layer("tpl-line-1").as_deref(), Some("lines/0"));
        assert_eq!(param_of_layer("tpl-bg"), None);
        assert_eq!(param_of_layer("title"), None);
    }

    /// The editor builds its params form from this file. Regenerate with
    /// `VIMMER_UPDATE_CATALOG=1 cargo test -p vimmer-dsl catalog`.
    #[test]
    fn catalog_file_is_up_to_date() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../remotion-renderer/editor/src/template-catalog.json"
        );
        let want = serde_json::to_string_pretty(CATALOG).unwrap() + "\n";
        if std::env::var_os("VIMMER_UPDATE_CATALOG").is_some() {
            std::fs::write(path, &want).unwrap();
        }
        let have = std::fs::read_to_string(path).unwrap_or_default();
        assert_eq!(
            have, want,
            "editor/src/template-catalog.json is stale; rerun with VIMMER_UPDATE_CATALOG=1"
        );
    }
}
