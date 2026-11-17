//! The narrator avatar (数字人讲解人) in the render pipeline.
//!
//! After narration and audio placement the scene lengths and the speech are final. Here:
//! 1. **Runs.** Each scene shows the avatar when it resolves to one (see `vimmer_dsl::avatar`) and
//!    someone speaks in it for at least [`MIN_SPEECH_MS`]: its narration or recorded scene audio,
//!    or a voice-over clip. Video-clip sound does not count. Consecutive such scenes with the same
//!    photo form one run, and each run gets one clip, so the head moves continuously across cuts.
//! 2. **Cache key** = engine version + generation settings + photo content + every speech part
//!    (file content, where it lands, trim, gain) + run length. The clip is
//!    `generated/avatar/<key>.mp4`, the speech `generated/avatar/<key>.speech.wav` (16 kHz mono,
//!    exactly as long as the run, built the same way the soundtrack mix places the clips).
//! 3. **Attach.** Each scene of a ready run gets `avatar_clip` (src + the clip frame on its first
//!    frame + its placement). Remotion draws it frame-locked, muted, so chunks match a full render.
//!
//! The render never generates clips: they take ~11x real time (`vimmer avatar generate` does it,
//! in the background in the app). A missing clip: [`AvatarMode::Show`] (default) renders that run
//! without the inset and warns with an estimate; [`AvatarMode::Require`] stops before rendering.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use vimmer_dsl::{AvatarClip, VideoTimeline};
use vimmer_media::{
    avatar_estimate_seconds, describe_estimate, fingerprint, fingerprint_str, run_limited,
    AvatarEngine, AvatarEvent, AvatarSettings, Limits, MediaError,
};

use crate::audio::{frames_to_samples, Placement, SAMPLE_RATE};
use crate::control::{Progress, RenderContext};
use crate::run::{SchedulerConfig, SchedulerError};

pub const AVATAR_DIR: &str = "generated/avatar";
/// Clip status by plan signature, read by `vimmer check`.
pub const STATUS_FILE: &str = "generated/avatar/status.json";
/// A scene needs at least this much speech to show the avatar.
pub const MIN_SPEECH_MS: u64 = 300;
const SPEECH_RATE: u32 = 16_000;

/// What the render does about avatar clips.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AvatarMode {
    /// Show ready clips; a run whose clip is missing is drawn without the inset (with a warning).
    #[default]
    Show,
    /// Stop before rendering when a clip is missing.
    Require,
    /// Never draw the avatar.
    Off,
    /// Generate the missing clips, then stop (no video is rendered).
    Generate,
    /// Report which clips are ready, then stop (nothing is generated or rendered).
    Status,
}

impl AvatarMode {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "show" | "auto" => AvatarMode::Show,
            "require" => AvatarMode::Require,
            "off" => AvatarMode::Off,
            _ => return None,
        })
    }
    /// Modes that stop before the video is rendered.
    pub fn stops_early(self) -> bool {
        matches!(self, AvatarMode::Generate | AvatarMode::Status)
    }
}

#[derive(Debug, Clone, Default)]
pub struct AvatarConfig {
    pub mode: AvatarMode,
    /// The engine install dir (default `$VIMMER_AVATAR_HOME` or ~/.local/share/vimmer/avatar).
    pub home: Option<PathBuf>,
    pub settings: AvatarSettings,
    /// Test hook: the engine to use instead of locating the installed one.
    pub engine: Option<AvatarEngine>,
}

/// One part of a run's speech, in samples at [`SAMPLE_RATE`].
#[derive(Debug, Clone, PartialEq)]
pub struct SpeechPart {
    pub source: String,
    /// Samples skipped at the start of the file.
    pub file_offset: u64,
    /// Where it starts, from the run start.
    pub delay: u64,
    pub len: u64,
    pub gain: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AvatarRun {
    pub scenes: Range<usize>,
    pub photo: String,
    pub start_frame: u32,
    pub frame_count: u32,
    pub speech: Vec<SpeechPart>,
    pub key: String,
}

impl AvatarRun {
    pub fn clip(&self) -> String {
        format!("{AVATAR_DIR}/{}.mp4", self.key)
    }
    pub fn speech_wav(&self) -> String {
        format!("{AVATAR_DIR}/{}.speech.wav", self.key)
    }
    pub fn duration_ms(&self, fps: u32) -> u64 {
        u64::from(self.frame_count) * 1000 / u64::from(fps.max(1))
    }
}

/// A run as reported to the CLI, the app and `vimmer check`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct RunStatus {
    pub scenes: Vec<String>,
    pub photo: String,
    pub clip: String,
    pub ready: bool,
    pub duration_ms: u64,
    pub estimate_s: u64,
}

