pub mod recompute_access_levels;

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

/// Per-run outcome counters for a background job, so a completed run can report
/// what actually happened instead of only "done / 100%".
///
/// Embedded in each reindex state; read by the status endpoints so an operator
/// can tell a partial reconciliation from a clean one and see the last error.
#[derive(Default)]
pub struct JobOutcome {
    /// Items that failed to (re)index.
    pub failed: AtomicU32,
    /// Items intentionally skipped (e.g. content missing from storage).
    pub skipped: AtomicU32,
    /// Most recent per-item error, for surfacing in the UI.
    pub last_error: Mutex<Option<String>>,
}

impl JobOutcome {
    /// Clear all counters at the start of a run.
    pub fn reset(&self) {
        self.failed.store(0, Ordering::Relaxed);
        self.skipped.store(0, Ordering::Relaxed);
        *self.last_error.lock().unwrap() = None;
    }

    /// Record a failed item and remember its error message.
    pub fn record_failure(&self, error: impl Into<String>) {
        self.failed.fetch_add(1, Ordering::Relaxed);
        *self.last_error.lock().unwrap() = Some(error.into());
    }

    /// Record a skipped item.
    pub fn record_skip(&self) {
        self.skipped.fetch_add(1, Ordering::Relaxed);
    }

    /// Snapshot `(failed, skipped, last_error)` for reporting.
    pub fn snapshot(&self) -> (u32, u32, Option<String>) {
        (
            self.failed.load(Ordering::Relaxed),
            self.skipped.load(Ordering::Relaxed),
            self.last_error.lock().unwrap().clone(),
        )
    }
}

/// Shared state of a background job that exposes an `is_running` flag.
///
/// Triggers gate concurrent runs with a `compare_exchange(false → true)` on this
/// flag, so it must always return to `false` when a run ends.
pub trait RunningFlag: Send + Sync {
    fn is_running(&self) -> &AtomicBool;
}

/// RAII guard that clears a job's `is_running` flag when dropped — including on
/// panic or early return.
///
/// Background reindex jobs are `tokio::spawn`ed; if one panics or is cancelled
/// before reaching its cleanup, a manually-reset flag would stay `true` for the
/// life of the process and permanently block every future run (which gates on a
/// `compare_exchange` of the same flag). Holding this guard for the duration of
/// the job makes the reset unconditional.
pub struct RunningGuard<T: RunningFlag> {
    state: Arc<T>,
}

impl<T: RunningFlag> RunningGuard<T> {
    /// Take ownership of the running flag; it is cleared when the guard drops.
    pub fn new(state: Arc<T>) -> Self {
        Self { state }
    }
}

impl<T: RunningFlag> Drop for RunningGuard<T> {
    fn drop(&mut self) {
        self.state.is_running().store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeJob {
        running: AtomicBool,
    }

    impl RunningFlag for FakeJob {
        fn is_running(&self) -> &AtomicBool {
            &self.running
        }
    }

    #[test]
    fn guard_clears_running_flag_on_normal_drop() {
        let job = Arc::new(FakeJob {
            running: AtomicBool::new(true),
        });
        {
            let _guard = RunningGuard::new(job.clone());
        }
        assert!(
            !job.running.load(Ordering::Acquire),
            "running flag must be cleared when the guard drops"
        );
    }

    #[test]
    fn job_outcome_tracks_failures_skips_and_last_error() {
        let outcome = JobOutcome::default();
        outcome.record_failure("first boom");
        outcome.record_skip();
        outcome.record_failure("second boom");

        let (failed, skipped, last_error) = outcome.snapshot();
        assert_eq!(failed, 2);
        assert_eq!(skipped, 1);
        assert_eq!(last_error.as_deref(), Some("second boom"));

        outcome.reset();
        let (failed, skipped, last_error) = outcome.snapshot();
        assert_eq!(failed, 0);
        assert_eq!(skipped, 0);
        assert!(last_error.is_none());
    }

    #[tokio::test]
    async fn guard_clears_running_flag_when_job_panics() {
        let job = Arc::new(FakeJob {
            running: AtomicBool::new(true),
        });
        let job_in_task = job.clone();
        let handle = tokio::spawn(async move {
            let _guard = RunningGuard::new(job_in_task);
            panic!("simulated reindex panic");
        });
        let _ = handle.await; // JoinError from the panic; ignore it.

        assert!(
            !job.running.load(Ordering::Acquire),
            "a panicked job must not leave the running flag stuck true"
        );
    }
}
