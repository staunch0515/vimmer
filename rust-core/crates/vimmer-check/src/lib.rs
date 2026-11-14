//! Rule-based quality check of a timeline, before or after rendering.
//!
//! Nothing here calls an AI model or the network. The checks are heuristics derived from the
//! renderer source (`remotion-renderer/src/components/*.tsx`, see [`layout`]) and from common
//! subtitle guidelines; each module documents its thresholds and error margin.
//!
//! Every finding names a severity, the scene or cue it is about, the time, what is wrong and a
//! suggested fix. Where the fix is mechanical, [`Suggestion::path`] is a JSON pointer into the
//! timeline and [`Suggestion::value`] the value to write there, so a tool can apply it.

pub mod assets;
pub mod audio;
pub mod layers;
pub mod layout;
pub mod motion;
pub mod rendered;
pub mod report;
pub mod text;
pub mod timing;
pub mod video;

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;
use vimmer_dsl::VideoTimeline;

/// Version tag of the JSON report. Bumped only on incompatible changes.
pub const SCHEMA: &str = "vimmer-check/1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }
}

/// What to change. `path` is a JSON pointer (RFC 6901) into the timeline, e.g.
/// `/scenes/2/duration_ms`; `value` is the new value, or absent when the change needs a person
/// (for example rewriting text). `/subtitles/3` with an array value means "replace this cue
/// with these cues".
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Suggestion {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

impl Suggestion {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            path: None,
            value: None,
        }
    }

    pub fn set(text: impl Into<String>, path: impl Into<String>, value: Value) -> Self {
        Self {
            text: text.into(),
            path: Some(path.into()),
            value: Some(value),
        }
    }

    /// A field to change, without a value that could be computed.
    pub fn field(text: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            path: Some(path.into()),
            value: None,
        }
    }
}

/// One problem found.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Finding {
    pub severity: Severity,
    /// Stable machine-readable id of the check, e.g. `text.overflow`.
    pub code: &'static str,
    /// Scene the finding is about, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scene_id: Option<String>,
    /// Index into `subtitles`, if the finding is about a cue.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cue_index: Option<usize>,
    /// Where on the timeline, in milliseconds from the video start.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_ms: Option<u64>,
    pub message: String,
    pub suggestion: Suggestion,
}

impl Finding {
    pub fn new(
        severity: Severity,
        code: &'static str,
        message: impl Into<String>,
        suggestion: Suggestion,
    ) -> Self {
        Self {
            severity,
            code,
            scene_id: None,
            cue_index: None,
            time_ms: None,
            message: message.into(),
            suggestion,
        }
    }

    pub fn scene(mut self, id: &str) -> Self {
        self.scene_id = Some(id.to_string());
        self
    }

    pub fn cue(mut self, index: usize) -> Self {
        self.cue_index = Some(index);
        self
    }

    pub fn at(mut self, ms: u64) -> Self {
        self.time_ms = Some(ms);
        self
    }
}

/// What to check and where the files are.
#[derive(Debug, Clone)]
pub struct CheckOptions {
    /// Renderer public dir that image and audio sources are relative to. `None`: asset checks
    /// report that they could not run.
    pub public_dir: Option<PathBuf>,
    /// A rendered MP4 of this timeline to check as well.
    pub rendered: Option<PathBuf>,
    /// Synthesize narration with `say` (through the render cache) and measure it, instead of
    /// estimating its length from the text.
    pub synth: bool,
    pub ffmpeg: PathBuf,
}

impl Default for CheckOptions {
    fn default() -> Self {
        Self {
            public_dir: None,
            rendered: None,
            synth: false,
            ffmpeg: PathBuf::from("ffmpeg"),
        }
    }
}

/// The result of a check.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub schema: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rendered: Option<String>,
    pub summary: Summary,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub errors: usize,
    pub warnings: usize,
    pub infos: usize,
}

impl Report {
    pub fn new(
        mut findings: Vec<Finding>,
        timeline: Option<&Path>,
        rendered: Option<&Path>,
    ) -> Self {
        // Timeline order, then most severe first; findings without a time come first.
        findings.sort_by(|a, b| {
            a.time_ms
                .cmp(&b.time_ms)
                .then(b.severity.cmp(&a.severity))
                .then(a.code.cmp(b.code))
        });
        let mut summary = Summary::default();
        for f in &findings {
            match f.severity {
                Severity::Error => summary.errors += 1,
                Severity::Warning => summary.warnings += 1,
                Severity::Info => summary.infos += 1,
            }
        }
        Self {
            schema: SCHEMA,
            timeline: timeline.map(|p| p.display().to_string()),
            rendered: rendered.map(|p| p.display().to_string()),
            summary,
            findings,
        }
    }

    /// 0 without errors, 1 with errors (or, when `strict`, with warnings).
    pub fn exit_code(&self, strict: bool) -> u8 {
        if self.summary.errors > 0 || (strict && self.summary.warnings > 0) {
            1
        } else {
            0
        }
    }
}

/// Runs every check on `timeline`.
pub fn check_timeline(timeline: &VideoTimeline, opts: &CheckOptions) -> Vec<Finding> {
    let mut findings = Vec::new();
    findings.extend(timing::validation(timeline));
    findings.extend(text::check(timeline));
    findings.extend(layout::check(timeline));
    findings.extend(layers::check(timeline));
    findings.extend(motion::check(timeline));
    findings.extend(video::check_cost(timeline));
    findings.extend(timing::check(timeline));
    let measured = if opts.synth {
        let (measured, notes) = timing::synthesize_narration(timeline, opts);
        findings.extend(notes);
        Some(measured)
    } else {
        None
    };
    findings.extend(timing::narration(timeline, measured.as_deref()));
    match &opts.public_dir {
        Some(dir) => {
            findings.extend(assets::check(timeline, dir));
            findings.extend(layers::check_assets(timeline, dir));
            findings.extend(audio::check_sources(timeline, dir, &opts.ffmpeg));
            findings.extend(video::check(timeline, dir, &opts.ffmpeg));
        }
        None => findings.push(Finding::new(
            Severity::Warning,
            "assets.public_dir_missing",
            "the renderer's public directory was not found, so images and audio files were not checked",
            Suggestion::text("pass --public-dir remotion-renderer/public (or run from the repository root)"),
        )),
    }
    if let Some(mp4) = &opts.rendered {
        findings.extend(rendered::check(
            timeline,
            mp4,
            measured.as_deref(),
            &opts.ffmpeg,
        ));
    }
    findings
}

/// JSON pointer to a scene field.
pub(crate) fn scene_path(index: usize, field: &str) -> String {
    format!("/scenes/{index}/{field}")
}

/// JSON pointer to a cue field.
pub(crate) fn cue_path(index: usize, field: &str) -> String {
    if field.is_empty() {
        format!("/subtitles/{index}")
    } else {
        format!("/subtitles/{index}/{field}")
    }
}

/// Start of each scene in milliseconds.
pub(crate) fn scene_starts_ms(timeline: &VideoTimeline) -> Vec<u64> {
    let mut acc = 0;
    timeline
        .scenes
        .iter()
        .map(|s| {
            let start = acc;
            acc += s.duration_ms;
            start
        })
        .collect()
}

pub(crate) fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

pub(crate) fn secs(ms: u64) -> String {
    format!("{:.2} s", ms as f64 / 1000.0)
}
