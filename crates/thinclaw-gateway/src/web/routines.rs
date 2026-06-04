//! Root-independent routine gateway policies.

use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use uuid::Uuid;

use super::types::{
    RoutineDetailResponse, RoutineEventActivityInfo, RoutineEventActivityResponse,
    RoutineEventCheckInfo, RoutineInfo, RoutineListResponse, RoutineRunInfo,
    RoutineSummaryResponse, RoutineTriggerCheckInfo,
};

pub const ROUTINE_WEBHOOK_BODY_LIMIT_BYTES: usize = 65_536;
pub const ROUTINE_EVENT_PREVIEW_LIMIT_BYTES: usize = 200;
pub const ROUTINE_DATABASE_UNAVAILABLE_MESSAGE: &str = "Database not available";
pub const ROUTINE_ENGINE_UNAVAILABLE_MESSAGE: &str = "Routine engine not available";
pub const ROUTINE_INVALID_ID_MESSAGE: &str = "Invalid routine ID";
pub const ROUTINE_NOT_FOUND_MESSAGE: &str = "Routine not found";
pub const ROUTINE_WEBHOOK_BODY_TOO_LARGE_MESSAGE: &str = "Request body exceeds 64KB limit";
pub const ROUTINE_NOT_WEBHOOK_TRIGGER_MESSAGE: &str = "Routine is not a webhook trigger";
pub const ROUTINE_DISABLED_MESSAGE: &str = "Routine is disabled";
pub const ROUTINE_ACTION_TRIGGERED_STATUS: &str = "triggered";
pub const ROUTINE_ACTION_ENABLED_STATUS: &str = "enabled";
pub const ROUTINE_ACTION_DISABLED_STATUS: &str = "disabled";
pub const ROUTINE_ACTION_DELETED_STATUS: &str = "deleted";

pub fn routine_database_unavailable_error() -> (StatusCode, String) {
    routine_error(
        StatusCode::SERVICE_UNAVAILABLE,
        ROUTINE_DATABASE_UNAVAILABLE_MESSAGE,
    )
}

pub fn routine_engine_unavailable_error() -> (StatusCode, String) {
    routine_error(
        StatusCode::SERVICE_UNAVAILABLE,
        ROUTINE_ENGINE_UNAVAILABLE_MESSAGE,
    )
}

pub fn routine_invalid_id_error() -> (StatusCode, String) {
    routine_error(StatusCode::BAD_REQUEST, ROUTINE_INVALID_ID_MESSAGE)
}

pub fn parse_routine_id(id: &str) -> Result<Uuid, (StatusCode, String)> {
    Uuid::parse_str(id).map_err(|_| routine_invalid_id_error())
}

pub fn parse_routine_uuid(id: &str) -> Result<Uuid, uuid::Error> {
    Uuid::parse_str(id)
}

pub fn routine_not_found_error() -> (StatusCode, String) {
    routine_error(StatusCode::NOT_FOUND, ROUTINE_NOT_FOUND_MESSAGE)
}

pub fn routine_not_found_message(routine_id: impl std::fmt::Display) -> String {
    format!("Routine {routine_id} not found")
}

pub fn routine_invalid_schedule_error(error: impl std::fmt::Display) -> (StatusCode, String) {
    routine_error(
        StatusCode::BAD_REQUEST,
        format!("Invalid schedule: {error}"),
    )
}

pub fn routine_webhook_body_too_large_error() -> (StatusCode, String) {
    routine_error(
        StatusCode::PAYLOAD_TOO_LARGE,
        ROUTINE_WEBHOOK_BODY_TOO_LARGE_MESSAGE,
    )
}

pub fn routine_not_webhook_trigger_error() -> (StatusCode, String) {
    routine_error(StatusCode::BAD_REQUEST, ROUTINE_NOT_WEBHOOK_TRIGGER_MESSAGE)
}

pub fn routine_disabled_error() -> (StatusCode, String) {
    routine_error(StatusCode::CONFLICT, ROUTINE_DISABLED_MESSAGE)
}

