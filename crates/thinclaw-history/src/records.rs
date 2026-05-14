use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Record for an LLM call to be persisted.
#[derive(Debug, Clone)]
pub struct LlmCallRecord<'a> {
    pub job_id: Option<Uuid>,
    pub conversation_id: Option<Uuid>,
    pub provider: &'a str,
    pub model: &'a str,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost: Decimal,
    pub purpose: Option<&'a str>,
}

/// Whether a conversation is a one-to-one direct thread or a shared group thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationKind {
    Direct,
    Group,
}

impl ConversationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Group => "group",
        }
    }

    pub fn from_db(value: Option<&str>) -> Self {
        match value {
            Some("group") => Self::Group,
            _ => Self::Direct,
        }
    }
}

/// Stable conversation scope shared across channels for the same direct or group thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationScope {
    pub conversation_scope_id: Uuid,
    pub conversation_kind: ConversationKind,
    pub channel: String,
    pub stable_external_conversation_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_conversation_id: Option<String>,
}

impl ConversationScope {
    pub fn direct(
        conversation_scope_id: Uuid,
        channel: impl Into<String>,
        stable_external_conversation_key: impl Into<String>,
        external_conversation_id: Option<String>,
    ) -> Self {
        Self {
            conversation_scope_id,
            conversation_kind: ConversationKind::Direct,
            channel: channel.into(),
            stable_external_conversation_key: stable_external_conversation_key.into(),
            external_conversation_id,
        }
    }

    pub fn group(
        conversation_scope_id: Uuid,
        channel: impl Into<String>,
        stable_external_conversation_key: impl Into<String>,
        external_conversation_id: Option<String>,
    ) -> Self {
        Self {
            conversation_scope_id,
            conversation_kind: ConversationKind::Group,
            channel: channel.into(),
            stable_external_conversation_key: stable_external_conversation_key.into(),
            external_conversation_id,
        }
    }
}

/// Compact metadata used to carry work forward between turns and channels.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationHandoffMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_actor_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_user_goal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_summary: Option<String>,
}

impl ConversationHandoffMetadata {
    pub fn is_empty(&self) -> bool {
        self.last_actor_id.is_none()
            && self.task_state.is_none()
            && self.last_user_goal.is_none()
            && self.handoff_summary.is_none()
    }
}

/// Summary of a conversation for the thread list.
#[derive(Debug, Clone)]
pub struct ConversationSummary {
    pub id: Uuid,
    pub user_id: String,
    pub actor_id: Option<String>,
    pub conversation_scope_id: Option<Uuid>,
    pub conversation_kind: ConversationKind,
    pub channel: String,
    /// First user message, truncated to 100 chars.
    pub title: Option<String>,
    pub message_count: i64,
    pub started_at: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
    /// Thread type extracted from metadata (e.g. "assistant", "thread").
    pub thread_type: Option<String>,
    pub handoff: Option<ConversationHandoffMetadata>,
    pub stable_external_conversation_key: Option<String>,
}

/// A single message in a conversation.
#[derive(Debug, Clone)]
pub struct ConversationMessage {
    pub id: Uuid,
    pub role: String,
    pub content: String,
    pub actor_id: Option<String>,
    pub actor_display_name: Option<String>,
    pub raw_sender_id: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Lightweight linked-DM recall payload used by the prompt assembler.
#[derive(Debug, Clone)]
pub struct LinkedConversationRecall {
    pub principal_id: String,
    pub actor_id: String,
    pub include_group_history: bool,
    pub conversations: Vec<ConversationSummary>,
}

impl LinkedConversationRecall {
    pub fn new(
        principal_id: impl Into<String>,
        actor_id: impl Into<String>,
        include_group_history: bool,
        conversations: Vec<ConversationSummary>,
    ) -> Self {
        Self {
            principal_id: principal_id.into(),
            actor_id: actor_id.into(),
            include_group_history,
            conversations,
        }
    }

    /// Render a compact handoff block that summarizes only the ongoing work.
    pub fn compact_block(&self) -> Option<String> {
        self.compact_block_for_channel(None)
    }

