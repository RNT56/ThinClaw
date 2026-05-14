//! Root-independent routine engine scheduling helpers.

use chrono::{DateTime, Utc};
use thinclaw_channels_core::IncomingMessage;
use thinclaw_llm_core::{ChatMessage, FinishReason};
use thinclaw_types::{ToolProfile, error::RoutineError};
use uuid::Uuid;

use crate::routine::{
    NotifyConfig, Routine, RoutineCatchUpMode, RoutineEvent, RoutineEventDecision,
    RoutineEventStatus, RoutineTrigger, RoutineTriggerKind, RoutineTriggerStatus, RunStatus,
    Trigger, content_hash, next_fire_for_routine,
};

pub const EVENT_CONTENT_PREVIEW_LIMIT: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueSlotPlan {
    pub due_times: Vec<DateTime<Utc>>,
    pub backlog_collapsed: bool,
}

pub fn due_slots_for_routine(
    routine: &Routine,
    user_timezone: Option<&str>,
    now: DateTime<Utc>,
    trigger_queue_batch_limit: usize,
) -> Result<DueSlotPlan, RoutineError> {
    let Some(first_due) = routine.next_fire_at else {
        return Ok(DueSlotPlan {
            due_times: Vec::new(),
            backlog_collapsed: false,
        });
    };
    if first_due > now {
        return Ok(DueSlotPlan {
            due_times: Vec::new(),
            backlog_collapsed: false,
        });
    }

    match routine.policy.catch_up_mode {
        RoutineCatchUpMode::Skip | RoutineCatchUpMode::RunOnceNow => {
            let backlog_collapsed = next_fire_for_routine(routine, user_timezone, first_due)?
                .is_some_and(|next| next <= now);
            Ok(DueSlotPlan {
                due_times: vec![first_due],
                backlog_collapsed,
            })
        }
        RoutineCatchUpMode::Replay => {
            let mut due_times = vec![first_due];
            let mut cursor = first_due;
            let mut backlog_collapsed = false;

            while due_times.len() < trigger_queue_batch_limit {
                let Some(next_due) = next_fire_for_routine(routine, user_timezone, cursor)? else {
                    break;
                };
                if next_due > now {
                    break;
                }
                due_times.push(next_due);
                cursor = next_due;
            }

            if let Some(next_due) = next_fire_for_routine(routine, user_timezone, cursor)?
                && next_due <= now
            {
                backlog_collapsed = true;
            }

            Ok(DueSlotPlan {
                due_times,
                backlog_collapsed,
            })
        }
    }
}

pub fn trigger_kind_for_routine(routine: &Routine) -> RoutineTriggerKind {
    match routine.trigger {
        Trigger::SystemEvent { .. } => RoutineTriggerKind::SystemEvent,
        _ => RoutineTriggerKind::Cron,
    }
}

pub fn active_key_for_scheduled_trigger(
    routine: &Routine,
    trigger_kind: RoutineTriggerKind,
    due_at: DateTime<Utc>,
) -> String {
    match routine.policy.catch_up_mode {
        RoutineCatchUpMode::Replay => format!(
            "routine:{}:{}:{}",
            routine.id,
            trigger_kind,
            due_at.timestamp_millis()
        ),
        RoutineCatchUpMode::Skip | RoutineCatchUpMode::RunOnceNow => {
            format!("routine:{}:{}", routine.id, trigger_kind)
        }
    }
}

pub fn scheduled_trigger_idempotency_key(
    routine: &Routine,
    trigger_kind: RoutineTriggerKind,
    due_at: DateTime<Utc>,
) -> String {
    format!(
        "routine:{}:{}:{}:v{}",
        routine.id,
        trigger_kind,
        due_at.timestamp_millis(),
        routine.config_version
    )
}

pub fn scheduled_trigger_label(routine: &Routine) -> Option<String> {
    match &routine.trigger {
        Trigger::Cron { schedule } => Some(schedule.clone()),
        Trigger::SystemEvent { message, .. } => Some(truncate(message, 120)),
        _ => None,
    }
}

