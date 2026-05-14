//! Worker runtime lifecycle helpers.

use std::time::{Duration, Instant};

use thinclaw_llm_core::{ChatMessage, Role};
use thinclaw_types::JobState;
use tokio::sync::watch;
use uuid::Uuid;

use crate::routine::RunStatus;

pub fn touch_worker_activity(activity_tx: &watch::Sender<Instant>) {
    let _ = activity_tx.send(Instant::now());
}

pub struct WorkerActivityKeepalive {
    cancel_tx: watch::Sender<bool>,
    join_handle: tokio::task::JoinHandle<()>,
}

impl WorkerActivityKeepalive {
    pub fn spawn(activity_tx: watch::Sender<Instant>, interval: Duration) -> Self {
        let (cancel_tx, mut cancel_rx) = watch::channel(false);
        let join_handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel_rx.changed() => break,
                    _ = tokio::time::sleep(interval) => {
                        touch_worker_activity(&activity_tx);
                    }
                }
            }
        });
        Self {
            cancel_tx,
            join_handle,
        }
    }
}

impl Drop for WorkerActivityKeepalive {
    fn drop(&mut self) {
        let _ = self.cancel_tx.send(true);
        self.join_handle.abort();
    }
}

/// Compact context messages after plan execution to prevent orphaned tool
/// result bloat.
///
/// Keeps system messages, the first user message, and a synthetic assistant
/// summary of the completed plan. Drops plan-era assistant/tool/user chatter.
pub fn compact_post_plan(messages: &mut Vec<ChatMessage>, plan_goal: &str) {
    let pre_count = messages.len();
    let pre_chars: usize = messages.iter().map(ChatMessage::estimated_chars).sum();

    let mut compacted = Vec::new();
    let mut first_user_seen = false;

    for msg in messages.iter() {
        match msg.role {
            Role::System => {
                compacted.push(msg.clone());
            }
            Role::User if !first_user_seen => {
                compacted.push(msg.clone());
                first_user_seen = true;
            }
            _ => {}
        }
    }

    compacted.push(ChatMessage::assistant(format!(
        "I executed a plan to accomplish: {}. \
         The plan has been completed. Now I'll check for any remaining work \
         or deliver final results.",
        plan_goal,
    )));

    let post_chars: usize = compacted.iter().map(ChatMessage::estimated_chars).sum();
    tracing::info!(
        "Post-plan compaction: {} messages ({} chars) -> {} messages ({} chars)",
        pre_count,
        pre_chars,
        compacted.len(),
        post_chars
    );

    *messages = compacted;
}

pub const MAX_WORKER_ITERATIONS: usize = 500;

pub fn capped_worker_iterations(requested: Option<u64>, default_value: usize) -> usize {
    (requested.unwrap_or(default_value as u64) as usize).min(MAX_WORKER_ITERATIONS)
}

pub fn is_worker_terminal_state(state: JobState) -> bool {
    matches!(
        state,
        JobState::Completed
            | JobState::Failed
            | JobState::Stuck
            | JobState::Cancelled
            | JobState::Abandoned
    )
}

pub fn build_worker_system_prompt(
    title: &str,
    description: &str,
    identity_block: Option<&str>,
) -> String {
    let identity_section = identity_block
        .map(|id| format!("\n\n---\n\n{id}"))
        .unwrap_or_default();

    format!(
        r#"You are an autonomous agent working on a job.

Job: {title}
Description: {description}

You have access to tools to complete this job. Plan your approach and execute tools as needed.
You may request multiple tools at once if they can be executed in parallel.

IMPORTANT: Use `emit_user_message` to send your results and findings to the user. This is \
how you deliver output — the user sees these messages in real-time in their chat interface. \
Use it for interim progress updates (message_type: "progress") and for your final results \
(message_type: "interim_result"). Do NOT just write results to memory files — the user needs \
to see them directly.

You can also use the `canvas` tool to display rich structured content (tables, panels, etc.) \
in the user's UI.

Report when the job is complete or if you encounter issues you cannot resolve.{identity_section}"#
    )
}

pub fn heartbeat_iteration_exhausted_summary(max_iterations: usize) -> String {
    format!(
        "Heartbeat ran out of iterations ({}/{}) before completing all checklist actions. \
         The agent may need a higher max_iterations setting, or the checklist \
         may contain tasks too complex for a single heartbeat run.",
        max_iterations, max_iterations
    )
}

pub fn heartbeat_iteration_exhausted_user_message(max_iterations: usize) -> String {
    format!(
        "⚠️ Heartbeat incomplete — ran out of tool iterations ({}/{}). \
         Some checklist actions may not have been completed. \
         You can increase the iteration budget in Settings → Heartbeat → Max iterations, \
         or help me finish by prompting me directly.",
        max_iterations, max_iterations
    )
}