    /// Render a compact handoff block, labelling cross-channel and group recall.
    pub fn compact_block_for_channel(&self, current_channel: Option<&str>) -> Option<String> {
        if self.conversations.is_empty() {
            return None;
        }

        let mut lines = vec![format!(
            "Linked recall for actor {} (principal {}):",
            self.actor_id, self.principal_id
        )];

        for convo in &self.conversations {
            let kind = convo.conversation_kind.as_str();
            let mut labels = Vec::new();
            if let Some(current_channel) = current_channel
                && convo.channel != current_channel
            {
                labels.push("cross-channel");
            }
            if convo.conversation_kind == ConversationKind::Group {
                labels.push("group");
            }
            let handoff = convo
                .handoff
                .as_ref()
                .and_then(|h| h.handoff_summary.as_deref())
                .unwrap_or_default();
            let goal = convo
                .handoff
                .as_ref()
                .and_then(|h| h.last_user_goal.as_deref())
                .unwrap_or_default();
            let state = convo
                .handoff
                .as_ref()
                .and_then(|h| h.task_state.as_deref())
                .unwrap_or_default();

            let label_suffix = if labels.is_empty() {
                String::new()
            } else {
                format!(" [{}]", labels.join(", "))
            };
            let mut parts = vec![format!(
                "{} / {}{} / {} messages",
                convo.channel, kind, label_suffix, convo.message_count
            )];
            if let Some(title) = convo.title.as_deref() {
                parts.push(format!("title={title}"));
            }
            if !goal.is_empty() {
                parts.push(format!("goal={goal}"));
            }
            if !state.is_empty() {
                parts.push(format!("state={state}"));
            }
            if !handoff.is_empty() {
                parts.push(format!("handoff={handoff}"));
            }
            lines.push(format!("- {}", parts.join(" | ")));
        }

        Some(lines.join("\n"))
    }
}

/// A search result hit from the conversation transcript index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSearchHit {
    pub conversation_id: Uuid,
    pub message_id: Uuid,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    pub channel: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    pub conversation_kind: ConversationKind,
    pub role: String,
    pub content: String,
    pub excerpt: String,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
}

