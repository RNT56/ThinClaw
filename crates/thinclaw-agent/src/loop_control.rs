//! Shared policy for bounded long-running agent loops.
//!
//! This module is deliberately runtime-agnostic: root crates own concrete
//! task spawning, cancellation channels, database leases, and metrics sinks.
//! The policy here gives those loops one vocabulary for budgets and terminal
//! stop reasons so dispatcher, worker, subagent, routine, outcome, and
//! supervisor code cannot drift into subtly different semantics.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopKind {
    AgentDispatcher,
    Worker,
    Subagent,
    RoutineCron,
    RoutineEventQueue,
    RoutineTriggerQueue,
    RoutineNotificationForwarder,
    RoutineZombieReaper,
    OutcomeService,
    RepoProjectSupervisor,
    SelfRepair,
    SessionPruning,
    JobContextPruning,
    Maintenance,
}

impl LoopKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AgentDispatcher => "agent_dispatcher",
            Self::Worker => "worker",
            Self::Subagent => "subagent",
            Self::RoutineCron => "routine_cron",
            Self::RoutineEventQueue => "routine_event_queue",
            Self::RoutineTriggerQueue => "routine_trigger_queue",
            Self::RoutineNotificationForwarder => "routine_notification_forwarder",
            Self::RoutineZombieReaper => "routine_zombie_reaper",
            Self::OutcomeService => "outcome_service",
            Self::RepoProjectSupervisor => "repo_project_supervisor",
            Self::SelfRepair => "self_repair",
            Self::SessionPruning => "session_pruning",
            Self::JobContextPruning => "job_context_pruning",
            Self::Maintenance => "maintenance",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopStopReason {
    Completed,
    NoWork,
    Cancelled,
    Interrupted,
    ChannelClosed,
    ExternalShutdown,
    IdleTimeout,
    IterationBudgetExceeded,
    RetryBudgetExceeded,
    WallTimeBudgetExceeded,
    FatalError,
}

