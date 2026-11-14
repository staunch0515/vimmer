//! Video clips (`scene.video` and `video` layers), prepared once before render.
//!
//! Each clip is probed and normalized by ffmpeg into `public/generated/videos/<key>.mp4`: exactly
//! the frames it shows (trimmed, at its speed, fitted to its scene or layer, constant fps, no
//! larger than the frame). The render copy of the timeline then points at that file with a trivial
//! clip (`trim 0..its length`, speed 1), so the renderer only maps a scene or layer frame to a clip
//! frame (freeze or loop, `motion/video.ts`), never seeks a long file at a speed. A clip that is
//! heard (`volume > 0`) also gets a WAV of its sound fitted to the same time, which `audio.rs`
//! mixes into the soundtrack and `duck.rs` ducks the music under.
//!
//! Outputs are cached by a fingerprint of the source bytes and every parameter, so a second render
//! of the same timeline reuses them.

use std::path::Path;

use vimmer_dsl::{clip_frames, FitTime, LayerContent, VideoClip, VideoTimeline};
use vimmer_media::{fingerprint, fingerprint_str, probe_video, ClipJob, VideoInfo};

use crate::audio::ClipSound;
use crate::control::{Progress, RenderContext};
use crate::run::{run_process, SchedulerConfig, SchedulerError};

/// Where normalized clips go, under the public dir.
pub const VIDEO_DIR: &str = "generated/videos";

/// One clip of the timeline: where it is and the frames it covers in its scene.
struct Occurrence {
    scene: usize,
    /// `None` for the scene background.
    layer: Option<usize>,
    /// First and end (exclusive) frame of the clip's time in its scene.
    from: u64,
    to: u64,
}

fn ms_to_frames(ms: u64, fps: u32) -> u64 {
    (ms * u64::from(fps) + 500) / 1000
}

fn frames_to_ms(frames: u64, fps: u32) -> u64 {
    (frames * 1000 + u64::from(fps) / 2) / u64::from(fps)
}

fn occurrences(timeline: &VideoTimeline) -> Vec<Occurrence> {
    let fps = timeline.fps;
    let mut out = Vec::new();
    for (si, scene) in timeline.scenes.iter().enumerate() {
        let scene_frames = u64::from(scene.duration_frames(fps));
        if scene.video.is_some() {
            out.push(Occurrence {
                scene: si,
                layer: None,
                from: 0,
                to: scene_frames,
            });
        }
        for (li, layer) in scene.layers.iter().enumerate() {
            if matches!(layer.content, LayerContent::Video(_)) {
                // Same rounding as `layerFrames` in remotion-renderer/src/motion/layers.ts.
                let from = ms_to_frames(layer.start_ms, fps);
                let to = layer
                    .end_ms
                    .map_or(scene_frames, |e| ms_to_frames(e, fps).min(scene_frames));
                out.push(Occurrence {
                    scene: si,
                    layer: Some(li),
                    from,
                    to,
                });
            }
        }
    }
    out
}

fn clip_mut<'a>(timeline: &'a mut VideoTimeline, o: &Occurrence) -> &'a mut VideoClip {
    let scene = &mut timeline.scenes[o.scene];
    match o.layer {
        None => scene.video.as_mut().expect("scene video"),
        Some(li) => match &mut scene.layers[li].content {
            LayerContent::Video(v) => v,
            _ => unreachable!("occurrence of a non-video layer"),
        },
    }
}

fn describe(timeline: &VideoTimeline, o: &Occurrence) -> String {
    let scene = &timeline.scenes[o.scene];
    match o.layer {
        None => format!("scene `{}` video", scene.id),
        Some(li) => format!("scene `{}` layer `{}`", scene.id, scene.layers[li].id),
    }
}

/// Fails early when a scene's background clip is missing (layer clips are checked with the other
/// layer files).
pub(crate) fn check_scene_videos(
    timeline: &VideoTimeline,
    cfg: &SchedulerConfig,
) -> Result<(), SchedulerError> {
    for scene in &timeline.scenes {
        if let Some(v) = &scene.video {
            let path = cfg.public_dir.join(&v.src);
            if !path.is_file() {
                return Err(SchedulerError::Video {
                    what: format!("scene `{}` video", scene.id),
                    message: format!("file not found: {}", path.display()),
                });
            }
        }
    }
    Ok(())
}

/// What one clip occurrence turns into, before anything is encoded.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipPlan {
    pub job: ClipJob,
    /// The rewritten clip the renderer gets.
    pub normalized: VideoClip,
    /// Frames of the clip's scene or layer time.
    pub span: u64,
}

