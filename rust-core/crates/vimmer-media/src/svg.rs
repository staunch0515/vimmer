//! Untrusted SVG -> sanitized SVG -> PNG.
//!
//! Illustrations are SVG documents written by a language model, so they are treated like any
//! other untrusted input:
//!
//! 1. [`sanitize_svg`] parses the text with a small, strict XML parser (no DTD, no entities beyond
//!    the five predefined ones and character references, size, depth and element limits) and
//!    rebuilds the document from an allow-list: unknown elements (`script`, `foreignObject`,
//!    `image`, `a`, animation elements, ...) are dropped with their content; event attributes
//!    (`on*`), external `href` / `xlink:href` (only `#id` stays), `xml:base`, foreign namespaces
//!    and any `url(...)` that does not point at `#id` are removed; CSS loses `@import` and
//!    external `url()`s, and CSS with escapes is dropped whole. The root must have a usable
//!    `viewBox` whose aspect ratio matches the canvas.
//! 2. [`rasterize_svg`] screenshots the sanitized file with headless Chrome (the one Remotion
//!    already downloads), with every network request sent to a dead proxy (scripts are already
//!    gone; `--blink-settings=scriptEnabled=false` is not used because it makes headless Chrome
//!    skip the screenshot), then checks that the PNG has the canvas size and is not one flat colour.
//!
//! The PNG then goes through the normal image pipeline ([`crate::normalize_image`]).

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{run_limited, Limits, MediaError};

/// Largest SVG accepted from the model, in bytes.
pub const MAX_SVG_BYTES: usize = 512 * 1024;
/// Most elements in one document.
pub const MAX_ELEMENTS: usize = 8000;
/// Deepest element nesting.
pub const MAX_DEPTH: usize = 48;
/// How far the viewBox aspect ratio may differ from the canvas (relative).
pub const ASPECT_TOLERANCE: f64 = 0.03;
/// Chrome to use for rasterizing; overrides the search in the Remotion directory.
pub const ENV_CHROME: &str = "VIMMER_CHROME";

/// A small picture used to check that Chrome can rasterize before any paid call is made.
pub const PREFLIGHT_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 160 90"><rect width="160" height="90" fill="#1f2a44"/><circle cx="80" cy="40" r="24" fill="#d9a441"/></svg>"##;

const SVG_NS: &str = "http://www.w3.org/2000/svg";

/// Elements kept by the sanitizer. Everything else is dropped together with its content.
const ALLOWED_ELEMENTS: &[&str] = &[
    "svg",
    "g",
    "defs",
    "desc",
    "title",
    "symbol",
    "use",
    "path",
    "rect",
    "circle",
    "ellipse",
    "line",
    "polyline",
    "polygon",
    "linearGradient",
    "radialGradient",
    "stop",
    "clipPath",
    "mask",
    "pattern",
    "marker",
    "style",
    "text",
    "tspan",
    "filter",
    "feBlend",
    "feColorMatrix",
    "feComponentTransfer",
    "feComposite",
    "feDisplacementMap",
    "feDropShadow",
    "feFlood",
    "feFuncA",
    "feFuncB",
    "feFuncG",
    "feFuncR",
    "feGaussianBlur",
    "feMerge",
    "feMergeNode",
    "feMorphology",
    "feOffset",
    "feTurbulence",
];

/// Why an SVG was rejected. The text is what the model gets back for a correction.
#[derive(Debug, Clone, PartialEq)]
pub enum SvgError {
    TooLarge {
        bytes: usize,
        max: usize,
    },
    /// Not well-formed XML (or uses a DTD / unknown entity).
    Parse(String),
    /// The root element is not `<svg>`.
    NotSvg(String),
    NoViewBox,
    BadViewBox(String),
    AspectMismatch {
        view_box: String,
        expected: String,
    },
    TooComplex(String),
    /// Nothing drawable is left after sanitizing.
    Empty,
    /// Chrome could not be run or produced no usable PNG.
    Raster(String),
    Cancelled,
    TimedOut {
        secs: u64,
    },
}

