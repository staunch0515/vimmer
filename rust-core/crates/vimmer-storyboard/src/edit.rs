//! Timeline edits as JSON Patch (RFC 6902), checked before they are accepted.
//!
//! [`apply_edit`] takes the timeline JSON and a patch, applies the patch atomically to a copy,
//! and accepts the result only when
//!
//! 1. every op is well formed and its paths stay inside the timeline model (no made-up keys,
//!    no replacing the whole document, values carry only known fields),
//! 2. the patch is small (op count, value size),
//! 3. the patched JSON deserializes into [`VideoTimeline`] and passes [`vimmer_dsl::validate`],
//! 4. new asset paths are relative paths inside the renderer's public dir (no absolute paths,
//!    no `..`, no URL schemes, no symlink escapes), and
//! 5. numbers are sane (scene length, transition length, keyframe ranges, counts).
//!
//! The accepted result keeps the input's JSON shape (only the patched parts change), so a
//! caller can write it back without reformatting the document. A rejection lists every problem
//! with the op it came from, in words that can be sent back to Claude for a correction.
//!
//! Supported ops: `add`, `remove`, `replace`, `move`, `copy`, `test`, with JSON Pointer
//! escaping (`~0` = `~`, `~1` = `/`) and the array append index `-`.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::path::{Component, Path};

use serde::Serialize;
use serde_json::{Map, Value};
use vimmer_dsl::{validate, Scene, VideoTimeline};

use crate::estimate::estimate_duration_ms;
use crate::model::Language;

/// Most ops one patch may have.
pub const MAX_OPS: usize = 64;
/// Largest serialized `value` of one op, in bytes.
pub const MAX_VALUE_BYTES: usize = 16 * 1024;
/// Largest serialized patch, in bytes.
pub const MAX_PATCH_BYTES: usize = 64 * 1024;
/// Most scenes an edited timeline may have.
pub const MAX_SCENES: usize = 60;
/// Shortest scene an edit may produce.
pub const MIN_SCENE_MS: u64 = 300;
/// Longest scene an edit may produce.
pub const MAX_SCENE_MS: u64 = 60_000;
/// Longest transition an edit may produce.
pub const MAX_TRANSITION_MS: u64 = 5_000;
/// Longest on-screen text of one scene, in characters.
pub const MAX_TEXT_CHARS: usize = 400;
pub const MAX_HIGHLIGHTS: usize = 12;
pub const MAX_KEYFRAMES: usize = 24;
/// Keyframe scale range (exclusive lower bound 0).
pub const MAX_KEYFRAME_SCALE: f32 = 10.0;
/// Keyframe x / y offset range, in percent of the frame.
pub const MAX_KEYFRAME_OFFSET: f32 = 100.0;
pub const MAX_KEYFRAME_ROTATION: f32 = 360.0;
pub const MAX_FADE_MS: u64 = 30_000;
pub const MAX_SUBTITLES: usize = 500;
pub const MAX_ASSET_PATH_CHARS: usize = 200;

/// One JSON Patch operation.
#[derive(Debug, Clone, PartialEq)]
pub enum PatchOp {
    Add { path: String, value: Value },
    Remove { path: String },
    Replace { path: String, value: Value },
    Move { from: String, path: String },
    Copy { from: String, path: String },
    Test { path: String, value: Value },
}

impl PatchOp {
    pub fn name(&self) -> &'static str {
        match self {
            PatchOp::Add { .. } => "add",
            PatchOp::Remove { .. } => "remove",
            PatchOp::Replace { .. } => "replace",
            PatchOp::Move { .. } => "move",
            PatchOp::Copy { .. } => "copy",
            PatchOp::Test { .. } => "test",
        }
    }

    pub fn path(&self) -> &str {
        match self {
            PatchOp::Add { path, .. }
            | PatchOp::Remove { path }
            | PatchOp::Replace { path, .. }
            | PatchOp::Move { path, .. }
            | PatchOp::Copy { path, .. }
            | PatchOp::Test { path, .. } => path,
        }
    }

    fn label(&self) -> String {
        match self {
            PatchOp::Move { from, path } | PatchOp::Copy { from, path } => {
                format!("{} {} -> {}", self.name(), from, path)
            }
            _ => format!("{} {}", self.name(), self.path()),
        }
    }

    /// The op as RFC 6902 JSON.
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("op".into(), Value::String(self.name().into()));
        match self {
            PatchOp::Move { from, .. } | PatchOp::Copy { from, .. } => {
                m.insert("from".into(), Value::String(from.clone()));
            }
            _ => {}
        }
        m.insert("path".into(), Value::String(self.path().into()));
        match self {
            PatchOp::Add { value, .. }
            | PatchOp::Replace { value, .. }
            | PatchOp::Test { value, .. } => {
                m.insert("value".into(), value.clone());
            }
            _ => {}
        }
        Value::Object(m)
    }
}

/// The patch as an RFC 6902 JSON array.
pub fn patch_to_json(ops: &[PatchOp]) -> Value {
    Value::Array(ops.iter().map(PatchOp::to_json).collect())
}

/// One reason a patch was rejected.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Problem {
    /// 0-based index of the op that caused it, when one op did.
    pub op: Option<usize>,
    /// `replace /scenes/1/duration_ms`, when one op did.
    pub op_label: Option<String>,
    pub message: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.op, &self.op_label) {
            (Some(i), Some(label)) => write!(f, "op {i} ({label}): {}", self.message),
            (Some(i), None) => write!(f, "op {i}: {}", self.message),
            _ => f.write_str(&self.message),
        }
    }
}

/// Where a patch failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// The patch JSON is not a list of well-formed ops, or it is too big.
    Parse,
    /// An op touches a path outside the timeline model or carries unknown fields.
    Guard,
    /// An op could not be applied (missing path, failed `test`, bad index).
    Apply,
    /// The patched JSON is not a timeline (wrong type, missing field).
    Model,
    /// The timeline breaks a DSL rule, an asset rule or a sanity limit.
    Validation,
}

/// The patch was not accepted; nothing was changed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Rejection {
    pub stage: Stage,
    pub problems: Vec<Problem>,
}

impl Rejection {
    fn one(stage: Stage, op: Option<(usize, &PatchOp)>, message: impl Into<String>) -> Self {
        Self {
            stage,
            problems: vec![problem(op, message)],
        }
    }
}

fn problem(op: Option<(usize, &PatchOp)>, message: impl Into<String>) -> Problem {
    Problem {
        op: op.map(|(i, _)| i),
        op_label: op.map(|(_, o)| o.label()),
        message: message.into(),
    }
}

impl fmt::Display for Rejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let lines: Vec<String> = self.problems.iter().map(|p| format!("- {p}")).collect();
        write!(f, "the patch was rejected:\n{}", lines.join("\n"))
    }
}

impl std::error::Error for Rejection {}

/// What to check besides the DSL rules.
#[derive(Debug, Clone, Default)]
pub struct EditCheck {
    /// The renderer's public dir. When set, new asset paths must not resolve outside it, and a
    /// missing file is a warning.
    pub public_dir: Option<std::path::PathBuf>,
}

/// An accepted edit.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EditOutcome {
    /// The patched document, in the input's shape.
    pub timeline: Value,
    /// What changed, one line per change, scene by scene.
    pub summary: Vec<String>,
    /// Accepted, but worth a look (missing asset file, highlight not in the text, ...).
    pub warnings: Vec<String>,
    /// False when the patch applied but changed nothing.
    pub changed: bool,
    /// The patch that was applied: the given ops, then any the checker added (a
    /// `replace /subtitles` when cues were moved to follow changed scene timing).
    pub patch: Value,
}

// ---------------------------------------------------------------------------------------------
// Parsing