/// A persisted job streaming event.
#[derive(Debug, Clone)]
pub struct JobEventRecord {
    pub id: i64,
    pub job_id: Uuid,
    pub event_type: String,
    pub data: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// A single setting row from the database.
#[derive(Debug, Clone)]
pub struct SettingRow {
    pub key: String,
    pub value: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}

/// Durable record of an observed learning signal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningEvent {
    pub id: Uuid,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<Uuid>,
    pub event_type: String,
    pub source: String,
    pub payload: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

/// Evaluation result for a learning event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningEvaluation {
    pub id: Uuid,
    pub learning_event_id: Uuid,
    pub user_id: String,
    pub evaluator: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    pub details: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Distilled improvement candidate derived from one or more learning events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningCandidate {
    pub id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub learning_event_id: Option<Uuid>,
    pub user_id: String,
    pub candidate_type: String,
    pub risk_tier: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub proposal: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Versioned snapshot of a learned artifact mutation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningArtifactVersion {
    pub id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_id: Option<Uuid>,
    pub user_id: String,
    pub artifact_type: String,
    pub artifact_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_label: Option<String>,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_content: Option<String>,
    pub provenance: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Explicit user/operator feedback on a candidate or artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningFeedbackRecord {
    pub id: Uuid,
    pub user_id: String,
    pub target_type: String,
    pub target_id: String,
    pub verdict: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Recorded rollback operations for learned artifacts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningRollbackRecord {
    pub id: Uuid,
    pub user_id: String,
    pub artifact_type: String,
    pub artifact_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_version_id: Option<Uuid>,
    pub reason: String,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Approval-gated code change proposal generated by the learning loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningCodeProposal {
    pub id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub learning_event_id: Option<Uuid>,
    pub user_id: String,
    pub status: String,
    pub title: String,
    pub rationale: String,
    pub target_files: Vec<String>,
    pub diff: String,
    pub validation_results: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollback_note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Deferred consequence contract that waits for downstream observations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutcomeContract {
    pub id: Uuid,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    pub source_kind: String,
    pub source_id: String,
    pub contract_type: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub due_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_verdict: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_score: Option<f64>,
    pub evaluation_details: serde_json::Value,
    pub metadata: serde_json::Value,
    pub dedupe_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluated_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Observation attached to an outcome contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutcomeObservation {
    pub id: Uuid,
    pub contract_id: Uuid,
    pub observation_kind: String,
    pub polarity: String,
    pub weight: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub evidence: serde_json::Value,
    pub fingerprint: String,
    pub observed_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

/// Filters for listing outcome contracts in APIs, tools, and services.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutcomeContractQuery {
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    pub limit: i64,
}

/// Aggregate metrics used by Learning Ledger surfaces.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OutcomeSummaryStats {
    pub open: u64,
    pub due: u64,
    pub evaluated_last_7d: u64,
    pub negative_ratio_last_7d: f64,
}

/// Distinct user with pending outcome evaluator work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutcomePendingUser {
    pub user_id: String,
}

/// Raw timing markers used to determine whether the outcome evaluator is stale.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OutcomeEvaluatorHealth {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oldest_due_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oldest_evaluating_claimed_at: Option<DateTime<Utc>>,
}

/// Statistics about jobs.
#[derive(Debug, Default)]
pub struct JobStats {
    pub total_jobs: u64,
    pub completed_jobs: u64,
    pub failed_jobs: u64,
    pub success_rate: f64,
    pub avg_duration_secs: f64,
    pub avg_cost: Decimal,
    pub total_cost: Decimal,
}

/// Statistics about tool usage.
#[derive(Debug)]
pub struct ToolStats {
    pub tool_name: String,
    pub total_calls: u64,
    pub successful_calls: u64,
    pub failed_calls: u64,
    pub success_rate: f64,
    pub avg_duration_ms: f64,
    pub total_cost: Decimal,
}

/// Estimation accuracy metrics.
#[derive(Debug, Default)]
pub struct EstimationAccuracy {
    pub cost_error_rate: f64,
    pub time_error_rate: f64,
    pub sample_count: u64,
}

/// Historical entry for a category.
#[derive(Debug)]
pub struct CategoryHistoryEntry {
    pub tool_names: Vec<String>,
    pub estimated_cost: Decimal,
    pub actual_cost: Option<Decimal>,
    pub estimated_time_secs: i32,
    pub actual_time_secs: Option<i32>,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conversation(channel: &str, kind: ConversationKind) -> ConversationSummary {
        let now = Utc::now();
        ConversationSummary {
            id: Uuid::new_v4(),
            user_id: "principal-1".to_string(),
            actor_id: Some("actor-1".to_string()),
            conversation_scope_id: Some(Uuid::new_v4()),
            conversation_kind: kind,
            channel: channel.to_string(),
            title: Some("prior work".to_string()),
            message_count: 3,
            started_at: now,
            last_activity: now,
            thread_type: None,
            handoff: Some(ConversationHandoffMetadata {
                last_actor_id: Some("actor-1".to_string()),
                task_state: Some("in_progress".to_string()),
                last_user_goal: Some("finish parity".to_string()),
                handoff_summary: Some("continue implementation".to_string()),
            }),
            stable_external_conversation_key: Some(format!("{channel}:thread")),
        }
    }

    #[test]
    fn linked_recall_labels_cross_channel_and_group_history() {
        let recall = LinkedConversationRecall::new(
            "principal-1",
            "actor-1",
            true,
            vec![
                conversation("telegram", ConversationKind::Direct),
                conversation("matrix", ConversationKind::Group),
            ],
        );

        let block = recall
            .compact_block_for_channel(Some("telegram"))
            .expect("recall block");
        assert!(block.contains("telegram / direct / 3 messages"));
        assert!(block.contains("matrix / group [cross-channel, group] / 3 messages"));
    }
}
