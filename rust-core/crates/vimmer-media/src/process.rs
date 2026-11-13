//! Child processes that can be cancelled and timed out.
//!
//! Recognition can take minutes on a slow CPU, so whisper and its ffmpeg helpers must stop when a
//! render is cancelled, and must not hang a render forever. Each child leads its own process
//! group, so killing the group also stops anything it started.

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::MediaError;

/// How often a running child is checked for cancellation and the deadline.
const POLL: Duration = Duration::from_millis(50);

/// Limits for one external command: a time budget and a cancellation check.
#[derive(Clone)]
pub struct Limits {
    /// Kill the command once it has run this long. `None`: no time limit.
    pub timeout: Option<Duration>,
    /// Polled while the command runs; returning true kills it.
    pub cancelled: Arc<dyn Fn() -> bool + Send + Sync>,
}

impl Limits {
    /// No timeout, never cancelled.
    pub fn none() -> Self {
        Self {
            timeout: None,
            cancelled: Arc::new(|| false),
        }
    }

    /// The same cancellation check with another time budget.
    pub fn with_timeout(&self, timeout: Duration) -> Self {
        Self {
            timeout: Some(timeout),
            cancelled: self.cancelled.clone(),
        }
    }
}

impl std::fmt::Debug for Limits {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Limits")
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

/// Runs `cmd` to completion like [`Command::output`], but kills its process group and returns
/// [`MediaError::Cancelled`] or [`MediaError::TimedOut`] when `limits` say so.
pub fn run_limited(mut cmd: Command, limits: &Limits) -> Result<Output, MediaError> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    if (limits.cancelled)() {
        return Err(MediaError::Cancelled);
    }
    let started = Instant::now();
    let mut child = cmd.spawn()?;
    let pid = child.id();
    // Drain both pipes on their own threads, so a chatty child never blocks on a full pipe.
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut buf);
            }
            buf
        })
    };
    let stdout = drain(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let stderr = drain(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );

    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
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
        if let Some(err) = stop {
            kill_group(pid);
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout.join();
            let _ = stderr.join();
            return Err(err);
        }
        thread::sleep(POLL);
    };
    Ok(Output {
        status,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

/// SIGKILL to the process group led by `pid`.
fn kill_group(pid: u32) {
    let _ = Command::new("kill")
        .args(["-KILL", "--", &format!("-{pid}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    fn sh(script: &str) -> Command {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", script]);
        cmd
    }

    #[test]
    fn collects_output_like_command_output() {
        let out = run_limited(sh("echo out; echo err >&2; exit 3"), &Limits::none()).unwrap();
        assert_eq!(out.stdout, b"out\n");
        assert_eq!(out.stderr, b"err\n");
        assert_eq!(out.status.code(), Some(3));
    }

    #[test]
    fn timeout_kills_a_hung_command_and_its_children() {
        let started = Instant::now();
        let limits = Limits::none().with_timeout(Duration::from_millis(300));
        let err = run_limited(sh("sleep 30 & sleep 30"), &limits).unwrap_err();
        assert!(matches!(err, MediaError::TimedOut { .. }), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn cancel_kills_a_running_command() {
        let flag = Arc::new(AtomicBool::new(false));
        let setter = flag.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            setter.store(true, Ordering::SeqCst);
        });
        let limits = Limits {
            timeout: None,
            cancelled: Arc::new(move || flag.load(Ordering::SeqCst)),
        };
        let started = Instant::now();
        let err = run_limited(sh("sleep 30"), &limits).unwrap_err();
        assert!(matches!(err, MediaError::Cancelled), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn already_cancelled_does_not_start() {
        let limits = Limits {
            timeout: None,
            cancelled: Arc::new(|| true),
        };
        assert!(matches!(
            run_limited(sh("sleep 30"), &limits),
            Err(MediaError::Cancelled)
        ));
    }
}