/// Reads a patch: a JSON array of ops. Each op takes `value` as JSON, or `value_json` as a
/// string holding JSON (the form Claude writes). Unknown op names and keys are rejected.
pub fn parse_patch(patch: &Value) -> Result<Vec<PatchOp>, Rejection> {
    let size = serde_json::to_string(patch).map(|s| s.len()).unwrap_or(0);
    if size > MAX_PATCH_BYTES {
        return Err(Rejection::one(
            Stage::Parse,
            None,
            format!("the patch is {size} bytes; the limit is {MAX_PATCH_BYTES}"),
        ));
    }
    let Value::Array(items) = patch else {
        return Err(Rejection::one(
            Stage::Parse,
            None,
            "the patch must be a JSON array of operations",
        ));
    };
    if items.len() > MAX_OPS {
        return Err(Rejection::one(
            Stage::Parse,
            None,
            format!(
                "the patch has {} operations; the limit is {MAX_OPS}",
                items.len()
            ),
        ));
    }
    let mut ops = Vec::with_capacity(items.len());
    let mut problems = Vec::new();
    for (i, item) in items.iter().enumerate() {
        match parse_op(item) {
            Ok(op) => ops.push(op),
            Err(message) => problems.push(Problem {
                op: Some(i),
                op_label: None,
                message,
            }),
        }
    }
    if problems.is_empty() {
        Ok(ops)
    } else {
        Err(Rejection {
            stage: Stage::Parse,
            problems,
        })
    }
}

fn parse_op(item: &Value) -> Result<PatchOp, String> {
    let Value::Object(obj) = item else {
        return Err("an operation must be a JSON object".into());
    };
    for key in obj.keys() {
        if !matches!(
            key.as_str(),
            "op" | "path" | "from" | "value" | "value_json"
        ) {
            return Err(format!("unknown key `{key}` in the operation"));
        }
    }
    let op = obj
        .get("op")
        .and_then(Value::as_str)
        .ok_or("missing string `op`")?;
    let path = obj
        .get("path")
        .and_then(Value::as_str)
        .ok_or("missing string `path`")?
        .to_string();
    let from = || -> Result<String, String> {
        obj.get("from")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| format!("`{op}` needs a string `from`"))
    };
    let value = || -> Result<Value, String> {
        let direct = obj.get("value");
        let encoded = obj
            .get("value_json")
            .map(|v| v.as_str().ok_or("`value_json` must be a string"))
            .transpose()?
            .filter(|s| !s.trim().is_empty());
        match (direct, encoded) {
            (Some(_), Some(_)) => Err("give `value` or `value_json`, not both".into()),
            (Some(v), None) => Ok(v.clone()),
            (None, Some(s)) => serde_json::from_str(s)
                .map_err(|e| format!("`value_json` is not valid JSON ({e}): {}", clip(s, 80))),
            (None, None) => Err(format!("`{op}` needs a `value` (or `value_json`)")),
        }
    };
    let op = match op {
        "add" => PatchOp::Add {
            path,
            value: value()?,
        },
        "remove" => PatchOp::Remove { path },
        "replace" => PatchOp::Replace {
            path,
            value: value()?,
        },
        "move" => PatchOp::Move {
            from: from()?,
            path,
        },
        "copy" => PatchOp::Copy {
            from: from()?,
            path,
        },
        "test" => PatchOp::Test {
            path,
            value: value()?,
        },
        other => {
            return Err(format!(
                "unknown op `{other}` (expected add, remove, replace, move, copy or test)"
            ))
        }
    };
    if let PatchOp::Add { value, .. }
    | PatchOp::Replace { value, .. }
    | PatchOp::Test { value, .. } = &op
    {
        let size = serde_json::to_string(value).map(|s| s.len()).unwrap_or(0);
        if size > MAX_VALUE_BYTES {
            return Err(format!(
                "the value is {size} bytes; the limit is {MAX_VALUE_BYTES}"
            ));
        }
    }
    Ok(op)
}

// ---------------------------------------------------------------------------------------------
// JSON Pointer (RFC 6901)

/// Splits a JSON Pointer into unescaped tokens. `""` is the whole document.
pub fn parse_pointer(pointer: &str) -> Result<Vec<String>, String> {
    if pointer.is_empty() {
        return Ok(Vec::new());
    }
    let Some(rest) = pointer.strip_prefix('/') else {
        return Err(format!(
            "`{pointer}` is not a JSON Pointer (it must start with `/`)"
        ));
    };
    rest.split('/').map(unescape_token).collect()
}

fn unescape_token(token: &str) -> Result<String, String> {
    let mut out = String::with_capacity(token.len());
    let mut chars = token.chars();
    while let Some(c) = chars.next() {
        if c == '~' {
            match chars.next() {
                Some('0') => out.push('~'),
                Some('1') => out.push('/'),
                _ => {
                    return Err(format!(
                        "bad escape in pointer token `{token}` (only ~0 and ~1)"
                    ))
                }
            }
        } else {
            out.push(c);
        }
    }
    Ok(out)
}

/// Escapes one token for a JSON Pointer.
pub fn escape_token(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

/// An array index token: `0`, `12`, not `01`, `-1` or `+1`.
fn array_index(token: &str) -> Option<usize> {
    if token.is_empty()
        || !token.bytes().all(|b| b.is_ascii_digit())
        || (token.len() > 1 && token.starts_with('0'))
    {
        return None;
    }
    token.parse().ok()
}

fn resolve<'a>(doc: &'a Value, tokens: &[String]) -> Result<&'a Value, String> {
    let mut cur = doc;
    for (depth, token) in tokens.iter().enumerate() {
        cur = match cur {
            Value::Object(m) => m.get(token),
            Value::Array(a) => array_index(token).and_then(|i| a.get(i)),
            _ => None,
        }
        .ok_or_else(|| format!("`{}` does not exist", pointer_of(&tokens[..=depth])))?;
    }
    Ok(cur)
}

fn resolve_mut<'a>(doc: &'a mut Value, tokens: &[String]) -> Result<&'a mut Value, String> {
    let mut cur = doc;
    for (depth, token) in tokens.iter().enumerate() {
        cur = match cur {
            Value::Object(m) => m.get_mut(token),
            Value::Array(a) => array_index(token).and_then(|i| a.get_mut(i)),
            _ => None,
        }
        .ok_or_else(|| format!("`{}` does not exist", pointer_of(&tokens[..=depth])))?;
    }
    Ok(cur)
}

fn pointer_of(tokens: &[String]) -> String {
    tokens
        .iter()
        .map(|t| format!("/{}", escape_token(t)))
        .collect()
}

fn add_at(doc: &mut Value, tokens: &[String], value: Value) -> Result<(), String> {
    let Some((last, parent_tokens)) = tokens.split_last() else {
        *doc = value;
        return Ok(());
    };
    let parent = resolve_mut(doc, parent_tokens)?;
    match parent {
        Value::Object(m) => {
            m.insert(last.clone(), value);
            Ok(())
        }
        Value::Array(a) => {
            if last == "-" {
                a.push(value);
                return Ok(());
            }
            let i = array_index(last).ok_or_else(|| {
                format!("`{last}` is not an array index (use 0..{} or -)", a.len())
            })?;
            if i > a.len() {
                return Err(format!(
                    "index {i} is past the end of `{}` (length {})",
                    pointer_of(parent_tokens),
                    a.len()
                ));
            }
            a.insert(i, value);
            Ok(())
        }
        _ => Err(format!(
            "`{}` is not an object or array",
            pointer_of(parent_tokens)
        )),
    }
}

fn remove_at(doc: &mut Value, tokens: &[String]) -> Result<Value, String> {
    let Some((last, parent_tokens)) = tokens.split_last() else {
        return Err("cannot remove the whole document".into());
    };
    let parent = resolve_mut(doc, parent_tokens)?;
    let missing = || format!("`{}` does not exist", pointer_of(tokens));
    match parent {
        Value::Object(m) => m.remove(last).ok_or_else(missing),
        Value::Array(a) => {
            let i = array_index(last)
                .filter(|&i| i < a.len())
                .ok_or_else(missing)?;
            Ok(a.remove(i))
        }
        _ => Err(missing()),
    }
}

