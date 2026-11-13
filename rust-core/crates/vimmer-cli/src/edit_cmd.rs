//! `vimmer edit`: change a timeline with a natural-language instruction (Claude writes a JSON
//! Patch, `vimmer_storyboard::edit` checks it). Never overwrites the input.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::Parser;
use serde_json::{json, Value};
use vimmer_storyboard::edit_claude::{
    propose_edit, EditOptions, EditStatus, Proposal, DEFAULT_EFFORT, DEFAULT_MAX_RETRIES,
    DEFAULT_MODEL,
};
use vimmer_storyboard::generate::{FailureKind, GenerateFailure, LlmHelper, EFFORTS};

/// Edit a timeline JSON with an instruction in plain language, e.g.
/// `vimmer edit t.json "第二段节奏快一点，加一个淡入转场"`.
///
/// Claude proposes a JSON Patch; it is applied to a copy and checked (DSL validation, asset
/// paths, sanity limits) before anything is written. Writes TIMELINE.edited.json (or -o), never
/// the input. Needs ANTHROPIC_API_KEY and the helper venv
/// (rust-core/crates/vimmer-storyboard/scripts/install_llm.sh). Every call costs money; the
/// cost is printed.
#[derive(Debug, Parser)]
#[command(name = "vimmer edit", version)]
pub struct EditCli {
    /// Timeline JSON to edit (`-` reads it from stdin).
    timeline: PathBuf,
    /// What to change, in any language.
    instruction: String,
    /// The scene the instruction is about ("this scene"), by id.
    #[arg(long)]
    scene: Option<String>,
    /// Claude model ID.
    #[arg(long, default_value = DEFAULT_MODEL)]
    model: String,
    /// Reasoning effort: low, medium, high, xhigh or max.
    #[arg(long, default_value = DEFAULT_EFFORT, value_parser = clap::builder::PossibleValuesParser::new(EFFORTS))]
    effort: String,
    /// Where to write the edited timeline (default: TIMELINE.edited.json).
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Allow -o to name the input file itself.
    #[arg(long)]
    overwrite_input: bool,
    /// Only show the proposed change; write nothing.
    #[arg(long)]
    dry_run: bool,
    /// Print the result (status, explanation, patch, summary, warnings, edited timeline, usage)
    /// as one JSON object on stdout. Failures are JSON too, with a non-zero exit.
    #[arg(long)]
    json: bool,
    /// Renderer public dir: asset files are offered to Claude and new paths must stay inside
    /// it (default: remotion-renderer/public when it exists).
    #[arg(long)]
    public_dir: Option<PathBuf>,
    /// How many times a rejected patch is sent back for a correction.
    #[arg(long, default_value_t = DEFAULT_MAX_RETRIES)]
    max_retries: u32,
    /// Output token cap per API call (thinking included; default 16000).
    #[arg(long)]
    max_tokens: Option<u32>,
    /// Seconds one API call may take.
    #[arg(long, default_value_t = 300)]
    llm_timeout: u64,
    /// Python interpreter with the anthropic SDK (default: $VIMMER_LLM_PYTHON, then
    /// ~/.local/share/vimmer/llm-venv/bin/python).
    #[arg(long)]
    llm_python: Option<PathBuf>,
    /// Save the raw API replies here.
    #[arg(long)]
    save_dir: Option<PathBuf>,
}

fn failure_kind(kind: &FailureKind) -> String {
    match kind {
        FailureKind::NoApiKey => "no_api_key".into(),
        FailureKind::Helper => "helper".into(),
        FailureKind::Api(k) => k.clone(),
        FailureKind::Invalid => "invalid".into(),
        FailureKind::TimedOut => "timed_out".into(),
        FailureKind::Cancelled => "cancelled".into(),
    }
}

fn status_name(s: EditStatus) -> &'static str {
    match s {
        EditStatus::Edited => "edited",
        EditStatus::Clarify => "clarify",
        EditStatus::Refused => "refused",
    }
}

/// TIMELINE.edited.json next to the input.
pub fn default_output(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "timeline".into());
    input.with_file_name(format!("{stem}.edited.json"))
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