/// The runs of `timeline` given the speech placements (scene audio + voice-over).
/// `fp(path)` fingerprints a file relative to the public dir.
pub fn plan_runs(
    timeline: &VideoTimeline,
    speech: &[Placement],
    settings_key: &str,
    fp: &dyn Fn(&str) -> String,
) -> Vec<AvatarRun> {
    let fps = timeline.fps;
    let starts = timeline.scene_start_frames();
    let min = MIN_SPEECH_MS * SAMPLE_RATE / 1000;
    let span = |i: usize| {
        let s = starts[i];
        let e = s + timeline.scenes[i].duration_frames(fps);
        (s, e)
    };
    let mut runs: Vec<AvatarRun> = vec![];
    for (i, _) in timeline.scenes.iter().enumerate() {
        let Some(av) = timeline.scene_avatar(i) else {
            continue;
        };
        let (sf, ef) = span(i);
        let (ss, es) = (
            frames_to_samples(sf.into(), fps),
            frames_to_samples(ef.into(), fps),
        );
        let spoken: u64 = speech
            .iter()
            .map(|p| p.end.min(es).saturating_sub(p.start.max(ss)))
            .sum();
        if spoken < min {
            continue;
        }
        match runs.last_mut() {
            Some(r) if r.scenes.end == i && r.photo == av.photo => {
                r.scenes.end = i + 1;
                r.frame_count = ef - r.start_frame;
            }
            _ => runs.push(AvatarRun {
                scenes: i..i + 1,
                photo: av.photo,
                start_frame: sf,
                frame_count: ef - sf,
                speech: vec![],
                key: String::new(),
            }),
        }
    }
    for r in &mut runs {
        let rs = frames_to_samples(r.start_frame.into(), fps);
        let re = frames_to_samples((r.start_frame + r.frame_count).into(), fps);
        for p in speech {
            let (s, e) = (p.start.max(rs), p.end.min(re));
            if e > s {
                r.speech.push(SpeechPart {
                    source: p.source.clone(),
                    file_offset: p.skip + (s - p.start),
                    delay: s - rs,
                    len: e - s,
                    gain: p.gain,
                });
            }
        }
        let mut d = format!("{settings_key}|photo={}|len={}", fp(&r.photo), re - rs);
        for part in &r.speech {
            d.push_str(&format!(
                "|{}:{}:{}:{}:{}",
                fp(&part.source),
                part.file_offset,
                part.delay,
                part.len,
                part.gain
            ));
        }
        r.key = fingerprint_str(&d);
    }
    runs
}

/// ffmpeg arguments that write `run`'s speech as a 16 kHz mono WAV of exactly the run length.
pub fn speech_args(run: &AvatarRun, fps: u32, public_dir: &Path, out: &Path) -> Vec<String> {
    let total = frames_to_samples(run.frame_count.into(), fps);
    let mut args: Vec<String> = vec!["-y".into(), "-loglevel".into(), "error".into()];
    let mut graph = String::new();
    for (i, p) in run.speech.iter().enumerate() {
        args.push("-i".into());
        args.push(public_dir.join(&p.source).to_string_lossy().into_owned());
        graph.push_str(&format!(
            "[{i}:a]aformat=sample_fmts=fltp:sample_rates={SAMPLE_RATE}:channel_layouts=mono,atrim=start_sample={}:end_sample={},asetpts=PTS-STARTPTS,volume={},adelay=delays={}S:all=1[s{i}];",
            p.file_offset,
            p.file_offset + p.len,
            p.gain,
            p.delay
        ));
    }
    let n = run.speech.len();
    let inputs: String = (0..n).map(|i| format!("[s{i}]")).collect();
    let mix = if n == 1 {
        format!("{inputs}anull")
    } else {
        format!("{inputs}amix=inputs={n}:normalize=0:dropout_transition=0")
    };
    graph.push_str(&format!(
        "{mix},apad=whole_len={total},atrim=end_sample={total},aresample={SPEECH_RATE}[out]"
    ));
    args.extend(
        [
            "-filter_complex",
            &graph,
            "-map",
            "[out]",
            "-ac",
            "1",
            "-c:a",
            "pcm_s16le",
        ]
        .map(String::from),
    );
    args.push(out.to_string_lossy().into_owned());
    args
}