/// JSON equality where numbers compare by value (`1` equals `1.0`), as RFC 6902 `test` asks.
pub fn json_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(x), Some(y)) => x == y,
            _ => x == y,
        },
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| json_equal(p, q))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| json_equal(v, w)))
        }
        _ => a == b,
    }
}

/// Applies one op in place. On error `doc` may be partly changed: callers work on a copy.
fn apply_op(doc: &mut Value, op: &PatchOp) -> Result<(), String> {
    match op {
        PatchOp::Add { path, value } => add_at(doc, &parse_pointer(path)?, value.clone()),
        PatchOp::Remove { path } => remove_at(doc, &parse_pointer(path)?).map(|_| ()),
        PatchOp::Replace { path, value } => {
            let target = resolve_mut(doc, &parse_pointer(path)?)?;
            *target = value.clone();
            Ok(())
        }
        PatchOp::Move { from, path } => {
            let from_t = parse_pointer(from)?;
            let to_t = parse_pointer(path)?;
            if to_t.len() > from_t.len() && to_t[..from_t.len()] == from_t[..] {
                return Err("cannot move a value into one of its own children".into());
            }
            if from_t == to_t {
                resolve(doc, &from_t)?;
                return Ok(());
            }
            let value = remove_at(doc, &from_t)?;
            add_at(doc, &to_t, value)
        }
        PatchOp::Copy { from, path } => {
            let value = resolve(doc, &parse_pointer(from)?)?.clone();
            add_at(doc, &parse_pointer(path)?, value)
        }
        PatchOp::Test { path, value } => {
            let actual = resolve(doc, &parse_pointer(path)?)?;
            if json_equal(actual, value) {
                Ok(())
            } else {
                Err(format!(
                    "test failed: the value is {}, not {}",
                    clip(&actual.to_string(), 80),
                    clip(&value.to_string(), 80)
                ))
            }
        }
    }
}

/// Applies a whole patch to a copy of `doc`; `doc` itself never changes. Stops at the first op
/// that fails. No model checks: see [`apply_edit`] for those.
pub fn apply_patch(doc: &Value, ops: &[PatchOp]) -> Result<Value, Rejection> {
    let mut out = doc.clone();
    for (i, op) in ops.iter().enumerate() {
        apply_op(&mut out, op).map_err(|m| Rejection::one(Stage::Apply, Some((i, op)), m))?;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// The timeline model, as far as paths are concerned

/// A place in the timeline document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Root,
    Resolution,
    Scenes,
    Scene,
    Highlights,
    Audio,
    Narration,
    Transition,
    Keyframes,
    Keyframe,
    Layers,
    Layer,
    LayerAnimation,
    TextBackground,
    Subtitles,
    Cue,
    Words,
    Word,
    Music,
    Duck,
    /// A number, string, bool or enum value: no children.
    Leaf,
}

impl Kind {
    fn fields(self) -> Option<&'static [(&'static str, Kind)]> {
        use Kind::*;
        Some(match self {
            Root => &[
                ("fps", Leaf),
                ("resolution", Resolution),
                ("scenes", Scenes),
                ("subtitles", Subtitles),
                ("music", Music),
            ],
            Resolution => &[("width", Leaf), ("height", Leaf)],
            Scene => &[
                ("id", Leaf),
                ("duration_ms", Leaf),
                ("text", Leaf),
                ("highlights", Highlights),
                ("image", Leaf),
                ("motion", Leaf),
                ("audio", Audio),
                ("narration", Narration),
                ("transition_in", Transition),
                ("keyframes", Keyframes),
                ("layers", Layers),
            ],
            // Every field any layer kind has; `check_value` narrows it to the value's `kind`.
            Layer => LAYER_FIELDS_ALL,
            LayerAnimation => &[("kind", Leaf), ("duration_ms", Leaf)],
            TextBackground => &[("color", Leaf), ("padding", Leaf), ("radius", Leaf)],
            Audio => &[("source", Leaf), ("offset_ms", Leaf)],
            Narration => &[("voice", Leaf)],
            Transition => &[("kind", Leaf), ("duration_ms", Leaf)],
            Keyframe => &[
                ("at_ms", Leaf),
                ("scale", Leaf),
                ("x", Leaf),
                ("y", Leaf),
                ("opacity", Leaf),
                ("rotation", Leaf),
                ("ease", Leaf),
            ],
            Cue => &[
                ("start_ms", Leaf),
                ("end_ms", Leaf),
                ("text", Leaf),
                ("words", Words),
            ],
            Word => &[("text", Leaf), ("start_ms", Leaf), ("end_ms", Leaf)],
            Music => &[
                ("source", Leaf),
                ("volume", Leaf),
                ("fade_in_ms", Leaf),
                ("fade_out_ms", Leaf),
                ("duck", Duck),
            ],
            Duck => &[
                ("enabled", Leaf),
                ("depth_db", Leaf),
                ("attack_ms", Leaf),
                ("release_ms", Leaf),
            ],
            _ => return None,
        })
    }

    fn element(self) -> Option<Kind> {
        match self {
            Kind::Scenes => Some(Kind::Scene),
            Kind::Highlights => Some(Kind::Leaf),
            Kind::Keyframes => Some(Kind::Keyframe),
            Kind::Layers => Some(Kind::Layer),
            Kind::Subtitles => Some(Kind::Cue),
            Kind::Words => Some(Kind::Word),
            _ => None,
        }
    }

    fn describe(self) -> &'static str {
        match self {
            Kind::Root => "the timeline",
            Kind::Resolution => "a resolution",
            Kind::Scenes => "the scene list",
            Kind::Scene => "a scene",
            Kind::Highlights => "a highlight list",
            Kind::Audio => "an audio mapping",
            Kind::Narration => "a narration",
            Kind::Transition => "a transition",
            Kind::Keyframes => "a keyframe list",
            Kind::Keyframe => "a keyframe",
            Kind::Layers => "a layer list",
            Kind::Layer => "a layer",
            Kind::LayerAnimation => "a layer enter/exit animation",
            Kind::TextBackground => "a text layer background",
            Kind::Subtitles => "the subtitle list",
            Kind::Cue => "a subtitle cue",
            Kind::Words => "a word list",
            Kind::Word => "a word timing",
            Kind::Music => "the music settings",
            Kind::Duck => "the music ducking settings",
            Kind::Leaf => "a plain value",
        }
    }
}

/// Fields every layer has (`vimmer_dsl::Layer`), `kind` included.
const LAYER_COMMON: &[(&str, Kind)] = &[
    ("id", Kind::Leaf),
    ("kind", Kind::Leaf),
    ("x", Kind::Leaf),
    ("y", Kind::Leaf),
    ("width", Kind::Leaf),
    ("height", Kind::Leaf),
    ("anchor", Kind::Leaf),
    ("z", Kind::Leaf),
    ("start_ms", Kind::Leaf),
    ("end_ms", Kind::Leaf),
    ("opacity", Kind::Leaf),
    ("rotation", Kind::Leaf),
    ("keyframes", Kind::Keyframes),
    ("enter", Kind::LayerAnimation),
    ("exit", Kind::LayerAnimation),
];

/// Kind-specific layer fields (`vimmer_dsl::LayerContent`), by `kind`.
const LAYER_KIND_FIELDS: &[(&str, &[(&str, Kind)])] = &[
    (
        "image",
        &[
            ("src", Kind::Leaf),
            ("fit", Kind::Leaf),
            ("corner_radius", Kind::Leaf),
        ],
    ),
    (
        "text",
        &[
            ("text", Kind::Leaf),
            ("font_size", Kind::Leaf),
            ("color", Kind::Leaf),
            ("weight", Kind::Leaf),
            ("align", Kind::Leaf),
            ("line_height", Kind::Leaf),
            ("background", Kind::TextBackground),
            ("highlights", Kind::Highlights),
            ("reveal", Kind::Leaf),
        ],
    ),
    (
        "shape",
        &[
            ("shape", Kind::Leaf),
            ("fill", Kind::Leaf),
            ("stroke", Kind::Leaf),
            ("stroke_width", Kind::Leaf),
            ("corner_radius", Kind::Leaf),
        ],
    ),
    ("icon", &[("src", Kind::Leaf), ("color", Kind::Leaf)]),
];

