//! In-memory job registry. Each query submission becomes a job whose
//! status transitions are published over a `watch` channel so SSE
//! subscribers see them without busy-polling.
//!
//! Results are kept until process restart (no TTL / eviction).

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::watch;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Running,
    Done,
    Error,
    Cancelled,
    TimedOut,
}

impl JobStatus {
    /// Terminal states never transition again — SSE streams close after
    /// emitting one.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            JobStatus::Done | JobStatus::Error | JobStatus::Cancelled | JobStatus::TimedOut
        )
    }
}

/// A point-in-time view of a job, serialized to clients as-is.
#[derive(Debug, Clone, Serialize)]
pub struct JobSnapshot {
    pub status: JobStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl JobSnapshot {
    fn queued() -> Self {
        Self {
            status: JobStatus::Queued,
            result: None,
            error: None,
        }
    }
}

struct JobHandle {
    tx: watch::Sender<JobSnapshot>,
    /// When the job first entered a terminal state. `None` while
    /// queued/running. Drives TTL eviction.
    finished_at: Option<Instant>,
}

/// Registry of all jobs seen this process lifetime.
pub struct JobRegistry {
    inner: Mutex<HashMap<String, JobHandle>>,
    counter: AtomicU64,
}

impl Default for JobRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl JobRegistry {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            counter: AtomicU64::new(1),
        }
    }

    /// Mint a new `queued` job and return its id.
    pub fn create(&self) -> String {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        let id = format!("job-{n}");
        let (tx, _rx) = watch::channel(JobSnapshot::queued());
        self.inner.lock().unwrap().insert(
            id.clone(),
            JobHandle {
                tx,
                finished_at: None,
            },
        );
        id
    }

    fn publish(&self, id: &str, snap: JobSnapshot) {
        if let Some(h) = self.inner.lock().unwrap().get_mut(id) {
            let terminal = snap.status.is_terminal();
            // send_replace (not send) so the value updates and notifies
            // even when no subscriber is currently attached — the latest
            // snapshot must always be retained for later GET/subscribe.
            h.tx.send_replace(snap);
            // Stamp the first terminal transition so the TTL sweeper can
            // evict it later. Cancel-then-finish keeps the original
            // stamp (finish_* is a no-op once terminal anyway).
            if terminal && h.finished_at.is_none() {
                h.finished_at = Some(Instant::now());
            }
        }
    }

    /// Drop terminal jobs whose result has outlived `ttl`. Queued/running
    /// jobs are never evicted. Called periodically by the serve sweeper.
    pub fn evict_expired(&self, ttl: Duration) -> usize {
        let mut map = self.inner.lock().unwrap();
        let before = map.len();
        map.retain(|_, h| match h.finished_at {
            Some(t) => t.elapsed() < ttl,
            None => true,
        });
        before - map.len()
    }

    /// Current snapshot, if the job exists.
    pub fn get(&self, id: &str) -> Option<JobSnapshot> {
        self.inner
            .lock()
            .unwrap()
            .get(id)
            .map(|h| h.tx.borrow().clone())
    }

    /// Subscribe to status transitions. The returned receiver yields the
    /// current value immediately, then each change.
    pub fn subscribe(&self, id: &str) -> Option<watch::Receiver<JobSnapshot>> {
        self.inner.lock().unwrap().get(id).map(|h| h.tx.subscribe())
    }

    pub fn status(&self, id: &str) -> Option<JobStatus> {
        self.get(id).map(|s| s.status)
    }

    pub fn mark_running(&self, id: &str) {
        self.publish(
            id,
            JobSnapshot {
                status: JobStatus::Running,
                result: None,
                error: None,
            },
        );
    }

    /// Record a successful result. A job that was cancelled or timed out
    /// while running keeps that terminal state — the result is discarded
    /// (cooperative-cancel semantics: we can't stop the query, so we
    /// drop its output).
    pub fn finish_ok(&self, id: &str, result: serde_json::Value) {
        let cur = self.status(id);
        if matches!(cur, Some(JobStatus::Cancelled) | Some(JobStatus::TimedOut)) {
            return;
        }
        self.publish(
            id,
            JobSnapshot {
                status: JobStatus::Done,
                result: Some(result),
                error: None,
            },
        );
    }

    pub fn finish_err(&self, id: &str, error: String) {
        let cur = self.status(id);
        if matches!(cur, Some(JobStatus::Cancelled) | Some(JobStatus::TimedOut)) {
            return;
        }
        self.publish(
            id,
            JobSnapshot {
                status: JobStatus::Error,
                result: None,
                error: Some(error),
            },
        );
    }

    pub fn mark_timeout(&self, id: &str) {
        self.publish(
            id,
            JobSnapshot {
                status: JobStatus::TimedOut,
                result: None,
                error: Some("query exceeded its timeout (still running in background)".into()),
            },
        );
    }

    /// Cooperative cancel. Returns the resulting status, or `None` if the
    /// job is unknown. A `queued`/`running` job is marked `cancelled`;
    /// a running query is *not* force-stopped (no DuckDB interrupt) — it
    /// runs to completion and its result is discarded.
    pub fn cancel(&self, id: &str) -> Option<JobStatus> {
        let cur = self.status(id)?;
        if cur.is_terminal() {
            return Some(cur);
        }
        self.publish(
            id,
            JobSnapshot {
                status: JobStatus::Cancelled,
                result: None,
                error: None,
            },
        );
        Some(JobStatus::Cancelled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_queued_running_done() {
        let reg = JobRegistry::new();
        let id = reg.create();
        assert_eq!(reg.status(&id), Some(JobStatus::Queued));
        reg.mark_running(&id);
        assert_eq!(reg.status(&id), Some(JobStatus::Running));
        reg.finish_ok(&id, serde_json::json!({"ok": true}));
        let snap = reg.get(&id).unwrap();
        assert_eq!(snap.status, JobStatus::Done);
        assert_eq!(snap.result, Some(serde_json::json!({"ok": true})));
    }

    #[test]
    fn cancel_then_finish_discards_result() {
        let reg = JobRegistry::new();
        let id = reg.create();
        reg.mark_running(&id);
        assert_eq!(reg.cancel(&id), Some(JobStatus::Cancelled));
        // Query finishes after cancel — result must be discarded.
        reg.finish_ok(&id, serde_json::json!({"late": true}));
        let snap = reg.get(&id).unwrap();
        assert_eq!(snap.status, JobStatus::Cancelled);
        assert!(snap.result.is_none());
    }

    #[test]
    fn cancel_unknown_job_is_none() {
        let reg = JobRegistry::new();
        assert_eq!(reg.cancel("nope"), None);
    }

    #[test]
    fn ttl_evicts_terminal_but_keeps_in_flight() {
        let reg = JobRegistry::new();
        let done = reg.create();
        reg.finish_ok(&done, serde_json::json!({}));
        let running = reg.create();
        reg.mark_running(&running);

        // Zero TTL → the finished job is past its window, the running
        // one is untouched.
        let removed = reg.evict_expired(Duration::from_secs(0));
        assert_eq!(removed, 1);
        assert!(reg.get(&done).is_none());
        assert_eq!(reg.status(&running), Some(JobStatus::Running));

        // A long TTL keeps a freshly-finished job.
        reg.finish_ok(&running, serde_json::json!({}));
        assert_eq!(reg.evict_expired(Duration::from_secs(3600)), 0);
        assert_eq!(reg.status(&running), Some(JobStatus::Done));
    }
}
