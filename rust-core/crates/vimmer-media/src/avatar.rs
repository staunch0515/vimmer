//! The narrator-avatar engine (数字人讲解人): one face photo + one speech WAV -> a muted
//! talking-head MP4, by JoyVASA + LivePortrait in a local Python env (see
//! `scripts/avatar/`). Apple silicon only; about 11x real time on an M1.
//!
//! The runner prints JSON lines (`stage`, `progress`, `face`, `info`, `done`, `error`); other
//! output (library logs) is ignored. Generation writes `<out>.part.mp4` and renames it, so a
//! cancelled or failed job never leaves a clip that looks finished.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::process::{kill_group, Limits};
use crate::MediaError;

pub const ENV_AVATAR_HOME: &str = "VIMMER_AVATAR_HOME";
pub const DEFAULT_AVATAR_HOME_IN_HOME: &str = ".local/share/vimmer/avatar";
/// Part of every cache key: bump it when the runner's output changes for the same inputs.
pub const AVATAR_ENGINE_VERSION: &str = "vimmer-avatar-2";
pub const AVATAR_INSTALL_HINT: &str = "install it on the Apple-silicon Mac that renders videos with `sh rust-core/crates/vimmer-media/scripts/avatar/install_avatar.sh` (it reuses ~/vimmer-avatar-trial by copying; ~3 GB in ~/.local/share/vimmer/avatar)";
/// Measured on the M1 iMac: 0.44 s per 25 fps frame = 11x real time, plus model load and motion.
pub const SECONDS_PER_AUDIO_SECOND: f64 = 11.0;
pub const FIXED_OVERHEAD_SECONDS: f64 = 25.0;
pub const AVATAR_FPS: u32 = 25;

/// The runner's Python files, written next to each other into a temp dir at first use.
const SCRIPTS: &[(&str, &str)] = &[
    (
        "vimmer_avatar.py",
        include_str!("../scripts/avatar/vimmer_avatar.py"),
    ),
    (
        "avatar_math.py",
        include_str!("../scripts/avatar/avatar_math.py"),
    ),
    (
        "speedpatch.py",
        include_str!("../scripts/avatar/speedpatch.py"),
    ),
    ("fastwarp.py", include_str!("../scripts/avatar/fastwarp.py")),
    (
        "face_mediapipe.py",
        include_str!("../scripts/avatar/face_mediapipe.py"),
    ),
];

/// Seconds a clip of `audio_ms` takes to generate on an M1 (a planning estimate).
pub fn estimate_seconds(audio_ms: u64) -> u64 {
    (FIXED_OVERHEAD_SECONDS + audio_ms as f64 / 1000.0 * SECONDS_PER_AUDIO_SECOND).round() as u64
}

/// "约 11 分钟" style text for an estimate, in both languages the app uses.
pub fn describe_estimate(secs: u64) -> String {
    if secs < 90 {
        format!("about {secs} s (约 {secs} 秒)")
    } else {
        let m = (secs + 30) / 60;
        format!("about {m} min (约 {m} 分钟)")
    }
}

/// Whether this machine can run the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Capability {
    Ready {
        home: PathBuf,
        python: PathBuf,
    },
    /// Apple silicon, but the env is not installed.
    NotInstalled {
        home: PathBuf,
    },
    /// Not an Apple-silicon Mac (e.g. this Intel MacBook): not supported.
    Unsupported {
        reason: String,
    },
}

impl Capability {
    pub fn message(&self) -> String {
        match self {
            Capability::Ready { home, .. } => format!("avatar engine ready ({})", home.display()),
            Capability::NotInstalled { home } => format!(
                "the avatar engine is not installed in {}: {AVATAR_INSTALL_HINT}",
                home.display()
            ),
            Capability::Unsupported { reason } => reason.clone(),
        }
    }
}

fn is_apple_silicon() -> bool {
    cfg!(target_os = "macos") && (cfg!(target_arch = "aarch64") || rosetta_on_arm())
}