/// The union of [`LAYER_COMMON`] and every kind's fields, for pointers (which do not know the
/// layer's kind). Values are checked against their own kind in [`check_value`].
const LAYER_FIELDS_ALL: &[(&str, Kind)] = &[
    ("id", Kind::Leaf),
    ("kind", Kind::Leaf),
    ("x", Kind::Leaf),
    ("y", Kind::Leaf),
    ("width", Kind::Leaf),
    ("height", Kind::Leaf),
    ("anchor", Kind::Leaf),
    ("z", Kind::Leaf),
    ("start_ms", Kind::Leaf),
    ("end_ms", Kind::Leaf),
    ("opacity", Kind::Leaf),
    ("rotation", Kind::Leaf),
    ("keyframes", Kind::Keyframes),
    ("enter", Kind::LayerAnimation),
    ("exit", Kind::LayerAnimation),
    ("src", Kind::Leaf),
    ("fit", Kind::Leaf),
    ("corner_radius", Kind::Leaf),
    ("text", Kind::Leaf),
    ("font_size", Kind::Leaf),
    ("color", Kind::Leaf),
    ("weight", Kind::Leaf),
    ("align", Kind::Leaf),
    ("line_height", Kind::Leaf),
    ("background", Kind::TextBackground),
    ("highlights", Kind::Highlights),
    ("reveal", Kind::Leaf),
    ("shape", Kind::Leaf),
    ("fill", Kind::Leaf),
    ("stroke", Kind::Leaf),
    ("stroke_width", Kind::Leaf),
];

