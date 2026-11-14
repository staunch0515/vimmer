use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

mod beats_cmd;
mod check_cmd;
mod edit_cmd;
mod expand_cmd;

use clap::{Args, Parser, ValueEnum};
use serde_json::json;
use vimmer_dsl::VideoTimeline;
use vimmer_media::{find_chrome, run_limited, FfmpegProbe, Limits, SaySpeech, TtsEngine};
use vimmer_scheduler::{render_timeline, SchedulerConfig, DEFAULT_CHUNK_MS};
use vimmer_storyboard::generate::{
    generate, GenerateFailure, GenerateOptions, Generated, LlmHelper, DEFAULT_EFFORT,
    DEFAULT_MAX_RETRIES, DEFAULT_MODEL, EFFORTS,
};
use vimmer_storyboard::illustrate::{
    estimate_cost, illustrate, IllustrateOptions, IllustrationReport,
    DEFAULT_CONCURRENCY as DEFAULT_ILLUSTRATE_JOBS, DEFAULT_EFFORT as DEFAULT_ILLUSTRATE_EFFORT,
    DEFAULT_MODEL as DEFAULT_ILLUSTRATE_MODEL,
};
use vimmer_storyboard::{convert, Language, Orientation, Storyboard};

/// Render a timeline JSON to MP4.
///
/// To make a video from plain text with Claude, run `vimmer generate --help`.
#[derive(Debug, Parser)]
#[command(
    name = "vimmer",
    version,
    after_help = "Generate from text: vimmer generate <input.txt> --lang zh -o out.mp4 (see `vimmer generate --help`)\nAdd illustrations to a saved storyboard: vimmer illustrate <storyboard.json> (see `vimmer illustrate --help`)"
)]
struct Cli {
    /// Timeline JSON (see fixtures/sample-timeline.json).
    timeline: PathBuf,
    /// Output MP4 path.
    #[arg(short, long)]
    output: PathBuf,
    /// SubRip file whose cues replace the timeline's subtitle track.
    #[arg(long)]
    subtitles: Option<PathBuf>,
    /// Brand JSON (colors, fonts, logo; see fixtures/brand.example.json) that replaces the
    /// timeline's `brand`, so one brand file can serve many timelines.
    #[arg(long)]
    brand: Option<PathBuf>,
    #[command(flatten)]
    render: RenderArgs,
}

/// Write a storyboard from a text with Claude, check it, and render it to MP4.
///
/// Needs ANTHROPIC_API_KEY in the environment and the Python helper venv
/// (rust-core/crates/vimmer-storyboard/scripts/install_llm.sh). Every API call costs money:
/// the tokens and estimated cost are printed at the end.
#[derive(Debug, Parser)]
#[command(name = "vimmer generate", version)]
struct GenerateCli {
    /// Source text file (UTF-8).
    input: PathBuf,
    /// Language of narration and on-screen text.
    #[arg(long, value_enum)]
    lang: Lang,
    /// Output MP4 path. Required unless --no-render.
    #[arg(short, long, required_unless_present = "no_render")]
    output: Option<PathBuf>,
    /// Only generate and check the storyboard; do not render. Without --save-dir, the
    /// storyboard JSON goes to stdout.
    #[arg(long)]
    no_render: bool,
    /// Save storyboard.json, timeline.json, emphasis.json, usage.json and the raw API replies
    /// in this directory.
    #[arg(long)]
    save_dir: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t = Orient::Landscape)]
    orientation: Orient,
    /// Wanted video length in seconds (default: Claude picks one that suits the text).
    #[arg(long)]
    target_seconds: Option<u32>,
    /// Claude model ID.
    #[arg(long, default_value = DEFAULT_MODEL)]
    model: String,
    /// Reasoning effort: low, medium, high, xhigh or max.
    #[arg(long, default_value = DEFAULT_EFFORT, value_parser = clap::builder::PossibleValuesParser::new(EFFORTS))]
    effort: String,
    /// Output token cap per API call (thinking included; default 32000).
    #[arg(long)]
    max_tokens: Option<u32>,
    /// How many times a storyboard that fails validation is sent back for a correction.
    #[arg(long, default_value_t = DEFAULT_MAX_RETRIES)]
    max_retries: u32,
    /// Seconds one API call may take before it is stopped.
    #[arg(long, default_value_t = 900)]
    llm_timeout: u64,
    /// Python interpreter with the anthropic SDK (default: $VIMMER_LLM_PYTHON, then
    /// ~/.local/share/vimmer/llm-venv/bin/python).
    #[arg(long)]
    llm_python: Option<PathBuf>,
    #[command(flatten)]
    illustrate: IllustrateArgs,
    #[command(flatten)]
    render: RenderArgs,
}

