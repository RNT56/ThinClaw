//! Routine audit log.
//!
//! Ring-buffer storage of routine execution records with filtering
//! by name, outcome, and success rate calculation.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Outcome of a routine execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RoutineOutcome {
    Success { duration_ms: u64 },
    Failed { error: String, duration_ms: u64 },
    Skipped { reason: String },
    TimedOut { timeout_ms: u64 },
}

impl RoutineOutcome {
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success { .. })
    }

    pub fn is_failure(&self) -> bool {
        matches!(self, Self::Failed { .. } | Self::TimedOut { .. })
    }
}

/// How the routine was triggered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TriggerKind {
    Cron,
    Manual,
    Event(String),
    Webhook,
}

/// A single routine execution record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutineRun {
    pub id: String,
    pub routine_name: String,
    pub triggered_by: TriggerKind,
    pub started_at: String,
    pub outcome: RoutineOutcome,
    pub job_id: Option<String>,
    pub agent_id: Option<String>,
}

/// Audit log for routine executions.
pub struct RoutineAuditLog {
    runs: VecDeque<RoutineRun>,
    max_entries: usize,
}

impl RoutineAuditLog {
    pub fn new(max_entries: usize) -> Self {
        Self {
            runs: VecDeque::with_capacity(max_entries.min(1024)),
            max_entries,
        }
    }

    /// Push a new run, evicting oldest if at capacity.
    pub fn push(&mut self, run: RoutineRun) {
        if self.runs.len() >= self.max_entries {
            self.runs.pop_front();
        }
        self.runs.push_back(run);
    }

    /// List all runs (newest last).
    pub fn list(&self) -> Vec<&RoutineRun> {
        self.runs.iter().collect()
    }

    /// Filter by routine name.
    pub fn by_routine(&self, name: &str) -> Vec<&RoutineRun> {
        self.runs
            .iter()
            .filter(|r| r.routine_name == name)
            .collect()
    }

    /// Get only failed/timed-out runs.
    pub fn failures(&self) -> Vec<&RoutineRun> {
        self.runs
            .iter()
            .filter(|r| r.outcome.is_failure())
            .collect()
    }

    /// Success rate for a named routine (0.0-1.0).
    pub fn success_rate(&self, routine_name: &str) -> f32 {
        let runs: Vec<_> = self.by_routine(routine_name);
        if runs.is_empty() {
            return 0.0;
        }
        let successes = runs.iter().filter(|r| r.outcome.is_success()).count();
        successes as f32 / runs.len() as f32
    }

    /// Most recent run for a named routine.
    pub fn last_run(&self, routine_name: &str) -> Option<&RoutineRun> {
        self.runs
            .iter()
            .rev()
            .find(|r| r.routine_name == routine_name)
    }

    pub fn len(&self) -> usize {
        self.runs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_run(name: &str, outcome: RoutineOutcome) -> RoutineRun {
        RoutineRun {
            id: format!("run-{}", name),
            routine_name: name.into(),
            triggered_by: TriggerKind::Cron,
            started_at: "2026-01-01T00:00:00Z".into(),
            outcome,
            job_id: None,
            agent_id: None,
        }
    }

    #[test]
    fn test_push_and_list() {
        let mut log = RoutineAuditLog::new(10);
        log.push(make_run(
            "daily",
            RoutineOutcome::Success { duration_ms: 100 },
        ));
        assert_eq!(log.list().len(), 1);
    }

    #[test]
    fn test_by_routine_filter() {
        let mut log = RoutineAuditLog::new(10);
        log.push(make_run("a", RoutineOutcome::Success { duration_ms: 10 }));
        log.push(make_run("b", RoutineOutcome::Success { duration_ms: 20 }));
        assert_eq!(log.by_routine("a").len(), 1);
    }

    #[test]
    fn test_failures_filter() {
        let mut log = RoutineAuditLog::new(10);
        log.push(make_run("x", RoutineOutcome::Success { duration_ms: 10 }));
        log.push(make_run(
            "y",
            RoutineOutcome::Failed {
                error: "boom".into(),
                duration_ms: 5,
            },
        ));
        assert_eq!(log.failures().len(), 1);
    }

    #[test]
    fn test_success_rate() {
        let mut log = RoutineAuditLog::new(10);
        log.push(make_run("r", RoutineOutcome::Success { duration_ms: 10 }));
        log.push(make_run(
            "r",
            RoutineOutcome::Failed {
                error: "e".into(),
                duration_ms: 5,
            },
        ));
        assert!((log.success_rate("r") - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_success_rate_no_runs() {
        let log = RoutineAuditLog::new(10);
        assert_eq!(log.success_rate("none"), 0.0);
    }

    #[test]
    fn test_last_run() {
        let mut log = RoutineAuditLog::new(10);
        let mut run1 = make_run("r", RoutineOutcome::Success { duration_ms: 1 });
        run1.id = "first".into();
        let mut run2 = make_run("r", RoutineOutcome::Success { duration_ms: 2 });
        run2.id = "second".into();
        log.push(run1);
        log.push(run2);
        assert_eq!(log.last_run("r").unwrap().id, "second");
    }

    #[test]
    fn test_eviction_at_capacity() {
        let mut log = RoutineAuditLog::new(2);
        log.push(make_run("a", RoutineOutcome::Success { duration_ms: 1 }));
        log.push(make_run("b", RoutineOutcome::Success { duration_ms: 2 }));
        log.push(make_run("c", RoutineOutcome::Success { duration_ms: 3 }));
        assert_eq!(log.len(), 2);
        // First entry "a" should have been evicted
        assert!(log.by_routine("a").is_empty());
    }

    #[test]
    fn test_trigger_kinds() {
        assert_eq!(TriggerKind::Cron, TriggerKind::Cron);
        assert_eq!(TriggerKind::Manual, TriggerKind::Manual);
        assert_eq!(TriggerKind::Webhook, TriggerKind::Webhook);
        assert_ne!(TriggerKind::Cron, TriggerKind::Manual);
    }

    #[test]
    fn test_outcome_helpers() {
        assert!(RoutineOutcome::Success { duration_ms: 1 }.is_success());
        assert!(!RoutineOutcome::Success { duration_ms: 1 }.is_failure());
        assert!(
            RoutineOutcome::Failed {
                error: "e".into(),
                duration_ms: 1
            }
            .is_failure()
        );
        assert!(RoutineOutcome::TimedOut { timeout_ms: 100 }.is_failure());
        assert!(!RoutineOutcome::Skipped { reason: "r".into() }.is_failure());
    }
}