/// The fields a layer object may have: common fields plus those of its `kind`, when the kind is
/// known. An unknown or missing kind gets the union (deserialization then reports the kind).
fn layer_fields_for(map: &Map<String, Value>) -> (Vec<(&'static str, Kind)>, Option<&str>) {
    let kind = map.get("kind").and_then(Value::as_str);
    match kind.and_then(|k| LAYER_KIND_FIELDS.iter().find(|(name, _)| *name == k)) {
        Some((name, fields)) => (
            LAYER_COMMON.iter().chain(fields.iter()).copied().collect(),
            Some(name),
        ),
        None => (LAYER_FIELDS_ALL.to_vec(), None),
    }
}

/// What a pointer points at in the model, or why it points outside the model.
fn kind_at(tokens: &[String]) -> Result<Kind, String> {
    let mut kind = Kind::Root;
    for (depth, token) in tokens.iter().enumerate() {
        let here = || {
            if depth == 0 {
                "the top level".to_string()
            } else {
                format!("`{}`", pointer_of(&tokens[..depth]))
            }
        };
        kind = if let Some(fields) = kind.fields() {
            match fields.iter().find(|(name, _)| name == token) {
                Some((_, k)) => *k,
                None => {
                    let names: Vec<&str> = fields.iter().map(|(n, _)| *n).collect();
                    return Err(format!(
                        "`{token}` is not a field of {} ({}); known fields: {}",
                        kind.describe(),
                        here(),
                        names.join(", ")
                    ));
                }
            }
        } else if let Some(element) = kind.element() {
            if token != "-" && array_index(token).is_none() {
                return Err(format!(
                    "`{token}` is not an index into {} ({})",
                    kind.describe(),
                    here()
                ));
            }
            element
        } else {
            return Err(format!(
                "{} is {}, which has no fields",
                here(),
                kind.describe()
            ));
        };
    }
    Ok(kind)
}

/// Checks that a value written at a place of `kind` has only known fields, recursively.
fn check_value(kind: Kind, value: &Value, at: &str) -> Result<(), String> {
    match value {
        Value::Object(map) => {
            let (fields, layer_kind) = if kind == Kind::Layer {
                let (f, k) = layer_fields_for(map);
                (f, k)
            } else {
                let Some(fields) = kind.fields() else {
                    return Ok(()); // a type mismatch: deserialization reports it
                };
                (fields.to_vec(), None)
            };
            for (key, child) in map {
                let Some((_, child_kind)) = fields.iter().find(|(n, _)| n == key) else {
                    let names: Vec<&str> = fields.iter().map(|(n, _)| *n).collect();
                    let what = match layer_kind {
                        Some(k) => format!("a {k} layer"),
                        None => kind.describe().to_string(),
                    };
                    return Err(format!(
                        "unknown field `{key}` in {what} at `{at}`; known fields: {}",
                        names.join(", ")
                    ));
                };
                check_value(*child_kind, child, &format!("{at}/{}", escape_token(key)))?;
            }
            Ok(())
        }
        Value::Array(items) => {
            if let Some(element) = kind.element() {
                for (i, item) in items.iter().enumerate() {
                    check_value(element, item, &format!("{at}/{i}"))?;
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Model-level checks of one op, before anything is applied.
fn guard_op(op: &PatchOp) -> Result<(), String> {
    let target = parse_pointer(op.path())?;
    if target.is_empty() && !matches!(op, PatchOp::Test { .. }) {
        return Err(
            "an edit may not replace or remove the whole timeline; patch its fields".into(),
        );
    }
    let kind = kind_at(&target)?;
    match op {
        PatchOp::Add { value, path }
        | PatchOp::Replace { value, path }
        | PatchOp::Test { value, path } => check_value(kind, value, path),
        PatchOp::Remove { .. } => {
            if target.len() == 1 && matches!(target[0].as_str(), "fps" | "resolution" | "scenes") {
                return Err(format!(
                    "`/{}` is required and cannot be removed",
                    target[0]
                ));
            }
            Ok(())
        }
        PatchOp::Move { from, .. } | PatchOp::Copy { from, .. } => {
            let source = parse_pointer(from)?;
            if source.is_empty() {
                return Err("`from` may not be the whole document".into());
            }
            let source_kind = kind_at(&source)?;
            if source_kind != kind {
                return Err(format!(
                    "cannot {} {} (`{from}`) to a place for {} (`{}`)",
                    op.name(),
                    source_kind.describe(),
                    kind.describe(),
                    op.path()
                ));
            }
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Asset paths

/// Checks a new asset path: relative, inside the public dir, no `..`, no URL scheme.
/// Returns a warning when the file does not exist (only with a public dir).
pub fn check_asset_path(raw: &str, public_dir: Option<&Path>) -> Result<Option<String>, String> {
    let p = raw.trim();
    if p.is_empty() {
        return Err("the path is empty".into());
    }
    if p != raw {
        return Err("the path has leading or trailing spaces".into());
    }
    if p.chars().count() > MAX_ASSET_PATH_CHARS {
        return Err(format!(
            "the path is longer than {MAX_ASSET_PATH_CHARS} characters"
        ));
    }
    if p.chars().any(char::is_control) {
        return Err("the path contains control characters".into());
    }
    if p.contains('\\') {
        return Err("use `/` as the separator, not `\\`".into());
    }
    if p.contains('%') {
        return Err("percent-encoded paths are not allowed".into());
    }
    if p.starts_with('/') || p.starts_with('~') {
        return Err(
            "asset paths must be relative to the public dir (no leading `/` or `~`)".into(),
        );
    }
    if has_scheme(p) {
        return Err("URLs and schemes are not allowed; use a file inside public/".into());
    }
    for comp in Path::new(p).components() {
        match comp {
            Component::Normal(_) => {}
            Component::CurDir | Component::ParentDir => {
                return Err("`.` and `..` are not allowed in asset paths".into())
            }
            _ => return Err("asset paths must be relative to the public dir".into()),
        }
    }
    if p.split('/')
        .any(|seg| seg == ".." || seg == "." || seg.is_empty())
    {
        return Err("`.`, `..` and empty segments are not allowed in asset paths".into());
    }
    let Some(dir) = public_dir else {
        return Ok(None);
    };
    let full = dir.join(p);
    if !full.exists() {
        return Ok(Some(format!("`{p}` was not found in {}", dir.display())));
    }
    let (Ok(root), Ok(real)) = (dir.canonicalize(), full.canonicalize()) else {
        return Ok(Some(format!(
            "`{p}` could not be resolved in {}",
            dir.display()
        )));
    };
    if !real.starts_with(&root) {
        return Err("the path leads outside the public dir (through a symlink)".into());
    }
    if !real.is_file() {
        return Err("the path is not a file".into());
    }
    Ok(None)
}

/// `http:`, `file:`, `data:`, `C:` ... anything that looks like `scheme:` before the first `/`.
fn has_scheme(p: &str) -> bool {
    let head = p.split('/').next().unwrap_or("");
    match head.find(':') {
        Some(i) => {
            let scheme = &head[..i];
            i == 0
                || scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        }
        None => false,
    }
}

/// A narration voice is a name for the speech engine, never a path.
fn check_voice(voice: &str) -> Result<(), String> {
    if voice.trim().is_empty() || voice.chars().count() > 64 {
        return Err("a voice name must be 1-64 characters".into());
    }
    if !voice
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.' | '(' | ')'))
    {
        return Err("a voice name may only have letters, digits, spaces and - _ . ( )".into());
    }
    if voice.contains("..") {
        return Err("a voice name may not contain `..`".into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Sanity limits

fn valid_scene_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Limits beyond the DSL rules. Each message names the scene or field.
fn sanity_problems(t: &VideoTimeline) -> Vec<String> {
    let mut out = Vec::new();
    if t.scenes.len() > MAX_SCENES {
        out.push(format!(
            "the timeline has {} scenes; the limit is {MAX_SCENES}",
            t.scenes.len()
        ));
    }
    for s in &t.scenes {
        if !valid_scene_id(&s.id) {
            out.push(format!(
                "scene id `{}` must be 1-64 characters of letters, digits, - or _",
                clip(&s.id, 40)
            ));
        }
        if s.duration_ms > 0 && s.duration_ms < MIN_SCENE_MS {
            out.push(format!(
                "scene `{}` is {} ms; a scene must be at least {MIN_SCENE_MS} ms",
                s.id, s.duration_ms
            ));
        }
        if s.duration_ms > MAX_SCENE_MS {
            out.push(format!(
                "scene `{}` is {} ms; a scene may be at most {MAX_SCENE_MS} ms",
                s.id, s.duration_ms
            ));
        }
        if let Some(text) = &s.text {
            let n = text.chars().count();
            if n > MAX_TEXT_CHARS {
                out.push(format!(
                    "scene `{}` text is {n} characters; the limit is {MAX_TEXT_CHARS}",
                    s.id
                ));
            }
        }
        if s.highlights.len() > MAX_HIGHLIGHTS {
            out.push(format!(
                "scene `{}` has {} highlights; the limit is {MAX_HIGHLIGHTS}",
                s.id,
                s.highlights.len()
            ));
        }
        if s.highlights.iter().any(|h| h.trim().is_empty()) {
            out.push(format!("scene `{}` has an empty highlight", s.id));
        }
        let distinct: HashSet<&str> = s.highlights.iter().map(String::as_str).collect();
        if distinct.len() < s.highlights.len() {
            out.push(format!("scene `{}` lists the same highlight twice", s.id));
        }
        if let Some(tr) = &s.transition_in {
            if tr.duration_ms > MAX_TRANSITION_MS {
                out.push(format!(
                    "scene `{}` transition is {} ms; the limit is {MAX_TRANSITION_MS} ms",
                    s.id, tr.duration_ms
                ));
            }
        }
        if let Some(audio) = &s.audio {
            if audio.offset_ms > s.duration_ms {
                out.push(format!(
                    "scene `{}` audio starts at {} ms, after the scene ends ({} ms)",
                    s.id, audio.offset_ms, s.duration_ms
                ));
            }
        }
        if s.keyframes.len() > MAX_KEYFRAMES {
            out.push(format!(
                "scene `{}` has {} keyframes; the limit is {MAX_KEYFRAMES}",
                s.id,
                s.keyframes.len()
            ));
        }
        for (i, k) in s.keyframes.iter().enumerate() {
            if k.scale > MAX_KEYFRAME_SCALE {
                out.push(format!(
                    "scene `{}` keyframe {i} scale {} is above {MAX_KEYFRAME_SCALE}",
                    s.id, k.scale
                ));
            }
            if k.x.abs() > MAX_KEYFRAME_OFFSET || k.y.abs() > MAX_KEYFRAME_OFFSET {
                out.push(format!(
                    "scene `{}` keyframe {i} x/y must be within ±{MAX_KEYFRAME_OFFSET} (percent of the frame)",
                    s.id
                ));
            }
            if k.rotation.abs() > MAX_KEYFRAME_ROTATION {
                out.push(format!(
                    "scene `{}` keyframe {i} rotation must be within ±{MAX_KEYFRAME_ROTATION} degrees",
                    s.id
                ));
            }
        }
    }
    for (i, cue) in t.subtitles.iter().enumerate() {
        if cue.words.is_empty() {
            continue;
        }
        let shown: String = cue.words.iter().map(|w| w.text.as_str()).collect();
        if shown.trim() != cue.text.trim() {
            out.push(format!(
                "subtitle {i}: its words read \"{}\" but its text is \"{}\"; the words are what is shown, so update them or remove `words`",
                clip(shown.trim(), 40),
                clip(cue.text.trim(), 40)
            ));
        }
    }
    if t.subtitles.len() > MAX_SUBTITLES {
        out.push(format!(
            "the timeline has {} subtitle cues; the limit is {MAX_SUBTITLES}",
            t.subtitles.len()
        ));
    }
    if let Some(m) = &t.music {
        if m.fade_in_ms > MAX_FADE_MS || m.fade_out_ms > MAX_FADE_MS {
            out.push(format!("music fades may be at most {MAX_FADE_MS} ms each"));
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// The whole check

/// Applies `patch` to `original` and checks the result (see the module docs). `original` is
/// the timeline JSON as the caller holds it; it is never modified.
pub fn apply_edit(
    original: &Value,
    patch: &Value,
    check: &EditCheck,
) -> Result<EditOutcome, Rejection> {
    let ops = parse_patch(patch)?;
    apply_ops(original, &ops, check)
}

/// [`apply_edit`] with an already parsed patch.
pub fn apply_ops(
    original: &Value,
    ops: &[PatchOp],
    check: &EditCheck,
) -> Result<EditOutcome, Rejection> {
    let before: VideoTimeline = serde_json::from_value(original.clone()).map_err(|e| {
        Rejection::one(
            Stage::Model,
            None,
            format!("the timeline to edit is not a valid timeline: {e}"),
        )
    })?;

    let guard: Vec<Problem> = ops
        .iter()
        .enumerate()
        .filter_map(|(i, op)| guard_op(op).err().map(|m| problem(Some((i, op)), m)))
        .collect();
    if !guard.is_empty() {
        return Err(Rejection {
            stage: Stage::Guard,
            problems: guard,
        });
    }

    let patched = apply_patch(original, ops)?;
    // The guard checked op values; this also catches unknown fields that reached the document
    // another way, unless they were already in the original.
    if let Err(m) = check_value(Kind::Root, &patched, "") {
        if check_value(Kind::Root, original, "").is_ok() {
            return Err(Rejection::one(Stage::Guard, None, m));
        }
    }
    let mut patched = patched;
    let mut after: VideoTimeline = serde_json::from_value(patched.clone()).map_err(|e| {
        Rejection::one(
            Stage::Model,
            None,
            format!("the patched JSON is not a valid timeline: {e}"),
        )
    })?;

    // Subtitle cues use absolute times. When scene timing changes and the patch leaves the
    // subtitles alone, they are moved with their scenes here (deterministic, no arithmetic for
    // Claude to get wrong). A patch that edits subtitles itself is taken as is.
    let ops_touch_subtitles = ops.iter().any(|op| {
        op.path().starts_with("/subtitles")
            || matches!(op, PatchOp::Move { from, .. } | PatchOp::Copy { from, .. } if from.starts_with("/subtitles"))
    });
    let mut all_ops: Vec<PatchOp> = ops.to_vec();
    let mut notes = Vec::new();
    let retime =
        !ops_touch_subtitles && !after.subtitles.is_empty() && timing_changed(&before, &after);
    if retime {
        let (cues, dropped) = retime_subtitles(&before, &after);
        if cues != after.subtitles {
            let value = serde_json::to_value(&cues).expect("cues serialize");
            if let Value::Object(map) = &mut patched {
                map.insert("subtitles".into(), value.clone());
            }
            after.subtitles = cues;
            all_ops.push(PatchOp::Replace {
                path: "/subtitles".into(),
                value,
            });
            if dropped > 0 {
                notes.push(format!(
                    "{dropped} subtitle cue(s) or word(s) belonged to removed time and were dropped"
                ));
            }
        }
    }

    let mut problems = Vec::new();
    let old_errors: HashSet<String> = validate(&before)
        .err()
        .unwrap_or_default()
        .iter()
        .map(ToString::to_string)
        .collect();
    if let Err(errors) = validate(&after) {
        for e in errors {
            let text = e.to_string();
            let note = if old_errors.contains(&text) {
                " (this was already wrong before the patch; fix it too)"
            } else {
                ""
            };
            problems.push(problem(None, format!("{text}{note}")));
        }
    }
    let old_sanity: HashSet<String> = sanity_problems(&before).into_iter().collect();
    for p in sanity_problems(&after) {
        if !old_sanity.contains(&p) {
            problems.push(problem(None, p));
        }
    }

    let mut warnings = Vec::new();
    if ops_touch_subtitles {
        let before_overlaps = overlaps(&before.subtitles).len();
        let now = overlaps(&after.subtitles);
        if now.len() > before_overlaps {
            for (i, j) in now {
                let (a, b) = (&after.subtitles[i], &after.subtitles[j]);
                problems.push(problem(
                    None,
                    format!(
                        "subtitle {i} ({}..{} ms) overlaps subtitle {j} ({}..{} ms); cues must not overlap",
                        a.start_ms, a.end_ms, b.start_ms, b.end_ms
                    ),
                ));
            }
        }
    }
    let old_assets: HashSet<String> = asset_refs(&before).into_iter().map(|(_, v)| v).collect();
    for (place, value) in asset_refs(&after) {
        if old_assets.contains(&value) {
            continue;
        }
        match check_asset_path(&value, check.public_dir.as_deref()) {
            Ok(Some(w)) => warnings.push(format!("{place}: {w}")),
            Ok(None) => {}
            Err(e) => problems.push(problem(
                None,
                format!("{place} `{}`: {e}", clip(&value, 80)),
            )),
        }
    }
    let old_voices: HashSet<&str> = voices(&before).into_iter().collect();
    for (scene, voice) in voices_by_scene(&after) {
        if !old_voices.contains(voice) {
            if let Err(e) = check_voice(voice) {
                problems.push(problem(
                    None,
                    format!("scene `{scene}` narration voice: {e}"),
                ));
            }
        }
    }

    if !problems.is_empty() {
        return Err(Rejection {
            stage: Stage::Validation,
            problems,
        });
    }

    warnings.extend(edit_warnings(
        &before,
        &after,
        ops_touch_subtitles || retime,
    ));
    warnings.extend(notes);
    let mut summary = summarize(&before, &after);
    if all_ops.len() > ops.len() {
        for line in summary.iter_mut().filter(|l| l.starts_with("subtitles:")) {
            line.push_str(" (moved with their scenes)");
        }
    }
    let changed = !json_equal(original, &patched);
    Ok(EditOutcome {
        timeline: patched,
        summary,
        warnings,
        changed,
        patch: patch_to_json(&all_ops),
    })
}

// ---------------------------------------------------------------------------------------------
// Subtitle retiming

/// (id, start, end) of every scene, in order.
fn scene_spans(t: &VideoTimeline) -> Vec<(&str, u64, u64)> {
    let mut at = 0;
    t.scenes
        .iter()
        .map(|s| {
            let span = (s.id.as_str(), at, at + s.duration_ms);
            at += s.duration_ms;
            span
        })
        .collect()
}

fn timing_changed(before: &VideoTimeline, after: &VideoTimeline) -> bool {
    let a: Vec<(&str, u64)> = before
        .scenes
        .iter()
        .map(|s| (s.id.as_str(), s.duration_ms))
        .collect();
    let b: Vec<(&str, u64)> = after
        .scenes
        .iter()
        .map(|s| (s.id.as_str(), s.duration_ms))
        .collect();
    a != b
}

/// Pairs of cue indices (in list order) whose times overlap.
fn overlaps(cues: &[vimmer_dsl::SubtitleCue]) -> Vec<(usize, usize)> {
    let mut order: Vec<usize> = (0..cues.len()).collect();
    order.sort_by_key(|&i| (cues[i].start_ms, cues[i].end_ms));
    order
        .windows(2)
        .filter(|w| cues[w[0]].end_ms > cues[w[1]].start_ms)
        .map(|w| (w[0].min(w[1]), w[0].max(w[1])))
        .collect()
}

/// Maps times on the old timeline to the new one by scene: a time keeps its offset inside its
/// scene (clamped to the scene's new length). Times inside a removed scene go to the start of
/// the next kept scene (`as_end` false) or the end of the previous kept one (`as_end` true).
struct TimeMap<'a> {
    old: Vec<(&'a str, u64, u64)>,
    new: BTreeMap<&'a str, (u64, u64)>,
    new_total: u64,
}

impl<'a> TimeMap<'a> {
    fn new(before: &'a VideoTimeline, after: &'a VideoTimeline) -> Self {
        let new_spans = scene_spans(after);
        let new_total = new_spans.last().map_or(0, |s| s.2);
        Self {
            old: scene_spans(before),
            new: new_spans
                .into_iter()
                .map(|(id, s, e)| (id, (s, e)))
                .collect(),
            new_total,
        }
    }

    fn old_index(&self, t: u64, as_end: bool) -> usize {
        let found = self.old.iter().position(|&(_, s, e)| {
            if as_end {
                s < t && t <= e
            } else {
                s <= t && t < e
            }
        });
        found.unwrap_or(if t == 0 {
            0
        } else {
            self.old.len().saturating_sub(1)
        })
    }

    /// The new time, and whether the old one was in a kept scene.
    fn map(&self, t: u64, as_end: bool) -> (u64, bool) {
        if self.old.is_empty() {
            return (t, false);
        }
        let i = self.old_index(t, as_end);
        let (id, s, _) = self.old[i];
        if let Some(&(ns, ne)) = self.new.get(id) {
            return (ns + t.saturating_sub(s).min(ne - ns), true);
        }
        let kept = |j: usize| self.new.get(self.old[j].0).copied();
        let t = if as_end {
            (0..i).rev().find_map(kept).map_or(0, |(_, e)| e)
        } else {
            (i + 1..self.old.len())
                .find_map(kept)
                .map_or(self.new_total, |(s, _)| s)
        };
        (t, false)
    }

    /// New end of the scene that held old time `t`, when that scene was kept.
    fn scene_end(&self, t: u64) -> Option<u64> {
        let i = self.old_index(t, false);
        self.old
            .get(i)
            .and_then(|(id, _, _)| self.new.get(id))
            .map(|&(_, e)| e)
    }
}

/// The subtitle track moved to follow changed scene timing. Returns the cues and how many cues
/// or words were dropped because their time was removed.
pub fn retime_subtitles(
    before: &VideoTimeline,
    after: &VideoTimeline,
) -> (Vec<vimmer_dsl::SubtitleCue>, usize) {
    let map = TimeMap::new(before, after);
    let mut dropped = 0;
    let mut out: Vec<vimmer_dsl::SubtitleCue> = Vec::new();
    for cue in &before.subtitles {
        let (start, _) = map.map(cue.start_ms, false);
        let (mut end, _) = map.map(cue.end_ms, true);
        if end <= start {
            // Reordered scenes can separate a cue's start and end: keep the start's scene.
            end = map.scene_end(cue.start_ms).unwrap_or(start).max(start);
        }
        let mut next = cue.clone();
        next.start_ms = start;
        next.end_ms = end;
        if !cue.words.is_empty() {
            let mut words = Vec::new();
            let mut prev_end = start;
            let mut ordered = true;
            for w in &cue.words {
                let (ws, kept_s) = map.map(w.start_ms, false);
                let (we, kept_e) = map.map(w.end_ms, true);
                let (ws, we) = (ws.clamp(start, end), we.clamp(start, end));
                let removed = !kept_s && !kept_e;
                if removed || we < ws {
                    dropped += 1;
                    continue;
                }
                if ws < prev_end {
                    ordered = false;
                }
                prev_end = prev_end.max(we);
                let mut w = w.clone();
                w.start_ms = ws;
                w.end_ms = we;
                words.push(w);
            }
            if !ordered {
                words.clear(); // shown as a plain line rather than out of order
            } else if words.is_empty() {
                dropped += 1;
                continue;
            } else if words.len() < cue.words.len() {
                let text: String = words.iter().map(|w| w.text.as_str()).collect();
                next.text = text.trim().to_string();
                if let Some(last) = words.last_mut() {
                    last.text = last.text.trim_end().to_string();
                }
            }
            next.words = words;
        }
        if next.end_ms <= next.start_ms || next.text.trim().is_empty() {
            dropped += 1;
            continue;
        }
        out.push(next);
    }
    out.sort_by_key(|c| (c.start_ms, c.end_ms));
    // A cue that now runs into the next one ends where the next one starts.
    let mut i = 0;
    while i + 1 < out.len() {
        let next_start = out[i + 1].start_ms;
        let cue = &mut out[i];
        if cue.end_ms > next_start {
            cue.end_ms = next_start;
            cue.words.retain(|w| {
                w.start_ms < next_start || w.start_ms == w.end_ms && w.start_ms <= next_start
            });
            for w in &mut cue.words {
                w.end_ms = w.end_ms.min(next_start);
            }
            if cue.end_ms <= cue.start_ms {
                out.remove(i);
                dropped += 1;
                continue;
            }
        }
        i += 1;
    }
    (out, dropped)
}

/// Every asset path in a timeline, with where it is.
fn asset_refs(t: &VideoTimeline) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for s in &t.scenes {
        if let Some(image) = &s.image {
            out.push((format!("scene `{}` image", s.id), image.clone()));
        }
        if let Some(audio) = &s.audio {
            out.push((
                format!("scene `{}` audio.source", s.id),
                audio.source.clone(),
            ));
        }
        for layer in &s.layers {
            if let Some(src) = layer.content.asset() {
                out.push((
                    format!("scene `{}` layer `{}` src", s.id, layer.id),
                    src.to_string(),
                ));
            }
        }
    }
    if let Some(m) = &t.music {
        out.push(("music.source".into(), m.source.clone()));
    }
    out
}

fn voices(t: &VideoTimeline) -> Vec<&str> {
    voices_by_scene(t).into_iter().map(|(_, v)| v).collect()
}

fn voices_by_scene(t: &VideoTimeline) -> Vec<(&str, &str)> {
    t.scenes
        .iter()
        .filter_map(|s| {
            s.narration
                .as_ref()
                .and_then(|n| n.voice.as_deref())
                .map(|v| (s.id.as_str(), v))
        })
        .collect()
}

fn guess_language(text: &str) -> Language {
    let kana = text.chars().any(|c| ('\u{3040}'..='\u{30ff}').contains(&c));
    let cjk = text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c));
    if kana {
        Language::Ja
    } else if cjk {
        Language::Zh
    } else {
        Language::En
    }
}

fn edit_warnings(
    before: &VideoTimeline,
    after: &VideoTimeline,
    touched_subtitles: bool,
) -> Vec<String> {
    let mut out = Vec::new();
    let old: BTreeMap<&str, &Scene> = before.scenes.iter().map(|s| (s.id.as_str(), s)).collect();
    for s in &after.scenes {
        let prev = old.get(s.id.as_str());
        let text_or_highlights_changed =
            prev.is_none_or(|p| p.text != s.text || p.highlights != s.highlights);
        if text_or_highlights_changed {
            let text = s.text.as_deref().unwrap_or("");
            for h in &s.highlights {
                if !h.trim().is_empty() && !text.contains(h.as_str()) {
                    out.push(format!(
                        "scene `{}`: highlight `{h}` does not occur in its text, so nothing is highlighted",
                        s.id
                    ));
                }
            }
        }
        if !s.keyframes.is_empty() && prev.is_none_or(|p| p.motion != s.motion) {
            out.push(format!(
                "scene `{}`: `motion` has no effect because the scene has keyframes",
                s.id
            ));
        }
        let narrated = s.narration.is_some() && s.audio.is_none();
        let shorter = prev.is_none_or(|p| s.duration_ms < p.duration_ms || p.text != s.text);
        if narrated && shorter {
            if let Some(text) = s.text.as_deref().filter(|t| !t.trim().is_empty()) {
                let need = estimate_duration_ms(text, guess_language(text));
                if s.duration_ms * 10 < need * 8 {
                    out.push(format!(
                        "scene `{}`: its narration takes about {:.1} s but the scene is {:.1} s; the render lengthens the scene to fit the narration",
                        s.id,
                        need as f64 / 1000.0,
                        s.duration_ms as f64 / 1000.0
                    ));
                }
            }
        }
    }
    let timing_changed = before.scenes.len() != after.scenes.len()
        || before
            .scenes
            .iter()
            .zip(&after.scenes)
            .any(|(a, b)| a.id != b.id || a.duration_ms != b.duration_ms);
    if timing_changed && !after.subtitles.is_empty() && !touched_subtitles {
        out.push(
            "scene timing changed but the subtitle cues (absolute times) were not moved; check that they still line up"
                .into(),
        );
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Diff summary

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

fn secs(ms: u64) -> String {
    format!("{:.2} s", ms as f64 / 1000.0)
}

fn show(v: &Value) -> String {
    match v {
        Value::Null => "none".into(),
        // DSL floats are f32; print them as such (0.1, not 0.10000000149011612).
        Value::Number(n) if !n.is_i64() && !n.is_u64() => n
            .as_f64()
            .map_or_else(|| n.to_string(), |f| (f as f32).to_string()),
        Value::String(s) => format!("\"{}\"", clip(s, 40)),
        other => clip(&other.to_string(), 60),
    }
}

fn show_field(key: &str, v: &Value) -> String {
    match key {
        "duration_ms" => v.as_u64().map_or_else(|| show(v), secs),
        "transition_in" => match v {
            Value::Object(m) => format!(
                "{} {}",
                m.get("kind").and_then(Value::as_str).unwrap_or("?"),
                m.get("duration_ms")
                    .and_then(Value::as_u64)
                    .map_or("?".into(), |ms| format!("{ms} ms"))
            ),
            _ => show(v),
        },
        "keyframes" => match v {
            Value::Array(a) if a.is_empty() => "none".into(),
            Value::Array(a) => format!("{} keyframes", a.len()),
            _ => show(v),
        },
        "narration" => match v {
            Value::Object(m) => match m.get("voice").and_then(Value::as_str) {
                Some(voice) => format!("on (voice {voice})"),
                None => "on".into(),
            },
            _ => "off".into(),
        },
        "highlights" => match v {
            Value::Array(a) if a.is_empty() => "none".into(),
            Value::Array(a) => a
                .iter()
                .map(|h| format!("\"{}\"", h.as_str().unwrap_or("?")))
                .collect::<Vec<_>>()
                .join(", "),
            _ => show(v),
        },
        _ => show(v),
    }
}

fn label(key: &str) -> &str {
    match key {
        "duration_ms" => "duration",
        "transition_in" => "transition in",
        other => other,
    }
}

/// What changed between two timelines, one line per change. Scenes are matched by id.
pub fn summarize(before: &VideoTimeline, after: &VideoTimeline) -> Vec<String> {
    let mut out = Vec::new();
    if before.resolution != after.resolution {
        out.push(format!(
            "format: {}x{} -> {}x{}",
            before.resolution.width,
            before.resolution.height,
            after.resolution.width,
            after.resolution.height
        ));
    }
    if before.fps != after.fps {
        out.push(format!("fps: {} -> {}", before.fps, after.fps));
    }
    let old: BTreeMap<&str, (usize, &Scene)> = before
        .scenes
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id.as_str(), (i, s)))
        .collect();
    let new_ids: HashSet<&str> = after.scenes.iter().map(|s| s.id.as_str()).collect();
    for s in &before.scenes {
        if !new_ids.contains(s.id.as_str()) {
            out.push(format!("{}: removed", s.id));
        }
    }
    for (i, s) in after.scenes.iter().enumerate() {
        let Some((_, prev)) = old.get(s.id.as_str()) else {
            out.push(format!(
                "{}: added as scene {} ({}, text {})",
                s.id,
                i + 1,
                secs(s.duration_ms),
                s.text
                    .as_deref()
                    .map_or("none".into(), |t| format!("\"{}\"", clip(t, 30)))
            ));
            continue;
        };
        let a = serde_json::to_value(prev).unwrap_or(Value::Null);
        let b = serde_json::to_value(s).unwrap_or(Value::Null);
        let (Value::Object(a), Value::Object(b)) = (a, b) else {
            continue;
        };
        let mut changes = Vec::new();
        for (key, new_value) in &b {
            let old_value = a.get(key).unwrap_or(&Value::Null);
            if json_equal(old_value, new_value) {
                continue;
            }
            if key == "keyframes" {
                changes.push(keyframe_change(old_value, new_value));
            } else if key == "layers" {
                changes.extend(layer_changes(old_value, new_value));
            } else {
                changes.push(format!(
                    "{} {} -> {}",
                    label(key),
                    show_field(key, old_value),
                    show_field(key, new_value)
                ));
            }
        }
        if !changes.is_empty() {
            out.push(format!("{}: {}", s.id, changes.join("; ")));
        }
    }
    let old_order: Vec<&str> = before
        .scenes
        .iter()
        .map(|s| s.id.as_str())
        .filter(|id| new_ids.contains(id))
        .collect();
    let new_order: Vec<&str> = after
        .scenes
        .iter()
        .map(|s| s.id.as_str())
        .filter(|id| old.contains_key(id))
        .collect();
    if old_order != new_order {
        out.push(format!(
            "scene order: {} -> {}",
            old_order.join(", "),
            new_order.join(", ")
        ));
    }
    if before.subtitles != after.subtitles {
        let changed = before
            .subtitles
            .iter()
            .zip(&after.subtitles)
            .filter(|(a, b)| a != b)
            .count();
        if before.subtitles.len() == after.subtitles.len() {
            out.push(format!(
                "subtitles: {changed} of {} cues changed",
                after.subtitles.len()
            ));
        } else {
            out.push(format!(
                "subtitles: {} -> {} cues",
                before.subtitles.len(),
                after.subtitles.len()
            ));
        }
    }
    if before.music != after.music {
        let describe = |m: &Option<vimmer_dsl::Music>| match m {
            None => "none".to_string(),
            Some(m) => {
                let duck = if !m.duck.enabled {
                    "ducking off".to_string()
                } else {
                    format!(
                        "ducking {} dB ({}/{} ms)",
                        m.duck.depth_db, m.duck.attack_ms, m.duck.release_ms
                    )
                };
                format!(
                    "\"{}\" volume {} fades {}/{} ms, {duck}",
                    clip(&m.source, 40),
                    m.volume,
                    m.fade_in_ms,
                    m.fade_out_ms
                )
            }
        };
        out.push(format!(
            "music: {} -> {}",
            describe(&before.music),
            describe(&after.music)
        ));
    }
    let (t0, t1) = (before.total_duration_ms(), after.total_duration_ms());
    if t0 != t1 {
        out.push(format!("total length: {} -> {}", secs(t0), secs(t1)));
    }
    out
}

/// One phrase per layer change, layers matched by id: added, removed, fields changed, reordered.
fn layer_changes(old: &Value, new: &Value) -> Vec<String> {
    let list = |v: &Value| -> Vec<Map<String, Value>> {
        v.as_array()
            .map(|a| a.iter().filter_map(|l| l.as_object().cloned()).collect())
            .unwrap_or_default()
    };
    let id = |l: &Map<String, Value>| {
        l.get("id")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_string()
    };
    let (a, b) = (list(old), list(new));
    let old_ids: Vec<String> = a.iter().map(id).collect();
    let new_ids: Vec<String> = b.iter().map(id).collect();
    let mut out = Vec::new();
    for l in &a {
        if !new_ids.contains(&id(l)) {
            out.push(format!("layer `{}` removed", id(l)));
        }
    }
    for l in &b {
        let lid = id(l);
        let kind = l.get("kind").and_then(Value::as_str).unwrap_or("?");
        let Some(prev) = a.iter().find(|p| id(p) == lid) else {
            let z = l.get("z").and_then(Value::as_i64).unwrap_or(0);
            let what = match l.get("text").and_then(Value::as_str) {
                Some(t) => format!(" \"{}\"", clip(t, 30)),
                None => l
                    .get("src")
                    .and_then(Value::as_str)
                    .map_or(String::new(), |s| format!(" {}", clip(s, 40))),
            };
            out.push(format!("layer `{lid}` added ({kind}{what}, z {z})"));
            continue;
        };
        let mut keys: Vec<&String> = l.keys().chain(prev.keys()).collect();
        keys.sort();
        keys.dedup();
        let mut fields = Vec::new();
        for key in keys {
            let (x, y) = (
                prev.get(key).unwrap_or(&Value::Null),
                l.get(key).unwrap_or(&Value::Null),
            );
            if json_equal(x, y) {
                continue;
            }
            if key == "keyframes" {
                fields.push(keyframe_change(x, y));
            } else {
                fields.push(format!("{key} {} -> {}", show(x), show(y)));
            }
        }
        if !fields.is_empty() {
            out.push(format!("layer `{lid}` {}", fields.join(", ")));
        }
    }
    let kept_old: Vec<&String> = old_ids.iter().filter(|i| new_ids.contains(i)).collect();
    let kept_new: Vec<&String> = new_ids.iter().filter(|i| old_ids.contains(i)).collect();
    if kept_old != kept_new {
        let names = |v: &[&String]| v.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ");
        out.push(format!(
            "layer order {} -> {}",
            names(&kept_old),
            names(&kept_new)
        ));
    }
    out
}

fn keyframe_change(old: &Value, new: &Value) -> String {
    let n = |v: &Value| v.as_array().map_or(0, Vec::len);
    let (a, b) = (n(old), n(new));
    if a == b {
        format!("keyframes: {b} keyframes edited")
    } else {
        format!("keyframes {a} -> {b}")
    }
}
