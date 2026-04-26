//! Job state machine compatibility facade.

pub use thinclaw_types::job::{JobContext, JobState, StateTransition};

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    #[test]
    fn test_state_transitions() {
        assert!(JobState::Pending.can_transition_to(JobState::InProgress));
        assert!(JobState::InProgress.can_transition_to(JobState::Completed));
        assert!(!JobState::Completed.can_transition_to(JobState::Pending));
        assert!(!JobState::Accepted.can_transition_to(JobState::InProgress));
    }

    #[test]
    fn test_terminal_states() {
        assert!(JobState::Accepted.is_terminal());
        assert!(JobState::Failed.is_terminal());
        assert!(JobState::Cancelled.is_terminal());
        assert!(!JobState::InProgress.is_terminal());
    }

    #[test]
    fn test_job_context_transitions() {
        let mut ctx = JobContext::new("Test", "Test job");
        assert_eq!(ctx.state, JobState::Pending);

        ctx.transition_to(JobState::InProgress, None).unwrap();
        assert_eq!(ctx.state, JobState::InProgress);
        assert!(ctx.started_at.is_some());

        ctx.transition_to(JobState::Completed, Some("Done".to_string()))
            .unwrap();
        assert_eq!(ctx.state, JobState::Completed);
    }

    #[test]
    fn test_transition_history_capped() {
        let mut ctx = JobContext::new("Test", "Transition cap test");
        ctx.transition_to(JobState::InProgress, None).unwrap();
        for i in 0..250 {
            ctx.mark_stuck(format!("stuck {i}")).unwrap();
            ctx.attempt_recovery().unwrap();
        }
        assert!(
            ctx.transitions.len() <= 200,
            "transitions should be capped at 200, got {}",
            ctx.transitions.len()
        );
    }

    #[test]
    fn test_add_tokens_enforces_budget() {
        let mut ctx = JobContext::new("Test", "Budget test");
        ctx.max_tokens = 1000;
        assert!(ctx.add_tokens(500).is_ok());
        assert_eq!(ctx.total_tokens_used, 500);
        assert!(ctx.add_tokens(600).is_err());
        assert_eq!(ctx.total_tokens_used, 1100);
    }

    #[test]
    fn test_add_tokens_unlimited() {
        let mut ctx = JobContext::new("Test", "No budget");
        assert!(ctx.add_tokens(1_000_000).is_ok());
    }

    #[test]
    fn test_budget_exceeded() {
        let mut ctx = JobContext::new("Test", "Money test");
        ctx.budget = Some(Decimal::new(100, 0));
        assert!(!ctx.budget_exceeded());
        ctx.add_cost(Decimal::new(50, 0));
        assert!(!ctx.budget_exceeded());
        ctx.add_cost(Decimal::new(60, 0));
        assert!(ctx.budget_exceeded());
    }

    #[test]
    fn test_budget_exceeded_none() {
        let ctx = JobContext::new("Test", "No budget");
        assert!(!ctx.budget_exceeded());
    }

    #[test]
    fn test_stuck_recovery() {
        let mut ctx = JobContext::new("Test", "Test job");
        ctx.transition_to(JobState::InProgress, None).unwrap();
        ctx.mark_stuck("Timed out").unwrap();
        assert_eq!(ctx.state, JobState::Stuck);

        ctx.attempt_recovery().unwrap();
        assert_eq!(ctx.state, JobState::InProgress);
        assert_eq!(ctx.repair_attempts, 1);
    }
}
