#![cfg_attr(not(feature = "postgres"), allow(unused_imports))]

use super::*;
#[cfg(feature = "postgres")]
pub(super) fn row_to_routine(row: &tokio_postgres::Row) -> Result<Routine, DatabaseError> {
    let trigger_type: String = row.get("trigger_type");
    let trigger_config: serde_json::Value = row.get("trigger_config");
    let action_type: String = row.get("action_type");
    let action_config: serde_json::Value = row.get("action_config");
    let cooldown_secs: i32 = row.get("cooldown_secs");
    let max_concurrent: i32 = row.get("max_concurrent");
    let dedup_window_secs: Option<i32> = row.get("dedup_window_secs");
    let policy_config: serde_json::Value = row
        .try_get("policy_config")
        .unwrap_or_else(|_| serde_json::json!({}));

    let trigger = Trigger::from_db(&trigger_type, trigger_config)
        .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
    let action = RoutineAction::from_db(&action_type, action_config)
        .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
    let policy = serde_json::from_value::<RoutinePolicy>(policy_config).unwrap_or_default();

    Ok(Routine {
        id: row.get("id"),
        name: row.get("name"),
        description: row.get("description"),
        user_id: row.get("user_id"),
        actor_id: row
            .try_get::<_, Option<String>>("actor_id")
            .ok()
            .flatten()
            .unwrap_or_else(|| row.get("user_id")),
        enabled: row.get("enabled"),
        trigger,
        action,
        guardrails: RoutineGuardrails {
            cooldown: std::time::Duration::from_secs(cooldown_secs as u64),
            max_concurrent: max_concurrent as u32,
            dedup_window: dedup_window_secs.map(|s| std::time::Duration::from_secs(s as u64)),
        },
        notify: NotifyConfig {
            channel: row.get("notify_channel"),
            user: row.get("notify_user"),
            on_attention: row.get("notify_on_attention"),
            on_failure: row.get("notify_on_failure"),
            on_success: row.get("notify_on_success"),
        },
        policy,
        last_run_at: row.get("last_run_at"),
        next_fire_at: row.get("next_fire_at"),
        run_count: row.get::<_, i64>("run_count") as u64,
        consecutive_failures: row.get::<_, i32>("consecutive_failures") as u32,
        state: row.get("state"),
        config_version: row.try_get::<_, i64>("config_version").unwrap_or(1),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

#[cfg(feature = "postgres")]
pub(super) fn row_to_routine_run(row: &tokio_postgres::Row) -> Result<RoutineRun, DatabaseError> {
    let status_str: String = row.get("status");
    let status: RunStatus =
        status_str
            .parse()
            .map_err(|e: thinclaw_types::error::RoutineError| {
                DatabaseError::Serialization(e.to_string())
            })?;

    Ok(RoutineRun {
        id: row.get("id"),
        routine_id: row.get("routine_id"),
        trigger_type: row.get("trigger_type"),
        trigger_detail: row.get("trigger_detail"),
        trigger_key: row.try_get("trigger_key").ok().flatten(),
        started_at: row.get("started_at"),
        completed_at: row.get("completed_at"),
        status,
        result_summary: row.get("result_summary"),
        tokens_used: row.get("tokens_used"),
        job_id: row.get("job_id"),
        created_at: row.get("created_at"),
    })
}

#[cfg(feature = "postgres")]
pub(super) fn row_to_routine_event(
    row: &tokio_postgres::Row,
) -> Result<RoutineEvent, DatabaseError> {
    let status_str: String = row.get("status");
    let status: RoutineEventStatus =
        status_str
            .parse()
            .map_err(|e: thinclaw_types::error::RoutineError| {
                DatabaseError::Serialization(e.to_string())
            })?;

    Ok(RoutineEvent {
        id: row.get("id"),
        principal_id: row.get("principal_id"),
        actor_id: row.get("actor_id"),
        channel: row.get("channel"),
        event_type: row
            .try_get::<_, Option<String>>("event_type")
            .ok()
            .flatten()
            .unwrap_or_else(|| "message".to_string()),
        raw_sender_id: row.get("raw_sender_id"),
        conversation_scope_id: row.get::<_, Uuid>("conversation_scope_id").to_string(),
        stable_external_conversation_key: row.get("stable_external_conversation_key"),
        idempotency_key: row
            .try_get::<_, Option<String>>("idempotency_key")
            .ok()
            .flatten()
            .unwrap_or_else(|| row.get::<_, Uuid>("id").to_string()),
        content: row.get("content"),
        content_hash: row.get("content_hash"),
        metadata: row.get("metadata"),
        status,
        diagnostics: row.get("diagnostics"),
        claimed_by: row.get("claimed_by"),
        claimed_at: row.get("claimed_at"),
        lease_expires_at: row.try_get("lease_expires_at").ok().flatten(),
        processed_at: row.get("processed_at"),
        error_message: row.get("error_message"),
        matched_routines: row.get::<_, i32>("matched_routines") as u32,
        fired_routines: row.get::<_, i32>("fired_routines") as u32,
        attempt_count: row.try_get::<_, i32>("attempt_count").unwrap_or(0) as u32,
        created_at: row.get("created_at"),
    })
}

#[cfg(feature = "postgres")]
pub(super) fn row_to_routine_event_evaluation(
    row: &tokio_postgres::Row,
) -> Result<RoutineEventEvaluation, DatabaseError> {
    let decision_str: String = row.get("decision");
    let decision: RoutineEventDecision =
        decision_str
            .parse()
            .map_err(|e: thinclaw_types::error::RoutineError| {
                DatabaseError::Serialization(e.to_string())
            })?;

    Ok(RoutineEventEvaluation {
        id: row.get("id"),
        event_id: row.get("event_id"),
        routine_id: row.get("routine_id"),
        decision,
        reason: row.get("reason"),
        details: row
            .try_get("details")
            .unwrap_or_else(|_| serde_json::json!({})),
        sequence_num: row.get::<_, i32>("sequence_num") as u32,
        channel: row.get("channel"),
        content_preview: row.get("content_preview"),
        created_at: row.get("created_at"),
    })
}

#[cfg(feature = "postgres")]
pub(super) fn row_to_routine_trigger(
    row: &tokio_postgres::Row,
) -> Result<RoutineTrigger, DatabaseError> {
    let trigger_kind: RoutineTriggerKind = row.get::<_, String>("trigger_kind").parse().map_err(
        |e: thinclaw_types::error::RoutineError| DatabaseError::Serialization(e.to_string()),
    )?;
    let status: RoutineTriggerStatus = row.get::<_, String>("status").parse().map_err(
        |e: thinclaw_types::error::RoutineError| DatabaseError::Serialization(e.to_string()),
    )?;
    let decision = row
        .try_get::<_, Option<String>>("decision")
        .ok()
        .flatten()
        .map(|value| {
            value
                .parse()
                .map_err(|e: thinclaw_types::error::RoutineError| {
                    DatabaseError::Serialization(e.to_string())
                })
        })
        .transpose()?;

    Ok(RoutineTrigger {
        id: row.get("id"),
        routine_id: row.get("routine_id"),
        trigger_kind,
        trigger_label: row.get("trigger_label"),
        due_at: row.get("due_at"),
        status,
        decision,
        active_key: row.get("active_key"),
        idempotency_key: row.get("idempotency_key"),
        claimed_by: row.get("claimed_by"),
        claimed_at: row.get("claimed_at"),
        lease_expires_at: row.get("lease_expires_at"),
        processed_at: row.get("processed_at"),
        error_message: row.get("error_message"),
        diagnostics: row.get("diagnostics"),
        coalesced_count: row.get::<_, i32>("coalesced_count") as u32,
        backlog_collapsed: row.get("backlog_collapsed"),
        routine_config_version: row.get("routine_config_version"),
        created_at: row.get("created_at"),
    })
}