fn routine_error(status: StatusCode, message: impl Into<String>) -> (StatusCode, String) {
    (status, message.into())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineInfoInput {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub trigger: RoutineInfoTrigger,
    pub action: RoutineInfoAction,
    pub last_run_at: Option<DateTime<Utc>>,
    pub next_fire_at: Option<DateTime<Utc>>,
    pub run_count: u64,
    pub consecutive_failures: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutineInfoTrigger {
    Cron {
        schedule: String,
    },
    Event {
        pattern: String,
        channel: Option<String>,
        event_type: Option<String>,
        actor: Option<String>,
        priority: i32,
    },
    Webhook {
        path: Option<String>,
    },
    Manual,
    SystemEvent {
        message: String,
        schedule: Option<String>,
        catch_up_mode: RoutineInfoCatchUpMode,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutineInfoAction {
    Lightweight,
    FullJob,
    Heartbeat,
    ExperimentCampaign,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutineInfoCatchUpMode {
    Skip,
    RunOnceNow,
    Replay,
}

pub fn project_routine_info(input: RoutineInfoInput) -> RoutineInfo {
    let (trigger_type, trigger_summary) = routine_trigger_projection(&input.trigger);
    let status = routine_status(input.enabled, input.consecutive_failures);

    RoutineInfo {
        id: input.id,
        name: input.name,
        description: input.description,
        enabled: input.enabled,
        trigger_type,
        trigger_summary,
        action_type: routine_action_type(&input.action).to_string(),
        last_run_at: input.last_run_at.map(|dt| dt.to_rfc3339()),
        next_fire_at: input.next_fire_at.map(|dt| dt.to_rfc3339()),
        run_count: input.run_count,
        consecutive_failures: input.consecutive_failures,
        status: status.to_string(),
    }
}

pub fn routine_list_response(routines: Vec<RoutineInfo>) -> RoutineListResponse {
    RoutineListResponse { routines }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutineCreateResponse {
    pub id: String,
    pub name: String,
    pub description: String,
    pub schedule: String,
    pub task: String,
    pub created_at: String,
    pub next_fire_at: Option<String>,
}

pub fn routine_create_response(
    id: Uuid,
    name: impl Into<String>,
    description: impl Into<String>,
    schedule: impl Into<String>,
    task: impl Into<String>,
    created_at: DateTime<Utc>,
    next_fire_at: Option<DateTime<Utc>>,
) -> RoutineCreateResponse {
    RoutineCreateResponse {
        id: id.to_string(),
        name: name.into(),
        description: description.into(),
        schedule: schedule.into(),
        task: task.into(),
        created_at: created_at.to_rfc3339(),
        next_fire_at: next_fire_at.map(|ts| ts.to_rfc3339()),
    }
}

pub fn routine_summary_response(
    total: u64,
    enabled: u64,
    disabled: u64,
    failing: u64,
    runs_today: u64,
) -> RoutineSummaryResponse {
    RoutineSummaryResponse {
        total,
        enabled,
        disabled,
        failing,
        runs_today,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineRunInfoInput {
    pub id: Uuid,
    pub trigger_type: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub status: String,
    pub result_summary: Option<String>,
    pub tokens_used: Option<i32>,
    pub job_id: Option<Uuid>,
}

pub fn routine_run_info(input: RoutineRunInfoInput) -> RoutineRunInfo {
    RoutineRunInfo {
        id: input.id,
        trigger_type: input.trigger_type,
        started_at: input.started_at.to_rfc3339(),
        completed_at: input.completed_at.map(|dt| dt.to_rfc3339()),
        status: input.status,
        result_summary: input.result_summary,
        tokens_used: input.tokens_used,
        job_id: input.job_id,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RoutineEventActivityInput {
    pub id: Uuid,
    pub channel: String,
    pub content: String,
    pub content_preview: Option<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub processed_at: Option<DateTime<Utc>>,
    pub matched_routines: u32,
    pub fired_routines: u32,
    pub error_message: Option<String>,
    pub diagnostics: serde_json::Value,
}

pub fn routine_event_activity_info(input: RoutineEventActivityInput) -> RoutineEventActivityInfo {
    RoutineEventActivityInfo {
        id: input.id,
        channel: input.channel,
        content_preview: input
            .content_preview
            .unwrap_or_else(|| truncate_for_ui(&input.content)),
        status: input.status,
        created_at: input.created_at.to_rfc3339(),
        processed_at: input.processed_at.map(|ts| ts.to_rfc3339()),
        matched_routines: input.matched_routines,
        fired_routines: input.fired_routines,
        error_message: input.error_message,
        diagnostics: input.diagnostics,
    }
}

pub fn routine_event_activity_response(
    events: Vec<RoutineEventActivityInfo>,
) -> RoutineEventActivityResponse {
    RoutineEventActivityResponse { events }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RoutineEventCheckInput {
    pub id: Uuid,
    pub event_id: Uuid,
    pub decision: String,
    pub reason: Option<String>,
    pub details: serde_json::Value,
    pub sequence_num: u32,
    pub channel: String,
    pub content_preview: String,
    pub created_at: DateTime<Utc>,
}

pub fn routine_event_check_info(input: RoutineEventCheckInput) -> RoutineEventCheckInfo {
    RoutineEventCheckInfo {
        id: input.id,
        event_id: input.event_id,
        decision: input.decision,
        reason: input.reason,
        details: input.details,
        sequence_num: input.sequence_num,
        channel: input.channel,
        content_preview: input.content_preview,
        created_at: input.created_at.to_rfc3339(),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RoutineTriggerCheckInput {
    pub id: Uuid,
    pub trigger_kind: String,
    pub due_at: DateTime<Utc>,
    pub status: String,
    pub decision: Option<String>,
    pub claimed_by: Option<String>,
    pub processed_at: Option<DateTime<Utc>>,
    pub coalesced_count: u32,
    pub backlog_collapsed: bool,
    pub diagnostics: serde_json::Value,
}

pub fn routine_trigger_check_info(input: RoutineTriggerCheckInput) -> RoutineTriggerCheckInfo {
    RoutineTriggerCheckInfo {
        id: input.id,
        trigger_kind: input.trigger_kind,
        due_at: input.due_at.to_rfc3339(),
        status: input.status,
        decision: input.decision,
        claimed_by: input.claimed_by,
        processed_at: input.processed_at.map(|ts| ts.to_rfc3339()),
        coalesced_count: input.coalesced_count,
        backlog_collapsed: input.backlog_collapsed,
        diagnostics: input.diagnostics,
    }
}

#[derive(Debug)]
pub struct RoutineDetailInput {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub trigger: serde_json::Value,
    pub action: serde_json::Value,
    pub guardrails: serde_json::Value,
    pub notify: serde_json::Value,
    pub policy: serde_json::Value,
    pub last_run_at: Option<DateTime<Utc>>,
    pub next_fire_at: Option<DateTime<Utc>>,
    pub run_count: u64,
    pub consecutive_failures: u32,
    pub created_at: DateTime<Utc>,
    pub recent_runs: Vec<RoutineRunInfo>,
    pub recent_event_checks: Vec<RoutineEventCheckInfo>,
    pub recent_trigger_checks: Vec<RoutineTriggerCheckInfo>,
}

pub fn routine_detail_response(input: RoutineDetailInput) -> RoutineDetailResponse {
    RoutineDetailResponse {
        id: input.id,
        name: input.name,
        description: input.description,
        enabled: input.enabled,
        trigger: input.trigger,
        action: input.action,
        guardrails: input.guardrails,
        notify: input.notify,
        policy: input.policy,
        last_run_at: input.last_run_at.map(|dt| dt.to_rfc3339()),
        next_fire_at: input.next_fire_at.map(|dt| dt.to_rfc3339()),
        run_count: input.run_count,
        consecutive_failures: input.consecutive_failures,
        created_at: input.created_at.to_rfc3339(),
        recent_runs: input.recent_runs,
        recent_event_checks: input.recent_event_checks,
        recent_trigger_checks: input.recent_trigger_checks,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutineActionResponse {
    pub status: String,
    pub routine_id: Uuid,
}

pub fn routine_action_response(
    status: impl Into<String>,
    routine_id: Uuid,
) -> RoutineActionResponse {
    RoutineActionResponse {
        status: status.into(),
        routine_id,
    }
}

pub fn routine_triggered_action_response(routine_id: Uuid) -> RoutineActionResponse {
    routine_action_response(ROUTINE_ACTION_TRIGGERED_STATUS, routine_id)
}

pub fn routine_toggle_action_response(enabled: bool, routine_id: Uuid) -> RoutineActionResponse {
    let status = if enabled {
        ROUTINE_ACTION_ENABLED_STATUS
    } else {
        ROUTINE_ACTION_DISABLED_STATUS
    };
    routine_action_response(status, routine_id)
}

pub fn routine_deleted_action_response(routine_id: Uuid) -> RoutineActionResponse {
    routine_action_response(ROUTINE_ACTION_DELETED_STATUS, routine_id)
}

#[derive(Debug, Serialize)]
pub struct RoutineRunsResponse {
    pub routine_id: Uuid,
    pub runs: Vec<RoutineRunInfo>,
}

pub fn routine_runs_response(routine_id: Uuid, runs: Vec<RoutineRunInfo>) -> RoutineRunsResponse {
    RoutineRunsResponse { routine_id, runs }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutineClearRunsResponse {
    pub deleted: u64,
    pub scope: String,
}

pub fn routine_clear_runs_response(
    deleted: u64,
    routine_id: Option<Uuid>,
) -> RoutineClearRunsResponse {
    RoutineClearRunsResponse {
        deleted,
        scope: routine_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "all".to_string()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutineWebhookTriggerResponse {
    pub status: String,
    pub routine_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<Uuid>,
}

pub fn routine_webhook_trigger_response(
    routine_id: Uuid,
    run_id: Option<Uuid>,
) -> RoutineWebhookTriggerResponse {
    RoutineWebhookTriggerResponse {
        status: "triggered".to_string(),
        routine_id,
        run_id,
    }
}

pub fn routine_trigger_projection(trigger: &RoutineInfoTrigger) -> (String, String) {
    match trigger {
        RoutineInfoTrigger::Cron { schedule } => (
            "cron".to_string(),
            if schedule.starts_with("every ") {
                format!("schedule: {}", schedule)
            } else {
                format!("cron: {}", schedule)
            },
        ),
        RoutineInfoTrigger::Event {
            pattern,
            channel,
            event_type,
            actor,
            priority,
        } => {
            let ch = channel.as_deref().unwrap_or("any");
            let event_label = event_type.as_deref().unwrap_or("message");
            let actor_label = actor
                .as_deref()
                .map(|value| format!(" actor {}", value))
                .unwrap_or_default();
            let summary = if *priority == 0 {
                format!("on {} {}{} /{}/", ch, event_label, actor_label, pattern)
            } else {
                format!(
                    "on {} {}{} /{}/ (prio {})",
                    ch, event_label, actor_label, pattern, priority
                )
            };
            ("event".to_string(), summary)
        }
        RoutineInfoTrigger::Webhook { path } => {
            let p = path.as_deref().unwrap_or("/");
            ("webhook".to_string(), format!("webhook: {}", p))
        }
        RoutineInfoTrigger::Manual => ("manual".to_string(), "manual only".to_string()),
        RoutineInfoTrigger::SystemEvent {
            message,
            schedule,
            catch_up_mode,
        } => {
            let sched = schedule.as_deref().unwrap_or("on-demand");
            (
                "system_event".to_string(),
                format!(
                    "event: {} ({}, {})",
                    &message[..message.len().min(40)],
                    sched,
                    routine_catch_up_mode_label(*catch_up_mode)
                ),
            )
        }
    }
}

pub fn routine_action_type(action: &RoutineInfoAction) -> &'static str {
    match action {
        RoutineInfoAction::Lightweight => "lightweight",
        RoutineInfoAction::FullJob => "full_job",
        RoutineInfoAction::Heartbeat => "heartbeat",
        RoutineInfoAction::ExperimentCampaign => "experiment_campaign",
    }
}

pub fn routine_status(enabled: bool, consecutive_failures: u32) -> &'static str {
    if !enabled {
        "disabled"
    } else if consecutive_failures > 0 {
        "failing"
    } else {
        "active"
    }
}

pub fn routine_catch_up_mode_label(mode: RoutineInfoCatchUpMode) -> &'static str {
    match mode {
        RoutineInfoCatchUpMode::Skip => "skip",
        RoutineInfoCatchUpMode::RunOnceNow => "run once",
        RoutineInfoCatchUpMode::Replay => "replay",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RoutineWebhookSignatureError {
    #[error(
        "Unsigned webhooks are disabled for this routine; configure a secret or opt in explicitly"
    )]
    UnsignedWebhookForbidden,
    #[error("Missing X-Webhook-Signature header")]
    MissingSignatureHeader,
    #[error("Signature must use sha256= prefix")]
    InvalidSignaturePrefix,
    #[error("Invalid webhook signature")]
    InvalidSignature,
}

impl RoutineWebhookSignatureError {
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::UnsignedWebhookForbidden | Self::InvalidSignature => StatusCode::FORBIDDEN,
            Self::MissingSignatureHeader => StatusCode::UNAUTHORIZED,
            Self::InvalidSignaturePrefix => StatusCode::BAD_REQUEST,
        }
    }
}

pub fn routine_webhook_body_too_large(body_len: usize) -> bool {
    body_len > ROUTINE_WEBHOOK_BODY_LIMIT_BYTES
}

pub fn verify_routine_webhook_signature(
    secret: Option<&str>,
    allow_unsigned_webhook: bool,
    signature_header: Option<&str>,
    body: &[u8],
) -> Result<(), RoutineWebhookSignatureError> {
    let Some(expected_secret) = secret else {
        return if allow_unsigned_webhook {
            Ok(())
        } else {
            Err(RoutineWebhookSignatureError::UnsignedWebhookForbidden)
        };
    };

    let sig_header =
        signature_header.ok_or(RoutineWebhookSignatureError::MissingSignatureHeader)?;
    let hex_digest = sig_header
        .strip_prefix("sha256=")
        .ok_or(RoutineWebhookSignatureError::InvalidSignaturePrefix)?;

    let expected_digest = hmac_sha256_hex(expected_secret.as_bytes(), body);
    if constant_time_eq(hex_digest.as_bytes(), expected_digest.as_bytes()) {
        Ok(())
    } else {
        Err(RoutineWebhookSignatureError::InvalidSignature)
    }
}

pub fn hmac_sha256_hex(key: &[u8], data: &[u8]) -> String {
    let block_size = 64;
    let mut key_padded = vec![0u8; block_size];

    if key.len() > block_size {
        let hash = Sha256::digest(key);
        key_padded[..hash.len()].copy_from_slice(&hash);
    } else {
        key_padded[..key.len()].copy_from_slice(key);
    }

    let mut ipad = vec![0x36u8; block_size];
    let mut opad = vec![0x5cu8; block_size];
    for i in 0..block_size {
        ipad[i] ^= key_padded[i];
        opad[i] ^= key_padded[i];
    }

    let mut inner = Sha256::new();
    inner.update(&ipad);
    inner.update(data);
    let inner_hash = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(&opad);
    outer.update(inner_hash);
    let digest = outer.finalize();

    digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.ct_eq(b).into()
}

pub fn truncate_for_ui(content: &str) -> String {
    if content.len() <= ROUTINE_EVENT_PREVIEW_LIMIT_BYTES {
        content.to_string()
    } else {
        let end = floor_char_boundary(content, ROUTINE_EVENT_PREVIEW_LIMIT_BYTES);
        format!("{}...", &content[..end])
    }
}

fn floor_char_boundary(text: &str, max: usize) -> usize {
    if max >= text.len() {
        return text.len();
    }

    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_limit_is_strictly_above_64_kib() {
        assert!(!routine_webhook_body_too_large(
            ROUTINE_WEBHOOK_BODY_LIMIT_BYTES
        ));
        assert!(routine_webhook_body_too_large(
            ROUTINE_WEBHOOK_BODY_LIMIT_BYTES + 1
        ));
    }

    #[test]
    fn routine_boundary_errors_preserve_existing_statuses_and_messages() {
        assert_eq!(
            routine_database_unavailable_error(),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                ROUTINE_DATABASE_UNAVAILABLE_MESSAGE.to_string()
            )
        );
        assert_eq!(
            routine_engine_unavailable_error(),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                ROUTINE_ENGINE_UNAVAILABLE_MESSAGE.to_string()
            )
        );
        assert_eq!(
            routine_not_found_error(),
            (StatusCode::NOT_FOUND, ROUTINE_NOT_FOUND_MESSAGE.to_string())
        );
        assert_eq!(
            routine_webhook_body_too_large_error(),
            (
                StatusCode::PAYLOAD_TOO_LARGE,
                ROUTINE_WEBHOOK_BODY_TOO_LARGE_MESSAGE.to_string()
            )
        );
        assert_eq!(
            routine_not_webhook_trigger_error(),
            (
                StatusCode::BAD_REQUEST,
                ROUTINE_NOT_WEBHOOK_TRIGGER_MESSAGE.to_string()
            )
        );
        assert_eq!(
            routine_disabled_error(),
            (StatusCode::CONFLICT, ROUTINE_DISABLED_MESSAGE.to_string())
        );
        assert_eq!(
            routine_invalid_schedule_error("bad expr"),
            (
                StatusCode::BAD_REQUEST,
                "Invalid schedule: bad expr".to_string()
            )
        );
    }

    #[test]
    fn parse_routine_id_uses_gateway_invalid_id_error() {
        let id = Uuid::from_u128(42);
        assert_eq!(parse_routine_id(&id.to_string()), Ok(id));
        assert_eq!(parse_routine_uuid(&id.to_string()), Ok(id));
        assert_eq!(
            parse_routine_id("not-a-uuid"),
            Err(routine_invalid_id_error())
        );
        assert!(parse_routine_uuid("not-a-uuid").is_err());
        assert_eq!(
            routine_not_found_message(id),
            format!("Routine {id} not found")
        );
    }

    #[test]
    fn hmac_sha256_matches_known_vector() {
        assert_eq!(
            hmac_sha256_hex(b"key", b"The quick brown fox jumps over the lazy dog"),
            "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8"
        );
    }

    #[test]
    fn webhook_signature_accepts_valid_header() {
        let signature = format!("sha256={}", hmac_sha256_hex(b"secret", b"payload"));
        assert_eq!(
            verify_routine_webhook_signature(Some("secret"), false, Some(&signature), b"payload"),
            Ok(())
        );
    }

    #[test]
    fn webhook_signature_rejects_missing_header() {
        let err =
            verify_routine_webhook_signature(Some("secret"), false, None, b"payload").unwrap_err();
        assert_eq!(err, RoutineWebhookSignatureError::MissingSignatureHeader);
        assert_eq!(err.status_code(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn webhook_signature_rejects_bad_prefix() {
        let err = verify_routine_webhook_signature(
            Some("secret"),
            false,
            Some("md5=deadbeef"),
            b"payload",
        )
        .unwrap_err();
        assert_eq!(err, RoutineWebhookSignatureError::InvalidSignaturePrefix);
        assert_eq!(err.status_code(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn webhook_signature_rejects_bad_digest() {
        let err = verify_routine_webhook_signature(
            Some("secret"),
            false,
            Some("sha256=deadbeef"),
            b"payload",
        )
        .unwrap_err();
        assert_eq!(err, RoutineWebhookSignatureError::InvalidSignature);
        assert_eq!(err.status_code(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn webhook_signature_allows_unsigned_when_enabled() {
        assert_eq!(
            verify_routine_webhook_signature(None, true, None, b"payload"),
            Ok(())
        );
    }

    #[test]
    fn webhook_signature_rejects_unsigned_when_disabled() {
        let err = verify_routine_webhook_signature(None, false, None, b"payload").unwrap_err();
        assert_eq!(err, RoutineWebhookSignatureError::UnsignedWebhookForbidden);
        assert_eq!(err.status_code(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn routine_create_response_preserves_existing_json_shape() {
        let id = Uuid::nil();
        let created_at = "2026-06-02T10:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let next_fire_at = "2026-06-02T11:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let response = routine_create_response(
            id,
            "Daily",
            "desc",
            "0 9 * * *",
            "do work",
            created_at,
            Some(next_fire_at),
        );
        let value = serde_json::to_value(response).expect("serialize response");

        assert_eq!(
            value,
            serde_json::json!({
                "id": id.to_string(),
                "name": "Daily",
                "description": "desc",
                "schedule": "0 9 * * *",
                "task": "do work",
                "created_at": "2026-06-02T10:00:00+00:00",
                "next_fire_at": "2026-06-02T11:00:00+00:00",
            })
        );
    }

    #[test]
    fn routine_action_and_clear_responses_preserve_existing_json_shapes() {
        let routine_id = Uuid::nil();
        let action =
            serde_json::to_value(routine_action_response("triggered", routine_id)).unwrap();
        assert_eq!(
            action,
            serde_json::json!({
                "status": "triggered",
                "routine_id": routine_id,
            })
        );

        let scoped = serde_json::to_value(routine_clear_runs_response(3, Some(routine_id)))
            .expect("serialize scoped response");
        assert_eq!(
            scoped,
            serde_json::json!({
                "deleted": 3,
                "scope": routine_id.to_string(),
            })
        );

        let all = serde_json::to_value(routine_clear_runs_response(5, None))
            .expect("serialize all response");
        assert_eq!(
            all,
            serde_json::json!({
                "deleted": 5,
                "scope": "all",
            })
        );
    }

    #[test]
    fn routine_action_status_helpers_are_stable() {
        let routine_id = Uuid::from_u128(7);
        assert_eq!(
            routine_triggered_action_response(routine_id),
            routine_action_response(ROUTINE_ACTION_TRIGGERED_STATUS, routine_id)
        );
        assert_eq!(
            routine_toggle_action_response(true, routine_id),
            routine_action_response(ROUTINE_ACTION_ENABLED_STATUS, routine_id)
        );
        assert_eq!(
            routine_toggle_action_response(false, routine_id),
            routine_action_response(ROUTINE_ACTION_DISABLED_STATUS, routine_id)
        );
        assert_eq!(
            routine_deleted_action_response(routine_id),
            routine_action_response(ROUTINE_ACTION_DELETED_STATUS, routine_id)
        );
    }

    #[test]
    fn routine_webhook_response_omits_run_id_when_absent() {
        let routine_id = Uuid::nil();
        let run_id = Uuid::from_u128(1);
        let with_run =
            serde_json::to_value(routine_webhook_trigger_response(routine_id, Some(run_id)))
                .unwrap();
        assert_eq!(
            with_run,
            serde_json::json!({
                "status": "triggered",
                "routine_id": routine_id,
                "run_id": run_id,
            })
        );

        let without_run =
            serde_json::to_value(routine_webhook_trigger_response(routine_id, None)).unwrap();
        assert_eq!(
            without_run,
            serde_json::json!({
                "status": "triggered",
                "routine_id": routine_id,
            })
        );
    }

    #[test]
    fn routine_runs_response_preserves_existing_json_shape() {
        let routine_id = Uuid::nil();
        let run_id = Uuid::from_u128(1);
        let response = routine_runs_response(
            routine_id,
            vec![RoutineRunInfo {
                id: run_id,
                trigger_type: "manual".to_string(),
                started_at: "2026-06-02T10:00:00+00:00".to_string(),
                completed_at: None,
                status: "Completed".to_string(),
                result_summary: Some("done".to_string()),
                tokens_used: Some(12),
                job_id: None,
            }],
        );
        let value = serde_json::to_value(response).expect("serialize response");

        assert_eq!(
            value,
            serde_json::json!({
                "routine_id": routine_id,
                "runs": [{
                    "id": run_id,
                    "trigger_type": "manual",
                    "started_at": "2026-06-02T10:00:00+00:00",
                    "completed_at": null,
                    "status": "Completed",
                    "result_summary": "done",
                    "tokens_used": 12
                }]
            })
        );
    }

    #[test]
    fn routine_projection_builders_preserve_timestamp_and_preview_shapes() {
        let id = Uuid::from_u128(42);
        let started_at = "2026-06-02T10:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let completed_at = "2026-06-02T10:05:00Z".parse::<DateTime<Utc>>().unwrap();
        let run = routine_run_info(RoutineRunInfoInput {
            id,
            trigger_type: "event".to_string(),
            started_at,
            completed_at: Some(completed_at),
            status: "Failed".to_string(),
            result_summary: None,
            tokens_used: None,
            job_id: Some(Uuid::from_u128(7)),
        });
        assert_eq!(run.started_at, "2026-06-02T10:00:00+00:00");
        assert_eq!(
            run.completed_at.as_deref(),
            Some("2026-06-02T10:05:00+00:00")
        );
        assert_eq!(run.status, "Failed");

        let event = routine_event_activity_info(RoutineEventActivityInput {
            id,
            channel: "gateway".to_string(),
            content: "content body".to_string(),
            content_preview: None,
            status: "matched".to_string(),
            created_at: started_at,
            processed_at: Some(completed_at),
            matched_routines: 2,
            fired_routines: 1,
            error_message: None,
            diagnostics: serde_json::json!({"source": "test"}),
        });
        assert_eq!(event.content_preview, "content body");
        assert_eq!(event.created_at, "2026-06-02T10:00:00+00:00");
        assert_eq!(
            event.processed_at.as_deref(),
            Some("2026-06-02T10:05:00+00:00")
        );
    }

    #[test]
    fn routine_detail_response_preserves_nested_checks() {
        let id = Uuid::from_u128(9);
        let created_at = "2026-06-02T10:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let event_check = routine_event_check_info(RoutineEventCheckInput {
            id: Uuid::from_u128(1),
            event_id: Uuid::from_u128(2),
            decision: "fire".to_string(),
            reason: Some("matched".to_string()),
            details: serde_json::json!({"score": 1}),
            sequence_num: 3,
            channel: "gateway".to_string(),
            content_preview: "preview".to_string(),
            created_at,
        });
        let trigger_check = routine_trigger_check_info(RoutineTriggerCheckInput {
            id: Uuid::from_u128(4),
            trigger_kind: "cron".to_string(),
            due_at: created_at,
            status: "processed".to_string(),
            decision: Some("run".to_string()),
            claimed_by: Some("worker".to_string()),
            processed_at: Some(created_at),
            coalesced_count: 0,
            backlog_collapsed: false,
            diagnostics: serde_json::json!({"ok": true}),
        });

        let response = routine_detail_response(RoutineDetailInput {
            id,
            name: "Routine".to_string(),
            description: "desc".to_string(),
            enabled: true,
            trigger: serde_json::json!({"type": "manual"}),
            action: serde_json::json!({"type": "heartbeat"}),
            guardrails: serde_json::json!({}),
            notify: serde_json::json!({}),
            policy: serde_json::json!({"catch_up_mode": "skip"}),
            last_run_at: None,
            next_fire_at: Some(created_at),
            run_count: 5,
            consecutive_failures: 0,
            created_at,
            recent_runs: Vec::new(),
            recent_event_checks: vec![event_check],
            recent_trigger_checks: vec![trigger_check],
        });

        assert_eq!(response.id, id);
        assert_eq!(
            response.next_fire_at.as_deref(),
            Some("2026-06-02T10:00:00+00:00")
        );
        assert_eq!(response.recent_event_checks.len(), 1);
        assert_eq!(response.recent_trigger_checks.len(), 1);
    }

    #[test]
    fn preview_truncation_preserves_short_content() {
        assert_eq!(truncate_for_ui("short"), "short");
    }

    #[test]
    fn preview_truncation_is_utf8_safe() {
        let content = format!("{}{}", "a".repeat(199), "é and extra content");
        let truncated = truncate_for_ui(&content);
        assert!(truncated.ends_with("..."));
        assert!(truncated.len() <= ROUTINE_EVENT_PREVIEW_LIMIT_BYTES + 3);
        assert!(truncated.is_char_boundary(truncated.len()));
    }

    #[test]
    fn cron_trigger_projection_uses_schedule_label_for_every_expressions() {
        let (trigger_type, summary) = routine_trigger_projection(&RoutineInfoTrigger::Cron {
            schedule: "every 2h".to_string(),
        });
        assert_eq!(trigger_type, "cron");
        assert_eq!(summary, "schedule: every 2h");

        let (trigger_type, summary) = routine_trigger_projection(&RoutineInfoTrigger::Cron {
            schedule: "0 9 * * MON-FRI".to_string(),
        });
        assert_eq!(trigger_type, "cron");
        assert_eq!(summary, "cron: 0 9 * * MON-FRI");
    }

    #[test]
    fn event_trigger_projection_preserves_defaults_and_priority() {
        let (trigger_type, summary) = routine_trigger_projection(&RoutineInfoTrigger::Event {
            pattern: "deploy".to_string(),
            channel: None,
            event_type: None,
            actor: None,
            priority: 0,
        });
        assert_eq!(trigger_type, "event");
        assert_eq!(summary, "on any message /deploy/");

        let (trigger_type, summary) = routine_trigger_projection(&RoutineInfoTrigger::Event {
            pattern: "deploy".to_string(),
            channel: Some("slack".to_string()),
            event_type: Some("reaction".to_string()),
            actor: Some("alice".to_string()),
            priority: 3,
        });
        assert_eq!(trigger_type, "event");
        assert_eq!(summary, "on slack reaction actor alice /deploy/ (prio 3)");
    }

    #[test]
    fn webhook_trigger_projection_preserves_default_path() {
        let (trigger_type, summary) =
            routine_trigger_projection(&RoutineInfoTrigger::Webhook { path: None });
        assert_eq!(trigger_type, "webhook");
        assert_eq!(summary, "webhook: /");

        let (trigger_type, summary) = routine_trigger_projection(&RoutineInfoTrigger::Webhook {
            path: Some("/hooks/deploy".to_string()),
        });
        assert_eq!(trigger_type, "webhook");
        assert_eq!(summary, "webhook: /hooks/deploy");
    }

    #[test]
    fn system_event_trigger_projection_truncates_message_and_labels_catch_up_mode() {
        let message = "0123456789012345678901234567890123456789EXTRA".to_string();
        let (trigger_type, summary) =
            routine_trigger_projection(&RoutineInfoTrigger::SystemEvent {
                message,
                schedule: Some("0 9 * * *".to_string()),
                catch_up_mode: RoutineInfoCatchUpMode::Skip,
            });
        assert_eq!(trigger_type, "system_event");
        assert_eq!(
            summary,
            "event: 0123456789012345678901234567890123456789 (0 9 * * *, skip)"
        );

        let (_, summary) = routine_trigger_projection(&RoutineInfoTrigger::SystemEvent {
            message: "heartbeat".to_string(),
            schedule: None,
            catch_up_mode: RoutineInfoCatchUpMode::RunOnceNow,
        });
        assert_eq!(summary, "event: heartbeat (on-demand, run once)");

        let (_, summary) = routine_trigger_projection(&RoutineInfoTrigger::SystemEvent {
            message: "heartbeat".to_string(),
            schedule: None,
            catch_up_mode: RoutineInfoCatchUpMode::Replay,
        });
        assert_eq!(summary, "event: heartbeat (on-demand, replay)");
    }

    #[test]
    fn routine_status_prefers_disabled_then_failing_then_active() {
        assert_eq!(routine_status(false, 0), "disabled");
        assert_eq!(routine_status(false, 4), "disabled");
        assert_eq!(routine_status(true, 1), "failing");
        assert_eq!(routine_status(true, 0), "active");
    }

    #[test]
    fn routine_action_type_strings_are_stable() {
        assert_eq!(
            routine_action_type(&RoutineInfoAction::Lightweight),
            "lightweight"
        );
        assert_eq!(routine_action_type(&RoutineInfoAction::FullJob), "full_job");
        assert_eq!(
            routine_action_type(&RoutineInfoAction::Heartbeat),
            "heartbeat"
        );
        assert_eq!(
            routine_action_type(&RoutineInfoAction::ExperimentCampaign),
            "experiment_campaign"
        );
    }
}
