//! The soundtrack: every scene's audio placed at its global time, plus background music (ducked
//! under the scene audio), mixed once and muxed onto the joined video.
//!
//! Chunks are rendered without audio. Each chunk's own AAC track would start with about 43 ms of
//! encoder priming, which `concat -c copy` keeps, so the audio of a chunked render fell another
//! 43 ms behind the picture at every seam. Building the track once from the timeline makes a
//! chunked render sound exactly like a one-chunk render.
//!
//! Video clips with a `volume` add their sound (prepared by `video.rs`, already trimmed, at the
//! clip's speed and looped with the picture) as more placed clips, with that volume as their gain.
//! They count as scene audio for ducking. Without such clips the graph is exactly as before.
//!
//! Sound effects (`scene.sfx`) come last: each starts on the frame of its moment (scene start
//! plus its time rounded to a frame, like scene audio) and plays to its end, cut only by the end
//! of the video. They duck the music only with `duck_music`. With at least one effect the mix
//! goes through a lookahead limiter (delay-compensated, so nothing moves in time) that keeps
//! peaks under [`LIMIT`]; without effects there is no limiter and the graph is unchanged.

use std::path::Path;

use vimmer_dsl::{Music, VideoTimeline};

use crate::control::RenderContext;
use crate::duck;
use crate::run::{run_process, SchedulerConfig, SchedulerError};

/// Sample rate of the mixed soundtrack (Remotion's own output rate).
pub const SAMPLE_RATE: u64 = 48_000;
/// AAC bitrate of the soundtrack (Remotion's default for AAC).
const AUDIO_BITRATE: &str = "320k";
/// Audio frames are cut this small before the ducking `volume` filter, which evaluates its
/// expression once per frame: 128 samples (2.7 ms) makes the ramps smooth.
const DUCK_FRAME_SAMPLES: u32 = 128;

/// Peak ceiling of the limiter used when sound effects are mixed in (about -0.9 dBFS).
pub const LIMIT: f64 = 0.9;

/// Where one scene's audio clip lands in the output, in samples at [`SAMPLE_RATE`].
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub scene: usize,
    /// Path relative to the public dir.
    pub source: String,
    /// First sample of the clip on the output timeline.
    pub start: u64,
    /// The clip is cut here: the end of its scene (or of its video layer).
    pub end: u64,
    /// Linear gain; 1.0 (scene audio) adds no filter.
    pub gain: f32,
}

/// The sound of a video clip, prepared by `video.rs`: a WAV that starts on the clip's first frame.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipSound {
    pub scene: usize,
    /// WAV path relative to the public dir.
    pub source: String,
    /// The clip's frames in its scene: [from, to).
    pub from_frame: u64,
    pub to_frame: u64,
    /// The clip's `volume`.
    pub gain: f32,
    /// Samples of sound in the WAV (one pass, or the whole span when looped).
    pub samples: u64,
}

impl Placement {
    /// Samples the clip may play before it is cut.
    pub fn room(&self) -> u64 {
        self.end - self.start
    }
}

fn frame_to_sample(frame: u64, fps: u32) -> u64 {
    (frame * SAMPLE_RATE + u64::from(fps) / 2) / u64::from(fps)
}

/// Samples in `frames` frames at `fps` (rounded like every frame-to-sample conversion here).
pub fn frames_to_samples(frames: u64, fps: u32) -> u64 {
    frame_to_sample(frames, fps)
}

fn ms_to_frames(ms: u64, fps: u32) -> u64 {
    (ms * u64::from(fps) + 500) / 1000
}

/// Places every scene's audio the way the Remotion composition does: at the scene's start frame
/// plus its offset rounded to a frame, cut at the scene's last frame. Clips that would start at
/// or after the end of their scene are dropped (Remotion never plays them either).
pub fn place_scene_audio(timeline: &VideoTimeline) -> Vec<Placement> {
    let fps = timeline.fps;
    timeline
        .scenes
        .iter()
        .zip(timeline.scene_start_frames())
        .enumerate()
        .filter_map(|(scene, (s, from))| {
            let audio = s.audio.as_ref()?;
            let from = u64::from(from);
            let end = from + u64::from(s.duration_frames(fps));
            let start = from + ms_to_frames(audio.offset_ms, fps);
            (start < end).then(|| Placement {
                scene,
                source: audio.source.clone(),
                start: frame_to_sample(start, fps),
                end: frame_to_sample(end, fps),
                gain: 1.0,
            })
        })
        .collect()
}

