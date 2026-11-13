use std::{path::PathBuf, process::ExitCode, sync::Arc};

use clap::{Parser, ValueEnum};
use vimmer_dsl::VideoTimeline;
use vimmer_media::{FfmpegProbe, SaySpeech, TtsEngine};
use vimmer_scheduler::{render_timeline, SchedulerConfig, DEFAULT_CHUNK_MS};

/// Render a timeline JSON to MP4.
#[derive(Debug, Parser)]
#[command(name = "vimmer", version)]
struct Cli {
    /// Timeline JSON (see fixtures/sample-timeline.json).
    timeline: PathBuf,
    /// Output MP4 path.
    #[arg(short, long)]
    output: PathBuf,
    /// Remotion project directory.
    #[arg(long, default_value = "remotion-renderer")]
    remotion_dir: PathBuf,
    /// Scratch directory for chunks.
    #[arg(long, default_value = "target/vimmer-work")]
    work_dir: PathBuf,
    /// Target length of each render chunk, in milliseconds.
    #[arg(long, default_value_t = DEFAULT_CHUNK_MS)]
    chunk_ms: u64,
    /// SubRip file whose cues replace the timeline's subtitle track.
    #[arg(long)]
    subtitles: Option<PathBuf>,
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

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli).await {
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

    let mut cfg = SchedulerConfig::new(cli.work_dir, cli.output, cli.remotion_dir);
    cfg.chunk_ms = cli.chunk_ms;
    cfg.preprocess_concurrency = cli.preprocess_jobs;
    cfg.render_concurrency = cli.render_jobs;
    cfg.word_subtitles.enabled = cli.word_subtitles == Toggle::On;
    cfg.word_subtitles.auto_subtitles = cli.auto_subtitles == Toggle::On;
    cfg.word_subtitles.whisper_cli = cli.whisper_cli;
    cfg.word_subtitles.model = cli.whisper_model;
    cfg.word_subtitles.language = cli.whisper_language;
    cfg.tts.engine = cli.tts.map(|t| match t {
        Tts::Say => TtsEngine::Say,
        Tts::Kokoro => TtsEngine::Kokoro,
    });
    cfg.tts.kokoro_python = cli.kokoro_python;
    cfg.tts.kokoro_models = cli.kokoro_models;

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
