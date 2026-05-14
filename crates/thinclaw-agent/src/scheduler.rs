//! Scheduler lifecycle types and root-independent policy helpers.

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Message to send to a worker.
#[derive(Debug)]
pub enum WorkerMessage {
    /// Start working on the job.
    Start,
    /// Stop the job.
    Stop,
    /// Check health.
    Ping,
}

/// Status of a scheduled job.
#[derive(Debug)]
pub struct ScheduledJob {
    pub handle: JoinHandle<()>,
    pub tx: mpsc::Sender<WorkerMessage>,
}

/// Status of a scheduled sub-task.
///
/// Stores only the raw `JoinHandle` needed for `is_finished()` polling during
/// subtask cleanup. The actual result is delivered via a `oneshot` channel from
/// the root scheduler adapter.
#[derive(Debug)]
pub struct ScheduledSubtask {
    handle: JoinHandle<()>,
}

impl ScheduledSubtask {
    pub fn new(handle: JoinHandle<()>) -> Self {
        Self { handle }
    }

    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }

    pub fn abort(self) {
        self.handle.abort();
    }
}

/// Capacity snapshot used by scheduler adapters before inserting a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchedulerCapacity {
    pub running: usize,
    pub limit: usize,
}

impl SchedulerCapacity {
    pub fn new(running: usize, limit: usize) -> Self {
        Self { running, limit }
    }

    pub fn allows_schedule(self) -> bool {
        self.running < self.limit
    }
}

/// Reserved system jobs get one overflow slot above normal user capacity.
pub fn reserved_job_limit(max_parallel_jobs: usize) -> usize {
    max_parallel_jobs.saturating_add(1)
}

/// Merge routine dispatch markers into job metadata.
pub fn routine_job_metadata(
    metadata: Option<serde_json::Value>,
    reserved_system_slot: bool,
) -> serde_json::Value {
    let mut merged = metadata.unwrap_or_else(|| serde_json::json!({}));
    if !merged.is_object() {
        merged = serde_json::json!({});
    }

    if let Some(obj) = merged.as_object_mut() {
        obj.insert(
            "routine_dispatched".to_string(),
            serde_json::Value::Bool(true),
        );
        if reserved_system_slot {
            obj.insert("system_reserved".to_string(), serde_json::Value::Bool(true));
        }
    }

    merged
}

pub const SUBTASK_CLEANUP_DELAYS_MS: [u64; 8] =
    [100, 500, 1000, 2000, 5000, 10_000, 10_000, 10_000];
pub const SUBTASK_CLEANUP_TIMEOUT_SECS: u64 = 600;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routine_metadata_marks_reserved_jobs() {
        let metadata = serde_json::json!({ "existing": true });
        let merged = routine_job_metadata(Some(metadata), true);

        assert_eq!(merged["existing"], true);
        assert_eq!(merged["routine_dispatched"], true);
        assert_eq!(merged["system_reserved"], true);
    }

    #[test]
    fn routine_metadata_replaces_non_object_input() {
        let merged = routine_job_metadata(Some(serde_json::json!("bad")), false);

        assert_eq!(merged["routine_dispatched"], true);
        assert!(merged.get("system_reserved").is_none());
    }

    #[test]
    fn capacity_uses_strict_less_than_limit() {
        assert!(SchedulerCapacity::new(1, 2).allows_schedule());
        assert!(!SchedulerCapacity::new(2, 2).allows_schedule());
    }

    #[test]
    fn reserved_limit_saturates() {
        assert_eq!(reserved_job_limit(2), 3);
        assert_eq!(reserved_job_limit(usize::MAX), usize::MAX);
    }
}