/// Places video clip sounds on the output timeline: from the clip's first frame to its last, in
/// its scene. Returns the placements and how long each one is audible, in ms (for ducking).
pub fn place_clip_sounds(
    timeline: &VideoTimeline,
    sounds: &[ClipSound],
) -> (Vec<Placement>, Vec<u64>) {
    let fps = timeline.fps;
    let starts = timeline.scene_start_frames();
    sounds
        .iter()
        .filter(|c| c.from_frame < c.to_frame)
        .map(|c| {
            let scene_start = u64::from(starts[c.scene]);
            let start = frame_to_sample(scene_start + c.from_frame, fps);
            let end = frame_to_sample(scene_start + c.to_frame, fps);
            let audible_ms = (c.samples.min(end - start) * 1000).div_ceil(SAMPLE_RATE);
            (
                Placement {
                    scene: c.scene,
                    source: c.source.clone(),
                    start,
                    end,
                    gain: c.gain,
                },
                audible_ms,
            )
        })
        .unzip()
}

/// Places every sound effect: on the frame of its moment in its scene, playing until the end of
/// the video at most, with its volume as gain. In scene order, then list order. Each comes with
/// its `duck_music`.
pub fn place_sfx(timeline: &VideoTimeline) -> Vec<(Placement, bool)> {
    let fps = timeline.fps;
    let total = total_samples(timeline);
    let mut out = Vec::new();
    for (index, (scene, from)) in timeline
        .scenes
        .iter()
        .zip(timeline.scene_start_frames())
        .enumerate()
    {
        for fx in &scene.sfx {
            let Some(frame) = fx.frame_in(scene, fps) else {
                continue;
            };
            let start = frame_to_sample(u64::from(from) + frame, fps);
            if start >= total {
                continue;
            }
            out.push((
                Placement {
                    scene: index,
                    source: fx.src.clone(),
                    start,
                    end: total,
                    gain: fx.volume,
                },
                fx.duck_music,
            ));
        }
    }
    out
}

/// Total length of the soundtrack in samples: exactly the video's frame count.
pub fn total_samples(timeline: &VideoTimeline) -> u64 {
    frame_to_sample(u64::from(timeline.total_frames()), timeline.fps)
}

/// When each clip is actually audible, in output milliseconds: from its start to the end of the
/// clip or the cut, whichever comes first. `durations_ms[i]` is the length of `placements[i]`.
pub fn speech_intervals(placements: &[Placement], durations_ms: &[u64]) -> Vec<(f64, f64)> {
    let ms = |samples: u64| samples as f64 * 1000.0 / SAMPLE_RATE as f64;
    placements
        .iter()
        .zip(durations_ms)
        .map(|(p, &d)| (ms(p.start), ms(p.end).min(ms(p.start) + d as f64)))
        .collect()
}

