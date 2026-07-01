//! conversations: rows.

use super::*;

pub(crate) fn handoff_from_metadata(
    metadata: &serde_json::Value,
) -> Option<ConversationHandoffMetadata> {
    let value = metadata.get("handoff").cloned().or_else(|| {
        let direct = serde_json::json!({
            "last_actor_id": metadata.get("last_actor_id"),
            "task_state": metadata.get("task_state"),
            "last_user_goal": metadata.get("last_user_goal"),
            "handoff_summary": metadata.get("handoff_summary"),
        });
        if direct
            .as_object()
            .map(|m| m.values().any(|v| !v.is_null()))
            .unwrap_or(false)
        {
            Some(direct)
        } else {
            None
        }
    })?;

    serde_json::from_value(value)
        .ok()
        .filter(|handoff: &ConversationHandoffMetadata| !handoff.is_empty())
}

pub(crate) fn kind_from_row(row: &libsql::Row) -> ConversationKind {
    ConversationKind::from_db(row.get::<String>(5).ok().as_deref())
}

pub(crate) fn summary_from_row(row: &libsql::Row) -> ConversationSummary {
    let metadata = get_json(row, 8);
    ConversationSummary {
        id: row
            .get::<String>(0)
            .unwrap_or_default()
            .parse()
            .unwrap_or_default(),
        user_id: get_text(row, 1),
        actor_id: get_opt_text(row, 2),
        conversation_scope_id: get_opt_text(row, 3).and_then(|s| s.parse().ok()),
        conversation_kind: kind_from_row(row),
        channel: get_text(row, 4),
        title: get_opt_text(row, 12),
        message_count: get_i64(row, 10),
        started_at: get_ts(row, 6),
        last_activity: get_ts(row, 7),
        thread_type: metadata
            .get("thread_type")
            .and_then(|v| v.as_str())
            .map(String::from),
        handoff: handoff_from_metadata(&metadata),
        stable_external_conversation_key: get_opt_text(row, 9),
    }
}

pub(crate) fn message_from_row(row: &libsql::Row) -> ConversationMessage {
    ConversationMessage {
        id: get_text(row, 0).parse().unwrap_or_default(),
        role: get_text(row, 1),
        content: get_text(row, 2),
        actor_id: get_opt_text(row, 3),
        actor_display_name: get_opt_text(row, 4),
        raw_sender_id: get_opt_text(row, 5),
        metadata: get_json(row, 6),
        created_at: get_ts(row, 7),
    }
}

pub(crate) fn search_hit_from_row(row: &libsql::Row) -> SessionSearchHit {
    SessionSearchHit {
        conversation_id: get_text(row, 0).parse().unwrap_or_default(),
        message_id: get_text(row, 1).parse().unwrap_or_default(),
        user_id: get_text(row, 2),
        actor_id: get_opt_text(row, 3),
        channel: get_text(row, 4),
        thread_id: get_opt_text(row, 5),
        conversation_kind: ConversationKind::from_db(row.get::<String>(6).ok().as_deref()),
        role: get_text(row, 7),
        content: get_text(row, 8),
        excerpt: get_text(row, 9),
        metadata: get_json(row, 10),
        created_at: get_ts(row, 11),
        score: row.get::<f64>(12).ok(),
    }
}

pub(crate) fn learning_event_from_row(row: &libsql::Row) -> LearningEvent {
    LearningEvent {
        id: get_text(row, 0).parse().unwrap_or_default(),
        user_id: get_text(row, 1),
        actor_id: get_opt_text(row, 2),
        channel: get_opt_text(row, 3),
        thread_id: get_opt_text(row, 4),
        conversation_id: get_opt_text(row, 5).and_then(|s| s.parse().ok()),
        message_id: get_opt_text(row, 6).and_then(|s| s.parse().ok()),
        job_id: get_opt_text(row, 7).and_then(|s| s.parse().ok()),
        event_type: get_text(row, 8),
        source: get_text(row, 9),
        payload: get_json(row, 10),
        metadata: match row.get::<String>(11) {
            Ok(value) => serde_json::from_str(&value).ok(),
            Err(_) => None,
        },
        created_at: get_ts(row, 12),
    }
}

pub(crate) fn learning_evaluation_from_row(row: &libsql::Row) -> LearningEvaluation {
    LearningEvaluation {
        id: get_text(row, 0).parse().unwrap_or_default(),
        learning_event_id: get_text(row, 1).parse().unwrap_or_default(),
        user_id: get_text(row, 2),
        evaluator: get_text(row, 3),
        status: get_text(row, 4),
        score: row.get::<f64>(5).ok(),
        details: get_json(row, 6),
        created_at: get_ts(row, 7),
    }
}