impl fmt::Display for SvgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SvgError::TooLarge { bytes, max } => {
                write!(f, "the SVG is {bytes} bytes; the limit is {max} bytes")
            }
            SvgError::Parse(m) => write!(f, "the SVG is not well-formed XML: {m}"),
            SvgError::NotSvg(name) => {
                write!(f, "the root element is <{name}>; it must be <svg>")
            }
            SvgError::NoViewBox => f.write_str("the root <svg> has no viewBox attribute"),
            SvgError::BadViewBox(v) => write!(
                f,
                "the viewBox `{v}` is not four numbers with a positive width and height"
            ),
            SvgError::AspectMismatch { view_box, expected } => write!(
                f,
                "the viewBox `{view_box}` does not have the canvas aspect ratio; use `{expected}`"
            ),
            SvgError::TooComplex(m) => write!(f, "the SVG is too complex: {m}"),
            SvgError::Empty => f.write_str("nothing drawable is left in the SVG"),
            SvgError::Raster(m) => write!(f, "rasterizing the SVG failed: {m}"),
            SvgError::Cancelled => f.write_str("cancelled"),
            SvgError::TimedOut { secs } => write!(f, "rasterizing timed out after {secs} s"),
        }
    }
}

impl std::error::Error for SvgError {}

/// A sanitized document.
#[derive(Debug, Clone, PartialEq)]
pub struct SanitizedSvg {
    /// The rebuilt document, with an XML declaration, ready to write to disk.
    pub svg: String,
    /// What was removed, e.g. `element <script>`, `attribute onload`. Sorted, unique.
    pub removed: Vec<String>,
}

// ---------------------------------------------------------------------------------------------
// XML parsing

#[derive(Debug, Clone)]
enum Node {
    Element(Element),
    Text(String),
}

#[derive(Debug, Clone)]
struct Element {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<Node>,
}

struct Parser<'a> {
    s: &'a str,
    pos: usize,
    elements: usize,
}

type PResult<T> = Result<T, SvgError>;

fn perr(msg: impl Into<String>) -> SvgError {
    SvgError::Parse(msg.into())
}

