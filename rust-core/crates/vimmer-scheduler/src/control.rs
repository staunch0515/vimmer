//! Progress reporting and cancellation for one render.
//!
//! Cancelling kills every external process the render started, each in its own process group,
//! so `npx` and the Chromium it launches stop together.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::Serialize;

use crate::run::SchedulerError;

/// What the pipeline is doing, for a progress bar and status line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Progress {
    Stage { name: String },
    Images { done: usize, total: usize },
    Chunks { done: usize, total: usize },
    Done { output: String },
}

pub type ProgressFn = Arc<dyn Fn(Progress) + Send + Sync>;

/// Shared cancellation state for one render.
#[derive(Default)]
pub struct CancelHandle {
    cancelled: AtomicBool,
    running: Mutex<HashSet<u32>>,
}

impl CancelHandle {
    /// Marks the render cancelled and kills all of its running process groups.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        let pids: Vec<u32> = self.lock_running().iter().copied().collect();
        for pid in pids {
            kill_group(pid);
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub(crate) fn track(&self, pid: u32) {
        self.lock_running().insert(pid);
        // A cancel that raced with the spawn must still stop this process.
        if self.is_cancelled() {
            kill_group(pid);
        }
    }

    pub(crate) fn untrack(&self, pid: u32) {
        self.lock_running().remove(&pid);
    }

    fn lock_running(&self) -> MutexGuard<'_, HashSet<u32>> {
        self.running.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Sends SIGKILL to a process group. Spawned processes lead their own group (`process_group(0)`),
/// so the group id equals the child's pid.
fn kill_group(pid: u32) {
    let _ = std::process::Command::new("kill")
        .args(["-KILL", "--", &format!("-{pid}")])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// Everything a render reports to and listens to.
#[derive(Clone)]
pub struct RenderContext {
    pub cancel: Arc<CancelHandle>,
    pub progress: ProgressFn,
}

impl RenderContext {
    /// A context that ignores progress and cannot be cancelled. Used by the CLI.
    pub fn silent() -> Self {
        Self {
            cancel: Arc::new(CancelHandle::default()),
            progress: Arc::new(|_| {}),
        }
    }

    pub fn emit(&self, event: Progress) {
        (self.progress)(event);
    }

    /// Returns `Err(Cancelled)` once the render has been cancelled. Call between stages.
    pub fn check(&self) -> Result<(), SchedulerError> {
        if self.cancel.is_cancelled() {
            Err(SchedulerError::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn cancel_kills_the_whole_process_group() {
        // A shell with a child, both in the same group: cancelling must stop both.
        let mut cmd = tokio::process::Command::new("sh");
        cmd.args(["-c", "sleep 30 & sleep 30"]).process_group(0);
        let mut child = cmd.spawn().unwrap();
        let pid = child.id().unwrap();

        let handle = CancelHandle::default();
        handle.track(pid);
        handle.cancel();

        let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .expect("process should die quickly after cancel")
            .unwrap();
        assert!(!status.success());
        assert!(handle.is_cancelled());
    }

    #[test]
    fn check_reports_cancelled_after_cancel() {
        let ctx = RenderContext::silent();
        assert!(ctx.check().is_ok());
        ctx.cancel.cancel();
        assert!(matches!(ctx.check(), Err(SchedulerError::Cancelled)));
    }
}