/// The `-filter_complex` graph. Input 0 is the video; inputs `1..=clips.len()` are the clips in
/// order; the music, if any, is the next input (looped with `-stream_loop -1`). The output is
/// `[a]`: stereo at [`SAMPLE_RATE`], exactly `total` samples long. `None` when there is no audio.
/// Mono clips are made stereo by ffmpeg's default rematrix (−3 dB per channel), the same
/// `-ac 2` conversion Remotion applied to them, so narration is exactly as loud as before.
pub fn filter_graph(
    clips: &[Placement],
    music: Option<&Music>,
    duck_expr: Option<&str>,
    total: u64,
    limit: bool,
) -> Option<String> {
    let format = format!(
        "aresample={SAMPLE_RATE},aformat=sample_fmts=fltp:channel_layouts=stereo,asetpts=N/SR/TB"
    );
    let mut chains = Vec::new();
    let mut labels = Vec::new();
    for (k, p) in clips.iter().enumerate() {
        let gain = if p.gain == 1.0 {
            String::new()
        } else {
            format!(",volume={}", p.gain)
        };
        chains.push(format!(
            "[{input}:a]{format},atrim=end_sample={room}{gain},adelay=delays={start}S:all=1[c{k}]",
            input = k + 1,
            room = p.room(),
            start = p.start,
        ));
        labels.push(format!("[c{k}]"));
    }
    if let Some(music) = music {
        let total_s = total as f64 / SAMPLE_RATE as f64;
        let mut chain = format!(
            "[{}:a]{format},atrim=end_sample={total},volume={}",
            clips.len() + 1,
            music.volume
        );
        let fade_in = music.fade_in_ms as f64 / 1000.0;
        let fade_out = music.fade_out_ms as f64 / 1000.0;
        if fade_in > 0.0 {
            chain.push_str(&format!(",afade=t=in:st=0:d={fade_in}"));
        }
        if fade_out > 0.0 {
            chain.push_str(&format!(
                ",afade=t=out:st={}:d={fade_out}",
                (total_s - fade_out).max(0.0)
            ));
        }
        if let Some(expr) = duck_expr {
            chain.push_str(&format!(
                ",asetnsamples=n={DUCK_FRAME_SAMPLES},volume=eval=frame:volume='{expr}'"
            ));
        }
        chain.push_str("[m]");
        chains.push(chain);
        labels.push("[m]".into());
    }
    if labels.is_empty() {
        return None;
    }
    let limiter = if limit {
        format!("alimiter=limit={LIMIT}:attack=5:release=50:level=disabled:latency=1,")
    } else {
        String::new()
    };
    let tail = format!("{limiter}apad=whole_len={total},atrim=end_sample={total}[a]");
    chains.push(if labels.len() == 1 {
        format!("{}{tail}", labels[0])
    } else {
        format!(
            "{}amix=inputs={}:duration=longest:normalize=0,{tail}",
            labels.concat(),
            labels.len()
        )
    });
    Some(chains.join(";"))
}

