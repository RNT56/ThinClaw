//! Shared job execution context types.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// State of a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Pending,
    InProgress,
    Completed,
    Submitted,
    Accepted,
    Failed,
    Stuck,
    Cancelled,
    Abandoned,
}

impl JobState {
    /// Check if this state allows transitioning to another state.
    pub fn can_transition_to(&self, target: JobState) -> bool {
        use JobState::*;

        matches!(
            (self, target),
            (Pending, InProgress)
                | (Pending, Cancelled)
                | (InProgress, Completed)
                | (InProgress, Failed)
                | (InProgress, Stuck)
                | (InProgress, Cancelled)
                | (Completed, Submitted)
                | (Completed, Failed)
                | (Submitted, Accepted)
                | (Submitted, Failed)
                | (Stuck, InProgress)
                | (Stuck, Failed)
                | (Stuck, Cancelled)
                | (Stuck, Abandoned)
                | (Pending, Abandoned)
                | (InProgress, Abandoned)
        )
    }

    /// Check if this is a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Accepted | Self::Failed | Self::Cancelled | Self::Abandoned
        )
    }

    /// Check if the job is active (not terminal).
    pub fn is_active(&self) -> bool {
        !self.is_terminal()
    }
}

impl std::fmt::Display for JobState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Submitted => "submitted",
            Self::Accepted => "accepted",
            Self::Failed => "failed",
            Self::Stuck => "stuck",
            Self::Cancelled => "cancelled",
            Self::Abandoned => "abandoned",
        };
        write!(f, "{s}")
    }
}

/// A state transition event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateTransition {
    pub from: JobState,
    pub to: JobState,
    pub timestamp: DateTime<Utc>,
    pub reason: Option<String>,
}

/// Context for a running job.
#[derive(Debug, Clone, Serialize)]
pub struct JobContext {
    pub job_id: Uuid,
    pub state: JobState,
    pub user_id: String,
    pub principal_id: String,
    #[serde(default)]
    pub actor_id: Option<String>,
    pub conversation_id: Option<Uuid>,
    pub title: String,
    pub description: String,
    pub category: Option<String>,
    pub budget: Option<Decimal>,
    pub budget_token: Option<String>,
    pub bid_amount: Option<Decimal>,
    pub estimated_cost: Option<Decimal>,
    pub estimated_duration: Option<Duration>,
    pub actual_cost: Decimal,
    pub total_tokens_used: u64,
    pub max_tokens: u64,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub repair_attempts: u32,
    pub transitions: Vec<StateTransition>,
    pub metadata: serde_json::Value,
    #[serde(skip)]
    pub extra_env: Arc<HashMap<String, String>>,
}

impl JobContext {
    pub fn new(title: impl Into<String>, description: impl Into<String>) -> Self {
        Self::with_identity("default", "default", title, description)
    }

    pub fn with_user(
        user_id: impl Into<String>,
        title: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        let user_id = user_id.into();
        Self::with_identity(user_id.clone(), user_id, title, description)
    }

    pub fn with_identity(
        principal_id: impl Into<String>,
        actor_id: impl Into<String>,
        title: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        let principal_id = principal_id.into();
        let actor_id = actor_id.into();
        Self {
            job_id: Uuid::new_v4(),
            state: JobState::Pending,
            user_id: principal_id.clone(),
            principal_id,
            actor_id: Some(actor_id),
            conversation_id: None,
            title: title.into(),
            description: description.into(),
            category: None,
            budget: None,
            budget_token: None,
            bid_amount: None,
            estimated_cost: None,
            estimated_duration: None,
            actual_cost: Decimal::ZERO,
            total_tokens_used: 0,
            max_tokens: 0,
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
            repair_attempts: 0,
            transitions: Vec::new(),
            extra_env: Arc::new(HashMap::new()),
            metadata: serde_json::Value::Null,
        }
    }

    pub fn with_user_and_actor(
        user_id: impl Into<String>,
        actor_id: impl Into<String>,
        title: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            actor_id: Some(actor_id.into()),
            ..Self::with_user(user_id, title, description)
        }
    }

    pub fn owner_actor_id(&self) -> &str {
        self.actor_id.as_deref().unwrap_or(&self.user_id)
    }

    pub fn transition_to(
        &mut self,
        new_state: JobState,
        reason: Option<String>,
    ) -> Result<(), String> {
        if !self.state.can_transition_to(new_state) {
            return Err(format!(
                "Cannot transition from {} to {}",
                self.state, new_state
            ));
        }

        self.transitions.push(StateTransition {
            from: self.state,
            to: new_state,
            timestamp: Utc::now(),
            reason,
        });

        const MAX_TRANSITIONS: usize = 200;
        if self.transitions.len() > MAX_TRANSITIONS {
            let drain_count = self.transitions.len() - MAX_TRANSITIONS;
            self.transitions.drain(..drain_count);
        }

        self.state = new_state;

        match new_state {
            JobState::InProgress if self.started_at.is_none() => {
                self.started_at = Some(Utc::now());
            }
            JobState::Completed
            | JobState::Accepted
            | JobState::Failed
            | JobState::Cancelled
            | JobState::Abandoned => {
                self.completed_at = Some(Utc::now());
            }
            _ => {}
        }

        Ok(())
    }

    pub fn add_cost(&mut self, cost: Decimal) {
        self.actual_cost += cost;
    }

    pub fn add_tokens(&mut self, tokens: u64) -> Result<(), String> {
        self.total_tokens_used += tokens;
        if self.max_tokens > 0 && self.total_tokens_used > self.max_tokens {
            Err(format!(
                "Token budget exceeded: used {} of {} allowed tokens",
                self.total_tokens_used, self.max_tokens
            ))
        } else {
            Ok(())
        }
    }

    pub fn budget_exceeded(&self) -> bool {
        self.budget
            .as_ref()
            .is_some_and(|budget| self.actual_cost > *budget)
    }

    pub fn elapsed(&self) -> Option<Duration> {
        self.started_at.map(|start| {
            let end = self.completed_at.unwrap_or_else(Utc::now);
            let duration = end.signed_duration_since(start);
            Duration::from_secs(duration.num_seconds().max(0) as u64)
        })
    }

    pub fn mark_stuck(&mut self, reason: impl Into<String>) -> Result<(), String> {
        self.transition_to(JobState::Stuck, Some(reason.into()))
    }

    pub fn attempt_recovery(&mut self) -> Result<(), String> {
        if self.state != JobState::Stuck {
            return Err("Job is not stuck".to_string());
        }
        self.repair_attempts += 1;
        self.transition_to(JobState::InProgress, Some("Recovery attempt".to_string()))
    }
}

impl Default for JobContext {
    fn default() -> Self {
        Self::with_identity("default", "default", "Untitled", "No description")
    }
}
