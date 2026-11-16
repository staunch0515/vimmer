//! `vimmer tts`: synthesizes the narration of a timeline's scenes exactly as a render would
//! (same engine choice, voices, cache file names) without rendering. The editor's dev server
//! calls it for "Listen" before a render; the desktop app calls the same library function.

use std::ffi::OsString;
use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use serde_json::json;
use vimmer_dsl::VideoTimeline;
use vimmer_media::{FfmpegProbe, SaySpeech, TtsEngine};
use vimmer_scheduler::{preview_narration, RenderContext, RenderOptions};

/// Synthesize the narration of a timeline (or of some scenes) into the render's TTS cache.
///
/// Prints `{"ok":true,"engine":"say"|"kokoro","clips":[{scene_id,index,text,voice,source,
/// speech_ms,scene_ms,render_ms,cached}],"warnings":[..]}` (with --json), or
/// `{"ok":false,"error":".."}` with exit code 1. `source` is relative to the public dir.
#[derive(Debug, Parser)]
#[command(name = "vimmer tts", version)]
struct TtsCli {
    /// Timeline JSON, or `-` for standard input.
    timeline: PathBuf,
    /// Only these scenes (by id); repeat for several. Default: every narrated scene.
    #[arg(long = "scene")]
    scenes: Vec<String>,
    /// The Remotion public dir; clips go to <public-dir>/generated/tts, as in a render.
    #[arg(long, default_value = "remotion-renderer/public")]
    public_dir: PathBuf,
    /// Narration engine: `say` or `kokoro`. Default: $VIMMER_TTS, else `say` (as a render).
    #[arg(long)]
    tts: Option<String>,
    /// Python interpreter with Kokoro installed (default as for a render).
    #[arg(long)]
    kokoro_python: Option<PathBuf>,
    /// Kokoro model directory (default as for a render).
    #[arg(long)]
    kokoro_models: Option<PathBuf>,
    /// Print JSON.
    #[arg(long)]
    json: bool,
}

pub async fn main(mut args: Vec<OsString>) -> ExitCode {
    args.remove(1);
    args[0] = "vimmer tts".into();
    let cli = TtsCli::parse_from(args);
    let json_out = cli.json;
    match run(cli).await {
        Ok(value) => {
            if json_out {
                println!("{value}");
            } else {
                for clip in value["clips"].as_array().into_iter().flatten() {
                    println!(
                        "{}: {} ms of speech ({}), scene {} ms → {} ms",
                        clip["scene_id"].as_str().unwrap_or(""),
                        clip["speech_ms"],
                        clip["source"].as_str().unwrap_or(""),
                        clip["scene_ms"],
                        clip["render_ms"]
                    );
                }
                for w in value["warnings"].as_array().into_iter().flatten() {
                    eprintln!("warning: {}", w.as_str().unwrap_or(""));
                }
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            if json_out {
                println!("{}", json!({ "ok": false, "error": error }));
            } else {
                eprintln!("error: {error}");
            }
            ExitCode::from(1)
        }
    }
}

async fn run(cli: TtsCli) -> Result<serde_json::Value, String> {
    let raw = if cli.timeline.as_os_str() == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).map(|_| s)
    } else {
        std::fs::read_to_string(&cli.timeline)
    }
    .map_err(|e| format!("cannot read the timeline: {e}"))?;
    let mut timeline: VideoTimeline =
        serde_json::from_str(&raw).map_err(|e| format!("the timeline is not valid: {e}"))?;
    // A render narrates the expanded timeline; template params errors are the render's to report.
    let _ = vimmer_dsl::expand_templates(&mut timeline);

    let tts = match cli.tts.as_deref() {
        Some(name) => Some(
            TtsEngine::parse(name)
                .ok_or_else(|| format!("unknown narration engine `{name}` (say or kokoro)"))?,
        ),
        None => None,
    };
    let work = std::env::temp_dir().join(format!("vimmer-tts-{}", std::process::id()));
    let opts = RenderOptions {
        tts,
        kokoro_python: cli.kokoro_python,
        kokoro_models: cli.kokoro_models,
        ..RenderOptions::default()
    };
    let mut cfg = opts.config(work.clone(), work.join("unused.mp4"), PathBuf::from("."));
    cfg.public_dir = cli.public_dir;
    let only = (!cli.scenes.is_empty()).then_some(cli.scenes.as_slice());
    let result = preview_narration(
        &timeline,
        only,
        &cfg,
        Arc::new(SaySpeech::default()),
        Arc::new(FfmpegProbe::default()),
        &RenderContext::silent(),
    )
    .await;
    let _ = std::fs::remove_dir_all(&work);
    let preview = result?;
    let mut value = serde_json::to_value(&preview).map_err(|e| e.to_string())?;
    value["ok"] = json!(true);
    Ok(value)
}