/// Mixes the soundtrack for `timeline` and muxes it onto `video` (video only) into `out`.
/// `durations_ms[i]` is how long `placements[i]` ducks the music (its probed length; 0 for a
/// sound effect without `duck_music`). `limit` puts the limiter on the mix (when there are sound
/// effects). Returns false, writing nothing, when the timeline has no audio at all; the caller
/// then keeps the silent video.
#[allow(clippy::too_many_arguments)]
pub async fn mux_soundtrack(
    timeline: &VideoTimeline,
    placements: &[Placement],
    durations_ms: &[u64],
    limit: bool,
    video: &Path,
    out: &Path,
    cfg: &SchedulerConfig,
    ctx: &RenderContext,
) -> Result<bool, SchedulerError> {
    let total = total_samples(timeline);
    let duck_expr = timeline.music.as_ref().and_then(|music| {
        let segments =
            duck::duck_segments(&speech_intervals(placements, durations_ms), &music.duck);
        duck::volume_expr(&segments, &music.duck)
    });
    let Some(graph) = filter_graph(
        placements,
        timeline.music.as_ref(),
        duck_expr.as_deref(),
        total,
        limit,
    ) else {
        return Ok(false);
    };

    let mut cmd = tokio::process::Command::new(&cfg.ffmpeg);
    cmd.args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(video);
    for p in placements {
        cmd.arg("-i").arg(cfg.public_dir.join(&p.source));
    }
    if let Some(music) = &timeline.music {
        cmd.args(["-stream_loop", "-1", "-i"])
            .arg(cfg.public_dir.join(&music.source));
    }
    cmd.args(["-filter_complex", &graph, "-map", "0:v", "-map", "[a]"])
        .args(["-c:v", "copy", "-c:a", "aac", "-b:a", AUDIO_BITRATE])
        .args(["-ar", &SAMPLE_RATE.to_string()])
        .args(["-movflags", "+faststart"])
        .arg(out);
    let output = run_process(cmd, cfg, ctx).await?;
    if output.status.success() {
        Ok(true)
    } else {
        Err(SchedulerError::Mix(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vimmer_dsl::{AudioMapping, Duck, MotionType, Resolution, Scene};

    fn scene(id: &str, ms: u64, audio: Option<(&str, u64)>) -> Scene {
        Scene {
            sfx: vec![],
            template: None,
            id: id.into(),
            duration_ms: ms,
            text: None,
            highlights: vec![],
            image: None,
            motion: MotionType::Static,
            video: None,
            audio: audio.map(|(source, offset_ms)| AudioMapping {
                source: source.into(),
                offset_ms,
            }),
            narration: None,
            transition_in: None,
            keyframes: vec![],
            layers: vec![],
        }
    }

    fn timeline(scenes: Vec<Scene>) -> VideoTimeline {
        VideoTimeline {
            brand: None,
            fps: 30,
            resolution: Resolution {
                width: 1920,
                height: 1080,
            },
            scenes,
            subtitles: vec![],
            music: None,
        }
    }

    fn music(duck: Duck) -> Music {
        Music {
            beats: None,
            beat_sync: None,
            source: "assets/m.m4a".into(),
            volume: 0.5,
            fade_in_ms: 500,
            fade_out_ms: 1_000,
            duck,
        }
    }

    #[test]
    fn clips_land_on_their_scene_frames_in_global_time() {
        // 1010 ms = 30.3 frames -> 30; 2990 ms -> 90 (89.7 rounds up); offset 520 ms -> 16 frames.
        let t = timeline(vec![
            scene("a", 1_010, Some(("a.wav", 0))),
            scene("b", 2_990, None),
            scene("c", 3_000, Some(("c.wav", 520))),
            scene("d", 1_000, Some(("d.wav", 1_000))),
        ]);
        let placed = place_scene_audio(&t);
        assert_eq!(
            placed,
            vec![
                Placement {
                    scene: 0,
                    source: "a.wav".into(),
                    start: 0,
                    end: 30 * 1_600,
                    gain: 1.0,
                },
                Placement {
                    scene: 2,
                    source: "c.wav".into(),
                    start: (120 + 16) * 1_600,
                    end: 210 * 1_600,
                    gain: 1.0,
                },
            ],
            "an offset at the scene's end plays nothing"
        );
        assert_eq!(total_samples(&t), 240 * 1_600);
    }

    #[test]
    fn placement_does_not_depend_on_chunking() {
        // Same placement whatever the chunk size: it only reads the full timeline.
        let t = timeline(
            (0..12)
                .map(|i| scene(&format!("s{i}"), 1_234 + i * 7, Some(("x.wav", 33))))
                .collect(),
        );
        let starts = t.scene_start_frames();
        for (p, from) in place_scene_audio(&t).iter().zip(starts) {
            assert_eq!(p.start, (u64::from(from) + 1) * 1_600);
        }
    }

    #[test]
    fn speech_ends_at_the_clip_end_or_the_cut() {
        let p = |start: u64, end: u64| Placement {
            scene: 0,
            source: "x".into(),
            start,
            end,
            gain: 1.0,
        };
        let iv = speech_intervals(&[p(48_000, 144_000), p(144_000, 192_000)], &[1_500, 5_000]);
        assert_eq!(iv, vec![(1_000.0, 2_500.0), (3_000.0, 4_000.0)]);
    }

    #[test]
    fn graph_places_clips_and_ducks_music() {
        let t = timeline(vec![
            scene("a", 1_000, None),
            scene("b", 2_000, Some(("b.wav", 0))),
        ]);
        let placed = place_scene_audio(&t);
        let m = music(Duck::default());
        let g = filter_graph(&placed, Some(&m), Some("EXPR"), total_samples(&t), false).unwrap();
        assert_eq!(
            g,
            "[1:a]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,asetpts=N/SR/TB,\
             atrim=end_sample=96000,adelay=delays=48000S:all=1[c0];\
             [2:a]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,asetpts=N/SR/TB,\
             atrim=end_sample=144000,volume=0.5,afade=t=in:st=0:d=0.5,afade=t=out:st=2:d=1,\
             asetnsamples=n=128,volume=eval=frame:volume='EXPR'[m];\
             [c0][m]amix=inputs=2:duration=longest:normalize=0,apad=whole_len=144000,atrim=end_sample=144000[a]"
        );
    }

    #[test]
    fn clip_sounds_land_on_their_layer_frames_with_their_volume() {
        let t = timeline(vec![
            scene("a", 1_000, Some(("a.wav", 0))),
            scene("b", 3_000, None),
        ]);
        let sounds = [
            ClipSound {
                scene: 1,
                source: "generated/videos/k.wav".into(),
                from_frame: 15,
                to_frame: 75,
                gain: 0.5,
                samples: 48_000,
            },
            ClipSound {
                scene: 1,
                source: "generated/videos/l.wav".into(),
                from_frame: 0,
                to_frame: 30,
                gain: 1.0,
                samples: 480_000,
            },
        ];
        let (placed, ms) = place_clip_sounds(&t, &sounds);
        assert_eq!((placed[0].start, placed[0].end), (45 * 1_600, 105 * 1_600));
        assert_eq!(
            ms,
            vec![1_000, 1_000],
            "one pass of 1 s; a long sound is cut at its end"
        );
        let mut all = place_scene_audio(&t);
        all.extend(placed);
        let g = filter_graph(&all, None, None, total_samples(&t), false).unwrap();
        assert!(g.contains("[2:a]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,asetpts=N/SR/TB,atrim=end_sample=96000,volume=0.5,adelay=delays=72000S:all=1[c1]"), "{g}");
        assert!(
            g.contains("atrim=end_sample=48000,adelay=delays=0S:all=1[c0]"),
            "scene audio unchanged: {g}"
        );
        assert!(
            g.contains("atrim=end_sample=48000,adelay=delays=48000S:all=1[c2]"),
            "gain 1 adds no filter: {g}"
        );
    }

    #[test]
    fn sound_effects_land_on_their_frames_and_play_to_the_video_end() {
        let mut t = timeline(vec![
            scene("a", 1_000, Some(("a.wav", 0))),
            scene("b", 2_000, None),
        ]);
        t.scenes[1].layers = vec![serde_json::from_value(serde_json::json!({
            "id": "t", "kind": "text", "x": 0.1, "y": 0.1, "width": 0.5, "text": "hi",
            "start_ms": 520, "end_ms": 1_800, "exit": {"kind": "fade", "duration_ms": 300}
        }))
        .unwrap()];
        let fx = |json: serde_json::Value| serde_json::from_value(json).unwrap();
        t.scenes[1].sfx = vec![
            fx(serde_json::json!({"src": "pop.wav", "on": {"layer": "t", "event": "enter"}})),
            fx(
                serde_json::json!({"src": "swoosh.wav", "on": {"layer": "t", "event": "exit"}, "volume": 1.0, "duck_music": true}),
            ),
            fx(serde_json::json!({"src": "ding.wav", "at_ms": 1_990, "volume": 0.5})),
        ];
        let placed = place_sfx(&t);
        let starts: Vec<u64> = placed.iter().map(|(p, _)| p.start).collect();
        // Scene b starts at frame 30. Enter: 520 ms -> frame 16. Exit: end frame 54 minus 9
        // frames of exit -> 45. 1990 ms rounds to frame 60, the cut: it plays on the scene's
        // last frame (59) instead.
        assert_eq!(starts, vec![46 * 1_600, 75 * 1_600, 89 * 1_600]);
        assert!(placed.iter().all(|(p, _)| p.end == total_samples(&t)));
        assert_eq!(
            placed.iter().map(|(p, d)| (p.gain, *d)).collect::<Vec<_>>(),
            vec![(0.8, false), (1.0, true), (0.5, false)]
        );

        let mut all = place_scene_audio(&t);
        all.extend(placed.into_iter().map(|(p, _)| p));
        let g = filter_graph(&all, None, None, total_samples(&t), true).unwrap();
        assert!(
            g.contains("atrim=end_sample=70400,volume=0.8,adelay=delays=73600S:all=1[c1]"),
            "{g}"
        );
        assert!(
            g.ends_with("amix=inputs=4:duration=longest:normalize=0,alimiter=limit=0.9:attack=5:release=50:level=disabled:latency=1,apad=whole_len=144000,atrim=end_sample=144000[a]"),
            "{g}"
        );
        // Without effects: no limiter, the graph is as before.
        let g = filter_graph(&place_scene_audio(&t), None, None, total_samples(&t), false).unwrap();
        assert!(!g.contains("alimiter"));
    }

    #[test]
    fn graph_without_music_or_clips() {
        let t = timeline(vec![scene("a", 1_000, Some(("a.wav", 0)))]);
        assert_eq!(
            filter_graph(&place_scene_audio(&t), None, None, 48_000, false).unwrap(),
            "[1:a]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,asetpts=N/SR/TB,\
             atrim=end_sample=48000,adelay=delays=0S:all=1[c0];[c0]apad=whole_len=48000,atrim=end_sample=48000[a]"
        );
        // Music alone (no scene audio): no duck filter, exactly as loud as before.
        let m = music(Duck::default());
        let g = filter_graph(&[], Some(&m), None, 48_000, false).unwrap();
        assert!(g.starts_with("[1:a]"));
        assert!(!g.contains("volume=eval"));
        assert!(!g.contains("amix"));
        assert_eq!(filter_graph(&[], None, None, 48_000, false), None);
    }
}