impl<'a> Parser<'a> {
    fn rest(&self) -> &'a str {
        &self.s[self.pos..]
    }

    fn eat(&mut self, prefix: &str) -> bool {
        if self.rest().starts_with(prefix) {
            self.pos += prefix.len();
            true
        } else {
            false
        }
    }

    fn skip_ws(&mut self) {
        let trimmed = self.rest().trim_start_matches([' ', '\t', '\r', '\n']);
        self.pos = self.s.len() - trimmed.len();
    }

    /// Skips up to and including `end`.
    fn skip_past(&mut self, end: &str, what: &str) -> PResult<()> {
        match self.rest().find(end) {
            Some(i) => {
                self.pos += i + end.len();
                Ok(())
            }
            None => Err(perr(format!("unterminated {what}"))),
        }
    }

    /// Comments, processing instructions and whitespace between markup. A DTD is refused.
    fn skip_misc(&mut self) -> PResult<()> {
        loop {
            self.skip_ws();
            if self.eat("<!--") {
                self.skip_past("-->", "comment")?;
            } else if self.eat("<?") {
                self.skip_past("?>", "processing instruction")?;
            } else if self.rest().starts_with("<!") {
                return Err(perr("DOCTYPE and other declarations are not allowed"));
            } else {
                return Ok(());
            }
        }
    }

    fn name(&mut self) -> PResult<String> {
        let rest = self.rest();
        let len = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.')))
            .unwrap_or(rest.len());
        let name = &rest[..len];
        let first = name.chars().next();
        if !first.is_some_and(|c| c.is_ascii_alphabetic() || c == '_') {
            return Err(perr(format!(
                "invalid name at byte {}: `{}`",
                self.pos,
                rest.chars().take(12).collect::<String>()
            )));
        }
        self.pos += len;
        Ok(name.to_string())
    }

    fn document(&mut self) -> PResult<Element> {
        self.eat("\u{feff}");
        self.skip_misc()?;
        if !self.rest().starts_with('<') {
            return Err(perr("the document does not start with an element"));
        }
        let root = self.element(1)?;
        self.skip_misc()?;
        if !self.rest().trim().is_empty() {
            return Err(perr("content after the root element"));
        }
        Ok(root)
    }

    /// Parses `<name ...>...</name>` or `<name .../>`; `self.pos` is at `<`.
    fn element(&mut self, depth: usize) -> PResult<Element> {
        if depth > MAX_DEPTH {
            return Err(SvgError::TooComplex(format!(
                "elements are nested more than {MAX_DEPTH} deep"
            )));
        }
        self.elements += 1;
        if self.elements > MAX_ELEMENTS {
            return Err(SvgError::TooComplex(format!(
                "more than {MAX_ELEMENTS} elements"
            )));
        }
        self.pos += 1; // '<'
        let name = self.name()?;
        let mut attrs: Vec<(String, String)> = Vec::new();
        loop {
            let before = self.pos;
            self.skip_ws();
            if self.eat("/>") {
                return Ok(Element {
                    name,
                    attrs,
                    children: Vec::new(),
                });
            }
            if self.eat(">") {
                break;
            }
            if self.pos == before {
                return Err(perr(format!("expected whitespace in <{name}> tag")));
            }
            let attr = self.name()?;
            self.skip_ws();
            if !self.eat("=") {
                return Err(perr(format!("attribute `{attr}` of <{name}> has no value")));
            }
            self.skip_ws();
            let quote = match self.rest().chars().next() {
                Some(q @ ('"' | '\'')) => q,
                _ => return Err(perr(format!("attribute `{attr}` value is not quoted"))),
            };
            self.pos += 1;
            let end = self
                .rest()
                .find(quote)
                .ok_or_else(|| perr(format!("unterminated value of `{attr}`")))?;
            let raw = &self.rest()[..end];
            if raw.contains('<') {
                return Err(perr(format!("`<` in the value of `{attr}`")));
            }
            let value = decode_entities(raw)?;
            self.pos += end + 1;
            if attrs.iter().any(|(n, _)| *n == attr) {
                return Err(perr(format!("duplicate attribute `{attr}` on <{name}>")));
            }
            attrs.push((attr, value));
        }
        let mut children = Vec::new();
        loop {
            if self.rest().is_empty() {
                return Err(perr(format!("<{name}> is not closed")));
            }
            if self.eat("</") {
                let end = self.name()?;
                self.skip_ws();
                if !self.eat(">") || end != name {
                    return Err(perr(format!("<{name}> is closed by </{end}>")));
                }
                return Ok(Element {
                    name,
                    attrs,
                    children,
                });
            }
            if self.eat("<!--") {
                self.skip_past("-->", "comment")?;
            } else if self.eat("<![CDATA[") {
                let end = self
                    .rest()
                    .find("]]>")
                    .ok_or_else(|| perr("unterminated CDATA section"))?;
                children.push(Node::Text(self.rest()[..end].to_string()));
                self.pos += end + 3;
            } else if self.eat("<?") {
                self.skip_past("?>", "processing instruction")?;
            } else if self.rest().starts_with("<!") {
                return Err(perr("declarations are not allowed inside elements"));
            } else if self.rest().starts_with('<') {
                children.push(Node::Element(self.element(depth + 1)?));
            } else {
                let end = self.rest().find('<').unwrap_or(self.rest().len());
                let text = decode_entities(&self.rest()[..end])?;
                children.push(Node::Text(text));
                self.pos += end;
            }
        }
    }
}

