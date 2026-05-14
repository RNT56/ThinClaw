#![cfg_attr(not(feature = "postgres"), allow(unused_imports))]

use super::*;

#[cfg(feature = "postgres")]
pub(super) fn session_search_hit_from_row(row: &tokio_postgres::Row) -> SessionSearchHit {
    SessionSearchHit {
        conversation_id: row.get("conversation_id"),
        message_id: row.get("message_id"),
        user_id: row.get("user_id"),
        actor_id: row.try_get::<_, Option<String>>("actor_id").ok().flatten(),
        channel: row.get("channel"),
        thread_id: row.try_get::<_, Option<String>>("thread_id").ok().flatten(),
        conversation_kind: ConversationKind::from_db(
            row.try_get::<_, Option<String>>("conversation_kind")
                .ok()
                .flatten()
                .as_deref(),
        ),
        role: row.get("role"),
        content: row.get("content"),
        excerpt: row.get("excerpt"),
        metadata: row
            .try_get::<_, Option<serde_json::Value>>("metadata")
            .ok()
            .flatten()
            .unwrap_or_else(|| serde_json::json!({})),
        created_at: row.get("created_at"),
        score: row.try_get::<_, Option<f64>>("score").ok().flatten(),
    }
}

#[cfg(feature = "postgres")]
pub(super) fn learning_event_from_row(row: &tokio_postgres::Row) -> LearningEvent {
    LearningEvent {
        id: row.get("id"),
        user_id: row.get("user_id"),
        actor_id: row.try_get::<_, Option<String>>("actor_id").ok().flatten(),
        channel: row.try_get::<_, Option<String>>("channel").ok().flatten(),
        thread_id: row.try_get::<_, Option<String>>("thread_id").ok().flatten(),
        conversation_id: row
            .try_get::<_, Option<Uuid>>("conversation_id")
            .ok()
            .flatten(),
        message_id: row.try_get::<_, Option<Uuid>>("message_id").ok().flatten(),
        job_id: row.try_get::<_, Option<Uuid>>("job_id").ok().flatten(),
        event_type: row.get("event_type"),
        source: row.get("source"),
        payload: row.get("payload"),
        metadata: row
            .try_get::<_, Option<serde_json::Value>>("metadata")
            .ok()
            .flatten(),
        created_at: row.get("created_at"),
    }
}

#[cfg(feature = "postgres")]
pub(super) fn learning_evaluation_from_row(row: &tokio_postgres::Row) -> LearningEvaluation {
    LearningEvaluation {
        id: row.get("id"),
        learning_event_id: row.get("learning_event_id"),
        user_id: row.get("user_id"),
        evaluator: row.get("evaluator"),
        status: row.get("status"),
        score: row.try_get::<_, Option<f64>>("score").ok().flatten(),
        details: row
            .try_get::<_, Option<serde_json::Value>>("details")
            .ok()
            .flatten()
            .unwrap_or_else(|| serde_json::json!({})),
        created_at: row.get("created_at"),
    }
}

#[cfg(feature = "postgres")]
pub(super) fn learning_candidate_from_row(row: &tokio_postgres::Row) -> LearningCandidate {
    LearningCandidate {
        id: row.get("id"),
        learning_event_id: row
            .try_get::<_, Option<Uuid>>("learning_event_id")
            .ok()
            .flatten(),
        user_id: row.get("user_id"),
        candidate_type: row.get("candidate_type"),
        risk_tier: row.get("risk_tier"),
        confidence: row.try_get::<_, Option<f64>>("confidence").ok().flatten(),
        target_type: row
            .try_get::<_, Option<String>>("target_type")
            .ok()
            .flatten(),
        target_name: row
            .try_get::<_, Option<String>>("target_name")
            .ok()
            .flatten(),
        summary: row.try_get::<_, Option<String>>("summary").ok().flatten(),
        proposal: row
            .try_get::<_, Option<serde_json::Value>>("proposal")
            .ok()
            .flatten()
            .unwrap_or_else(|| serde_json::json!({})),
        created_at: row.get("created_at"),
    }
}

#[cfg(feature = "postgres")]
pub(super) fn learning_artifact_version_from_row(
    row: &tokio_postgres::Row,
) -> LearningArtifactVersion {
    LearningArtifactVersion {
        id: row.get("id"),
        candidate_id: row
            .try_get::<_, Option<Uuid>>("candidate_id")
            .ok()
            .flatten(),
        user_id: row.get("user_id"),
        artifact_type: row.get("artifact_type"),
        artifact_name: row.get("artifact_name"),
        version_label: row
            .try_get::<_, Option<String>>("version_label")
            .ok()
            .flatten(),
        status: row.get("status"),
        diff_summary: row
            .try_get::<_, Option<String>>("diff_summary")
            .ok()
            .flatten(),
        before_content: row
            .try_get::<_, Option<String>>("before_content")
            .ok()
            .flatten(),
        after_content: row
            .try_get::<_, Option<String>>("after_content")
            .ok()
            .flatten(),
        provenance: row
            .try_get::<_, Option<serde_json::Value>>("provenance")
            .ok()
            .flatten()
            .unwrap_or_else(|| serde_json::json!({})),
        created_at: row.get("created_at"),
    }
}

#[cfg(feature = "postgres")]
pub(super) fn learning_feedback_from_row(row: &tokio_postgres::Row) -> LearningFeedbackRecord {
    LearningFeedbackRecord {
        id: row.get("id"),
        user_id: row.get("user_id"),
        target_type: row.get("target_type"),
        target_id: row.get("target_id"),
        verdict: row.get("verdict"),
        note: row.try_get::<_, Option<String>>("note").ok().flatten(),
        metadata: row
            .try_get::<_, Option<serde_json::Value>>("metadata")
            .ok()
            .flatten()
            .unwrap_or_else(|| serde_json::json!({})),
        created_at: row.get("created_at"),
    }
}