/// Sets `avatar_clip` on every scene of the ready runs. Returns the runs whose clip is missing.
pub fn attach(
    timeline: &mut VideoTimeline,
    runs: &[AvatarRun],
    ready: &dyn Fn(&AvatarRun) -> bool,
) -> Vec<usize> {
    let starts = timeline.scene_start_frames();
    let mut missing = vec![];
    for (k, r) in runs.iter().enumerate() {
        if !ready(r) {
            missing.push(k);
            continue;
        }
        for i in r.scenes.clone() {
            let Some(av) = timeline.scene_avatar(i) else {
                continue;
            };
            // The clip is 25 fps; Remotion maps scene frames to clip time itself.
            timeline.scenes[i].avatar_clip = Some(AvatarClip {
                src: r.clip(),
                offset_frames: starts[i] - r.start_frame,
                corner: av.corner,
                size: av.size,
                margin: av.margin,
                shape: av.shape,
                border_width: av.border_width,
                border_color: av.border_color,
            });
        }
    }
    missing
}

pub fn status(
    timeline: &VideoTimeline,
    runs: &[AvatarRun],
    ready: &dyn Fn(&AvatarRun) -> bool,
) -> Vec<RunStatus> {
    runs.iter()
        .map(|r| {
            let ms = r.duration_ms(timeline.fps);
            RunStatus {
                scenes: r
                    .scenes
                    .clone()
                    .map(|i| timeline.scenes[i].id.clone())
                    .collect(),
                photo: r.photo.clone(),
                clip: r.clip(),
                ready: ready(r),
                duration_ms: ms,
                estimate_s: avatar_estimate_seconds(ms),
            }
        })
        .collect()
}

/// Writes the status under `signature` into [`STATUS_FILE`] (keeps the newest 50 entries).
pub fn write_status(public_dir: &Path, signature: &str, runs: &[RunStatus]) {
    let path = public_dir.join(STATUS_FILE);
    let mut doc: serde_json::Map<String, serde_json::Value> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    doc.remove(signature);
    while doc.len() >= 50 {
        let first = doc.keys().next().cloned().unwrap_or_default();
        doc.remove(&first);
    }
    doc.insert(signature.to_string(), serde_json::json!(runs));
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, serde_json::Value::Object(doc).to_string()).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// The avatar step of the render: plan, maybe generate, attach. `speech` = scene audio +
/// voice-over placements. Returns the runs' status.
pub(crate) async fn apply(
    timeline: &mut VideoTimeline,
    speech: &[Placement],
    signature: Option<&str>,
    cfg: &SchedulerConfig,
    ctx: &RenderContext,
    warnings: &mut Vec<String>,
) -> Result<Vec<RunStatus>, SchedulerError> {
    let mode = cfg.avatar.mode;
    if mode == AvatarMode::Off
        || timeline.default_avatar().is_none() && timeline.scenes.iter().all(|s| s.avatar.is_none())
    {
        return Ok(vec![]);
    }
    let public = cfg.public_dir.clone();
    for i in 0..timeline.scenes.len() {
        if let Some(a) = timeline.scene_avatar(i) {
            if !public.join(&a.photo).is_file() {
                return Err(SchedulerError::Media(MediaError::Avatar(format!(
                    "photo `{}` (scene `{}`) not found",
                    a.photo, timeline.scenes[i].id
                ))));
            }
        }
    }
    let fp =
        |rel: &str| fingerprint(&public.join(rel)).unwrap_or_else(|_| format!("missing:{rel}"));
    let runs = plan_runs(timeline, speech, &cfg.avatar.settings.key(), &fp);
    let ready = |r: &AvatarRun| public.join(r.clip()).is_file();

    if mode == AvatarMode::Generate {
        let todo: Vec<&AvatarRun> = runs.iter().filter(|r| !ready(r)).collect();
        if !todo.is_empty() {
            let engine = match &cfg.avatar.engine {
                Some(e) => e.clone(),
                None => {
                    AvatarEngine::locate(cfg.avatar.home.as_deref(), cfg.child_path.as_deref())?
                }
            };
            let n = todo.len();
            for (k, run) in todo.into_iter().enumerate() {
                generate_one(run, k, n, timeline.fps, &engine, cfg, ctx).await?;
            }
        }
    }
    let st = status(timeline, &runs, &ready);
    if let Some(sig) = signature {
        write_status(&public, sig, &st);
    }
    if mode.stops_early() {
        return Ok(st);
    }
    let missing = attach(timeline, &runs, &ready);
    if !missing.is_empty() {
        let ms: u64 = missing
            .iter()
            .map(|&k| runs[k].duration_ms(timeline.fps))
            .sum();
        let scenes: Vec<String> = missing
            .iter()
            .flat_map(|&k| {
                runs[k]
                    .scenes
                    .clone()
                    .map(|i| timeline.scenes[i].id.clone())
            })
            .collect();
        let msg = format!(
            "narrator avatar not generated yet for scene(s) {} ({:.0} s of speech; generating takes {} on an M1): run `vimmer avatar generate` first (数字人还没生成)",
            scenes.join(", "),
            ms as f64 / 1000.0,
            describe_estimate(avatar_estimate_seconds(ms))
        );
        if mode == AvatarMode::Require {
            return Err(SchedulerError::Media(MediaError::Avatar(msg)));
        }
        warnings.push(format!("{msg}; rendered without it"));
    }
    Ok(st)
}