/// Draw one SVG illustration per scene of a saved storyboard with Claude, and write (and
/// optionally render) a timeline that uses them.
///
/// Scenes that already have an `image` keep it. Pictures are cached in
/// remotion-renderer/public/generated/illustrations/, so running this again with the same
/// storyboard and model makes no API call. Costs money otherwise: the estimate is printed
/// before and the actual cost after.
#[derive(Debug, Parser)]
#[command(name = "vimmer illustrate", version)]
struct IllustrateCli {
    /// Storyboard JSON (e.g. storyboard.json from `vimmer generate --save-dir`).
    storyboard: PathBuf,
    /// Timeline to add the pictures to (same scenes as the storyboard, e.g. an edited
    /// timeline.json). Default: converted from the storyboard.
    #[arg(long)]
    timeline: Option<PathBuf>,
    /// Where to write the illustrated timeline. Default: timeline.illustrated.json next to the
    /// storyboard.
    #[arg(long)]
    timeline_out: Option<PathBuf>,
    /// Also render the illustrated timeline to this MP4.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Seconds one API call may take before it is stopped.
    #[arg(long, default_value_t = 600)]
    llm_timeout: u64,
    /// Python interpreter with the anthropic SDK (default: $VIMMER_LLM_PYTHON, then
    /// ~/.local/share/vimmer/llm-venv/bin/python).
    #[arg(long)]
    llm_python: Option<PathBuf>,
    /// Save the raw API replies and illustrations.json (per-scene usage) here.
    #[arg(long)]
    save_dir: Option<PathBuf>,
    #[command(flatten)]
    illustrate: IllustrateArgs,
    #[command(flatten)]
    render: RenderArgs,
}

/// Illustration options shared by `vimmer generate` and `vimmer illustrate`.
#[derive(Debug, Args)]
struct IllustrateArgs {
    /// Draw one SVG illustration per scene with Claude (costs money; the estimate is printed
    /// first). Always on for `vimmer illustrate`.
    #[arg(long)]
    illustrate: bool,
    /// Claude model for the illustrations.
    #[arg(long, default_value = DEFAULT_ILLUSTRATE_MODEL)]
    illustrate_model: String,
    /// Reasoning effort for the illustrations: low, medium, high, xhigh or max.
    #[arg(long, default_value = DEFAULT_ILLUSTRATE_EFFORT, value_parser = clap::builder::PossibleValuesParser::new(EFFORTS))]
    illustrate_effort: String,
    /// Illustrations drawn at the same time.
    #[arg(long, default_value_t = DEFAULT_ILLUSTRATE_JOBS)]
    illustrate_jobs: usize,
    /// Headless Chrome used to turn the SVGs into PNGs (default: $VIMMER_CHROME, then the
    /// browser Remotion downloads into <remotion-dir>/node_modules/.remotion/).
    #[arg(long)]
    chrome: Option<PathBuf>,
}

