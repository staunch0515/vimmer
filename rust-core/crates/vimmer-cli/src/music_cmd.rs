//! `vimmer music list` and `vimmer music pick`: the music library
//! (`remotion-renderer/public/assets/music/library/library.json`) and which track a timeline's
//! `music.pick` gets, without rendering.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use serde_json::json;
use vimmer_dsl::{list_json, pick_json, wanted_pick, MusicLibrary, VideoTimeline};

/// The music library: list the tracks, or show which one a timeline's `music.pick` gets.
#[derive(Debug, Parser)]
#[command(name = "vimmer music", version)]
struct MusicCli {
    #[command(subcommand)]
    command: Command,
    /// Renderer public dir (default: <remotion-dir>/public, else the nearest
    /// remotion-renderer/public above the current directory).
    #[arg(long, global = true)]
    public_dir: Option<PathBuf>,
    /// Remotion project directory.
    #[arg(long, global = true, default_value = "remotion-renderer")]
    remotion_dir: PathBuf,
    /// Print JSON instead of text.
    #[arg(long, global = true)]
    json: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// List the tracks (and the moods there are).
    List {
        /// Only tracks with this mood.
        #[arg(long)]
        mood: Option<String>,
    },
    /// Show which track the timeline's `music.pick` gets and why, without rendering. With
    /// --mood (and --energy) the pick is taken from the flags instead of the timeline (the
    /// editor uses this while a choice is not saved yet).
    Pick {
        /// Timeline JSON (`-` for stdin).
        timeline: PathBuf,
        #[arg(long)]
        mood: Option<String>,
        #[arg(long)]
        energy: Option<f32>,
    },
}

pub fn main(mut args: Vec<OsString>) -> ExitCode {
    args.remove(1);
    args[0] = "vimmer music".into();
    let cli = MusicCli::parse_from(args);
    let result = match &cli.command {
        Command::List { mood } => list(&cli, mood.as_deref()),
        Command::Pick {
            timeline,
            mood,
            energy,
        } => pick(&cli, timeline, mood.as_deref(), *energy),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            if cli.json {
                println!("{}", json!({ "ok": false, "error": e }));
            } else {
                eprintln!("error: {e}");
            }
            ExitCode::from(1)
        }
    }
}

fn public_dir(cli: &MusicCli) -> PathBuf {
    if let Some(dir) = &cli.public_dir {
        return dir.clone();
    }
    let direct = cli.remotion_dir.join("public");
    if direct.is_dir() {
        return direct;
    }
    std::env::current_dir()
        .ok()
        .and_then(|cwd| {
            cwd.ancestors()
                .map(|a| a.join("remotion-renderer").join("public"))
                .find(|p| p.is_dir())
        })
        .unwrap_or(direct)
}

fn load(cli: &MusicCli) -> Result<(MusicLibrary, Vec<String>), String> {
    MusicLibrary::load(&public_dir(cli)).map_err(|e| e.to_string())
}

fn list(cli: &MusicCli, mood: Option<&str>) -> Result<(), String> {
    let (lib, missing) = load(cli)?;
    let out = list_json(&lib, &missing, mood).map_err(|e| e.to_string())?;
    if cli.json {
        println!("{out}");
        return Ok(());
    }
    println!(
        "{:<14} {:<16} {:>6} {:>5} {:>7}  moods",
        "id", "title", "energy", "bpm", "length"
    );
    for t in out["tracks"].as_array().into_iter().flatten() {
        let moods: Vec<&str> = t["moods"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| m.as_str())
            .collect();
        println!(
            "{:<14} {:<16} {:>6.2} {:>5} {:>7}  {}",
            t["id"].as_str().unwrap_or(""),
            t["title"].as_str().unwrap_or(""),
            t["energy"].as_f64().unwrap_or(0.0),
            t["bpm"].as_f64().map_or("-".into(), |b| format!("{b}")),
            t["duration_ms"]
                .as_f64()
                .map_or("-".into(), |d| format!("{:.1} s", d / 1000.0)),
            moods.join(", ")
        );
    }
    println!("moods: {}", lib.moods().join(", "));
    if !missing.is_empty() {
        println!("missing files (not offered): {}", missing.join(", "));
    }
    Ok(())
}

fn pick(
    cli: &MusicCli,
    path: &Path,
    mood: Option<&str>,
    energy: Option<f32>,
) -> Result<(), String> {
    let raw = if path == Path::new("-") {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut s).map_err(|e| e.to_string())?;
        s
    } else {
        std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?
    };
    let timeline: VideoTimeline =
        serde_json::from_str(&raw).map_err(|e| format!("cannot parse the timeline: {e}"))?;
    let wanted = wanted_pick(&timeline, mood, energy)
        .ok_or("the timeline's music has no `pick` (give --mood to try one)")?;
    let (lib, missing) = load(cli)?;
    let out = pick_json(&timeline, &lib, &missing, &wanted).map_err(|e| e.to_string())?;
    if cli.json {
        println!("{out}");
        return Ok(());
    }
    println!(
        "{} ({}): {}",
        out["track"]["id"].as_str().unwrap_or(""),
        out["track"]["source"].as_str().unwrap_or(""),
        out["reason"].as_str().unwrap_or("")
    );
    let others: Vec<&str> = out["candidates"]
        .as_array()
        .into_iter()
        .flatten()
        .skip(1)
        .filter_map(|c| c["id"].as_str())
        .collect();
    if !others.is_empty() {
        println!("other tracks with this mood: {}", others.join(", "));
    }
    if !missing.is_empty() {
        println!("missing files (not offered): {}", missing.join(", "));
    }
    Ok(())
}