pub fn build_scheduled_routine_triggers(
    routine: &Routine,
    user_timezone: Option<&str>,
    now: DateTime<Utc>,
    trigger_queue_batch_limit: usize,
) -> Result<Vec<RoutineTrigger>, RoutineError> {
    let due_plan = due_slots_for_routine(routine, user_timezone, now, trigger_queue_batch_limit)?;
    let trigger_kind = trigger_kind_for_routine(routine);

    Ok(due_plan
        .due_times
        .into_iter()
        .map(|due_at| {
            let active_key = Some(active_key_for_scheduled_trigger(
                routine,
                trigger_kind,
                due_at,
            ));
            RoutineTrigger {
                id: Uuid::new_v4(),
                routine_id: routine.id,
                trigger_kind,
                trigger_label: scheduled_trigger_label(routine),
                due_at,
                status: RoutineTriggerStatus::Pending,
                decision: None,
                active_key,
                idempotency_key: scheduled_trigger_idempotency_key(routine, trigger_kind, due_at),
                claimed_by: None,
                claimed_at: None,
                lease_expires_at: None,
                processed_at: None,
                error_message: None,
                diagnostics: serde_json::json!({
                    "enqueued_at": now.to_rfc3339(),
                    "catch_up_mode": catch_up_mode_label(routine.policy.catch_up_mode),
                    "backlog_collapsed": due_plan.backlog_collapsed,
                    "scheduled_due_at": due_at.to_rfc3339(),
                    "config_version": routine.config_version,
                }),
                coalesced_count: 0,
                backlog_collapsed: due_plan.backlog_collapsed,
                routine_config_version: routine.config_version,
                created_at: now,
            }
        })
        .collect())
}

pub fn scheduled_run_trigger_key(trigger: &RoutineTrigger) -> String {
    format!("scheduled:{}", trigger.idempotency_key)
}

pub fn event_run_trigger_key(event: &RoutineEvent) -> String {
    format!("event:{}", event.idempotency_key)
}

pub fn catch_up_mode_label(mode: RoutineCatchUpMode) -> &'static str {
    match mode {
        RoutineCatchUpMode::Skip => "skip",
        RoutineCatchUpMode::RunOnceNow => "run_once_now",
        RoutineCatchUpMode::Replay => "replay",
    }
}

pub fn routine_event_idempotency_key(
    channel: &str,
    principal_id: &str,
    actor_id: &str,
    event_type: &str,
    metadata: &serde_json::Value,
    message_id: Uuid,
) -> String {
    let source_id = [
        "message_id",
        "external_message_id",
        "gmail_message_id",
        "imessage_guid",
    ]
    .iter()
    .find_map(|key| metadata.get(key).and_then(|value| value.as_str()))
    .map(str::to_string)
    .unwrap_or_else(|| message_id.to_string());

    format!("event:{channel}:{principal_id}:{actor_id}:{event_type}:{source_id}")
}

pub fn build_routine_event_from_message(message: &IncomingMessage) -> RoutineEvent {
    let identity = message.resolved_identity();
    let event_type = message
        .metadata
        .get("event_type")
        .and_then(|value| value.as_str())
        .unwrap_or("message")
        .to_string();
    let idempotency_key = routine_event_idempotency_key(
        &message.channel,
        &identity.principal_id,
        &identity.actor_id,
        &event_type,
        &message.metadata,
        message.id,
    );
    let diagnostics = serde_json::json!({
        "message_id": message.id.to_string(),
        "received_at": message.received_at.to_rfc3339(),
        "content_preview": truncate(&message.content, EVENT_CONTENT_PREVIEW_LIMIT),
        "thread_id": message.thread_id,
        "attachment_count": message.attachments.len(),
        "event_type": event_type,
        "idempotency_key": idempotency_key,
    });

    RoutineEvent {
        id: Uuid::new_v4(),
        principal_id: identity.principal_id.clone(),
        actor_id: identity.actor_id.clone(),
        channel: message.channel.clone(),
        event_type,
        raw_sender_id: identity.raw_sender_id.clone(),
        conversation_scope_id: identity.conversation_scope_id.to_string(),
        stable_external_conversation_key: identity.stable_external_conversation_key.clone(),
        idempotency_key,
        content: message.content.clone(),
        content_hash: content_hash(&message.content).to_string(),
        metadata: if message.metadata.is_null() {
            serde_json::json!({})
        } else {
            message.metadata.clone()
        },
        status: RoutineEventStatus::Pending,
        diagnostics,
        claimed_by: None,
        claimed_at: None,
        lease_expires_at: None,
        processed_at: None,
        error_message: None,
        matched_routines: 0,
        fired_routines: 0,
        attempt_count: 0,
        created_at: Utc::now(),
    }
}

