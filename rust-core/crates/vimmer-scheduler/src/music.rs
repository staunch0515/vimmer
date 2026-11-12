//! Background music: loops a track under the whole joined video, with optional fades.
//! Runs once after the segments are joined, so music never restarts at a chunk boundary.

use std::path::Path;

use vimmer_dsl::Music;
use vimmer_media::has_audio_stream;

use crate::control::RenderContext;
use crate::run::{run_process, SchedulerConfig, SchedulerError};

/// Writes `out` = `video` with the music mixed under its audio (or as its audio, when the video is silent).
/// The result is cut to `total_ms`.
pub async fn mix_music(
    video: &Path,
    out: &Path,
    music: &Music,
    total_ms: u64,
    cfg: &SchedulerConfig,
    ctx: &RenderContext,
) -> Result<(), SchedulerError> {
    let track = cfg.public_dir.join(&music.source);
    let ffmpeg = cfg.ffmpeg.clone();
    let video_owned = video.to_path_buf();
    let has_audio = tokio::task::spawn_blocking(move || has_audio_stream(&ffmpeg, &video_owned))
        .await
        .map_err(|e| SchedulerError::Join(e.to_string()))?;

    let total = total_ms as f64 / 1000.0;
    let fade_in = music.fade_in_ms as f64 / 1000.0;
    let fade_out = music.fade_out_ms as f64 / 1000.0;

    let mut chain = format!("[1:a]volume={}", music.volume);
    if fade_in > 0.0 {
        chain.push_str(&format!(",afade=t=in:st=0:d={fade_in}"));
    }
    if fade_out > 0.0 {
        chain.push_str(&format!(
            ",afade=t=out:st={}:d={fade_out}",
            (total - fade_out).max(0.0)
        ));
    }
    chain.push_str("[m]");
    let filter = if has_audio {
        format!("{chain};[0:a][m]amix=inputs=2:duration=first:normalize=0[a]")
    } else {
        format!("{chain};[m]anull[a]")
    };

    let mut cmd = tokio::process::Command::new(&cfg.ffmpeg);
    cmd.args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(video)
        .args(["-stream_loop", "-1", "-i"])
        .arg(&track)
        .args(["-filter_complex", &filter, "-map", "0:v", "-map", "[a]"])
        .args(["-c:v", "copy", "-c:a", "aac", "-b:a", "192k"])
        .args(["-t", &format!("{total}")])
        .arg(out);
    let output = run_process(cmd, cfg, ctx).await?;
    if output.status.success() {
        Ok(())
    } else {
        Err(SchedulerError::Mix(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
    }
}
