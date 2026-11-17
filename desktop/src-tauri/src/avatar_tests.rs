//! Tests of the 数字人 commands with a fake engine (a shell script standing in for the Python
//! runner), so they run on any Mac, including the Intel MacBook.

use std::os::unix::fs::PermissionsExt;
use std::sync::mpsc;
use std::time::Duration;

use vimmer_scheduler::RenderOptions;

use super::*;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vimmer-desk-avatar-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A fake interpreter: `python -u <script> <cmd> --photo … --out …`. `generate` writes the half
/// clip first, then reports frames every `delay` seconds, then finishes. Its pid and the pid of
/// a child it starts go to `pids`, so a test can see that cancel killed the whole group.
fn fake_engine(dir: &Path, frames: u32, delay: &str) -> AvatarEngine {
    let script = format!(
        r#"#!/bin/sh
shift 2
cmd=$1; shift
out=""; photo=""
while [ $# -gt 0 ]; do
  case "$1" in --out) out=$2; shift;; --photo) photo=$2; shift;; esac; shift
done
case "$photo" in *noface*) echo '{{"event":"error","code":"no_face","message":"no face"}}'; exit 2;; esac
if [ "$cmd" = probe ]; then echo '{{"event":"face","found":true,"width":640,"height":480}}'; exit 0; fi
sleep 600 &
echo "$$ $!" > "{pids}"
echo '{{"event":"stage","stage":"render"}}'
echo '{{"event":"info","frames":{frames},"pitch_range_raw":[-0.5,0.4],"pitch_range_used":[-0.7,0.3]}}'
printf 'half' > "$out"
i=1
while [ $i -le {frames} ]; do
  echo "{{\"event\":\"progress\",\"done\":$i,\"total\":{frames}}}"
  sleep {delay}
  i=$((i+1))
done
kill $! 2>/dev/null
printf 'clip' > "$out"
echo '{{"event":"done","frames":{frames}}}'
"#,
        pids = dir.join("pids").display()
    );
    let python = dir.join("fake-python");
    std::fs::write(&python, script).unwrap();
    std::fs::set_permissions(&python, std::fs::Permissions::from_mode(0o755)).unwrap();
    AvatarEngine::with_python(dir.to_path_buf(), python, Some(dir.to_path_buf()))
}

const TIMELINE: &str = r#"{"fps":30,"resolution":{"width":1920,"height":1080},
    "scenes":[{"id":"a","duration_ms":2000,"text":"spoken by the file","audio":{"source":"speech.wav"}},
              {"id":"b","duration_ms":1000,"text":"silent","avatar":{"enabled":false}}],
    "subtitles":[],"avatar":{"photo":"face.png","corner":"bottom_left"}}"#;

fn ffmpeg() -> PathBuf {
    find_in_path("ffmpeg", &user_path())
}

/// A public folder with the photo and 2 s of speech; the config renders from it.
fn project(name: &str) -> (PathBuf, SchedulerConfig) {
    let root = temp(name);
    let public = root.join("public");
    std::fs::create_dir_all(&public).unwrap();
    std::fs::write(public.join("face.png"), b"not really a png").unwrap();
    let ok = std::process::Command::new(ffmpeg())
        .args(["-v", "error", "-f", "lavfi", "-i", "sine=frequency=440:duration=2", "-ar", "16000", "-y"])
        .arg(public.join("speech.wav"))
        .status()
        .unwrap()
        .success();
    assert!(ok, "ffmpeg could not make the test speech");
    let mut cfg = RenderOptions::default().config(root.join("work"), root.join("out.mp4"), root.join("remotion"));
    cfg.public_dir = public;
    cfg.ffmpeg = ffmpeg();
    cfg.child_path = Some(user_path());
    cfg.word_subtitles.enabled = false;
    (root, cfg)
}

fn run(cfg: SchedulerConfig, ctx: RenderContext) -> Finished {
    let timeline: VideoTimeline = serde_json::from_str(TIMELINE).unwrap();
    let probe: Arc<dyn AudioProbe> = Arc::new(FfmpegProbe { binary: ffmpeg() });
    let speech: Arc<dyn Speech> = Arc::new(SaySpeech { say: PathBuf::from("say"), ffmpeg: ffmpeg() });
    tauri::async_runtime::block_on(generate_in(timeline, cfg, probe, speech, ctx))
}

fn clips(public: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(public.join("generated/avatar"))
        .map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    names.retain(|n| n.ends_with(".mp4"));
    names.sort();
    names
}

fn alive(pid: &str) -> bool {
    std::process::Command::new("kill").args(["-0", pid]).status().is_ok_and(|s| s.success())
}