async fn generate_one(
    run: &AvatarRun,
    k: usize,
    n: usize,
    fps: u32,
    engine: &AvatarEngine,
    cfg: &SchedulerConfig,
    ctx: &RenderContext,
) -> Result<(), SchedulerError> {
    let public = cfg.public_dir.clone();
    let wav = public.join(run.speech_wav());
    if let Some(d) = wav.parent() {
        std::fs::create_dir_all(d)?;
    }
    let ms = run.duration_ms(fps);
    ctx.emit(Progress::Stage {
        name: format!(
            "Narrator avatar {}/{n}: preparing speech ({:.0} s, {})",
            k + 1,
            ms as f64 / 1000.0,
            describe_estimate(avatar_estimate_seconds(ms))
        ),
    });
    let cancel = ctx.cancel.clone();
    let limits = Limits {
        timeout: None,
        cancelled: Arc::new(move || cancel.is_cancelled()),
    };
    let mut cmd = Command::new(&cfg.ffmpeg);
    cmd.args(speech_args(run, fps, &public, &wav));
    if let Some(p) = &cfg.child_path {
        cmd.env("PATH", p);
    }
    let out = {
        let limits = limits.with_timeout(Duration::from_secs(300));
        tokio::task::spawn_blocking(move || run_limited(cmd, &limits))
            .await
            .map_err(|e| SchedulerError::Join(e.to_string()))??
    };
    if !out.status.success() {
        return Err(SchedulerError::Media(MediaError::Avatar(format!(
            "could not build the speech track: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))));
    }
    ctx.check()?;
    let engine = engine.clone();
    let photo = public.join(&run.photo);
    let clip = public.join(run.clip());
    let settings = cfg.avatar.settings.clone();
    let ctx2 = ctx.clone();
    let started = std::time::Instant::now();
    let result = tokio::task::spawn_blocking(move || {
        let mut last = 0;
        engine.generate(
            &photo,
            &wav,
            &clip,
            &settings,
            &limits,
            &mut |ev| match ev {
                AvatarEvent::Frames { done, total } if done >= last + 25 || done == total => {
                    last = done;
                    let el = started.elapsed().as_secs_f64();
                    let left = if done > 0 {
                        el / done as f64 * (total - done) as f64
                    } else {
                        0.0
                    };
                    ctx2.emit(Progress::Stage {
                        name: format!(
                            "Narrator avatar {}/{n}: {done}/{total} frames, {} left",
                            k + 1,
                            describe_estimate(left.round() as u64)
                        ),
                    });
                }
                AvatarEvent::Stage(s) => ctx2.emit(Progress::Stage {
                    name: format!("Narrator avatar {}/{n}: {s}", k + 1),
                }),
                _ => {}
            },
        )
    })
    .await
    .map_err(|e| SchedulerError::Join(e.to_string()))?;
    match result {
        Ok(_) => Ok(()),
        Err(MediaError::Cancelled) => Err(SchedulerError::Cancelled),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vimmer_dsl::{Avatar, Corner, SceneAvatar};

    fn timeline(n: usize) -> VideoTimeline {
        let scenes: Vec<serde_json::Value> = (0..n)
            .map(|i| serde_json::json!({"id": format!("s{i}"), "duration_ms": 2000, "text": "x"}))
            .collect();
        let mut t: VideoTimeline = serde_json::from_value(serde_json::json!({
            "fps": 30, "resolution": {"width": 1920, "height": 1080},
            "scenes": scenes, "subtitles": [], "music": null
        }))
        .unwrap();
        t.avatar = Some(Avatar::with_photo("me.png"));
        t
    }

    fn sec(s: f64) -> u64 {
        (s * SAMPLE_RATE as f64) as u64
    }

    fn place(src: &str, start: f64, end: f64) -> Placement {
        Placement {
            scene: 0,
            source: src.into(),
            start: sec(start),
            end: sec(end),
            gain: 1.0,
            skip: 0,
        }
    }

    fn fp(p: &str) -> String {
        format!("fp-{p}")
    }

    #[test]
    fn runs_follow_speech_photo_and_overrides() {
        let mut t = timeline(5);
        t.scenes[3].avatar = Some(SceneAvatar {
            photo: Some("b.png".into()),
            ..Default::default()
        });
        // speech in scenes 0, 1 (narration), none in 2, voice-over across 3-4
        let speech = vec![
            place("n0.wav", 0.0, 2.0),
            place("n1.wav", 2.0, 4.0),
            place("vo.wav", 6.5, 9.5),
        ];
        let runs = plan_runs(&t, &speech, "k", &fp);
        let got: Vec<_> = runs
            .iter()
            .map(|r| (r.scenes.clone(), r.photo.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![(0..2, "me.png"), (3..4, "b.png"), (4..5, "me.png")],
            "{runs:?}"
        );
        assert_eq!((runs[0].start_frame, runs[0].frame_count), (0, 120));
        t.scenes[3].avatar = None;
        let runs = plan_runs(&t, &speech, "k", &fp);
        assert_eq!(runs[1].scenes, 3..5);
        assert_eq!(
            runs[1].speech,
            vec![SpeechPart {
                source: "vo.wav".into(),
                file_offset: 0,
                delay: sec(0.5),
                len: sec(3.0),
                gain: 1.0
            }]
        );
    }

    #[test]
    fn scene_4_alone_after_other_photo() {
        let mut t = timeline(5);
        t.scenes[3].avatar = Some(SceneAvatar {
            photo: Some("b.png".into()),
            ..Default::default()
        });
        let speech = vec![place("vo.wav", 6.5, 9.5)];
        let runs = plan_runs(&t, &speech, "k", &fp);
        assert_eq!(
            runs.iter().map(|r| r.scenes.clone()).collect::<Vec<_>>(),
            vec![3..4, 4..5]
        );
        assert_eq!(runs[1].speech[0].file_offset, sec(1.5));
    }

    #[test]
    fn cache_key_changes_with_inputs_but_not_placement() {
        let mut t = timeline(2);
        let speech = vec![place("n0.wav", 0.0, 2.0)];
        let a = plan_runs(&t, &speech, "k", &fp)[0].key.clone();
        t.avatar.as_mut().unwrap().corner = Some(Corner::TopLeft);
        t.avatar.as_mut().unwrap().size = Some(0.4);
        assert_eq!(plan_runs(&t, &speech, "k", &fp)[0].key, a);
        assert_ne!(plan_runs(&t, &speech, "k2", &fp)[0].key, a);
        let fp2 = |p: &str| format!("other-{p}");
        assert_ne!(plan_runs(&t, &speech, "k", &fp2)[0].key, a);
        let speech2 = vec![place("n0.wav", 0.1, 2.0)];
        assert_ne!(plan_runs(&t, &speech2, "k", &fp)[0].key, a);
    }

    #[test]
    fn no_avatar_or_no_speech_means_no_runs() {
        let mut t = timeline(2);
        assert!(plan_runs(&t, &[], "k", &fp).is_empty());
        assert!(plan_runs(&t, &[place("x.wav", 1.9, 2.1)], "k", &fp).is_empty()); // 0.1 s each side
        t.avatar = None;
        assert!(plan_runs(&t, &[place("n.wav", 0.0, 4.0)], "k", &fp).is_empty());
        t.scenes[1].avatar = Some(SceneAvatar {
            enabled: Some(false),
            ..Default::default()
        });
        t.avatar = Some(Avatar::with_photo("me.png"));
        let runs = plan_runs(&t, &[place("n.wav", 0.0, 4.0)], "k", &fp);
        assert_eq!(runs[0].scenes, 0..1);
    }

    #[test]
    fn attach_sets_offsets_only_for_ready_runs_and_is_chunk_safe() {
        let mut t = timeline(4);
        t.scenes[2].avatar = Some(SceneAvatar {
            corner: Some(Corner::TopLeft),
            ..Default::default()
        });
        let speech = vec![place("n.wav", 0.0, 8.0)];
        let runs = plan_runs(&t, &speech, "k", &fp);
        assert_eq!(runs.len(), 1);
        let missing = attach(&mut t, &runs, &|_| true);
        assert!(missing.is_empty());
        let offs: Vec<u32> = t
            .scenes
            .iter()
            .map(|s| s.avatar_clip.as_ref().unwrap().offset_frames)
            .collect();
        assert_eq!(offs, vec![0, 60, 120, 180]);
        assert_eq!(
            t.scenes[2].avatar_clip.as_ref().unwrap().corner,
            Corner::TopLeft
        );
        // A chunk keeps whole scenes with their avatar_clip: the clip frame at any global frame
        // is the same in a chunk as in the full timeline.
        let chunks = crate::plan::plan_chunks(&t, 4000);
        for c in &chunks {
            let ct = crate::plan::chunk_timeline(&t, c);
            for (j, s) in ct.scenes.iter().enumerate() {
                assert_eq!(s.avatar_clip, t.scenes[c.scenes.start + j].avatar_clip);
            }
        }
        let mut t2 = timeline(2);
        let runs = plan_runs(&t2, &speech, "k", &fp);
        assert_eq!(attach(&mut t2, &runs, &|_| false), vec![0]);
        assert!(t2.scenes.iter().all(|s| s.avatar_clip.is_none()));
    }

    #[test]
    fn speech_args_place_trim_and_pad() {
        let t = timeline(2);
        let speech = vec![
            Placement {
                skip: sec(1.0),
                ..place("vo.wav", 0.5, 3.0)
            },
            place("n1.wav", 2.0, 4.0),
        ];
        let runs = plan_runs(&t, &speech, "k", &fp);
        let args = speech_args(&runs[0], 30, Path::new("/pub"), Path::new("/o.wav")).join(" ");
        assert!(args.contains("-i /pub/vo.wav -i /pub/n1.wav"), "{args}");
        assert!(
            args.contains(&format!(
                "atrim=start_sample={}:end_sample={}",
                sec(1.0),
                sec(3.5)
            )),
            "{args}"
        );
        assert!(
            args.contains(&format!("adelay=delays={}S", sec(0.5))),
            "{args}"
        );
        assert!(
            args.contains("amix=inputs=2:normalize=0"),
            "{args}"
        );
        assert!(
            args.contains(&format!("apad=whole_len={}", sec(4.0))),
            "{args}"
        );
        assert!(args.ends_with("pcm_s16le /o.wav"), "{args}");
    }

    #[test]
    fn speech_wav_is_16k_mono_and_exactly_the_run_length() {
        if Command::new("ffmpeg").arg("-version").output().is_err() {
            return; // no ffmpeg on this machine
        }
        let dir = std::env::temp_dir().join(format!("vimmer-avatar-speech-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, secs) in [("a.wav", "1.5"), ("b.wav", "3")] {
            let st = Command::new("ffmpeg")
                .args([
                    "-y",
                    "-loglevel",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=f=440:sample_rate=44100",
                    "-t",
                    secs,
                    "-ac",
                    "2",
                ])
                .arg(dir.join(name))
                .status()
                .unwrap();
            assert!(st.success());
        }
        let t = timeline(2);
        let speech = vec![
            place("a.wav", 0.2, 4.0),
            Placement {
                skip: sec(0.5),
                ..place("b.wav", 2.0, 4.0)
            },
        ];
        let runs = plan_runs(&t, &speech, "k", &fp);
        let out = dir.join("s.wav");
        let st = Command::new("ffmpeg")
            .args(speech_args(&runs[0], 30, &dir, &out))
            .status()
            .unwrap();
        assert!(st.success());
        let bytes = std::fs::read(&out).unwrap();
        // 44-byte header (ffmpeg may add a LIST chunk): data = 4 s * 16000 * 2 bytes
        let data = 4 * 16_000 * 2;
        assert!(
            bytes.len() >= data && bytes.len() < data + 200,
            "{}",
            bytes.len()
        );
        assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), 1, "mono");
        assert_eq!(
            u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]),
            16_000
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn modes_parse() {
        assert_eq!(AvatarMode::parse("require"), Some(AvatarMode::Require));
        assert_eq!(AvatarMode::parse("x"), None);
        assert!(AvatarMode::Generate.stops_early() && !AvatarMode::Show.stops_early());
    }
}