/// Rendering options shared by `vimmer` and `vimmer generate`.
#[derive(Debug, Args)]
struct RenderArgs {
    /// Remotion project directory.
    #[arg(long, default_value = "remotion-renderer")]
    remotion_dir: PathBuf,
    /// Scratch directory for chunks.
    #[arg(long, default_value = "target/vimmer-work")]
    work_dir: PathBuf,
    /// Target length of each render chunk, in milliseconds.
    #[arg(long, default_value_t = DEFAULT_CHUNK_MS)]
    chunk_ms: u64,
    /// Parallel image normalization jobs.
    #[arg(long, default_value_t = 4)]
    preprocess_jobs: usize,
    /// Parallel Remotion render jobs.
    #[arg(long, default_value_t = 2)]
    render_jobs: usize,
    /// Time subtitles word by word from synthesized narration, using local speech recognition
    /// (whisper.cpp). Falls back to plain subtitles with a warning when whisper-cli or the model
    /// is missing.
    #[arg(long, value_enum, default_value_t = Toggle::On)]
    word_subtitles: Toggle,
    /// Create subtitle cues from the narration text for narrated scenes that no cue covers.
    /// Off by default: only cues already in the timeline get word timing, so a timeline keeps
    /// the subtitles it always had.
    #[arg(long, value_enum, default_value_t = Toggle::Off)]
    auto_subtitles: Toggle,
    /// whisper-cli binary (default: $VIMMER_WHISPER_CLI, then `whisper-cli` on PATH).
    #[arg(long)]
    whisper_cli: Option<PathBuf>,
    /// Whisper model file (default: $VIMMER_WHISPER_MODEL, then
    /// ~/.local/share/vimmer/models/ggml-large-v3-turbo-q5_0.bin).
    #[arg(long)]
    whisper_model: Option<PathBuf>,
    /// Recognition language: a whisper code such as zh, en or ja, or `auto` to let whisper detect
    /// it (default: guessed from each scene's text).
    #[arg(long)]
    whisper_language: Option<String>,
    /// Narration engine: `say` (macOS, the default) or `kokoro` (local neural TTS, see README).
    /// Without this flag, $VIMMER_TTS decides, else `say`. When Kokoro cannot run, narration
    /// falls back to `say` with a warning.
    #[arg(long, value_enum)]
    tts: Option<Tts>,
    /// Python interpreter with Kokoro installed (default: $VIMMER_KOKORO_PYTHON, then
    /// ~/.local/share/vimmer/kokoro-venv/bin/python).
    #[arg(long)]
    kokoro_python: Option<PathBuf>,
    /// Kokoro model directory (default: $VIMMER_KOKORO_MODELS, then
    /// ~/.local/share/vimmer/models/kokoro).
    #[arg(long)]
    kokoro_models: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Tts {
    Say,
    Kokoro,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Toggle {
    On,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Lang {
    Zh,
    En,
    Ja,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Orient {
    Landscape,
    Portrait,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().collect();
    let result = if args.get(1).is_some_and(|a| a == "generate") {
        let mut rest = args;
        rest.remove(1);
        rest[0] = "vimmer generate".into();
        run_generate(GenerateCli::parse_from(rest)).await
    } else if args.get(1).is_some_and(|a| a == "edit") {
        let mut rest = args;
        rest.remove(1);
        rest[0] = "vimmer edit".into();
        edit_cmd::run_edit(edit_cmd::EditCli::parse_from(rest)).await
    } else if args.get(1).is_some_and(|a| a == "illustrate") {
        let mut rest = args;
        rest.remove(1);
        rest[0] = "vimmer illustrate".into();
        run_illustrate(IllustrateCli::parse_from(rest)).await
    } else if args.get(1).is_some_and(|a| a == "check") {
        return check_cmd::main(args);
    } else if args.get(1).is_some_and(|a| a == "expand") {
        return expand_cmd::main(args);
    } else if args.get(1).is_some_and(|a| a == "beats") {
        return beats_cmd::main(args);
    } else {
        run(Cli::parse_from(args)).await
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let raw = tokio::fs::read_to_string(&cli.timeline).await?;
    let mut timeline: VideoTimeline = serde_json::from_str(&raw)?;
    if let Some(srt_path) = &cli.subtitles {
        let srt = tokio::fs::read_to_string(srt_path).await?;
        timeline.subtitles = vimmer_dsl::parse_srt(&srt)?;
    }
    if let Some(path) = &cli.brand {
        timeline.brand = Some(expand_cmd::load_brand(path)?);
    }
    render(timeline, cli.render, cli.output).await
}

async fn render(
    timeline: VideoTimeline,
    args: RenderArgs,
    output: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut cfg = SchedulerConfig::new(args.work_dir, output, args.remotion_dir);
    cfg.chunk_ms = args.chunk_ms;
    cfg.preprocess_concurrency = args.preprocess_jobs;
    cfg.render_concurrency = args.render_jobs;
    cfg.word_subtitles.enabled = args.word_subtitles == Toggle::On;
    cfg.word_subtitles.auto_subtitles = args.auto_subtitles == Toggle::On;
    cfg.word_subtitles.whisper_cli = args.whisper_cli;
    cfg.word_subtitles.model = args.whisper_model;
    cfg.word_subtitles.language = args.whisper_language;
    cfg.tts.engine = args.tts.map(|t| match t {
        Tts::Say => TtsEngine::Say,
        Tts::Kokoro => TtsEngine::Kokoro,
    });
    cfg.tts.kokoro_python = args.kokoro_python;
    cfg.tts.kokoro_models = args.kokoro_models;

    let report = render_timeline(
        timeline,
        cfg,
        Arc::new(FfmpegProbe::default()),
        Arc::new(SaySpeech::default()),
    )
    .await?;
    for warning in &report.warnings {
        eprintln!("warning: {warning}");
    }
    for note in &report.notes {
        println!("{note}");
    }
    println!(
        "rendered {} chunk(s), normalized {} image(s) -> {}",
        report.chunks,
        report.normalized_images,
        report.output.display()
    );
    if report.encoded_videos > 0 {
        println!("encoded {} video clip file(s)", report.encoded_videos);
    }
    Ok(())
}

async fn run_generate(cli: GenerateCli) -> Result<(), Box<dyn std::error::Error>> {
    let text = tokio::fs::read_to_string(&cli.input)
        .await
        .map_err(|e| format!("cannot read {}: {e}", cli.input.display()))?;
    let helper = LlmHelper::locate(cli.llm_python.as_deref())?;
    let mut opts = GenerateOptions::new(
        text,
        match cli.lang {
            Lang::Zh => Language::Zh,
            Lang::En => Language::En,
            Lang::Ja => Language::Ja,
        },
    );
    opts.orientation = match cli.orientation {
        Orient::Landscape => Orientation::Landscape,
        Orient::Portrait => Orientation::Portrait,
    };
    opts.target_seconds = cli.target_seconds;
    opts.model = cli.model.clone();
    opts.effort = cli.effort.clone();
    opts.max_tokens = cli.max_tokens;
    opts.max_retries = cli.max_retries;
    opts.timeout = Duration::from_secs(cli.llm_timeout.max(1));
    opts.record_dir = cli.save_dir.clone();

    let (limits, watcher) = ctrl_c_limits();
    let llm_work = cli.render.work_dir.join("llm");
    eprintln!(
        "generating a {} storyboard with {} (effort {}) ...",
        lang_name(cli.lang),
        opts.model,
        opts.effort
    );
    let helper_for_illustrations = helper.clone();
    let generate_limits = limits.clone();
    let outcome =
        tokio::task::spawn_blocking(move || generate(&helper, &opts, &llm_work, &generate_limits))
            .await?;

    let mut generated = match outcome {
        Ok(g) => g,
        Err(failure) => {
            watcher.abort();
            report_failure(&failure, &cli);
            return Err(Box::new(failure));
        }
    };
    for attempt in &generated.attempts {
        for w in &attempt.warnings {
            eprintln!("note: {w}");
        }
    }
    for w in &generated.conversion.warnings {
        eprintln!("warning: {w}");
    }
    let timeline = &generated.conversion.timeline;
    eprintln!(
        "storyboard ok: \"{}\", {} scenes, {:.1} s; {} attempt(s), {} retr{}",
        generated.storyboard.title,
        timeline.scenes.len(),
        timeline.total_duration_ms() as f64 / 1000.0,
        generated.attempts.len(),
        generated.retries(),
        if generated.retries() == 1 { "y" } else { "ies" }
    );
    eprintln!("usage: {}", generated.usage);

    let illustrations = if cli.illustrate.illustrate {
        let report = run_illustrations(
            helper_for_illustrations,
            generated.storyboard.clone(),
            &cli.illustrate,
            &cli.render,
            Duration::from_secs(cli.llm_timeout.max(1)),
            cli.save_dir.clone(),
            limits.clone(),
        )
        .await?;
        report.apply(&mut generated.conversion.timeline)?;
        eprintln!(
            "total usage (storyboard + illustrations): ${:.4}",
            generated.usage.cost_usd + report.usage.cost_usd
        );
        Some(report)
    } else {
        None
    };
    watcher.abort();
    if limits.cancelled.as_ref()() {
        return Err("cancelled".into());
    }

    if let Some(dir) = &cli.save_dir {
        save_artifacts(dir, &generated, &cli, illustrations.as_ref())?;
        eprintln!("saved storyboard, timeline and usage in {}", dir.display());
    } else if cli.no_render {
        println!("{}", serde_json::to_string_pretty(&generated.storyboard)?);
    }
    if cli.no_render {
        return Ok(());
    }
    let output = cli
        .output
        .clone()
        .expect("clap requires --output without --no-render");
    render(generated.conversion.timeline, cli.render, output).await
}

/// Ctrl-C stops the helpers (they run in their own process group, so they would not see the
/// terminal's SIGINT) instead of leaving a paid API call running.
fn ctrl_c_limits() -> (Limits, tokio::task::JoinHandle<()>) {
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let watcher = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("cancelling ...");
            flag.store(true, Ordering::SeqCst);
        }
    });
    let limits = Limits {
        timeout: None,
        cancelled: Arc::new(move || cancelled.load(Ordering::SeqCst)),
    };
    (limits, watcher)
}

/// The headless Chrome for rasterizing: --chrome, $VIMMER_CHROME, Remotion's download; when
/// Remotion has not downloaded one yet, `remotion browser ensure` fetches it.
fn locate_chrome(chrome: Option<PathBuf>, remotion_dir: &Path, limits: &Limits) -> Option<PathBuf> {
    if chrome.is_some() {
        return chrome;
    }
    if let Some(p) = find_chrome(remotion_dir) {
        return Some(p);
    }
    let remotion = remotion_dir.join("node_modules/.bin/remotion");
    if !remotion.is_file() {
        return None;
    }
    eprintln!("downloading Remotion's headless Chrome (remotion browser ensure) ...");
    let mut cmd = std::process::Command::new(remotion);
    cmd.arg("browser").arg("ensure").current_dir(remotion_dir);
    let _ = run_limited(cmd, &limits.with_timeout(Duration::from_secs(900)));
    find_chrome(remotion_dir)
}

/// Prints the estimate, draws the illustrations and prints what they cost. Problems become
/// warnings; only a crashed task is an error.
async fn run_illustrations(
    helper: LlmHelper,
    storyboard: Storyboard,
    args: &IllustrateArgs,
    render: &RenderArgs,
    timeout: Duration,
    record_dir: Option<PathBuf>,
    limits: Limits,
) -> Result<IllustrationReport, Box<dyn std::error::Error>> {
    let todo = storyboard
        .scenes
        .iter()
        .filter(|s| s.image.is_none())
        .count();
    let model = args.illustrate_model.clone();
    match estimate_cost(&model, &args.illustrate_effort, todo) {
        Some((typical, worst)) => eprintln!(
            "illustrating {todo} scene(s) with {model} (effort {}): estimated ${typical:.2} (at most about ${worst:.2} if every picture needs a correction; cached pictures are free)",
            args.illustrate_effort
        ),
        None => eprintln!(
            "illustrating {todo} scene(s) with {model}: no price known for this model, cost not estimated"
        ),
    }
    let chrome_limits = limits.clone();
    let (chrome_arg, remotion_dir) = (args.chrome.clone(), render.remotion_dir.clone());
    let chrome = tokio::task::spawn_blocking(move || {
        locate_chrome(chrome_arg, &remotion_dir, &chrome_limits)
    })
    .await?;
    let Some(chrome) = chrome else {
        eprintln!(
            "warning: no headless Chrome to turn the SVGs into pictures (run `npm ci` in {}, or pass --chrome); rendering without illustrations",
            render.remotion_dir.display()
        );
        return Ok(IllustrationReport::default());
    };
    let mut opts = IllustrateOptions::new(render.remotion_dir.join("public"), chrome);
    opts.model = model;
    opts.effort = args.illustrate_effort.clone();
    opts.concurrency = args.illustrate_jobs.max(1);
    opts.timeout = timeout;
    opts.record_dir = record_dir;
    let work = render.work_dir.join("illustrate");
    let report = tokio::task::spawn_blocking(move || {
        illustrate(&helper, &storyboard, &opts, &work, &limits, &|e| {
            eprintln!("{e}")
        })
    })
    .await?;
    for w in report.all_warnings() {
        eprintln!("warning: illustration: {w}");
    }
    if let Some(reason) = &report.unavailable {
        eprintln!(
            "warning: illustrations unavailable: {reason}. The video is made without the missing pictures."
        );
    }
    let calls: u32 = report.scenes.iter().map(|s| s.calls).sum();
    eprintln!(
        "illustrations: {} of {todo} scene(s) drawn ({} cached), {} scene call(s); actual cost: {}",
        report.drawn(),
        report.scenes.iter().filter(|s| s.cached).count(),
        calls,
        report.usage
    );
    Ok(report)
}

async fn run_illustrate(cli: IllustrateCli) -> Result<(), Box<dyn std::error::Error>> {
    let raw = tokio::fs::read_to_string(&cli.storyboard)
        .await
        .map_err(|e| format!("cannot read {}: {e}", cli.storyboard.display()))?;
    let storyboard: Storyboard = serde_json::from_str(&raw)
        .map_err(|e| format!("{} is not a storyboard: {e}", cli.storyboard.display()))?;
    let mut timeline = match &cli.timeline {
        Some(path) => {
            let raw = tokio::fs::read_to_string(path)
                .await
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            serde_json::from_str::<VideoTimeline>(&raw)?
        }
        None => convert(&storyboard)?.timeline,
    };
    if timeline.scenes.len() != storyboard.scenes.len() {
        return Err(format!(
            "the timeline has {} scenes but the storyboard has {}",
            timeline.scenes.len(),
            storyboard.scenes.len()
        )
        .into());
    }
    let helper = LlmHelper::locate(cli.llm_python.as_deref())?;
    let (limits, watcher) = ctrl_c_limits();
    let report = run_illustrations(
        helper,
        storyboard,
        &cli.illustrate,
        &cli.render,
        Duration::from_secs(cli.llm_timeout.max(1)),
        cli.save_dir.clone(),
        limits.clone(),
    )
    .await?;
    watcher.abort();
    if limits.cancelled.as_ref()() {
        return Err("cancelled".into());
    }
    report.apply(&mut timeline)?;
    let out = cli.timeline_out.clone().unwrap_or_else(|| {
        cli.storyboard
            .parent()
            .unwrap_or(Path::new("."))
            .join("timeline.illustrated.json")
    });
    std::fs::write(&out, serde_json::to_string_pretty(&timeline)? + "\n")
        .map_err(|e| format!("cannot write {}: {e}", out.display()))?;
    eprintln!("wrote {}", out.display());
    if let Some(dir) = &cli.save_dir {
        std::fs::create_dir_all(dir)?;
        std::fs::write(
            dir.join("illustrations.json"),
            serde_json::to_string_pretty(&report)? + "\n",
        )?;
    }
    match cli.output {
        Some(output) => render(timeline, cli.render, output).await,
        None => Ok(()),
    }
}

fn lang_name(l: Lang) -> &'static str {
    match l {
        Lang::Zh => "zh",
        Lang::En => "en",
        Lang::Ja => "ja",
    }
}

fn report_failure(failure: &GenerateFailure, cli: &GenerateCli) {
    for a in &failure.attempts {
        if let Some(e) = &a.error {
            eprintln!(
                "attempt {}: {}",
                a.attempt,
                e.lines().next().unwrap_or_default()
            );
        }
    }
    if !failure.attempts.is_empty() {
        eprintln!("usage so far: {}", failure.usage);
    }
    if let Some(dir) = &cli.save_dir {
        let summary = usage_json(
            cli,
            &failure.attempts,
            &failure.usage,
            Some(&failure.message),
        );
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(
            dir.join("usage.json"),
            serde_json::to_string_pretty(&summary).unwrap_or_default() + "\n",
        );
    }
}

fn usage_json(
    cli: &GenerateCli,
    attempts: &[vimmer_storyboard::generate::Attempt],
    total: &vimmer_storyboard::generate::Usage,
    error: Option<&str>,
) -> serde_json::Value {
    json!({
        "model": cli.model,
        "effort": cli.effort,
        "language": lang_name(cli.lang),
        "input": cli.input,
        "attempts": attempts,
        "retries": attempts.len().saturating_sub(1),
        "total": total,
        "error": error,
    })
}

fn save_artifacts(
    dir: &Path,
    generated: &Generated,
    cli: &GenerateCli,
    illustrations: Option<&IllustrationReport>,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dir)?;
    let write = |name: &str, value: String| -> Result<(), Box<dyn std::error::Error>> {
        std::fs::write(dir.join(name), value + "\n")
            .map_err(|e| format!("cannot write {}: {e}", dir.join(name).display()).into())
    };
    write(
        "storyboard.json",
        serde_json::to_string_pretty(&generated.storyboard)?,
    )?;
    write(
        "timeline.json",
        serde_json::to_string_pretty(&generated.conversion.timeline)?,
    )?;
    write(
        "emphasis.json",
        serde_json::to_string_pretty(&generated.conversion.emphasis)?,
    )?;
    write(
        "usage.json",
        serde_json::to_string_pretty(&usage_json(
            cli,
            &generated.attempts,
            &generated.usage,
            None,
        ))?,
    )?;
    if let Some(report) = illustrations {
        write("illustrations.json", serde_json::to_string_pretty(report)?)?;
    }
    Ok(())
}