/// Decodes the five predefined entities and character references. Any other entity is an error
/// (there is no DTD to define one).
fn decode_entities(raw: &str) -> PResult<String> {
    if !raw.contains('&') {
        return Ok(raw.to_string());
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i + 1..];
        let end = rest
            .find(';')
            .filter(|&e| e <= 12)
            .ok_or_else(|| perr("`&` that does not start an entity"))?;
        let entity = &rest[..end];
        let ch = match entity {
            "lt" => '<',
            "gt" => '>',
            "amp" => '&',
            "quot" => '"',
            "apos" => '\'',
            _ => {
                let code = if let Some(hex) = entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                {
                    u32::from_str_radix(hex, 16).ok()
                } else if let Some(dec) = entity.strip_prefix('#') {
                    dec.parse::<u32>().ok()
                } else {
                    return Err(perr(format!("unknown entity `&{entity};`")));
                };
                code.and_then(char::from_u32)
                    .filter(|c| *c != '\0')
                    .ok_or_else(|| perr(format!("bad character reference `&{entity};`")))?
            }
        };
        out.push(ch);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Sanitizing

struct Scrubber {
    removed: BTreeSet<String>,
}

/// A `url(...)` reference that stays inside the document.
fn is_local_ref(target: &str) -> bool {
    let t = target.trim().trim_matches(['"', '\'']).trim();
    t.starts_with('#')
        && t.len() > 1
        && t[1..]
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
}

/// Replaces every `url(...)` that does not point at `#id` by `none`. Case-insensitive.
fn scrub_urls(value: &str) -> (String, bool) {
    let lower = value.to_ascii_lowercase();
    let mut out = String::with_capacity(value.len());
    let mut changed = false;
    let mut i = 0;
    while let Some(found) = lower[i..].find("url(") {
        let start = i + found;
        out.push_str(&value[i..start]);
        let inner_start = start + 4;
        match value[inner_start..].find(')') {
            Some(close) => {
                let inner = &value[inner_start..inner_start + close];
                if is_local_ref(inner) {
                    out.push_str(&value[start..inner_start + close + 1]);
                } else {
                    out.push_str("none");
                    changed = true;
                }
                i = inner_start + close + 1;
            }
            None => {
                // Unterminated url( swallows the rest.
                out.push_str("none");
                changed = true;
                i = value.len();
            }
        }
    }
    out.push_str(&value[i..]);
    (out, changed)
}

/// Strips `/* ... */` comments. They are removed without a trace (not replaced by a space), so
/// `@im/**/port` turns into `@import` and is then caught: stricter than a browser, never looser.
fn strip_css_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(i) = rest.find("/*") {
        out.push_str(&rest[..i]);
        match rest[i + 2..].find("*/") {
            Some(e) => rest = &rest[i + 2 + e + 2..],
            None => rest = "",
        }
    }
    out.push_str(rest);
    out
}

/// Sanitized CSS, or `None` when the CSS must be dropped whole (escapes or script-like
/// constructs that a simple scrubber cannot judge safely).
fn scrub_css(css: &str) -> (Option<String>, Vec<&'static str>) {
    let mut notes = Vec::new();
    if css.contains('\\') {
        notes.push("CSS with escapes");
        return (None, notes);
    }
    let mut css = strip_css_comments(css);
    // Remove @import / @namespace rules up to their `;` (or the end).
    for rule in ["@import", "@namespace"] {
        loop {
            let lower = css.to_ascii_lowercase();
            let Some(start) = lower.find(rule) else { break };
            let end = lower[start..]
                .find(';')
                .map_or(css.len(), |e| start + e + 1);
            css.replace_range(start..end, "");
            notes.push(if rule == "@import" {
                "CSS @import"
            } else {
                "CSS @namespace"
            });
        }
    }
    let (scrubbed, changed) = scrub_urls(&css);
    if changed {
        notes.push("external url()");
    }
    let lower = scrubbed.to_ascii_lowercase();
    for bad in [
        "expression(",
        "javascript:",
        "-moz-binding",
        "image-set(",
        "behavior:",
    ] {
        if lower.contains(bad) {
            notes.push("script-like CSS");
            return (None, notes);
        }
    }
    (Some(scrubbed), notes)
}

