//! Background jobs with cancellation, progress and panic isolation.
//!
//! The UI thread never touches the disk. It starts a [`Job`], keeps rendering,
//! and is woken by `on_done` (for egui: `ctx.request_repaint()`), so an idle
//! app uses no CPU.

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, TryRecvError, bounded};
use difflux_core::{CancelToken, Progress};

/// The job ended without producing a value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JobFailed {
    #[error("internal error: {0}")]
    Panicked(String),
    #[error("worker thread exited unexpectedly")]
    Disconnected,
}

/// Handle to work running on another thread. Dropping it cancels the work.
#[derive(Debug)]
pub struct Job<T> {
    rx: Receiver<Result<T, JobFailed>>,
    cancel: CancelToken,
    progress: Progress,
    started: Instant,
    label: String,
}

impl<T: Send + 'static> Job<T> {
    /// Starts `work` on a new thread. `on_done` runs on that thread after the
    /// result is ready (or after a panic), and is the place to wake the UI.
    pub fn spawn(
        label: impl Into<String>,
        work: impl FnOnce(&CancelToken, &Progress) -> T + Send + 'static,
        on_done: impl FnOnce() + Send + 'static,
    ) -> std::io::Result<Self> {
        let (tx, rx) = bounded(1);
        let cancel = CancelToken::new();
        let progress = Progress::new();
        let (c, p) = (cancel.clone(), progress.clone());
        let label = label.into();
        thread::Builder::new().name(format!("difflux-job: {label}")).spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(|| work(&c, &p))).map_err(|e| JobFailed::Panicked(panic_message(&*e)));
            let _ = tx.send(result);
            on_done();
        })?;
        Ok(Self { rx, cancel, progress, started: Instant::now(), label })
    }

    /// Returns the result once it is ready, without blocking.
    pub fn poll(&self) -> Option<Result<T, JobFailed>> {
        match self.rx.try_recv() {
            Ok(r) => Some(r),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(JobFailed::Disconnected)),
        }
    }

    /// Blocks until the result is ready (for tests and the CLI).
    pub fn wait(self) -> Result<T, JobFailed> {
        self.rx.recv().unwrap_or(Err(JobFailed::Disconnected))
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    #[must_use]
    pub fn progress(&self) -> &Progress {
        &self.progress
    }

    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }
}

impl<T> Drop for Job<T> {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(ToString::to_string)
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn delivers_result_and_wakes_caller() -> std::io::Result<()> {
        let woke = Arc::new(AtomicBool::new(false));
        let w = woke.clone();
        let job = Job::spawn(
            "sum",
            |_, p| {
                p.add_items(3);
                1 + 2
            },
            move || w.store(true, Ordering::SeqCst),
        )?;
        assert_eq!(job.wait(), Ok(3));
        // on_done runs right after the send; give it a moment.
        for _ in 0..100 {
            if woke.load(Ordering::SeqCst) {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("on_done never ran");
    }

    #[test]
    fn panics_become_errors() -> std::io::Result<()> {
        let job: Job<()> = Job::spawn("boom", |_, _| panic!("kaboom"), || {})?;
        assert_eq!(job.wait(), Err(JobFailed::Panicked("kaboom".into())));
        Ok(())
    }

    #[test]
    fn cancel_reaches_the_worker() -> std::io::Result<()> {
        let job = Job::spawn(
            "spin",
            |c, _| {
                while !c.is_cancelled() {
                    thread::sleep(Duration::from_millis(1));
                }
                "stopped"
            },
            || {},
        )?;
        job.cancel();
        assert_eq!(job.wait(), Ok("stopped"));
        Ok(())
    }
}
