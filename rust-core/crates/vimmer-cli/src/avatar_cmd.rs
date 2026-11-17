//! `vimmer avatar`: the narrator avatar (数字人讲解人).
//!
//! ```text
//! vimmer avatar env [--json]                          # can this Mac run it? is it installed?
//! vimmer avatar status   <timeline.json> [render options] [--json]
//! vimmer avatar generate <timeline.json> [render options] [--json]
//! vimmer avatar clip --photo me.png --audio speech.wav -o out.mp4
//! ```
//!
//! `status` and `generate` run the render pipeline up to the audio (narration is synthesized
//! and cached as for a render), then report or generate the clips under
//! `generated/avatar/`. Progress goes to stderr (`--json`: one JSON object per line on stdout,
//! then the result). Ctrl-C stops a generation; no half-made clip is kept.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use serde_json::json;
use vimmer_dsl::VideoTimeline;
use vimmer_media::{
    avatar_capability, AvatarCapability, AvatarEngine, AvatarEvent, AvatarSettings, FfmpegProbe,
    Limits, SaySpeech,
};
use vimmer_scheduler::{render_timeline_with, AvatarMode, CancelHandle, Progress, RenderContext};

use crate::{render_config, timeline_assets, RenderArgs};

#[derive(Debug, Parser)]
#[command(name = "vimmer avatar", version)]
struct AvatarCli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Whether this Mac can make avatar clips and whether the engine is installed.
    Env {
        #[arg(long)]
        avatar_home: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Which avatar clips of a timeline are ready, with time estimates for the others.
    Status(TimelineArgs),
    /// Generate the timeline's missing avatar clips (slow: about 11x the speech length).
    Generate(TimelineArgs),
    /// One clip from one photo and one speech file.
    Clip {
        #[arg(long)]
        photo: PathBuf,
        #[arg(long)]
        audio: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long)]
        avatar_home: Option<PathBuf>,
    },
}

#[derive(Debug, clap::Args)]
struct TimelineArgs {
    timeline: PathBuf,
    #[arg(long)]
    json: bool,
    #[command(flatten)]
    render: RenderArgs,
}

pub async fn main(mut args: Vec<OsString>) -> ExitCode {
    args.remove(1);
    args[0] = "vimmer avatar".into();
    let cli = AvatarCli::parse_from(args);
    let json_out = matches!(
        &cli.cmd,
        Cmd::Env { json: true, .. }
            | Cmd::Status(TimelineArgs { json: true, .. })
            | Cmd::Generate(TimelineArgs { json: true, .. })
    );
    match run(cli.cmd, json_out).await {
        Ok(value) => {
            if json_out {
                println!("{value}");
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

async fn run(cmd: Cmd, json_out: bool) -> Result<serde_json::Value, String> {
    match cmd {
        Cmd::Env { avatar_home, .. } => {
            let cap = avatar_capability(avatar_home.as_deref());
            let state = match &cap {
                AvatarCapability::Ready { .. } => "ready",
                AvatarCapability::NotInstalled { .. } => "not_installed",
                AvatarCapability::Unsupported { .. } => "unsupported",
            };
            if !json_out {
                println!("{}", cap.message());
            }
            Ok(json!({"ok": true, "state": state, "message": cap.message()}))
        }
        Cmd::Status(a) => timeline(a, AvatarMode::Status, json_out).await,
        Cmd::Generate(a) => timeline(a, AvatarMode::Generate, json_out).await,
        Cmd::Clip {
            photo,
            audio,
            output,
            avatar_home,
        } => {
            let engine =
                AvatarEngine::locate(avatar_home.as_deref(), None).map_err(|e| e.to_string())?;
            let cancel = Arc::new(CancelHandle::default());
            watch_ctrl_c(cancel.clone());
            let limits = Limits {
                timeout: None,
                cancelled: Arc::new(move || cancel.is_cancelled()),
            };
            let frames = tokio::task::spawn_blocking(move || {
                engine.generate(
                    &photo,
                    &audio,
                    &output,
                    &AvatarSettings::default(),
                    &limits,
                    &mut |ev| {
                        if let AvatarEvent::Frames { done, total } = ev {
                            if done % 100 == 0 || done == total {
                                eprintln!("{done}/{total} frames");
                            }
                        } else if let AvatarEvent::Stage(s) = ev {
                            eprintln!("{s}");
                        }
                    },
                )
            })
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
            Ok(json!({"ok": true, "frames": frames}))
        }
    }
}

fn watch_ctrl_c(cancel: Arc<CancelHandle>) {
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            cancel.cancel();
        }
    });
}

async fn timeline(
    a: TimelineArgs,
    mode: AvatarMode,
    json_out: bool,
) -> Result<serde_json::Value, String> {
    let raw = std::fs::read_to_string(&a.timeline)
        .map_err(|e| format!("cannot read the timeline: {e}"))?;
    let t: VideoTimeline =
        serde_json::from_str(&raw).map_err(|e| format!("the timeline is not valid: {e}"))?;
    let assets = timeline_assets(&a.timeline);
    let output = a.render.work_dir_path().join("avatar-unused.mp4");
    let mut cfg = render_config(a.render, output, assets)?;
    cfg.avatar.mode = mode;
    let cancel = Arc::new(CancelHandle::default());
    watch_ctrl_c(cancel.clone());
    let ctx = RenderContext {
        cancel,
        progress: Arc::new(move |p| {
            if let Progress::Stage { name } = p {
                if json_out {
                    println!("{}", json!({"progress": name}));
                } else {
                    eprintln!("{name}");
                }
            }
        }),
    };
    let report = render_timeline_with(
        t,
        cfg,
        Arc::new(FfmpegProbe::default()),
        Arc::new(SaySpeech::default()),
        ctx,
    )
    .await
    .map_err(|e| e.to_string())?;
    if !json_out {
        if report.avatar.is_empty() {
            println!("no avatar: the timeline has none, or no scene with an avatar has speech");
        }
        for r in &report.avatar {
            println!(
                "{} {} ({:.1} s, photo {}): {}",
                if r.ready { "ready  " } else { "missing" },
                r.scenes.join(","),
                r.duration_ms as f64 / 1000.0,
                r.photo,
                if r.ready {
                    r.clip.clone()
                } else {
                    format!("needs {}", vimmer_media::describe_estimate(r.estimate_s))
                }
            );
        }
        for w in &report.warnings {
            eprintln!("warning: {w}");
        }
    }
    let missing_s: u64 = report
        .avatar
        .iter()
        .filter(|r| !r.ready)
        .map(|r| r.estimate_s)
        .sum();
    Ok(
        json!({"ok": true, "runs": report.avatar, "missing_estimate_s": missing_s, "warnings": report.warnings}),
    )
}