fn compact(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_control())
        .collect::<String>()
        .to_ascii_lowercase()
}

impl Scrubber {
    fn note(&mut self, what: impl Into<String>) {
        self.removed.insert(what.into());
    }

    /// Returns the kept attribute (possibly renamed / rewritten), or `None` to drop it.
    fn attribute(&mut self, name: &str, value: &str) -> Option<(String, String)> {
        let lower = name.to_ascii_lowercase();
        if lower.starts_with("on") {
            self.note(format!("attribute {name}"));
            return None;
        }
        let (prefix, local) = match name.split_once(':') {
            Some((p, l)) => (Some(p), l),
            None => (None, name),
        };
        if local.eq_ignore_ascii_case("href") {
            if prefix.is_some_and(|p| p != "xlink") {
                self.note(format!("attribute {name}"));
                return None;
            }
            if is_local_ref(value) && !value.contains(['"', '\'']) {
                return Some(("href".into(), value.trim().to_string()));
            }
            self.note(format!("attribute {name} (external link)"));
            return None;
        }
        if lower == "xmlns" || lower.starts_with("xmlns:") {
            // The root's namespace is set by the serializer; other declarations are not needed.
            return None;
        }
        if let Some(p) = prefix {
            if !(p == "xml" && local == "space") {
                self.note(format!("attribute {name}"));
                return None;
            }
        }
        let flat = compact(value);
        for scheme in ["javascript:", "vbscript:", "data:"] {
            if flat.contains(scheme) {
                self.note(format!("attribute {name} ({scheme} value)"));
                return None;
            }
        }
        if lower == "style" {
            let (css, notes) = scrub_css(value);
            for n in notes {
                self.note(n);
            }
            return css.map(|c| (name.to_string(), c));
        }
        let (scrubbed, changed) = scrub_urls(value);
        if changed {
            self.note("external url()");
        }
        Some((name.to_string(), scrubbed))
    }

    /// The element with disallowed parts removed, or `None` when the whole element goes.
    fn element(&mut self, el: Element) -> Option<Element> {
        if !ALLOWED_ELEMENTS.contains(&el.name.as_str()) {
            self.note(format!("element <{}>", el.name));
            return None;
        }
        let attrs = el
            .attrs
            .iter()
            .filter_map(|(n, v)| self.attribute(n, v))
            .collect();
        if el.name == "style" {
            let css: String = el
                .children
                .iter()
                .map(|c| match c {
                    Node::Text(t) => t.as_str(),
                    Node::Element(_) => "",
                })
                .collect();
            if el.children.iter().any(|c| matches!(c, Node::Element(_))) {
                self.note("elements inside <style>");
            }
            let (css, notes) = scrub_css(&css);
            for n in notes {
                self.note(n);
            }
            let css = css?;
            return Some(Element {
                name: el.name,
                attrs,
                children: vec![Node::Text(css)],
            });
        }
        let children = el
            .children
            .into_iter()
            .filter_map(|c| match c {
                Node::Text(t) => Some(Node::Text(t)),
                Node::Element(e) => self.element(e).map(Node::Element),
            })
            .collect();
        Some(Element {
            name: el.name,
            attrs,
            children,
        })
    }
}

fn parse_view_box(v: &str) -> Option<[f64; 4]> {
    let nums: Vec<f64> = v
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<f64>().ok())
        .collect::<Option<_>>()?;
    let [x, y, w, h]: [f64; 4] = nums.try_into().ok()?;
    if [x, y, w, h].iter().all(|n| n.is_finite()) && w > 0.0 && h > 0.0 {
        Some([x, y, w, h])
    } else {
        None
    }
}

