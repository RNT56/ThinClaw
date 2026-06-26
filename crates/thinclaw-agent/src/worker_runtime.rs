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
pub const DEFAULT_WORKER_ITERATIONS: usize = 50;
pub const WORKER_STUCK_NUDGE_AFTER_ITERATION: usize = 8;
pub const WORKER_STUCK_NUDGE_EVERY: usize = 10;
pub const WORKER_DIRECT_LOOP_DELAY_MS: u64 = 100;
pub const WORKER_TOOL_KEEPALIVE_SECS: u64 = 15;
pub const WORKER_TASK_FAILED_DURING_EXECUTION_REASON: &str = "Task failed during execution";

pub fn capped_worker_iterations(requested: Option<u64>, default_value: usize) -> usize {
    (requested.unwrap_or(default_value as u64) as usize).min(MAX_WORKER_ITERATIONS)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerLoopMetadata {
    pub max_iterations: usize,
    pub is_heartbeat: bool,
    pub allowed_tools: Option<Vec<String>>,
    pub allowed_skills: Option<Vec<String>>,
    /// When true, suppress user-visible output delivery (heartbeat `target=none`).
    pub suppress_output: bool,
    /// Channel override for output delivery (heartbeat `target=<channel>`).
    pub notify_channel: Option<String>,
}

impl WorkerLoopMetadata {
    pub fn from_metadata(metadata: &serde_json::Value, default_iterations: usize) -> Self {
        Self {
            max_iterations: capped_worker_iterations(
                metadata
                    .get("max_iterations")
                    .and_then(|value| value.as_u64()),
                default_iterations,
            ),
            is_heartbeat: metadata
                .get("heartbeat")
                .and_then(|value| value.as_bool())
                .unwrap_or(false),
            allowed_tools: metadata_string_list(metadata, "allowed_tools"),
            allowed_skills: metadata_string_list(metadata, "allowed_skills"),
            suppress_output: metadata
                .get("suppress_output")
                .and_then(|value| value.as_bool())
                .unwrap_or(false),
            notify_channel: metadata
                .get("notify_channel")
                .and_then(|value| value.as_str())
                .filter(|value| !value.is_empty())
                .map(str::to_string),
        }
    }
}

pub fn metadata_string_list(metadata: &serde_json::Value, key: &str) -> Option<Vec<String>> {
    metadata.get(key).and_then(|value| {
        value.as_array().map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
    })
}

pub fn worker_iteration_exceeded(iteration: usize, max_iterations: usize) -> bool {
    iteration > max_iterations
}

pub fn should_nudge_worker(iteration: usize) -> bool {
    iteration > WORKER_STUCK_NUDGE_AFTER_ITERATION
        && iteration.is_multiple_of(WORKER_STUCK_NUDGE_EVERY)
}

pub fn should_finish_heartbeat_after_output(is_heartbeat: bool, has_output: bool) -> bool {
    is_heartbeat && has_output
}

pub fn should_persist_heartbeat_completion_critique(success: bool, quality_score: u32) -> bool {
    !success || quality_score < 100
}

pub fn heartbeat_completion_critique(
    job_id: Uuid,
    quality_score: u32,
    reasoning: impl Into<String>,
) -> serde_json::Value {
    serde_json::json!({
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "job_id": job_id.to_string(),
        "quality": quality_score,
        "reasoning": reasoning.into(),
    })
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

/// Reorder joined parallel worker results into request order.
///
/// Root adapters own task spawning and concrete error construction. This helper
/// only applies the deterministic policy used after joins finish: successful
/// results are placed at their original index, and missing slots receive join
/// failure reasons in arrival order before falling back to the default reason.
pub fn order_parallel_worker_results<T>(
    count: usize,
    completed: impl IntoIterator<Item = (usize, T)>,
    failed_reasons: impl IntoIterator<Item = String>,
    default_missing_reason: &str,
) -> Vec<Result<T, String>> {
    let mut ordered: Vec<Option<T>> = (0..count).map(|_| None).collect();
    for (idx, result) in completed {
        if idx < count {
            ordered[idx] = Some(result);
        }
    }

    let mut failed_reasons = failed_reasons.into_iter();
    ordered
        .into_iter()
        .map(|result| {
            result.ok_or_else(|| {
                failed_reasons
                    .next()
                    .unwrap_or_else(|| default_missing_reason.to_string())
            })
        })
        .collect()
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
    fn worker_loop_metadata_extracts_heartbeat_iterations_and_capabilities() {
        let metadata = serde_json::json!({
            "heartbeat": true,
            "max_iterations": 12,
            "allowed_tools": ["shell", 5, "read_file"],
            "allowed_skills": ["github"]
        });

        let parsed = WorkerLoopMetadata::from_metadata(&metadata, DEFAULT_WORKER_ITERATIONS);

        assert_eq!(parsed.max_iterations, 12);
        assert!(parsed.is_heartbeat);
        assert_eq!(
            parsed.allowed_tools,
            Some(vec!["shell".to_string(), "read_file".to_string()])
        );
        assert_eq!(parsed.allowed_skills, Some(vec!["github".to_string()]));
    }

    #[test]
    fn worker_loop_metadata_defaults_missing_fields() {
        let parsed =
            WorkerLoopMetadata::from_metadata(&serde_json::Value::Null, DEFAULT_WORKER_ITERATIONS);

        assert_eq!(parsed.max_iterations, DEFAULT_WORKER_ITERATIONS);
        assert!(!parsed.is_heartbeat);
        assert!(parsed.allowed_tools.is_none());
        assert!(parsed.allowed_skills.is_none());
    }

    #[test]
    fn worker_loop_metadata_caps_requested_iterations() {
        let metadata = serde_json::json!({
            "max_iterations": (MAX_WORKER_ITERATIONS as u64) + 100
        });

        let parsed = WorkerLoopMetadata::from_metadata(&metadata, DEFAULT_WORKER_ITERATIONS);

        assert_eq!(parsed.max_iterations, MAX_WORKER_ITERATIONS);
    }

    #[test]
    fn worker_loop_iteration_policy_matches_legacy_boundaries() {
        assert!(!worker_iteration_exceeded(50, 50));
        assert!(worker_iteration_exceeded(51, 50));
        assert!(!should_nudge_worker(8));
        assert!(should_nudge_worker(10));
        assert!(!should_nudge_worker(11));
        assert!(should_finish_heartbeat_after_output(true, true));
        assert!(!should_finish_heartbeat_after_output(true, false));
        assert!(!should_finish_heartbeat_after_output(false, true));
    }

    #[test]
    fn heartbeat_completion_critique_policy_flags_imperfect_runs() {
        assert!(!should_persist_heartbeat_completion_critique(true, 100));
        assert!(should_persist_heartbeat_completion_critique(true, 99));
        assert!(should_persist_heartbeat_completion_critique(false, 100));

        let job_id = Uuid::new_v4();
        let critique = heartbeat_completion_critique(job_id, 80, "needs follow-up");
        assert_eq!(critique["job_id"], job_id.to_string());
        assert_eq!(critique["quality"], 80);
        assert_eq!(critique["reasoning"], "needs follow-up");
    }

    #[test]
    fn completed_is_terminal_for_worker_cleanup() {
        assert!(is_worker_terminal_state(JobState::Completed));
        assert!(!is_worker_terminal_state(JobState::InProgress));
    }

    #[test]
    fn parallel_worker_results_are_reordered_by_original_index() {
        let ordered = order_parallel_worker_results(
            3,
            vec![(2, "third"), (0, "first"), (1, "second")],
            Vec::new(),
            WORKER_TASK_FAILED_DURING_EXECUTION_REASON,
        );

        assert_eq!(ordered, vec![Ok("first"), Ok("second"), Ok("third")]);
    }

    #[test]
    fn parallel_worker_results_fill_missing_slots_with_join_reasons() {
        let ordered = order_parallel_worker_results(
            3,
            vec![(1, "second")],
            vec!["Task panicked: boom".to_string()],
            WORKER_TASK_FAILED_DURING_EXECUTION_REASON,
        );

        assert_eq!(
            ordered,
            vec![
                Err("Task panicked: boom".to_string()),
                Ok("second"),
                Err(WORKER_TASK_FAILED_DURING_EXECUTION_REASON.to_string())
            ]
        );
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