pub fn heartbeat_iteration_exhausted_critique(
    job_id: Uuid,
    max_iterations: usize,
) -> serde_json::Value {
    serde_json::json!({
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "job_id": job_id.to_string(),
        "quality": 0,
        "reasoning": format!(
            "Heartbeat exhausted all {} iterations without completing. \
             Partial work may have been saved. Pick up where the previous \
             run left off — check MEMORY.md and daily logs for what was \
             already done, then continue.",
            max_iterations
        ),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineFinalizationOutcome {
    pub status: RunStatus,
    pub event: &'static str,
    pub summary: String,
    pub job_user_id: Option<String>,
    pub job_actor_id: Option<String>,
}

impl RoutineFinalizationOutcome {
    pub fn from_job_state(
        state: JobState,
        reason: Option<String>,
        user_id: impl Into<String>,
        actor_id: impl Into<String>,
    ) -> Self {
        let job_user_id = Some(user_id.into());
        let job_actor_id = Some(actor_id.into());
        match state {
            JobState::Completed => Self {
                status: RunStatus::Ok,
                event: "completed",
                summary: "Job completed successfully".to_string(),
                job_user_id,
                job_actor_id,
            },
            JobState::Failed => Self {
                status: RunStatus::Failed,
                event: "failed",
                summary: reason.unwrap_or_else(|| "Job failed".to_string()),
                job_user_id,
                job_actor_id,
            },
            JobState::Stuck => Self {
                status: RunStatus::Failed,
                event: "failed",
                summary: reason.unwrap_or_else(|| "Job stuck".to_string()),
                job_user_id,
                job_actor_id,
            },
            JobState::Cancelled => Self {
                status: RunStatus::Failed,
                event: "failed",
                summary: "Job cancelled".to_string(),
                job_user_id,
                job_actor_id,
            },
            JobState::Abandoned => Self {
                status: RunStatus::Failed,
                event: "failed",
                summary: reason.unwrap_or_else(|| "Job abandoned".to_string()),
                job_user_id,
                job_actor_id,
            },
            other => Self {
                status: RunStatus::Failed,
                event: "failed",
                summary: format!("Job ended in unexpected state: {:?}", other),
                job_user_id,
                job_actor_id,
            },
        }
    }

    pub fn from_context_error(error: impl std::fmt::Display) -> Self {
        Self {
            status: RunStatus::Failed,
            event: "failed",
            summary: format!("Could not read final job state: {error}"),
            job_user_id: None,
            job_actor_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn touch_worker_activity_updates_watch_value() {
        let before = Instant::now();
        let (tx, mut rx) = watch::channel(before);

        touch_worker_activity(&tx);
        rx.changed().await.unwrap();

        assert!(*rx.borrow() >= before);
    }

    #[tokio::test]
    async fn keepalive_can_be_dropped_without_waiting() {
        let (tx, _rx) = watch::channel(Instant::now());
        let keepalive = WorkerActivityKeepalive::spawn(tx, Duration::from_millis(1));
        drop(keepalive);
    }

    #[test]
    fn compact_post_plan_keeps_system_and_first_user() {
        let mut messages = vec![
            ChatMessage::system("system"),
            ChatMessage::user("first"),
            ChatMessage::assistant("tool plan"),
            ChatMessage::tool_result("call-1", "tool", "result"),
            ChatMessage::user("follow-up"),
        ];

        compact_post_plan(&mut messages, "test goal");

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, Role::System);
        assert_eq!(messages[1].role, Role::User);
        assert_eq!(messages[1].content, "first");
        assert_eq!(messages[2].role, Role::Assistant);
        assert!(messages[2].content.contains("test goal"));
    }

    #[test]
    fn worker_prompt_includes_identity_when_present() {
        let prompt = build_worker_system_prompt("title", "description", Some("identity"));

        assert!(prompt.contains("Job: title"));
        assert!(prompt.contains("Description: description"));
        assert!(prompt.contains("identity"));
    }

    #[test]
    fn worker_iteration_cap_is_enforced() {
        assert_eq!(capped_worker_iterations(None, 50), 50);
        assert_eq!(capped_worker_iterations(Some(10), 50), 10);
        assert_eq!(
            capped_worker_iterations(Some((MAX_WORKER_ITERATIONS + 1) as u64), 50),
            MAX_WORKER_ITERATIONS
        );
    }

    #[test]
    fn completed_is_terminal_for_worker_cleanup() {
        assert!(is_worker_terminal_state(JobState::Completed));
        assert!(!is_worker_terminal_state(JobState::InProgress));
    }

    #[test]
    fn routine_finalization_maps_completed_job_to_ok() {
        let outcome =
            RoutineFinalizationOutcome::from_job_state(JobState::Completed, None, "user", "actor");

        assert_eq!(outcome.status, RunStatus::Ok);
        assert_eq!(outcome.event, "completed");
        assert_eq!(outcome.summary, "Job completed successfully");
        assert_eq!(outcome.job_user_id.as_deref(), Some("user"));
        assert_eq!(outcome.job_actor_id.as_deref(), Some("actor"));
    }

    #[test]
    fn routine_finalization_preserves_failure_reason() {
        let outcome = RoutineFinalizationOutcome::from_job_state(
            JobState::Failed,
            Some("tool failed".to_string()),
            "user",
            "actor",
        );

        assert_eq!(outcome.status, RunStatus::Failed);
        assert_eq!(outcome.event, "failed");
        assert_eq!(outcome.summary, "tool failed");
    }
}
