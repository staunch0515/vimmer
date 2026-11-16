//! `vimmer align-scenes`: 按旁白对齐场景. Recognizes a voice-over clip with the local whisper and
//! proposes scene lengths whose cuts land at its sentence boundaries (see
//! `vimmer_scheduler::align_scenes`). The editor's dev server calls it; the desktop app calls the
//! same library function. Nothing is written unless `--apply` names a file.

use std::ffi::OsString;
use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use serde_json::json;
use vimmer_dsl::VideoTimeline;
use vimmer_media::FfmpegProbe;
use vimmer_scheduler::{align_to_voiceover, RenderContext, RenderOptions};

/// Propose scene lengths that follow a voice-over clip.
///
/// Prints `{"ok":true,"clip":..,"changes":[{index,id,old_ms,new_ms,said_from_ms,said_to_ms}],
/// "durations":[..],"notes":[..]}` (with --json), or `{"ok":false,"error":".."}` with exit 1.
#[derive(Debug, Parser)]
#[command(name = "vimmer align-scenes", version)]
struct AlignCli {
    /// Timeline JSON, or `-` for standard input.
    timeline: PathBuf,
    /// The voice-over clip (by id). Default: the first clip.
    #[arg(long)]
    clip: Option<String>,
    /// The Remotion public dir the timeline's paths are relative to.
    #[arg(long, default_value = "remotion-renderer/public")]
    public_dir: PathBuf,
    /// A project folder whose files the timeline names (searched before the public dir).
    #[arg(long)]
    assets: Option<PathBuf>,
    /// Write the timeline with the proposed lengths to this file.
    #[arg(long)]
    apply: Option<PathBuf>,
    /// Print JSON.
    #[arg(long)]
    json: bool,
}

pub async fn main(mut args: Vec<OsString>) -> ExitCode {
    args.remove(1);
    args[0] = "vimmer align-scenes".into();
    let cli = AlignCli::parse_from(args);
    let json_out = cli.json;
    match run(cli).await {
        Ok(value) => {
            if json_out {
                println!("{value}");
            } else {
                for c in value["changes"].as_array().into_iter().flatten() {
                    println!(
                        "{}: {} ms -> {} ms",
                        c["id"].as_str().unwrap_or(""),
                        c["old_ms"],
                        c["new_ms"]
                    );
                }
                for n in value["notes"].as_array().into_iter().flatten() {
                    eprintln!("note: {}", n.as_str().unwrap_or(""));
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

async fn run(cli: AlignCli) -> Result<serde_json::Value, String> {
    let raw = if cli.timeline.as_os_str() == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).map(|_| s)
    } else {
        std::fs::read_to_string(&cli.timeline)
    }
    .map_err(|e| format!("cannot read the timeline: {e}"))?;
    let original: VideoTimeline =
        serde_json::from_str(&raw).map_err(|e| format!("the timeline is not valid: {e}"))?;
    let mut timeline = original.clone();
    let work = std::env::temp_dir().join(format!("vimmer-align-{}", std::process::id()));
    let mut cfg = RenderOptions::default().config(work.clone(), work.join("unused.mp4"), PathBuf::from("."));
    cfg.public_dir = cli.public_dir;
    if let Some(dir) = cli.assets {
        cfg.asset_dirs = vec![dir];
    }
    let mut notes = Vec::new();
    vimmer_scheduler::localize::stage_assets(&mut timeline, &cfg, &mut notes)
        .map_err(|e| format!("the voice-over file could not be read: {e}"))?;
    let result = align_to_voiceover(
        &timeline,
        cli.clip.as_deref(),
        &cfg,
        Arc::new(FfmpegProbe::default()),
        &RenderContext::silent(),
    )
    .await;
    let _ = std::fs::remove_dir_all(&work);
    let alignment = result?;
    if let Some(out) = &cli.apply {
        let mut applied = original;
        for (scene, ms) in applied.scenes.iter_mut().zip(&alignment.durations) {
            scene.duration_ms = *ms;
        }
        let text = serde_json::to_string_pretty(&applied).map_err(|e| e.to_string())? + "\n";
        std::fs::write(out, text).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
    }
    let mut value = serde_json::to_value(&alignment).map_err(|e| e.to_string())?;
    value["ok"] = json!(true);
    Ok(value)
}
