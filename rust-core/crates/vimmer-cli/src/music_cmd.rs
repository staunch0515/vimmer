//! `vimmer music list` and `vimmer music pick`: the music library
//! (`remotion-renderer/public/assets/music/library/library.json`) and which track a timeline's
//! `music.pick` gets, without rendering.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use serde_json::{json, Value};
use vimmer_dsl::{pick_seed, LibraryTrack, MusicLibrary, VideoTimeline};

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

fn track_json(t: &LibraryTrack) -> Value {
    json!({
        "id": t.id,
        "title": t.title,
        "source": t.source(),
        "moods": t.moods,
        "energy": (f64::from(t.energy) * 100.0).round() / 100.0,
        "bpm": t.bpm,
        "duration_ms": t.duration_ms,
        "loopable": t.loopable,
        "license": t.license,
    })
}

fn list(cli: &MusicCli, mood: Option<&str>) -> Result<(), String> {
    let (lib, missing) = load(cli)?;
    let wanted = mood.map(|m| m.trim().to_lowercase());
    let tracks: Vec<&LibraryTrack> = lib
        .tracks
        .iter()
        .filter(|t| {
            wanted
                .as_ref()
                .is_none_or(|m| t.moods.iter().any(|x| x.trim().eq_ignore_ascii_case(m)))
        })
        .collect();
    if let (Some(m), true) = (&wanted, tracks.is_empty()) {
        return Err(format!(
            "no track has the mood `{m}`; available moods: {}",
            lib.moods().join(", ")
        ));
    }
    if cli.json {
        println!(
            "{}",
            json!({
                "ok": true,
                "moods": lib.moods(),
                "main_moods": lib.main_moods(),
                "tracks": tracks.iter().map(|t| track_json(t)).collect::<Vec<_>>(),
                "missing": missing,
            })
        );
        return Ok(());
    }
    println!(
        "{:<14} {:<16} {:>6} {:>5} {:>7}  moods",
        "id", "title", "energy", "bpm", "length"
    );
    for t in tracks {
        println!(
            "{:<14} {:<16} {:>6.2} {:>5} {:>7}  {}",
            t.id,
            t.title,
            t.energy,
            t.bpm.map_or("-".into(), |b| format!("{b}")),
            t.duration_ms
                .map_or("-".into(), |d| format!("{:.1} s", d as f64 / 1000.0)),
            t.moods.join(", ")
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
    let wanted = match mood {
        Some(m) => vimmer_dsl::MusicPick {
            mood: m.into(),
            energy,
            exclude: timeline
                .music
                .as_ref()
                .and_then(|m| m.pick.as_ref())
                .map(|p| p.exclude.clone())
                .unwrap_or_default(),
        },
        None => timeline
            .music
            .as_ref()
            .and_then(|m| m.pick.clone())
            .ok_or("the timeline's music has no `pick` (give --mood to try one)")?,
    };
    let (lib, missing) = load(cli)?;
    let choice = lib
        .pick(&wanted, pick_seed(&timeline))
        .map_err(|e| e.to_string())?;
    let video_ms = timeline.total_duration_ms();
    if cli.json {
        println!(
            "{}",
            json!({
                "ok": true,
                "mood": wanted.mood,
                "energy": wanted.energy,
                "track": track_json(&choice.track),
                "reason": choice.reason,
                "candidates": choice.candidates.iter().map(|(id, d)| json!({"id": id, "energy_distance": d})).collect::<Vec<_>>(),
                "video_ms": video_ms,
                "missing": missing,
            })
        );
        return Ok(());
    }
    println!(
        "{} ({}): {}",
        choice.track.id,
        choice.track.source(),
        choice.reason
    );
    let others: Vec<&str> = choice.candidates[1..]
        .iter()
        .map(|c| c.0.as_str())
        .collect();
    if !others.is_empty() {
        println!("other tracks with this mood: {}", others.join(", "));
    }
    if !missing.is_empty() {
        println!("missing files (not offered): {}", missing.join(", "));
    }
    Ok(())
}