#[test]
fn generates_then_status_is_ready() {
    let (root, mut cfg) = project("gen");
    let public = cfg.public_dir.clone();
    let timeline: VideoTimeline = serde_json::from_str(TIMELINE).unwrap();
    // Before: no record for this timeline; the speaking scene (2 s) needs generating.
    let before = status_in(&timeline, &public);
    assert_eq!(before["known"], json!(false));
    assert_eq!(before["missing_estimate_s"], json!(avatar_estimate_seconds(2000)));

    cfg.avatar.engine = Some(fake_engine(&root, 5, "0"));
    let (tx, rx) = mpsc::channel::<String>();
    let ctx = RenderContext {
        cancel: Arc::new(CancelHandle::default()),
        progress: Arc::new(move |p| {
            if let Progress::Stage { name } = p {
                let _ = tx.send(name);
            }
        }),
    };
    let done = run(cfg, ctx);
    assert!(done.ok, "{:?}", done.error);
    assert_eq!(done.runs.len(), 1);
    assert!(done.runs[0].ready);
    assert_eq!(done.runs[0].scenes, vec!["a".to_string()]);
    let log: Vec<String> = rx.try_iter().collect();
    assert!(log.iter().any(|l| l.contains("5/5 frames")), "{log:?}");
    // The pose-drift numbers reach the log.
    assert!(log.iter().any(|l| l.contains("head pose pitch [-0.5, 0.4]° raw")), "{log:?}");
    assert_eq!(clips(&public).len(), 1, "one finished clip, no .part");

    let after = status_in(&timeline, &public);
    assert_eq!(after["known"], json!(true));
    assert_eq!(after["runs"][0]["ready"], json!(true));
    assert_eq!(after["missing_estimate_s"], json!(0));
    // A look change (corner) keeps the clip; a text change needs a new one.
    let moved: VideoTimeline = serde_json::from_str(&TIMELINE.replace("bottom_left", "top_right")).unwrap();
    assert_eq!(status_in(&moved, &public)["runs"][0]["ready"], json!(true));
    let edited: VideoTimeline =
        serde_json::from_str(&TIMELINE.replace(r#""duration_ms":2000"#, r#""duration_ms":2500"#)).unwrap();
    assert_eq!(status_in(&edited, &public)["known"], json!(false));
    // The file is gone: missing again.
    std::fs::remove_file(public.join(after["runs"][0]["clip"].as_str().unwrap())).unwrap();
    assert_eq!(status_in(&timeline, &public)["runs"][0]["ready"], json!(false));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn cancel_kills_the_runner_group_and_leaves_no_clip() {
    let (root, mut cfg) = project("cancel");
    let public = cfg.public_dir.clone();
    cfg.avatar.engine = Some(fake_engine(&root, 200, "0.1"));
    let cancel = Arc::new(CancelHandle::default());
    let watcher = cancel.clone();
    let ctx = RenderContext {
        cancel: cancel.clone(),
        progress: Arc::new(move |p| {
            if let Progress::Stage { name } = p {
                if name.contains("/200 frames") {
                    watcher.cancel();
                }
            }
        }),
    };
    let done = run(cfg, ctx);
    assert!(done.cancelled && !done.ok, "{done:?}");
    assert!(clips(&public).is_empty(), "no clip and no .part: {:?}", clips(&public));
    // The runner and its child are gone.
    let pids = std::fs::read_to_string(root.join("pids")).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    for pid in pids.split_whitespace() {
        assert!(!alive(pid), "process {pid} still runs");
    }
    // Nothing was recorded as ready.
    let timeline: VideoTimeline = serde_json::from_str(TIMELINE).unwrap();
    assert_eq!(status_in(&timeline, &public)["known"], json!(false));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn no_avatar_has_nothing_to_generate() {
    let dir = temp("none");
    let t: VideoTimeline = serde_json::from_str(r#"{"fps":30,"resolution":{"width":1920,"height":1080},
        "scenes":[{"id":"a","duration_ms":2000,"text":"hi"}],"subtitles":[]}"#)
    .unwrap();
    let s = status_in(&t, &dir);
    assert_eq!((s["known"].clone(), s["runs"].clone(), s["missing_estimate_s"].clone()), (json!(true), json!([]), json!(0)));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn env_messages_say_where_to_generate() {
    let v = env_value(&AvatarCapability::Unsupported { reason: "Intel".into() });
    assert_eq!(v["state"], json!("unsupported"));
    assert!(v["zh"].as_str().unwrap().contains("请在 iMac 上生成"));
    let v = env_value(&AvatarCapability::NotInstalled { home: PathBuf::from("/x") });
    assert_eq!(v["state"], json!("not_installed"));
    let v = env_value(&AvatarCapability::Ready { home: PathBuf::from("/x"), python: PathBuf::from("/p") });
    assert_eq!(v["state"], json!("ready"));
}

#[test]
fn probe_reports_each_case() {
    let dir = temp("probe");
    let engine = fake_engine(&dir, 1, "0");
    std::fs::write(dir.join("me.png"), b"x").unwrap();
    std::fs::write(dir.join("noface.png"), b"x").unwrap();
    let found = probe_with(Ok(engine.clone()), Some(dir.join("me.png")));
    assert_eq!(found["state"], json!("found"));
    let none = probe_with(Ok(engine), Some(dir.join("noface.png")));
    assert_eq!(none["state"], json!("no_face"));
    assert_eq!(none["zh"], json!("没找到清晰的正脸"));
    assert_eq!(probe_with(Err("Intel".into()), Some(dir.join("me.png")))["state"], json!("unavailable"));
    assert_eq!(probe_with(Err("x".into()), None)["state"], json!("missing"));
    // The project's copy wins; unsafe paths are refused.
    let project = dir.join("proj");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("me.png"), b"y").unwrap();
    assert_eq!(locate_photo("me.png", Some(&project), &dir), Some(project.join("me.png")));
    assert_eq!(locate_photo("me.png", None, &dir), Some(dir.join("me.png")));
    assert_eq!(locate_photo("../me.png", None, &project), None);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn render_panel_avatar_modes() {
    use crate::render::avatar_mode;
    let panel = |m: Option<&str>| PanelOptions { avatar: m.map(String::from), ..PanelOptions::default() };
    assert_eq!(avatar_mode(&panel(None)), Ok(AvatarMode::Show));
    assert_eq!(avatar_mode(&panel(Some("require"))), Ok(AvatarMode::Require));
    assert_eq!(avatar_mode(&panel(Some("off"))), Ok(AvatarMode::Off));
    assert!(avatar_mode(&panel(Some("generate"))).is_err());
}
