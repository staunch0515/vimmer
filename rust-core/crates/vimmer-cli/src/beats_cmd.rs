//! `vimmer beats`: detects the beats of a music file and prints tempo, confidence and the first
//! beats (or everything as JSON, for the editor's "Detect beats" button).

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use serde_json::json;
use vimmer_media::{detect_beats, detect_beats_cached, LOW_CONFIDENCE};

/// Detect the beats of an audio file: tempo (BPM), a confidence from 0 to 1, and beat times.
///
/// With --json prints `{"ok":true,"bpm":..,"confidence":..,"low_confidence":..,"downbeat":..,
/// "duration_ms":..,"beats_ms":[..]}`, or `{"ok":false,"error":".."}` with exit code 1.
#[derive(Debug, Parser)]
#[command(name = "vimmer beats", version)]
struct BeatsCli {
    /// Audio file (anything ffmpeg can decode).
    audio: PathBuf,
    /// Print JSON instead of text.
    #[arg(long)]
    json: bool,
    /// Cache the result here, keyed by the file's contents (a render uses
    /// remotion-renderer/public/generated/beats).
    #[arg(long)]
    cache_dir: Option<PathBuf>,
    /// How many beats the text output lists.
    #[arg(long, default_value_t = 8)]
    first: usize,
    /// ffmpeg binary.
    #[arg(long, default_value = "ffmpeg")]
    ffmpeg: PathBuf,
}

pub fn main(mut args: Vec<OsString>) -> ExitCode {
    args.remove(1);
    args[0] = "vimmer beats".into();
    let cli = BeatsCli::parse_from(args);
    let result = match &cli.cache_dir {
        Some(dir) => detect_beats_cached(&cli.ffmpeg, &cli.audio, dir),
        None => detect_beats(&cli.ffmpeg, &cli.audio),
    };
    let a = match result {
        Ok(a) => a,
        Err(e) => {
            if cli.json {
                println!("{}", json!({ "ok": false, "error": e.to_string() }));
            } else {
                eprintln!("error: {e}");
            }
            return ExitCode::from(1);
        }
    };
    let low = a.confidence < LOW_CONFIDENCE;
    if cli.json {
        let beats: Vec<f64> = a
            .beats_ms
            .iter()
            .map(|b| (b * 10.0).round() / 10.0)
            .collect();
        println!(
            "{}",
            json!({
                "ok": true,
                "bpm": (a.bpm * 100.0).round() / 100.0,
                "confidence": (a.confidence * 1000.0).round() / 1000.0,
                "low_confidence": low,
                "downbeat": a.downbeat,
                "duration_ms": a.duration_ms.round(),
                "beats_ms": beats,
            })
        );
        return ExitCode::SUCCESS;
    }
    println!("tempo: {:.2} BPM", a.bpm);
    println!(
        "confidence: {:.2}{}",
        a.confidence,
        if low {
            " (low: the beat is unclear; set music.beats by hand for beat sync)"
        } else {
            ""
        }
    );
    println!("beats: {} in {:.0} ms", a.beats_ms.len(), a.duration_ms);
    let first: Vec<String> = a
        .beats_ms
        .iter()
        .take(cli.first)
        .enumerate()
        .map(|(i, b)| {
            let mark = if i >= a.downbeat && (i - a.downbeat) % 4 == 0 {
                "|"
            } else {
                ""
            };
            format!("{mark}{b:.0}")
        })
        .collect();
    println!("first beats (ms, | = bar start): {}", first.join(" "));
    ExitCode::SUCCESS
}