impl LoopStopReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::NoWork => "no_work",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
            Self::ChannelClosed => "channel_closed",
            Self::ExternalShutdown => "external_shutdown",
            Self::IdleTimeout => "idle_timeout",
            Self::IterationBudgetExceeded => "iteration_budget_exceeded",
            Self::RetryBudgetExceeded => "retry_budget_exceeded",
            Self::WallTimeBudgetExceeded => "wall_time_budget_exceeded",
            Self::FatalError => "fatal_error",
        }
    }

    pub fn is_failure(self) -> bool {
        matches!(
            self,
            Self::IterationBudgetExceeded
                | Self::RetryBudgetExceeded
                | Self::WallTimeBudgetExceeded
                | Self::FatalError
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoopBudget {
    pub max_iterations: Option<usize>,
    pub max_retries: Option<u32>,
    pub max_wall_time: Option<Duration>,
    pub idle_timeout: Option<Duration>,
}

impl LoopBudget {
    pub const UNBOUNDED: Self = Self {
        max_iterations: None,
        max_retries: None,
        max_wall_time: None,
        idle_timeout: None,
    };

    pub fn iterations(max_iterations: usize) -> Self {
        Self {
            max_iterations: Some(max_iterations),
            ..Self::UNBOUNDED
        }
    }

    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = Some(max_retries);
        self
    }

    pub fn with_max_wall_time(mut self, max_wall_time: Duration) -> Self {
        self.max_wall_time = Some(max_wall_time);
        self
    }

    pub fn with_idle_timeout(mut self, idle_timeout: Duration) -> Self {
        self.idle_timeout = Some(idle_timeout);
        self
    }

    pub fn capped_iterations(
        requested: Option<u64>,
        default_value: usize,
        hard_cap: usize,
    ) -> usize {
        (requested.unwrap_or(default_value as u64) as usize).min(hard_cap)
    }

    pub fn iteration_stop_reason(self, iteration: usize) -> Option<LoopStopReason> {
        self.max_iterations
            .is_some_and(|max| iteration > max)
            .then_some(LoopStopReason::IterationBudgetExceeded)
    }

    pub fn retry_stop_reason(self, retries_used: u32) -> Option<LoopStopReason> {
        self.max_retries
            .is_some_and(|max| retries_used > max)
            .then_some(LoopStopReason::RetryBudgetExceeded)
    }

    pub fn wall_time_stop_reason(self, started_at: Instant) -> Option<LoopStopReason> {
        self.max_wall_time
            .is_some_and(|max| started_at.elapsed() > max)
            .then_some(LoopStopReason::WallTimeBudgetExceeded)
    }

    pub fn idle_stop_reason(self, last_activity_at: Instant) -> Option<LoopStopReason> {
        self.idle_timeout
            .is_some_and(|max| last_activity_at.elapsed() > max)
            .then_some(LoopStopReason::IdleTimeout)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoopRetryPolicy {
    max_retries: u32,
    initial_delay: Duration,
    max_delay: Duration,
}

impl LoopRetryPolicy {
    pub fn bounded(max_retries: u32, initial_delay: Duration, max_delay: Duration) -> Self {
        Self {
            max_retries,
            initial_delay,
            max_delay,
        }
    }

    pub fn delay_for_retry(self, retries_used: u32) -> Option<Duration> {
        if retries_used >= self.max_retries {
            return None;
        }
        let shift = retries_used.min(31);
        let factor = 1u32.checked_shl(shift).unwrap_or(u32::MAX);
        Some((self.initial_delay * factor).min(self.max_delay))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopRunSummary {
    pub kind: LoopKind,
    pub stop_reason: LoopStopReason,
    pub iterations: usize,
    pub retries: u32,
}

impl LoopRunSummary {
    pub fn new(
        kind: LoopKind,
        stop_reason: LoopStopReason,
        iterations: usize,
        retries: u32,
    ) -> Self {
        Self {
            kind,
            stop_reason,
            iterations,
            retries,
        }
    }

    pub fn labels(&self) -> [(&'static str, &'static str); 2] {
        [
            ("loop", self.kind.as_str()),
            ("stop_reason", self.stop_reason.as_str()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_caps_iterations_and_reports_stop_reason() {
        let max = LoopBudget::capped_iterations(Some(1_000), 50, 500);
        assert_eq!(max, 500);

        let budget = LoopBudget::iterations(3);
        assert_eq!(budget.iteration_stop_reason(3), None);
        assert_eq!(
            budget.iteration_stop_reason(4),
            Some(LoopStopReason::IterationBudgetExceeded)
        );
    }

    #[test]
    fn retry_policy_is_bounded_and_capped() {
        let policy =
            LoopRetryPolicy::bounded(3, Duration::from_millis(100), Duration::from_secs(1));
        assert_eq!(policy.delay_for_retry(0), Some(Duration::from_millis(100)));
        assert_eq!(policy.delay_for_retry(1), Some(Duration::from_millis(200)));
        assert_eq!(policy.delay_for_retry(2), Some(Duration::from_millis(400)));
        assert_eq!(policy.delay_for_retry(3), None);

        let capped = LoopRetryPolicy::bounded(10, Duration::from_secs(1), Duration::from_secs(3));
        assert_eq!(capped.delay_for_retry(8), Some(Duration::from_secs(3)));
    }

    #[test]
    fn summaries_expose_stable_labels() {
        let summary = LoopRunSummary::new(
            LoopKind::RepoProjectSupervisor,
            LoopStopReason::ExternalShutdown,
            7,
            1,
        );

        assert_eq!(
            summary.labels(),
            [
                ("loop", "repo_project_supervisor"),
                ("stop_reason", "external_shutdown")
            ]
        );
        assert!(!summary.stop_reason.is_failure());
        assert!(LoopStopReason::FatalError.is_failure());
    }
}