/// An x86_64 build running under Rosetta on an M1 still has the M1's GPU for the Python child.
fn rosetta_on_arm() -> bool {
    Command::new("sysctl")
        .args(["-in", "sysctl.proc_translated"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
        .unwrap_or(false)
}

/// The install dir: `home`, else `$VIMMER_AVATAR_HOME`, else `~/.local/share/vimmer/avatar`.
pub fn avatar_home(home: Option<&Path>) -> PathBuf {
    if let Some(h) = home {
        return h.to_path_buf();
    }
    if let Some(h) = std::env::var_os(ENV_AVATAR_HOME).filter(|v| !v.is_empty()) {
        return PathBuf::from(h);
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(DEFAULT_AVATAR_HOME_IN_HOME)
}

/// Capability of this machine, without starting Python.
pub fn capability(home: Option<&Path>) -> Capability {
    capability_on(is_apple_silicon(), &avatar_home(home))
}

fn capability_on(apple_silicon: bool, home: &Path) -> Capability {
    if !apple_silicon {
        return Capability::Unsupported {
            reason: format!(
                "the narrator avatar (数字人) needs an Apple-silicon Mac (M1 or newer); this {} {} Mac is not supported (it would take hours per minute even if it ran). Generate it on the Mac that renders videos.",
                std::env::consts::OS,
                std::env::consts::ARCH
            ),
        };
    }
    let python = home.join("venv/bin/python");
    if python.is_file() && home.join("JoyVASA/src").is_dir() {
        Capability::Ready {
            home: home.to_path_buf(),
            python,
        }
    } else {
        Capability::NotInstalled {
            home: home.to_path_buf(),
        }
    }
}

/// Generation settings that change the picture (all part of the cache key).
#[derive(Debug, Clone, PartialEq)]
pub struct AvatarSettings {
    pub size: u32,
    pub seed: u64,
    pub cfg_scale: f32,
    pub pose_ref: &'static str,
    pub pose_window_s: f32,
}

impl Default for AvatarSettings {
    fn default() -> Self {
        AvatarSettings {
            size: 512,
            seed: 42,
            cfg_scale: 2.0,
            pose_ref: "window",
            pose_window_s: 8.0,
        }
    }
}

impl AvatarSettings {
    pub fn key(&self) -> String {
        format!(
            "{AVATAR_ENGINE_VERSION}|size={}|fps={AVATAR_FPS}|seed={}|cfg={}|pose={}:{}",
            self.size, self.seed, self.cfg_scale, self.pose_ref, self.pose_window_s
        )
    }
}

/// What the runner reports while it works.
#[derive(Debug, Clone, PartialEq)]
pub enum AvatarEvent {
    Stage(String),
    Frames { done: u64, total: u64 },
    Info(String),
}

/// Result of a face probe on a photo.
#[derive(Debug, Clone, PartialEq)]
pub struct FaceProbe {
    pub found: bool,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone)]
pub struct AvatarEngine {
    pub home: PathBuf,
    pub python: PathBuf,
    /// PATH for the child (ffmpeg must be on it).
    pub path_var: Option<String>,
    script_dir: Option<PathBuf>,
}

impl AvatarEngine {
    /// The installed engine, or why it cannot run here.
    pub fn locate(home: Option<&Path>, path_var: Option<&str>) -> Result<Self, MediaError> {
        match capability(home) {
            Capability::Ready { home, python } => Ok(AvatarEngine {
                home,
                python,
                path_var: path_var.map(str::to_string),
                script_dir: None,
            }),
            other => Err(MediaError::Avatar(other.message())),
        }
    }

    /// An engine with an explicit interpreter and script dir (tests use a fake runner).
    pub fn with_python(home: PathBuf, python: PathBuf, script_dir: Option<PathBuf>) -> Self {
        AvatarEngine {
            home,
            python,
            path_var: None,
            script_dir,
        }
    }

    fn scripts(&self) -> Result<PathBuf, MediaError> {
        if let Some(d) = &self.script_dir {
            return Ok(d.clone());
        }
        let all: String = SCRIPTS.iter().map(|(_, s)| *s).collect();
        let dir =
            std::env::temp_dir().join(format!("vimmer-avatar-{}", crate::fingerprint_str(&all)));
        if !dir.join(SCRIPTS[0].0).is_file() {
            let tmp = dir.with_extension(format!("tmp{}", std::process::id()));
            std::fs::create_dir_all(&tmp)?;
            for (name, body) in SCRIPTS {
                std::fs::write(tmp.join(name), body)?;
            }
            if std::fs::rename(&tmp, &dir).is_err() {
                let _ = std::fs::remove_dir_all(&tmp); // another process won the race
            }
        }
        Ok(dir)
    }

    fn command(&self, args: &[&str]) -> Result<Command, MediaError> {
        let script = self.scripts()?.join("vimmer_avatar.py");
        let mut cmd = Command::new(&self.python);
        cmd.arg("-u").arg(script).args(args);
        cmd.env(ENV_AVATAR_HOME, &self.home);
        cmd.env("PYTHONNOUSERSITE", "1");
        if let Some(p) = &self.path_var {
            cmd.env("PATH", p);
        }
        Ok(cmd)
    }

    /// Runs `args`, calling `on_event` for each JSON line; returns the `done`/`face` line.
    fn run(
        &self,
        args: &[&str],
        limits: &Limits,
        on_event: &mut dyn FnMut(AvatarEvent),
    ) -> Result<serde_json::Value, MediaError> {
        let mut cmd = self.command(args)?;
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        if (limits.cancelled)() {
            return Err(MediaError::Cancelled);
        }
        let started = Instant::now();
        let mut child = cmd.spawn()?;
        let pid = child.id();
        let (tx, rx) = mpsc::channel::<String>();
        let out = child.stdout.take();
        let reader = thread::spawn(move || {
            if let Some(out) = out {
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            }
        });
        let err = child.stderr.take();
        let err_reader = thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut e) = err {
                let _ = e.read_to_end(&mut buf);
            }
            buf
        });
        let mut last: Option<serde_json::Value> = None;
        let mut error: Option<(String, String)> = None;
        let mut handle = |line: &str, last: &mut Option<serde_json::Value>| {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                return;
            };
            match v.get("event").and_then(|e| e.as_str()) {
                Some("stage") => on_event(AvatarEvent::Stage(
                    v["stage"].as_str().unwrap_or("").to_string(),
                )),
                Some("progress") => on_event(AvatarEvent::Frames {
                    done: v["done"].as_u64().unwrap_or(0),
                    total: v["total"].as_u64().unwrap_or(0),
                }),
                Some("info") => on_event(AvatarEvent::Info(line.to_string())),
                Some("error") => {
                    error = Some((
                        v["code"].as_str().unwrap_or("runner").to_string(),
                        v["message"].as_str().unwrap_or("").to_string(),
                    ))
                }
                Some("done") | Some("face") => *last = Some(v),
                _ => {}
            }
        };
        let status = loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(line) => {
                    handle(&line, &mut last);
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    break child.wait()?;
                }
            }
            let stop = if (limits.cancelled)() {
                Some(MediaError::Cancelled)
            } else if limits.timeout.is_some_and(|t| started.elapsed() >= t) {
                Some(MediaError::TimedOut {
                    secs: limits.timeout.unwrap_or_default().as_secs(),
                })
            } else {
                None
            };
            if let Some(e) = stop {
                kill_group(pid);
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                let _ = err_reader.join();
                return Err(e);
            }
        };
        let _ = reader.join();
        let stderr = err_reader.join().unwrap_or_default();
        if let Some((code, message)) = error {
            return Err(MediaError::Avatar(match code.as_str() {
                "no_face" => "no face found in the photo; use a clear, front-facing photo of one person (照片里没有检测到人脸)".to_string(),
                _ => format!("{code}: {message}"),
            }));
        }
        if !status.success() && last.is_none() {
            let text = String::from_utf8_lossy(&stderr);
            let tail: Vec<&str> = text.lines().rev().take(3).collect();
            return Err(MediaError::Avatar(format!(
                "the avatar runner failed ({status}): {}",
                tail.into_iter().rev().collect::<Vec<_>>().join(" | ")
            )));
        }
        last.ok_or_else(|| MediaError::Avatar("the avatar runner gave no result".into()))
    }

    /// Generates `out` (muted MP4) from `photo` and `speech_wav`. Writes `out` only on success.
    pub fn generate(
        &self,
        photo: &Path,
        speech_wav: &Path,
        out: &Path,
        settings: &AvatarSettings,
        limits: &Limits,
        on_event: &mut dyn FnMut(AvatarEvent),
    ) -> Result<u64, MediaError> {
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let part = out.with_extension("part.mp4");
        // The runner's framed photo, `<part>.src.png`. It deletes it itself, except when cancel
        // kills its process group; then it is removed here, as is the half-written clip.
        let src_png = PathBuf::from(format!("{}.src.png", part.display()));
        let _ = std::fs::remove_file(&part);
        let _ = std::fs::remove_file(&src_png);
        let size = settings.size.to_string();
        let seed = settings.seed.to_string();
        let cfg = settings.cfg_scale.to_string();
        let window = settings.pose_window_s.to_string();
        let args = [
            "generate",
            "--photo",
            &photo.to_string_lossy(),
            "--audio",
            &speech_wav.to_string_lossy(),
            "--out",
            &part.to_string_lossy(),
            "--size",
            &size,
            "--fps",
            "25",
            "--seed",
            &seed,
            "--cfg-scale",
            &cfg,
            "--pose-ref",
            settings.pose_ref,
            "--pose-window",
            &window,
        ]
        .map(|s| s.to_string());
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let result = self.run(&args, limits, on_event);
        let done = match result {
            Ok(v) => v,
            Err(e) => {
                let _ = std::fs::remove_file(&part);
                let _ = std::fs::remove_file(&src_png);
                return Err(e);
            }
        };
        if !part.is_file() {
            return Err(MediaError::Avatar("the avatar runner wrote no clip".into()));
        }
        std::fs::rename(&part, out)?;
        Ok(done["frames"].as_u64().unwrap_or(0))
    }

    /// Whether MediaPipe finds a face in `photo` (~1-3 s, no model load).
    pub fn probe_face(&self, photo: &Path) -> Result<FaceProbe, MediaError> {
        let p = photo.to_string_lossy();
        let limits = Limits::none().with_timeout(Duration::from_secs(60));
        let v = match self.run(&["probe", "--photo", &p], &limits, &mut |_| {}) {
            Ok(v) => v,
            Err(MediaError::Avatar(m)) if m.contains("no face") => {
                return Ok(FaceProbe {
                    found: false,
                    width: 0,
                    height: 0,
                })
            }
            Err(e) => return Err(e),
        };
        Ok(FaceProbe {
            found: v["found"].as_bool().unwrap_or(false),
            width: v["width"].as_u64().unwrap_or(0) as u32,
            height: v["height"].as_u64().unwrap_or(0) as u32,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    /// A fake `python`: ignores the script, acts on the subcommand.
    pub const FAKE_RUNNER: &str = r#"#!/bin/sh
shift 2   # -u <script>
cmd=$1; shift
out=""; photo=""
while [ $# -gt 0 ]; do
  case "$1" in --out) out=$2; shift;; --photo) photo=$2; shift;; esac; shift
done
case "$photo" in *noface*) echo '{"event":"error","code":"no_face","message":"no face"}'; exit 2;; esac
if [ "$cmd" = probe ]; then echo '{"event":"face","found":true,"width":640,"height":480}'; exit 0; fi
echo 'some library log line'
printf 'png' > "$out.src.png"
echo '{"event":"stage","stage":"render","total":3}'
case "$photo" in *slow*) sleep 30;; esac
rm -f "$out.src.png"
echo '{"event":"progress","done":3,"total":3}'
printf 'mp4' > "$out"
echo '{"event":"done","frames":3,"out":"x","seconds":0.1}'
"#;

    fn fake(dir: &Path) -> AvatarEngine {
        std::fs::create_dir_all(dir).unwrap();
        let py = dir.join("python");
        std::fs::write(&py, FAKE_RUNNER).unwrap();
        std::fs::set_permissions(&py, std::fs::Permissions::from_mode(0o755)).unwrap();
        AvatarEngine::with_python(dir.to_path_buf(), py, Some(dir.to_path_buf()))
    }

    fn tmp(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("vimmer-avatar-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn generate_reports_progress_and_renames_on_success() {
        let d = tmp("ok");
        let e = fake(&d);
        let out = d.join("clips/a.mp4");
        let mut events = vec![];
        let n = e
            .generate(
                Path::new("me.png"),
                Path::new("s.wav"),
                &out,
                &AvatarSettings::default(),
                &Limits::none(),
                &mut |ev| events.push(ev),
            )
            .unwrap();
        assert_eq!(n, 3);
        assert!(out.is_file() && !out.with_extension("part.mp4").exists());
        assert!(events.contains(&AvatarEvent::Frames { done: 3, total: 3 }));
        assert!(events.contains(&AvatarEvent::Stage("render".into())));
    }

    #[test]
    fn no_face_is_a_clear_error_and_leaves_no_clip() {
        let d = tmp("noface");
        let e = fake(&d);
        let out = d.join("b.mp4");
        let err = e
            .generate(
                Path::new("noface.png"),
                Path::new("s.wav"),
                &out,
                &AvatarSettings::default(),
                &Limits::none(),
                &mut |_| {},
            )
            .unwrap_err();
        assert!(err.to_string().contains("没有检测到人脸"), "{err}");
        assert!(!out.exists());
        assert!(!e.probe_face(Path::new("noface.png")).unwrap().found);
        assert!(e.probe_face(Path::new("me.png")).unwrap().found);
    }

    #[test]
    fn cancel_kills_the_runner() {
        let d = tmp("cancel");
        let e = fake(&d);
        let flag = Arc::new(AtomicBool::new(false));
        let f2 = flag.clone();
        let limits = Limits {
            timeout: None,
            cancelled: Arc::new(move || f2.load(Ordering::SeqCst)),
        };
        let started = Instant::now();
        let out = d.join("c.mp4");
        let err = e
            .generate(
                Path::new("slow.png"),
                Path::new("s.wav"),
                &out,
                &AvatarSettings::default(),
                &limits,
                &mut |ev| {
                    if matches!(ev, AvatarEvent::Stage(_)) {
                        flag.store(true, Ordering::SeqCst);
                    }
                },
            )
            .unwrap_err();
        assert!(matches!(err, MediaError::Cancelled), "{err}");
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(!out.exists());
        // Killed before its own cleanup: neither the half clip nor the framed photo is left.
        let left: Vec<_> = std::fs::read_dir(&d)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name())
            .collect();
        assert!(
            !left.iter().any(|n| n.to_string_lossy().starts_with("c.")),
            "{left:?}"
        );
    }

    #[test]
    fn capability_says_plainly_when_unsupported_or_missing() {
        let d = tmp("cap");
        let c = capability_on(false, &d);
        assert!(matches!(c, Capability::Unsupported { .. }));
        assert!(c.message().contains("Apple-silicon"), "{}", c.message());
        let c = capability_on(true, &d);
        assert!(c.message().contains("install_avatar.sh"), "{}", c.message());
        std::fs::create_dir_all(d.join("venv/bin")).unwrap();
        std::fs::create_dir_all(d.join("JoyVASA/src")).unwrap();
        std::fs::write(d.join("venv/bin/python"), "").unwrap();
        assert!(matches!(capability_on(true, &d), Capability::Ready { .. }));
    }

    #[test]
    fn estimates_follow_the_measured_speed() {
        assert_eq!(estimate_seconds(60_000), 685);
        assert!(describe_estimate(685).contains("11 分钟"));
        assert!(describe_estimate(40).contains("40 秒"));
        assert_ne!(
            AvatarSettings::default().key(),
            AvatarSettings {
                seed: 1,
                ..Default::default()
            }
            .key()
        );
    }
}
