use std::{fs, path::PathBuf, process::ExitCode};

use clap::Parser;
use vimmer_media::TtsEngine;
use vimmer_storyboard::{convert_for, missing_assets, Storyboard};

/// Convert a storyboard JSON into a timeline JSON for `vimmer`.
#[derive(Debug, Parser)]
#[command(name = "vimmer-storyboard", version)]
struct Cli {
    /// Storyboard JSON.
    storyboard: PathBuf,
    /// Output timeline JSON.
    #[arg(short, long)]
    output: PathBuf,
    /// Also write the emphasis map (scene id -> words) to this file.
    #[arg(long)]
    emphasis_out: Option<PathBuf>,
    /// Renderer public dir; when given, image/music files must exist there.
    #[arg(long)]
    public_dir: Option<PathBuf>,
}

fn run(cli: Cli) -> Result<(), String> {
    let raw = fs::read_to_string(&cli.storyboard)
        .map_err(|e| format!("cannot read {}: {e}", cli.storyboard.display()))?;
    let sb: Storyboard =
        serde_json::from_str(&raw).map_err(|e| format!("invalid storyboard JSON: {e}"))?;
    if let Some(dir) = &cli.public_dir {
        let missing = missing_assets(&sb, dir);
        if !missing.is_empty() {
            let lines: Vec<String> = missing
                .iter()
                .map(|(scene, p)| match scene {
                    Some(n) => format!(
                        "scene {n}: field `image`: `{p}` not found in {}",
                        dir.display()
                    ),
                    None => format!("field `music`: `{p}` not found in {}", dir.display()),
                })
                .collect();
            return Err(lines.join("\n"));
        }
    }
    // Scenes without a duration are sized for the engine the render will use ($VIMMER_TTS).
    let result = convert_for(&sb, TtsEngine::from_env()).map_err(|e| e.to_string())?;
    for w in &result.warnings {
        eprintln!("warning: {w}");
    }
    let json = serde_json::to_string_pretty(&result.timeline).map_err(|e| e.to_string())?;
    fs::write(&cli.output, json + "\n")
        .map_err(|e| format!("cannot write {}: {e}", cli.output.display()))?;
    if let Some(path) = &cli.emphasis_out {
        let json = serde_json::to_string_pretty(&result.emphasis).map_err(|e| e.to_string())?;
        fs::write(path, json + "\n")
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    eprintln!(
        "{} scenes, {} ms -> {}",
        result.timeline.scenes.len(),
        result.timeline.total_duration_ms(),
        cli.output.display()
    );
    Ok(())
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("error: {msg}");
            ExitCode::FAILURE
        }
    }
}