fn escape(s: &str, attr: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attr => out.push_str("&quot;"),
            '\n' if attr => out.push_str("&#10;"),
            c if c.is_control() && !matches!(c, '\n' | '\t' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

fn serialize(el: &Element, out: &mut String) {
    out.push('<');
    out.push_str(&el.name);
    for (n, v) in &el.attrs {
        out.push(' ');
        out.push_str(n);
        out.push_str("=\"");
        out.push_str(&escape(v, true));
        out.push('"');
    }
    if el.children.is_empty() {
        out.push_str("/>");
        return;
    }
    out.push('>');
    for child in &el.children {
        match child {
            Node::Text(t) => out.push_str(&escape(t, false)),
            Node::Element(e) => serialize(e, out),
        }
    }
    out.push_str("</");
    out.push_str(&el.name);
    out.push('>');
}

fn has_drawable(el: &Element) -> bool {
    el.children.iter().any(|c| match c {
        Node::Element(e) => {
            !matches!(e.name.as_str(), "title" | "desc" | "defs" | "style") || has_drawable(e)
        }
        Node::Text(_) => false,
    })
}

/// Parses and sanitizes an untrusted SVG for a `width` x `height` canvas. See the module docs
/// for the rules. The result has `width`/`height` set to the canvas and
/// `preserveAspectRatio="xMidYMid slice"`.
pub fn sanitize_svg(input: &str, width: u32, height: u32) -> Result<SanitizedSvg, SvgError> {
    if input.len() > MAX_SVG_BYTES {
        return Err(SvgError::TooLarge {
            bytes: input.len(),
            max: MAX_SVG_BYTES,
        });
    }
    let mut parser = Parser {
        s: input,
        pos: 0,
        elements: 0,
    };
    let root = parser.document()?;
    if root.name != "svg" {
        return Err(SvgError::NotSvg(root.name));
    }
    let view_box = root
        .attrs
        .iter()
        .find(|(n, _)| n == "viewBox")
        .map(|(_, v)| v.clone())
        .ok_or(SvgError::NoViewBox)?;
    let [_, _, vw, vh] =
        parse_view_box(&view_box).ok_or_else(|| SvgError::BadViewBox(view_box.clone()))?;
    let want = f64::from(width) / f64::from(height);
    if ((vw / vh) - want).abs() / want > ASPECT_TOLERANCE {
        return Err(SvgError::AspectMismatch {
            view_box,
            expected: format!("0 0 {width} {height}"),
        });
    }

    let mut scrubber = Scrubber {
        removed: BTreeSet::new(),
    };
    let mut root = scrubber.element(root).ok_or(SvgError::Empty)?;
    if !has_drawable(&root) {
        return Err(SvgError::Empty);
    }
    root.attrs.retain(|(n, _)| {
        !matches!(
            n.as_str(),
            "width" | "height" | "preserveAspectRatio" | "x" | "y"
        )
    });
    root.attrs.insert(0, ("xmlns".into(), SVG_NS.into()));
    root.attrs.push(("width".into(), width.to_string()));
    root.attrs.push(("height".into(), height.to_string()));
    root.attrs
        .push(("preserveAspectRatio".into(), "xMidYMid slice".into()));

    let mut svg = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    serialize(&root, &mut svg);
    svg.push('\n');
    Ok(SanitizedSvg {
        svg,
        removed: scrubber.removed.into_iter().collect(),
    })
}

// ---------------------------------------------------------------------------------------------
// Rasterizing

/// The headless Chrome to rasterize with: `$VIMMER_CHROME`, else the `chrome-headless-shell`
/// that Remotion downloads into `<remotion_dir>/node_modules/.remotion/`.
pub fn find_chrome(remotion_dir: &Path) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os(ENV_CHROME).filter(|p| !p.is_empty()) {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    let base = remotion_dir
        .join("node_modules")
        .join(".remotion")
        .join("chrome-headless-shell");
    for platform in std::fs::read_dir(&base).ok()?.flatten() {
        let Ok(inner) = std::fs::read_dir(platform.path()) else {
            continue;
        };
        for dir in inner.flatten() {
            for exe in ["chrome-headless-shell", "chrome-headless-shell.exe"] {
                let candidate = dir.path().join(exe);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// Screenshots the (already sanitized) SVG at `svg` into a `width` x `height` PNG at `png`.
/// `profile_dir` is a scratch Chrome profile, removed afterwards.
pub fn rasterize_svg(
    chrome: &Path,
    svg: &Path,
    png: &Path,
    width: u32,
    height: u32,
    profile_dir: &Path,
    limits: &Limits,
) -> Result<(), SvgError> {
    let svg = std::path::absolute(svg).map_err(|e| SvgError::Raster(e.to_string()))?;
    let png = std::path::absolute(png).map_err(|e| SvgError::Raster(e.to_string()))?;
    let _ = std::fs::remove_file(&png);
    let mut cmd = Command::new(chrome);
    cmd.arg("--headless")
        .arg("--disable-gpu")
        .arg("--hide-scrollbars")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-extensions")
        .arg("--disable-background-networking")
        .arg("--disable-sync")
        .arg("--mute-audio")
        // Any network request (there should be none left) goes to a closed port.
        .arg("--proxy-server=127.0.0.1:9")
        .arg("--force-device-scale-factor=1")
        .arg(format!("--user-data-dir={}", profile_dir.display()))
        .arg(format!("--window-size={width},{height}"))
        .arg(format!("--screenshot={}", png.display()))
        .arg(format!("file://{}", svg.display()));
    let result = run_limited(cmd, limits);
    let _ = std::fs::remove_dir_all(profile_dir);
    let output = match result {
        Ok(o) => o,
        Err(MediaError::Cancelled) => return Err(SvgError::Cancelled),
        Err(MediaError::TimedOut { secs }) => return Err(SvgError::TimedOut { secs }),
        Err(e) => {
            return Err(SvgError::Raster(format!(
                "cannot run {}: {e}",
                chrome.display()
            )))
        }
    };
    if !png.is_file() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let last = stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("");
        return Err(SvgError::Raster(format!(
            "Chrome exited with {} and wrote no PNG: {last}",
            output.status
        )));
    }
    check_png(&png, width, height)
}

/// The PNG must have the canvas size and must not be a single flat colour (which is what a
/// broken SVG renders as).
pub fn check_png(png: &Path, width: u32, height: u32) -> Result<(), SvgError> {
    let img = image::open(png)
        .map_err(|e| SvgError::Raster(format!("unreadable PNG: {e}")))?
        .to_rgb8();
    if (img.width(), img.height()) != (width, height) {
        return Err(SvgError::Raster(format!(
            "the PNG is {}x{}, expected {width}x{height}",
            img.width(),
            img.height()
        )));
    }
    let first = img.get_pixel(0, 0).0;
    let varied = img.pixels().step_by(97).any(|p| {
        p.0.iter()
            .zip(first.iter())
            .any(|(a, b)| a.abs_diff(*b) > 6)
    });
    if !varied {
        return Err(SvgError::Raster(
            "the picture rendered as one flat colour".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "svg_tests.rs"]
mod tests;

/// Rasterizes [`PREFLIGHT_SVG`] at 160x90 in `scratch` (created and removed here), to find out
/// before any API call whether `chrome` works.
pub fn preflight_chrome(chrome: &Path, scratch: &Path, limits: &Limits) -> Result<(), SvgError> {
    std::fs::create_dir_all(scratch).map_err(|e| SvgError::Raster(e.to_string()))?;
    let svg = scratch.join("preflight.svg");
    let png = scratch.join("preflight.png");
    let clean = sanitize_svg(PREFLIGHT_SVG, 160, 90)?;
    std::fs::write(&svg, clean.svg).map_err(|e| SvgError::Raster(e.to_string()))?;
    let result = rasterize_svg(
        chrome,
        &svg,
        &png,
        160,
        90,
        &scratch.join("profile"),
        limits,
    );
    let _ = std::fs::remove_file(&svg);
    let _ = std::fs::remove_file(&png);
    result
}
