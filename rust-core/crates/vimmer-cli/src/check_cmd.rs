//! `vimmer check`: rule-based quality check of a timeline (no AI, no network).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use vimmer_check::{check_timeline, report::render_human, CheckOptions, Report, SCHEMA};
use vimmer_dsl::VideoTimeline;

/// Find quality problems in a timeline before (or after) rendering: text that overflows,
/// subtitles that are too long or too fast, safe-area and motion problems, missing or clipped
/// audio, cut-off clips and narration, missing or low-resolution images.
///
/// Exit code: 0 without errors, 1 with errors (with --strict also with warnings), 2 when the
/// timeline cannot be read or parsed.
#[derive(Debug, Parser)]
#[command(name = "vimmer check", version)]
struct CheckCli {
    /// Timeline JSON.
    timeline: PathBuf,
    /// Print the report as JSON (schema `vimmer-check/1`, see README).
    #[arg(long)]
    json: bool,
    /// Treat warnings as errors for the exit code.
    #[arg(long)]
    strict: bool,
    /// Also check this rendered MP4: streams, frame size, duration, peak, clipping, loudness.
    #[arg(long)]
    rendered: Option<PathBuf>,
    /// Synthesize narration with `say` (through the render's cache) and measure it instead of
    /// estimating its length. Slow the first time; later renders reuse the files.
    #[arg(long)]
    synth: bool,
    /// Directory that image and audio paths are relative to (default: <remotion-dir>/public,
    /// else the nearest remotion-renderer/public above the timeline or the current directory).
    #[arg(long)]
    public_dir: Option<PathBuf>,
    /// Remotion project directory.
    #[arg(long, default_value = "remotion-renderer")]
    remotion_dir: PathBuf,
    /// ffmpeg binary.
    #[arg(long, default_value = "ffmpeg")]
    ffmpeg: PathBuf,
}

pub fn main(mut args: Vec<OsString>) -> ExitCode {
    args.remove(1);
    args[0] = "vimmer check".into();
    let cli = CheckCli::parse_from(args);

    let timeline = match load(&cli.timeline) {
        Ok(t) => t,
        Err(e) => {
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({ "schema": SCHEMA, "timeline": cli.timeline.display().to_string(), "error": e })
                );
            } else {
                eprintln!("error: {e}");
            }
            return ExitCode::from(2);
        }
    };
    let opts = CheckOptions {
        public_dir: find_public_dir(&cli),
        rendered: cli.rendered.clone(),
        synth: cli.synth,
        ffmpeg: cli.ffmpeg.clone(),
    };
    let findings = check_timeline(&timeline, &opts);
    let report = Report::new(findings, Some(&cli.timeline), cli.rendered.as_deref());
    if cli.json {
        match serde_json::to_string_pretty(&report) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::from(2);
            }
        }
    } else {
        print!("{}", render_human(&report, &timeline));
    }
    ExitCode::from(report.exit_code(cli.strict))
}

fn load(path: &Path) -> Result<VideoTimeline, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    serde_json::from_str(&raw).map_err(|e| format!("cannot parse {}: {e}", path.display()))
}

fn find_public_dir(cli: &CheckCli) -> Option<PathBuf> {
    if let Some(dir) = &cli.public_dir {
        return Some(dir.clone());
    }
    let direct = cli.remotion_dir.join("public");
    if direct.is_dir() {
        return Some(direct);
    }
    let starts = [
        std::path::absolute(&cli.timeline).ok(),
        std::env::current_dir().ok(),
    ];
    starts.into_iter().flatten().find_map(|start| {
        start
            .ancestors()
            .map(|a| a.join("remotion-renderer").join("public"))
            .find(|p| p.is_dir())
    })
}