#[cfg(feature = "postgres")]
pub(super) fn learning_rollback_from_row(row: &tokio_postgres::Row) -> LearningRollbackRecord {
    LearningRollbackRecord {
        id: row.get("id"),
        user_id: row.get("user_id"),
        artifact_type: row.get("artifact_type"),
        artifact_name: row.get("artifact_name"),
        artifact_version_id: row
            .try_get::<_, Option<Uuid>>("artifact_version_id")
            .ok()
            .flatten(),
        reason: row.get("reason"),
        metadata: row
            .try_get::<_, Option<serde_json::Value>>("metadata")
            .ok()
            .flatten()
            .unwrap_or_else(|| serde_json::json!({})),
        created_at: row.get("created_at"),
    }
}

#[cfg(feature = "postgres")]
pub(super) fn learning_code_proposal_from_row(row: &tokio_postgres::Row) -> LearningCodeProposal {
    LearningCodeProposal {
        id: row.get("id"),
        learning_event_id: row
            .try_get::<_, Option<Uuid>>("learning_event_id")
            .ok()
            .flatten(),
        user_id: row.get("user_id"),
        status: row.get("status"),
        title: row.get("title"),
        rationale: row.get("rationale"),
        target_files: row
            .try_get::<_, Option<serde_json::Value>>("target_files")
            .ok()
            .flatten()
            .and_then(|value| serde_json::from_value::<Vec<String>>(value).ok())
            .unwrap_or_default(),
        diff: row.get("diff"),
        validation_results: row
            .try_get::<_, Option<serde_json::Value>>("validation_results")
            .ok()
            .flatten()
            .unwrap_or_else(|| serde_json::json!({})),
        rollback_note: row
            .try_get::<_, Option<String>>("rollback_note")
            .ok()
            .flatten(),
        confidence: row.try_get::<_, Option<f64>>("confidence").ok().flatten(),
        branch_name: row
            .try_get::<_, Option<String>>("branch_name")
            .ok()
            .flatten(),
        pr_url: row.try_get::<_, Option<String>>("pr_url").ok().flatten(),
        metadata: row
            .try_get::<_, Option<serde_json::Value>>("metadata")
            .ok()
            .flatten()
            .unwrap_or_else(|| serde_json::json!({})),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

#[cfg(feature = "postgres")]
pub(super) fn outcome_contract_from_row(row: &tokio_postgres::Row) -> OutcomeContract {
    OutcomeContract {
        id: row.get("id"),
        user_id: row.get("user_id"),
        actor_id: row.try_get::<_, Option<String>>("actor_id").ok().flatten(),
        channel: row.try_get::<_, Option<String>>("channel").ok().flatten(),
        thread_id: row.try_get::<_, Option<String>>("thread_id").ok().flatten(),
        source_kind: row.get("source_kind"),
        source_id: row.get("source_id"),
        contract_type: row.get("contract_type"),
        status: row.get("status"),
        summary: row.try_get::<_, Option<String>>("summary").ok().flatten(),
        due_at: row.get("due_at"),
        expires_at: row.get("expires_at"),
        final_verdict: row
            .try_get::<_, Option<String>>("final_verdict")
            .ok()
            .flatten(),
        final_score: row.try_get::<_, Option<f64>>("final_score").ok().flatten(),
        evaluation_details: row
            .try_get::<_, Option<serde_json::Value>>("evaluation_details")
            .ok()
            .flatten()
            .unwrap_or_else(|| serde_json::json!({})),
        metadata: row
            .try_get::<_, Option<serde_json::Value>>("metadata")
            .ok()
            .flatten()
            .unwrap_or_else(|| serde_json::json!({})),
        dedupe_key: row.get("dedupe_key"),
        claimed_at: row
            .try_get::<_, Option<DateTime<Utc>>>("claimed_at")
            .ok()
            .flatten(),
        evaluated_at: row
            .try_get::<_, Option<DateTime<Utc>>>("evaluated_at")
            .ok()
            .flatten(),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

#[cfg(feature = "postgres")]
pub(super) fn outcome_observation_from_row(row: &tokio_postgres::Row) -> OutcomeObservation {
    OutcomeObservation {
        id: row.get("id"),
        contract_id: row.get("contract_id"),
        observation_kind: row.get("observation_kind"),
        polarity: row.get("polarity"),
        weight: row.get("weight"),
        summary: row.try_get::<_, Option<String>>("summary").ok().flatten(),
        evidence: row
            .try_get::<_, Option<serde_json::Value>>("evidence")
            .ok()
            .flatten()
            .unwrap_or_else(|| serde_json::json!({})),
        fingerprint: row.get("fingerprint"),
        observed_at: row.get("observed_at"),
        created_at: row.get("created_at"),
    }
}

#[cfg(feature = "postgres")]
pub(super) fn parse_job_state(s: &str) -> JobState {
    match s {
        "pending" => JobState::Pending,
        "in_progress" => JobState::InProgress,
        "completed" => JobState::Completed,
        "submitted" => JobState::Submitted,
        "accepted" => JobState::Accepted,
        "failed" => JobState::Failed,
        "stuck" => JobState::Stuck,
        "cancelled" => JobState::Cancelled,
        "abandoned" => JobState::Abandoned,
        _ => JobState::Pending,
    }
}