pub async fn run_edit(cli: EditCli) -> Result<(), Box<dyn std::error::Error>> {
    let from_stdin = cli.timeline.as_os_str() == "-";
    let raw = if from_stdin {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        s
    } else {
        tokio::fs::read_to_string(&cli.timeline)
            .await
            .map_err(|e| format!("cannot read {}: {e}", cli.timeline.display()))?
    };
    let output = match (&cli.output, cli.dry_run, from_stdin) {
        (_, true, _) => None,
        (Some(o), false, _) => Some(o.clone()),
        (None, false, false) => Some(default_output(&cli.timeline)),
        (None, false, true) => {
            return Err("reading the timeline from stdin needs -o or --dry-run".into())
        }
    };
    if let Some(o) = &output {
        if !from_stdin && !cli.overwrite_input && same_file(o, &cli.timeline) {
            return Err(format!(
                "{} is the input file; choose another -o, or pass --overwrite-input",
                o.display()
            )
            .into());
        }
    }
    let timeline: Value =
        serde_json::from_str(&raw).map_err(|e| format!("the timeline is not valid JSON: {e}"))?;

    let helper = LlmHelper::locate(cli.llm_python.as_deref());
    let helper = match helper {
        Ok(h) => h,
        Err(message) => {
            if cli.json {
                println!(
                    "{}",
                    json!({"ok": false, "error": {"kind": "helper", "message": message}})
                );
            }
            return Err(message.into());
        }
    };
    let public_dir = cli.public_dir.clone().or_else(|| {
        let d = PathBuf::from("remotion-renderer/public");
        d.is_dir().then_some(d)
    });
    let mut opts = EditOptions::new(cli.instruction.clone());
    opts.scene = cli.scene.clone();
    opts.model = cli.model.clone();
    opts.effort = cli.effort.clone();
    opts.max_tokens = cli.max_tokens;
    opts.max_retries = cli.max_retries;
    opts.timeout = Duration::from_secs(cli.llm_timeout.max(1));
    opts.public_dir = public_dir;
    opts.record_dir = cli.save_dir.clone();
    let work_dir = std::env::temp_dir().join(format!("vimmer-edit-{}", std::process::id()));

    if !cli.json {
        eprintln!(
            "asking {} (effort {}) for an edit ...",
            opts.model, opts.effort
        );
    }
    let (limits, watcher) = super::ctrl_c_limits();
    let call_timeline = timeline.clone();
    let wd = work_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        propose_edit(&helper, &call_timeline, &opts, &wd, &limits)
    })
    .await?;
    watcher.abort();
    let _ = std::fs::remove_dir_all(&work_dir);

    match result {
        Ok(p) => finish(&cli, &p, output.as_deref()),
        Err(f) => {
            report_failure(&cli, &f);
            Err(Box::new(f))
        }
    }
}

fn finish(
    cli: &EditCli,
    p: &Proposal,
    output: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    let write_to = output.filter(|_| p.status == EditStatus::Edited);
    if let Some(path) = write_to {
        let text = serde_json::to_string_pretty(&p.outcome.timeline)? + "\n";
        std::fs::write(path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    if cli.json {
        let mut v = serde_json::to_value(p)?;
        v["ok"] = json!(true);
        v["retries"] = json!(p.retries());
        v["written"] = json!(write_to.map(|p| p.display().to_string()));
        println!("{}", serde_json::to_string(&v)?);
        return Ok(());
    }
    eprintln!("{}: {}", status_name(p.status), p.explanation);
    if !p.outcome.summary.is_empty() {
        eprintln!("changes:");
        for line in &p.outcome.summary {
            eprintln!("  - {line}");
        }
    }
    for w in &p.outcome.warnings {
        eprintln!("warning: {w}");
    }
    for a in &p.attempts {
        if let Some(e) = &a.error {
            eprintln!("attempt {} rejected: {}", a.attempt, first_problem(e));
        }
        for w in &a.warnings {
            eprintln!("note: {w}");
        }
    }
    eprintln!(
        "{} attempt(s), {} retr{}; usage: {}",
        p.attempts.len(),
        p.retries(),
        if p.retries() == 1 { "y" } else { "ies" },
        p.usage
    );
    match (p.status, write_to) {
        (EditStatus::Edited, Some(path)) => eprintln!("wrote {}", path.display()),
        (EditStatus::Edited, None) => {
            eprintln!("dry run: nothing written. Patch:");
            println!("{}", serde_json::to_string_pretty(&p.patch)?);
        }
        _ => eprintln!("no change made"),
    }
    Ok(())
}

/// The first rejection line, e.g. `op 0 (add /x): ...`.
fn first_problem(e: &str) -> &str {
    e.lines()
        .map(|l| l.trim_start_matches("- "))
        .find(|l| !l.starts_with("the patch was rejected"))
        .unwrap_or(e)
}

fn report_failure(cli: &EditCli, f: &GenerateFailure) {
    if cli.json {
        let v = json!({
            "ok": false,
            "error": {"kind": failure_kind(&f.kind), "message": f.message},
            "attempts": f.attempts,
            "usage": f.usage,
        });
        println!("{v}");
        return;
    }
    for a in &f.attempts {
        if let Some(e) = &a.error {
            eprintln!("attempt {}: {}", a.attempt, first_problem(e));
        }
    }
    if !f.attempts.is_empty() {
        eprintln!("usage so far: {}", f.usage);
    }
}