/// Plans one clip from its probe: resolves the trim against the file length, fits `stretch`, and
/// cuts a clip longer than its time (it would never be shown). `Err` is a message for the user.
pub fn plan_clip(
    clip: &VideoClip,
    info: &VideoInfo,
    src: &Path,
    span: u64,
    timeline: &VideoTimeline,
    warnings: &mut Vec<String>,
    what: &str,
) -> Result<ClipPlan, String> {
    let fps = timeline.fps;
    let span = span.max(1);
    let file_ms = info.duration_ms;
    if clip.trim_start_ms >= file_ms {
        return Err(format!(
            "trim_start_ms ({}) is at or after the end of the file ({file_ms} ms)",
            clip.trim_start_ms
        ));
    }
    let end = match clip.trim_end_ms {
        Some(e) if e > file_ms => {
            warnings.push(format!(
                "{what}: trim_end_ms ({e}) is past the end of `{}` ({file_ms} ms); the clip ends with the file",
                clip.src
            ));
            file_ms
        }
        Some(e) => e,
        None => file_ms,
    };
    let source_ms = end - clip.trim_start_ms;
    let span_ms = span as f64 * 1000.0 / f64::from(fps);
    let (speed, frames) = match clip.fit_time {
        FitTime::Stretch => (source_ms as f64 / span_ms, span),
        _ => {
            let speed = f64::from(clip.speed);
            (speed, clip_frames(source_ms, speed, fps).min(span))
        }
    };
    let job = ClipJob {
        src: src.to_path_buf(),
        trim_start_ms: clip.trim_start_ms,
        source_ms,
        speed,
        fps,
        frames,
        max: timeline.resolution,
    };
    let normalized = VideoClip {
        src: String::new(), // set once the key is known
        trim_start_ms: 0,
        trim_end_ms: Some(frames_to_ms(frames, fps)),
        speed: 1.0,
        // A stretched clip is now exactly as long as its time, so holding is the same thing.
        fit_time: match clip.fit_time {
            FitTime::Stretch => FitTime::Freeze,
            f => f,
        },
        fit: clip.fit,
        volume: clip.volume,
    };
    Ok(ClipPlan {
        job,
        normalized,
        span,
    })
}

async fn run_ffmpeg(
    args: Vec<String>,
    dst: &Path,
    cfg: &SchedulerConfig,
    ctx: &RenderContext,
    what: &str,
) -> Result<(), SchedulerError> {
    // Write next to the target and rename, so an interrupted encode never looks cached.
    let tmp = dst.with_extension(format!(
        "part.{}",
        dst.extension().and_then(|e| e.to_str()).unwrap_or("bin")
    ));
    let mut args = args;
    if let Some(last) = args.last_mut() {
        *last = tmp.display().to_string();
    }
    let mut cmd = tokio::process::Command::new(&cfg.ffmpeg);
    cmd.args(&args);
    let output = run_process(cmd, cfg, ctx).await?;
    if !output.status.success() {
        let _ = tokio::fs::remove_file(&tmp).await;
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(3).collect();
        return Err(SchedulerError::Video {
            what: what.to_string(),
            message: format!(
                "ffmpeg could not decode or convert it: {}",
                tail.into_iter().rev().collect::<Vec<_>>().join(" / ")
            ),
        });
    }
    tokio::fs::rename(&tmp, dst).await?;
    Ok(())
}