pub(crate) fn learning_candidate_from_row(row: &libsql::Row) -> LearningCandidate {
    LearningCandidate {
        id: get_text(row, 0).parse().unwrap_or_default(),
        learning_event_id: get_opt_text(row, 1).and_then(|v| v.parse().ok()),
        user_id: get_text(row, 2),
        candidate_type: get_text(row, 3),
        risk_tier: get_text(row, 4),
        confidence: row.get::<f64>(5).ok(),
        target_type: get_opt_text(row, 6),
        target_name: get_opt_text(row, 7),
        summary: get_opt_text(row, 8),
        proposal: get_json(row, 9),
        created_at: get_ts(row, 10),
    }
}

pub(crate) fn learning_artifact_version_from_row(row: &libsql::Row) -> LearningArtifactVersion {
    LearningArtifactVersion {
        id: get_text(row, 0).parse().unwrap_or_default(),
        candidate_id: get_opt_text(row, 1).and_then(|v| v.parse().ok()),
        user_id: get_text(row, 2),
        artifact_type: get_text(row, 3),
        artifact_name: get_text(row, 4),
        version_label: get_opt_text(row, 5),
        status: get_text(row, 6),
        diff_summary: get_opt_text(row, 7),
        before_content: get_opt_text(row, 8),
        after_content: get_opt_text(row, 9),
        provenance: get_json(row, 10),
        created_at: get_ts(row, 11),
    }
}

pub(crate) fn learning_feedback_from_row(row: &libsql::Row) -> LearningFeedbackRecord {
    LearningFeedbackRecord {
        id: get_text(row, 0).parse().unwrap_or_default(),
        user_id: get_text(row, 1),
        target_type: get_text(row, 2),
        target_id: get_text(row, 3),
        verdict: get_text(row, 4),
        note: get_opt_text(row, 5),
        metadata: get_json(row, 6),
        created_at: get_ts(row, 7),
    }
}

pub(crate) fn learning_rollback_from_row(row: &libsql::Row) -> LearningRollbackRecord {
    LearningRollbackRecord {
        id: get_text(row, 0).parse().unwrap_or_default(),
        user_id: get_text(row, 1),
        artifact_type: get_text(row, 2),
        artifact_name: get_text(row, 3),
        artifact_version_id: get_opt_text(row, 4).and_then(|v| v.parse().ok()),
        reason: get_text(row, 5),
        metadata: get_json(row, 6),
        created_at: get_ts(row, 7),
    }
}

pub(crate) fn learning_code_proposal_from_row(row: &libsql::Row) -> LearningCodeProposal {
    LearningCodeProposal {
        id: get_text(row, 0).parse().unwrap_or_default(),
        learning_event_id: get_opt_text(row, 1).and_then(|v| v.parse().ok()),
        user_id: get_text(row, 2),
        status: get_text(row, 3),
        title: get_text(row, 4),
        rationale: get_text(row, 5),
        target_files: get_json(row, 6)
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        diff: get_text(row, 7),
        validation_results: get_json(row, 8),
        rollback_note: get_opt_text(row, 9),
        confidence: row.get::<f64>(10).ok(),
        branch_name: get_opt_text(row, 11),
        pr_url: get_opt_text(row, 12),
        metadata: get_json(row, 13),
        created_at: get_ts(row, 14),
        updated_at: get_ts(row, 15),
    }
}

pub(crate) fn outcome_contract_from_row(row: &libsql::Row) -> OutcomeContract {
    OutcomeContract {
        id: get_text(row, 0).parse().unwrap_or_default(),
        user_id: get_text(row, 1),
        actor_id: get_opt_text(row, 2),
        channel: get_opt_text(row, 3),
        thread_id: get_opt_text(row, 4),
        source_kind: get_text(row, 5),
        source_id: get_text(row, 6),
        contract_type: get_text(row, 7),
        status: get_text(row, 8),
        summary: get_opt_text(row, 9),
        due_at: get_ts(row, 10),
        expires_at: get_ts(row, 11),
        final_verdict: get_opt_text(row, 12),
        final_score: row.get::<f64>(13).ok(),
        evaluation_details: get_json(row, 14),
        metadata: get_json(row, 15),
        dedupe_key: get_text(row, 16),
        claimed_at: get_opt_text(row, 17).and_then(|value| {
            DateTime::parse_from_rfc3339(&value)
                .ok()
                .map(|ts| ts.with_timezone(&Utc))
        }),
        evaluated_at: get_opt_text(row, 18).and_then(|value| {
            DateTime::parse_from_rfc3339(&value)
                .ok()
                .map(|ts| ts.with_timezone(&Utc))
        }),
        created_at: get_ts(row, 19),
        updated_at: get_ts(row, 20),
    }
}

pub(crate) fn outcome_observation_from_row(row: &libsql::Row) -> OutcomeObservation {
    OutcomeObservation {
        id: get_text(row, 0).parse().unwrap_or_default(),
        contract_id: get_text(row, 1).parse().unwrap_or_default(),
        observation_kind: get_text(row, 2),
        polarity: get_text(row, 3),
        weight: row.get::<f64>(4).unwrap_or_default(),
        summary: get_opt_text(row, 5),
        evidence: get_json(row, 6),
        fingerprint: get_text(row, 7),
        observed_at: get_ts(row, 8),
        created_at: get_ts(row, 9),
    }
}