pub fn should_refresh_event_cache(
    cache_empty: bool,
    last_refreshed_at: Option<DateTime<Utc>>,
    current_version: i64,
    observed_version: Option<i64>,
    ttl_secs: u64,
    now: DateTime<Utc>,
) -> bool {
    let ttl_expired = last_refreshed_at
        .map(|ts| now.signed_duration_since(ts).num_seconds() >= ttl_secs as i64)
        .unwrap_or(true);
    let version_changed = observed_version.is_some_and(|version| version != current_version);

    cache_empty || ttl_expired || version_changed
}

/// Sanitize a routine name for use in workspace paths.
///
/// Only keeps alphanumeric, dash, and underscore characters; replaces
/// everything else.
pub fn sanitize_routine_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub fn routine_requests_desktop_capabilities(allowed_tools: Option<&[String]>) -> bool {
    const DESKTOP_TOOLS: &[&str] = &[
        "desktop_apps",
        "desktop_ui",
        "desktop_screen",
        "desktop_calendar_native",
        "desktop_numbers_native",
        "desktop_pages_native",
    ];
    allowed_tools.is_some_and(|tools| {
        tools.iter().any(|tool| {
            DESKTOP_TOOLS
                .iter()
                .any(|desktop| desktop == &tool.as_str())
        })
    })
}

pub fn routine_cooldown_allows(routine: &Routine, now: DateTime<Utc>) -> bool {
    let Some(last_run) = routine.last_run_at else {
        return true;
    };
    let elapsed = now.signed_duration_since(last_run);
    let cooldown = chrono::Duration::from_std(routine.guardrails.cooldown)
        .unwrap_or(chrono::Duration::seconds(300));
    elapsed >= cooldown
}

pub fn active_hour_allows(now_hour: u8, active_start_hour: u8, active_end_hour: u8) -> bool {
    if active_start_hour <= active_end_hour {
        now_hour >= active_start_hour && now_hour < active_end_hour
    } else {
        now_hour >= active_start_hour || now_hour < active_end_hour
    }
}

pub fn summarize_runtime_capabilities(
    tool_profile: ToolProfile,
    allowed_tools: Option<&[String]>,
    allowed_skills: Option<&[String]>,
) -> String {
    let tool_grants = allowed_tools
        .map(|items| {
            if items.is_empty() {
                "none".to_string()
            } else {
                items.join(", ")
            }
        })
        .unwrap_or_else(|| {
            if matches!(tool_profile, ToolProfile::ExplicitOnly) {
                "none".to_string()
            } else {
                "implicit".to_string()
            }
        });
    let skill_grants = allowed_skills
        .map(|items| {
            if items.is_empty() {
                "none".to_string()
            } else {
                items.join(", ")
            }
        })
        .unwrap_or_else(|| "implicit".to_string());
    format!(
        "profile `{}` | tool grants: {} | skill grants: {}",
        tool_profile.as_str(),
        tool_grants,
        skill_grants
    )
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
}

pub fn metadata_contains_subset(expected: &serde_json::Value, actual: &serde_json::Value) -> bool {
    match (expected, actual) {
        (serde_json::Value::Object(expected), serde_json::Value::Object(actual)) => {
            expected.iter().all(|(key, value)| {
                actual
                    .get(key)
                    .is_some_and(|actual_value| metadata_contains_subset(value, actual_value))
            })
        }
        (serde_json::Value::Array(expected), serde_json::Value::Array(actual)) => {
            expected.iter().all(|expected_value| {
                actual
                    .iter()
                    .any(|actual_value| metadata_contains_subset(expected_value, actual_value))
            })
        }
        _ => expected == actual,
    }
}

