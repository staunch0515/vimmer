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

use clap::{Args, Parser, ValueEnum};
use serde_json::json;
use vimmer_dsl::VideoTimeline;
use vimmer_media::{FfmpegProbe, Limits, SaySpeech, TtsEngine};
use vimmer_scheduler::{render_timeline, SchedulerConfig, DEFAULT_CHUNK_MS};
use vimmer_storyboard::generate::{
    generate, GenerateFailure, GenerateOptions, Generated, LlmHelper, DEFAULT_EFFORT,
    DEFAULT_MAX_RETRIES, DEFAULT_MODEL, EFFORTS,
};
use vimmer_storyboard::{Language, Orientation};

/// Render a timeline JSON to MP4.
///
/// To make a video from plain text with Claude, run `vimmer generate --help`.
#[derive(Debug, Parser)]
#[command(
    name = "vimmer",
    version,
    after_help = "Generate from text: vimmer generate <input.txt> --lang zh -o out.mp4 (see `vimmer generate --help`)"
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
    render: RenderArgs,
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
    println!(
        "rendered {} chunk(s), normalized {} image(s) -> {}",
        report.chunks,
        report.normalized_images,
        report.output.display()
    );
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

    // Ctrl-C stops the helper (it runs in its own process group, so it would not see the
    // terminal's SIGINT) instead of leaving a paid API call running.
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
    let llm_work = cli.render.work_dir.join("llm");
    eprintln!(
        "generating a {} storyboard with {} (effort {}) ...",
        lang_name(cli.lang),
        opts.model,
        opts.effort
    );
    let outcome =
        tokio::task::spawn_blocking(move || generate(&helper, &opts, &llm_work, &limits)).await?;
    watcher.abort();

    let generated = match outcome {
        Ok(g) => g,
        Err(failure) => {
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

    if let Some(dir) = &cli.save_dir {
        save_artifacts(dir, &generated, &cli)?;
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
    Ok(())
}
