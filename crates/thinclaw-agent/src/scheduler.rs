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

/// Which capacity policy to apply when admitting a scheduled worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerAdmissionKind {
    /// Normal user-visible jobs count against the configured worker limit.
    Standard,
    /// System routine jobs may use one overflow slot above normal capacity.
    ReservedSystem,
}

impl SchedulerAdmissionKind {
    pub fn capacity_limit(self, max_parallel_jobs: usize) -> usize {
        match self {
            Self::Standard => max_parallel_jobs,
            Self::ReservedSystem => reserved_job_limit(max_parallel_jobs),
        }
    }

    pub fn transition_reason(self) -> &'static str {
        match self {
            Self::Standard => "Scheduled for execution",
            Self::ReservedSystem => "Scheduled for execution (reserved slot)",
        }
    }
}

/// Deterministic outcome of a scheduler admission check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerAdmissionOutcome {
    AlreadyScheduled,
    Accepted { capacity: SchedulerCapacity },
    AtCapacity { capacity: SchedulerCapacity },
}

/// Decide whether a worker can be inserted before root creates tasks/channels.
pub fn scheduler_admission(
    already_scheduled: bool,
    running: usize,
    max_parallel_jobs: usize,
    kind: SchedulerAdmissionKind,
) -> SchedulerAdmissionOutcome {
    if already_scheduled {
        return SchedulerAdmissionOutcome::AlreadyScheduled;
    }

    let capacity = SchedulerCapacity::new(running, kind.capacity_limit(max_parallel_jobs));
    if capacity.allows_schedule() {
        SchedulerAdmissionOutcome::Accepted { capacity }
    } else {
        SchedulerAdmissionOutcome::AtCapacity { capacity }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubtaskCleanupDecision {
    KeepWaiting,
    RemoveFinished,
    ForceRemoveTimedOut,
}

/// Decide what to do after each subtask cleanup polling interval.
pub fn subtask_cleanup_decision(deadline_reached: bool, finished: bool) -> SubtaskCleanupDecision {
    if deadline_reached {
        SubtaskCleanupDecision::ForceRemoveTimedOut
    } else if finished {
        SubtaskCleanupDecision::RemoveFinished
    } else {
        SubtaskCleanupDecision::KeepWaiting
    }
}

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

    #[test]
    fn scheduler_admission_uses_standard_capacity() {
        assert_eq!(
            scheduler_admission(false, 1, 2, SchedulerAdmissionKind::Standard),
            SchedulerAdmissionOutcome::Accepted {
                capacity: SchedulerCapacity::new(1, 2)
            }
        );
        assert_eq!(
            scheduler_admission(false, 2, 2, SchedulerAdmissionKind::Standard),
            SchedulerAdmissionOutcome::AtCapacity {
                capacity: SchedulerCapacity::new(2, 2)
            }
        );
    }

    #[test]
    fn scheduler_admission_grants_reserved_overflow_slot() {
        assert_eq!(
            scheduler_admission(false, 2, 2, SchedulerAdmissionKind::ReservedSystem),
            SchedulerAdmissionOutcome::Accepted {
                capacity: SchedulerCapacity::new(2, 3)
            }
        );
    }

    #[test]
    fn scheduler_admission_short_circuits_existing_job() {
        assert_eq!(
            scheduler_admission(true, 99, 1, SchedulerAdmissionKind::Standard),
            SchedulerAdmissionOutcome::AlreadyScheduled
        );
    }

    #[test]
    fn schedule_transition_reasons_match_legacy_status_text() {
        assert_eq!(
            SchedulerAdmissionKind::Standard.transition_reason(),
            "Scheduled for execution"
        );
        assert_eq!(
            SchedulerAdmissionKind::ReservedSystem.transition_reason(),
            "Scheduled for execution (reserved slot)"
        );
    }

    #[test]
    fn subtask_cleanup_prefers_timeout_over_finished_state() {
        assert_eq!(
            subtask_cleanup_decision(false, false),
            SubtaskCleanupDecision::KeepWaiting
        );
        assert_eq!(
            subtask_cleanup_decision(false, true),
            SubtaskCleanupDecision::RemoveFinished
        );
        assert_eq!(
            subtask_cleanup_decision(true, true),
            SubtaskCleanupDecision::ForceRemoveTimedOut
        );
    }
}