/// Normalizes every clip of the timeline (see the module docs) and rewrites the clips in place to
/// the normalized files. Returns how many clip files were encoded (not cached) and the clip sounds
/// to mix.
pub(crate) async fn preprocess_videos(
    timeline: &mut VideoTimeline,
    cfg: &SchedulerConfig,
    ctx: &RenderContext,
    warnings: &mut Vec<String>,
) -> Result<(usize, Vec<ClipSound>), SchedulerError> {
    let occ = occurrences(timeline);
    if occ.is_empty() {
        return Ok((0, Vec::new()));
    }
    ctx.emit(Progress::Stage {
        name: "Preparing video clips".into(),
    });
    let out_dir = cfg.public_dir.join(VIDEO_DIR);
    tokio::fs::create_dir_all(&out_dir).await?;

    let mut probes: Vec<(String, VideoInfo, String)> = Vec::new();
    let mut encoded = 0;
    let mut sounds = Vec::new();
    for o in &occ {
        ctx.check()?;
        let what = describe(timeline, o);
        let clip = clip_mut(timeline, o).clone();
        let src = cfg.public_dir.join(&clip.src);
        let (info, src_fp) = match probes.iter().find(|(s, _, _)| *s == clip.src) {
            Some((_, info, fp)) => (info.clone(), fp.clone()),
            None => {
                let ffmpeg = cfg.ffmpeg.clone();
                let path = src.clone();
                let probed = tokio::task::spawn_blocking(move || {
                    let info = probe_video(&ffmpeg, &path)?;
                    let fp = fingerprint(&path)?;
                    Ok::<_, vimmer_media::MediaError>((info, fp))
                })
                .await
                .map_err(|e| SchedulerError::Join(e.to_string()))?;
                let (info, fp) = probed.map_err(|e| SchedulerError::Video {
                    what: what.clone(),
                    message: e.to_string(),
                })?;
                probes.push((clip.src.clone(), info.clone(), fp.clone()));
                (info, fp)
            }
        };
        let mut plan = plan_clip(&clip, &info, &src, o.to - o.from, timeline, warnings, &what)
            .map_err(|message| SchedulerError::Video {
                what: what.clone(),
                message,
            })?;
        let key = fingerprint_str(&format!("{src_fp}|{}", plan.job.key()));
        let file = format!("{key}.mp4");
        let dst = out_dir.join(&file);
        if !dst.is_file() {
            run_ffmpeg(plan.job.video_args(&dst), &dst, cfg, ctx, &what).await?;
            encoded += 1;
        }
        plan.normalized.src = format!("{VIDEO_DIR}/{file}");

        if clip.volume > 0.0 {
            if !info.has_audio {
                warnings.push(format!(
                    "{what}: `{}` has no sound, so its volume is ignored",
                    clip.src
                ));
            } else {
                let looped = clip.fit_time == FitTime::Loop;
                let total = crate::audio::frames_to_samples(plan.span, timeline.fps);
                let sound_key =
                    fingerprint_str(&format!("{src_fp}|{}|a1|{looped}|{total}", plan.job.key()));
                let wav = out_dir.join(format!("{sound_key}.wav"));
                if !wav.is_file() {
                    run_ffmpeg(
                        plan.job.audio_args(looped, total, &wav),
                        &wav,
                        cfg,
                        ctx,
                        &what,
                    )
                    .await?;
                    encoded += 1;
                }
                let pass = plan.job.clip_samples();
                sounds.push(ClipSound {
                    scene: o.scene,
                    source: format!("{VIDEO_DIR}/{sound_key}.wav"),
                    from_frame: o.from,
                    to_frame: o.to,
                    gain: clip.volume,
                    samples: if looped { total } else { pass.min(total) },
                });
            }
        }
        *clip_mut(timeline, o) = plan.normalized;
    }
    Ok((encoded, sounds))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timeline() -> VideoTimeline {
        serde_json::from_str(
            r#"{"fps":30,"resolution":{"width":1920,"height":1080},"subtitles":[],"scenes":[
              {"id":"a","duration_ms":3000,"video":{"src":"c/a.mp4","trim_start_ms":500,"speed":2}},
              {"id":"b","duration_ms":2000,"text":"x","layers":[
                {"id":"pip","kind":"video","x":0,"y":0,"width":0.3,"height":0.3,"start_ms":500,"end_ms":1700,"src":"c/b.webm","fit_time":"loop"},
                {"id":"t","kind":"text","x":0,"y":0,"width":0.3,"text":"hi"}]}]}"#,
        )
        .unwrap()
    }

    fn info(ms: u64) -> VideoInfo {
        VideoInfo {
            duration_ms: ms,
            width: 640,
            height: 360,
            fps: Some(30.0),
            has_audio: false,
        }
    }

    #[test]
    fn finds_backgrounds_and_video_layers_with_their_frames() {
        let t = timeline();
        let occ = occurrences(&t);
        assert_eq!(occ.len(), 2);
        assert_eq!(
            (occ[0].scene, occ[0].layer, occ[0].from, occ[0].to),
            (0, None, 0, 90)
        );
        assert_eq!(
            (occ[1].scene, occ[1].layer, occ[1].from, occ[1].to),
            (1, Some(0), 15, 51)
        );
    }

    #[test]
    fn plans_trim_speed_and_cuts_at_the_span() {
        let t = timeline();
        let clip = t.scenes[0].video.clone().unwrap();
        let mut w = Vec::new();
        // 6 s file, from 0.5 s at 2x: 5.5 s -> 82.5 frames -> 83, more than the 90-frame scene? no.
        let p = plan_clip(
            &clip,
            &info(6_000),
            Path::new("/p/a.mp4"),
            90,
            &t,
            &mut w,
            "x",
        )
        .unwrap();
        assert_eq!(p.job.source_ms, 5_500);
        assert_eq!(p.job.frames, 83);
        assert_eq!(p.normalized.trim_end_ms, Some(2_767));
        assert_eq!(p.normalized.speed, 1.0);
        assert_eq!(
            vimmer_dsl::clip_frames(2_767, 1.0, 30),
            83,
            "the renderer gets the same frame count back"
        );
        // A 20 s file is cut to the scene: never more frames than shown.
        let p = plan_clip(
            &clip,
            &info(20_000),
            Path::new("/p/a.mp4"),
            90,
            &t,
            &mut w,
            "x",
        )
        .unwrap();
        assert_eq!(p.job.frames, 90);
        assert!(w.is_empty());
    }

    #[test]
    fn stretch_fits_exactly_and_bad_trims_are_reported() {
        let t = timeline();
        let mut clip = VideoClip::new("c/a.mp4");
        clip.fit_time = FitTime::Stretch;
        clip.trim_end_ms = Some(9_000);
        let mut w = Vec::new();
        let p = plan_clip(&clip, &info(4_000), Path::new("/p"), 60, &t, &mut w, "s").unwrap();
        assert_eq!(p.job.frames, 60);
        assert!((p.job.speed - 2.0).abs() < 1e-9, "4 s into 2 s");
        assert_eq!(p.normalized.fit_time, FitTime::Freeze);
        assert_eq!(w.len(), 1, "trim_end past the file warns: {w:?}");
        clip.trim_start_ms = 4_000;
        let err = plan_clip(&clip, &info(4_000), Path::new("/p"), 60, &t, &mut w, "s");
        assert!(err.unwrap_err().contains("at or after the end"));
    }
}