pub fn increment_decision_count(
    counts: &mut serde_json::Map<String, serde_json::Value>,
    decision: RoutineEventDecision,
) {
    let key = decision.to_string();
    let next = counts
        .get(&key)
        .and_then(|value| value.as_u64())
        .unwrap_or(0)
        + 1;
    counts.insert(key, serde_json::json!(next));
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineNotification {
    pub content: String,
    pub metadata: serde_json::Value,
}

pub fn build_routine_notification(
    notify: &NotifyConfig,
    routine_name: &str,
    status: RunStatus,
    summary: Option<&str>,
) -> Option<RoutineNotification> {
    let should_notify = match status {
        RunStatus::Ok => notify.on_success,
        RunStatus::Attention => notify.on_attention,
        RunStatus::Failed => notify.on_failure,
        RunStatus::Running => false,
    };

    if !should_notify {
        return None;
    }

    let icon = match status {
        RunStatus::Ok => "✅",
        RunStatus::Attention => "🔔",
        RunStatus::Failed => "❌",
        RunStatus::Running => "⏳",
    };

    let content = match summary {
        Some(summary) => format!(
            "{} *Routine '{}'*: {}\n\n{}",
            icon, routine_name, status, summary
        ),
        None => format!("{} *Routine '{}'*: {}", icon, routine_name, status),
    };

    Some(RoutineNotification {
        content,
        metadata: serde_json::json!({
            "source": "routine",
            "routine_name": routine_name,
            "status": status.to_string(),
            "notify_user": notify.user,
            "notify_channel": notify.channel,
        }),
    })
}

pub const LIGHTWEIGHT_ROUTINE_OK_SENTINEL: &str = "ROUTINE_OK";

/// Default heartbeat prompt body.
pub const DEFAULT_HEARTBEAT_PROMPT: &str = "\
Read the HEARTBEAT.md checklist below and follow it strictly. \
Do not infer or repeat old tasks from prior chats. Check each item and report findings.\n\
\n\
If nothing needs attention, reply EXACTLY with: HEARTBEAT_OK\n\
\n\
If something needs attention, provide a short, specific summary of what needs action. \
Do NOT echo these instructions back — give real findings only. \
Use `emit_user_message` to deliver your findings to the user.\n\
\n\
You may edit HEARTBEAT.md to add, remove, or update checklist items as needed.";

pub fn build_lightweight_routine_prompt(
    prompt: &str,
    context_parts: &[String],
    state_content: Option<&str>,
) -> String {
    let mut full_prompt = String::new();
    full_prompt.push_str(prompt);

    if !context_parts.is_empty() {
        full_prompt.push_str("\n\n---\n\n# Context\n\n");
        full_prompt.push_str(&context_parts.join("\n\n"));
    }

    if let Some(state) = state_content {
        full_prompt.push_str("\n\n---\n\n# Previous State\n\n");
        full_prompt.push_str(state);
    }

    full_prompt.push_str(
        "\n\n---\n\nIf nothing needs attention, reply EXACTLY with: ROUTINE_OK\n\
         If something needs attention, provide a concise summary.",
    );
    full_prompt
}

pub fn lightweight_routine_messages(system_prompt: &str, full_prompt: &str) -> Vec<ChatMessage> {
    if system_prompt.is_empty() {
        vec![ChatMessage::user(full_prompt)]
    } else {
        vec![
            ChatMessage::system(system_prompt),
            ChatMessage::user(full_prompt),
        ]
    }
}

pub fn effective_lightweight_max_tokens(
    requested_max_tokens: u32,
    model_context_length: Option<u32>,
) -> u32 {
    model_context_length
        .map(|context_length| context_length / 2)
        .unwrap_or(requested_max_tokens)
        .max(requested_max_tokens)
}

pub fn classify_lightweight_routine_response(
    content: &str,
    finish_reason: FinishReason,
    input_tokens: u32,
    output_tokens: u32,
) -> Result<(RunStatus, Option<String>, Option<i32>), RoutineError> {
    let content = content.trim();
    let tokens_used = Some((input_tokens + output_tokens) as i32);

    if content.is_empty() {
        return if finish_reason == FinishReason::Length {
            Err(RoutineError::TruncatedResponse)
        } else {
            Err(RoutineError::EmptyResponse)
        };
    }

    if content == LIGHTWEIGHT_ROUTINE_OK_SENTINEL
        || content.contains(LIGHTWEIGHT_ROUTINE_OK_SENTINEL)
    {
        return Ok((RunStatus::Ok, None, tokens_used));
    }

    Ok((RunStatus::Attention, Some(content.to_string()), tokens_used))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FullJobRuntimeMetadata {
    pub allowed_tools: Option<Vec<String>>,
    pub allowed_skills: Option<Vec<String>>,
    pub tool_profile: Option<ToolProfile>,
    pub desktop: Option<serde_json::Value>,
}

pub fn full_job_metadata(
    routine: &Routine,
    run_id: Uuid,
    max_iterations: u32,
    runtime: FullJobRuntimeMetadata,
) -> serde_json::Value {
    let mut metadata = serde_json::json!({
        "max_iterations": max_iterations,
        "actor_id": routine.owner_actor_id(),
        "conversation_kind": "direct",
    });
    if let Some(obj) = metadata.as_object_mut() {
        if let Some(allowed_tools) = runtime.allowed_tools {
            obj.insert(
                "allowed_tools".to_string(),
                serde_json::json!(allowed_tools),
            );
        }
        if let Some(allowed_skills) = runtime.allowed_skills {
            obj.insert(
                "allowed_skills".to_string(),
                serde_json::json!(allowed_skills),
            );
        }
        if let Some(tool_profile) = runtime.tool_profile {
            obj.insert(
                "tool_profile".to_string(),
                serde_json::json!(tool_profile.as_str()),
            );
        }
        if let Some(serde_json::Value::Object(desktop)) = runtime.desktop {
            for (key, value) in desktop {
                obj.insert(key, value);
            }
            obj.entry("desktop_run_id".to_string())
                .or_insert_with(|| serde_json::json!(run_id.to_string()));
            obj.entry("recovery_count".to_string())
                .or_insert_with(|| serde_json::json!(0));
            obj.entry("last_verified_snapshot".to_string())
                .or_insert(serde_json::Value::Null);
        }
    }
    metadata
}

pub fn heartbeat_job_metadata(routine: &Routine, max_iterations: u32) -> serde_json::Value {
    serde_json::json!({
        "max_iterations": max_iterations,
        "heartbeat": true,
        "actor_id": routine.owner_actor_id(),
        "conversation_kind": "direct",
    })
}

pub fn build_heartbeat_prompt(
    custom_prompt: Option<&str>,
    checklist: &str,
    daily_context: &str,
    critique_context: &str,
    outcome_summary: Option<&str>,
) -> String {
    let prompt_body = custom_prompt.unwrap_or(DEFAULT_HEARTBEAT_PROMPT);
    let logs_note = if daily_context.is_empty() {
        "\n\nNote: No daily logs exist yet (no conversations recorded). \
         Any checklist items that reference daily logs are automatically satisfied. \
         If all items depend on daily logs, reply HEARTBEAT_OK."
    } else {
        ""
    };
    let outcome_summary = outcome_summary
        .map(|summary| format!("\n\n## {summary}\n"))
        .unwrap_or_default();

    format!(
        "{}\n\n## HEARTBEAT.md\n\n{}{}{}{}{}",
        prompt_body, checklist, daily_context, critique_context, outcome_summary, logs_note
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_routine_name_replaces_path_unsafe_chars() {
        assert_eq!(
            sanitize_routine_name("daily/profile sync"),
            "daily_profile_sync"
        );
    }

    #[test]
    fn desktop_capability_detection_matches_known_tools() {
        assert!(routine_requests_desktop_capabilities(Some(&[
            "shell".to_string(),
            "desktop_screen".to_string(),
        ])));
        assert!(!routine_requests_desktop_capabilities(Some(&[
            "shell".to_string()
        ])));
        assert!(!routine_requests_desktop_capabilities(None));
    }

    #[test]
    fn runtime_summary_reports_explicit_empty_grants() {
        let summary = summarize_runtime_capabilities(
            ToolProfile::ExplicitOnly,
            None,
            Some(&["skill-a".to_string()]),
        );

        assert_eq!(
            summary,
            "profile `explicit_only` | tool grants: none | skill grants: skill-a"
        );
    }

    #[test]
    fn truncate_preserves_utf8_boundaries() {
        assert_eq!(truncate("abécd", 4), "abé...");
    }

    #[test]
    fn event_idempotency_prefers_external_message_id() {
        let id = Uuid::new_v4();
        let key = routine_event_idempotency_key(
            "mail",
            "principal",
            "actor",
            "message",
            &serde_json::json!({ "external_message_id": "abc" }),
            id,
        );
        assert_eq!(key, "event:mail:principal:actor:message:abc");
    }

    #[test]
    fn event_cache_refresh_policy_checks_empty_ttl_and_version() {
        let now = Utc::now();

        assert!(should_refresh_event_cache(false, None, 1, Some(1), 60, now));
        assert!(should_refresh_event_cache(
            false,
            Some(now - chrono::Duration::seconds(61)),
            1,
            Some(1),
            60,
            now,
        ));
        assert!(should_refresh_event_cache(
            false,
            Some(now),
            1,
            Some(2),
            60,
            now,
        ));
        assert!(!should_refresh_event_cache(
            false,
            Some(now),
            1,
            Some(1),
            60,
            now,
        ));
    }

    #[test]
    fn active_hour_policy_handles_wrapping_windows() {
        assert!(active_hour_allows(10, 9, 17));
        assert!(!active_hour_allows(17, 9, 17));
        assert!(active_hour_allows(23, 22, 6));
        assert!(active_hour_allows(2, 22, 6));
        assert!(!active_hour_allows(12, 22, 6));
    }

    #[test]
    fn metadata_subset_matches_nested_objects_and_arrays() {
        let expected = serde_json::json!({
            "event": {
                "labels": ["important"],
                "source": "mail"
            }
        });
        let actual = serde_json::json!({
            "event": {
                "labels": ["later", "important"],
                "source": "mail",
                "extra": true
            }
        });

        assert!(metadata_contains_subset(&expected, &actual));
    }

    #[test]
    fn decision_count_increments_existing_value() {
        let mut counts = serde_json::Map::new();
        increment_decision_count(&mut counts, RoutineEventDecision::Fired);
        increment_decision_count(&mut counts, RoutineEventDecision::Fired);

        assert_eq!(
            counts.get("fired").and_then(|value| value.as_u64()),
            Some(2)
        );
    }

    #[test]
    fn notification_builder_respects_status_preferences() {
        let notify = NotifyConfig {
            on_success: false,
            ..NotifyConfig::default()
        };
        assert!(build_routine_notification(&notify, "routine", RunStatus::Ok, None).is_none());

        let notification =
            build_routine_notification(&notify, "routine", RunStatus::Attention, Some("check"))
                .unwrap();
        assert!(notification.content.contains("check"));
        assert_eq!(notification.metadata["routine_name"], "routine");
        assert_eq!(notification.metadata["status"], "attention");
    }

    #[test]
    fn lightweight_prompt_includes_context_state_and_sentinel() {
        let prompt = build_lightweight_routine_prompt(
            "Do work",
            &["## file.md\n\nbody".to_string()],
            Some("previous"),
        );

        assert!(prompt.contains("Do work"));
        assert!(prompt.contains("# Context"));
        assert!(prompt.contains("# Previous State"));
        assert!(prompt.contains(LIGHTWEIGHT_ROUTINE_OK_SENTINEL));
    }

    #[test]
    fn lightweight_response_classifies_ok_and_empty() {
        let ok = classify_lightweight_routine_response(
            LIGHTWEIGHT_ROUTINE_OK_SENTINEL,
            FinishReason::Stop,
            1,
            2,
        )
        .unwrap();
        assert_eq!(ok, (RunStatus::Ok, None, Some(3)));

        let empty =
            classify_lightweight_routine_response("", FinishReason::Length, 1, 2).unwrap_err();
        assert!(matches!(empty, RoutineError::TruncatedResponse));
    }

    #[test]
    fn heartbeat_prompt_adds_no_logs_note() {
        let prompt = build_heartbeat_prompt(None, "checks", "", "critique", Some("outcome"));

        assert!(prompt.contains("## HEARTBEAT.md"));
        assert!(prompt.contains("No daily logs exist yet"));
        assert!(prompt.contains("critique"));
        assert!(prompt.contains("outcome"));
    }
}
