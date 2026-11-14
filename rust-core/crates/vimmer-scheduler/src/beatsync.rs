//! Beat sync in the render: finds the music's beats (from `music.beats`, or detected and cached
//! under `public/generated/beats/`) and snaps the scene cuts to them with
//! [`vimmer_dsl::snap_cuts`].
//!
//! It runs right after narration, when every scene has its final length, and before anything
//! that reads scene start times: word-by-word subtitles (timed from the scene start), clip
//! fitting, audio placement and chunk planning. User subtitle cues move with the start of the
//! scene they begin in.

use std::sync::Arc;

use vimmer_dsl::{beat_grid, manual_beats, snap_cuts, FitTime, SnapReport, VideoTimeline};
use vimmer_media::{detect_beats_cached, probe_video, AudioProbe, LOW_CONFIDENCE};

use crate::control::RenderContext;
use crate::narrate::TAIL_MS;
use crate::run::{SchedulerConfig, SchedulerError};

/// Snaps the cuts when `music.beat_sync` is on. Returns the report (`None` when beat sync is off
/// or there are no usable beats) and adds a warning for each cut that stayed off the beat.
pub async fn apply_beat_sync(
    timeline: &mut VideoTimeline,
    cfg: &SchedulerConfig,
    probe: Arc<dyn AudioProbe>,
    ctx: &RenderContext,
    warnings: &mut Vec<String>,
) -> Result<Option<SnapReport>, SchedulerError> {
    let Some(music) = timeline.music.clone() else {
        return Ok(None);
    };
    let Some(sync) = music.beat_sync_on().copied() else {
        return Ok(None);
    };
    if timeline.scenes.len() < 2 {
        return Ok(None);
    }
    let path = cfg.public_dir.join(music.source_path());
    let (beats, downbeat, file_ms) = match &music.beats {
        Some(grid) => {
            let probe = probe.clone();
            let p = path.clone();
            let file_ms = tokio::task::spawn_blocking(move || probe.duration_ms(&p))
                .await
                .map_err(|e| SchedulerError::Join(e.to_string()))??
                as f64;
            (manual_beats(grid, file_ms), 0, file_ms)
        }
        None => {
            let ffmpeg = cfg.ffmpeg.clone();
            let cache = cfg.public_dir.join("generated").join("beats");
            let p = path.clone();
            let analysis =
                tokio::task::spawn_blocking(move || detect_beats_cached(&ffmpeg, &p, &cache))
                    .await
                    .map_err(|e| SchedulerError::Join(e.to_string()))??;
            if analysis.confidence < LOW_CONFIDENCE {
                warnings.push(format!(
                    "beat sync skipped: the beats of `{}` are unclear (confidence {:.2}); set music.beats {{bpm, offset_ms}} by hand",
                    music.source_path(), analysis.confidence
                ));
                return Ok(None);
            }
            (analysis.beats_ms, analysis.downbeat, analysis.duration_ms)
        }
    };
    ctx.check()?;
    let total_ms = timeline.total_frames() as f64 * 1000.0 / f64::from(timeline.fps);
    let grid = beat_grid(&beats, downbeat, sync.every, file_ms, total_ms);
    let needs = scene_needs(timeline, cfg, probe).await?;
    let report = snap_cuts(timeline, &grid, &sync, &needs);
    warnings.extend(report.warnings(timeline.fps));
    Ok(Some(report))
}

/// What each scene's sound and clip need, beyond what the timeline itself says: the scene audio
/// (narration plus its tail, or `audio` from its offset to its end, as probed) and a background
/// clip that holds its last frame (`freeze`) until its natural end.
async fn scene_needs(
    timeline: &VideoTimeline,
    cfg: &SchedulerConfig,
    probe: Arc<dyn AudioProbe>,
) -> Result<Vec<(u64, String)>, SchedulerError> {
    let mut needs = Vec::with_capacity(timeline.scenes.len());
    for scene in &timeline.scenes {
        let mut need = (0, String::new());
        if let Some(audio) = &scene.audio {
            let path = cfg.public_dir.join(&audio.source);
            let probe = probe.clone();
            let len = tokio::task::spawn_blocking(move || probe.duration_ms(&path))
                .await
                .map_err(|e| SchedulerError::Join(e.to_string()))??;
            let narrated = audio.source.starts_with("generated/tts/");
            let tail = if narrated { TAIL_MS } else { 0 };
            need = (
                audio.offset_ms + len + tail,
                if narrated {
                    "its narration"
                } else {
                    "its audio"
                }
                .into(),
            );
        }
        if let Some(clip) = scene
            .video
            .as_ref()
            .filter(|c| c.fit_time == FitTime::Freeze)
        {
            let ffmpeg = cfg.ffmpeg.clone();
            let path = cfg.public_dir.join(&clip.src);
            let info = tokio::task::spawn_blocking(move || probe_video(&ffmpeg, &path))
                .await
                .map_err(|e| SchedulerError::Join(e.to_string()))??;
            let end = clip
                .trim_end_ms
                .unwrap_or(info.duration_ms)
                .min(info.duration_ms);
            let natural = (end.saturating_sub(clip.trim_start_ms) as f64 / f64::from(clip.speed))
                .round() as u64;
            // A clip longer than its scene is cut already: it needs no more than the scene has.
            let natural = natural.min(scene.duration_ms);
            if natural > need.0 {
                need = (natural, "its video clip".into());
            }
        }
        needs.push(need);
    }
    Ok(needs)
}
