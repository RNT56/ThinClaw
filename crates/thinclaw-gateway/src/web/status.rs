use axum::http::StatusCode;
use thinclaw_channels_core::StatusUpdate;

use crate::web::types::{
    ActionResponse, CacheStatsResponse, ChannelSetupStatus, GatewayStatusResponse, HealthResponse,
    ModelUsageEntry, PartialChannelSetupStatus, SseEvent,
};

pub fn health_response() -> HealthResponse {
    HealthResponse {
        status: "healthy",
        channel: "gateway",
    }
}

/// Map readiness-probe results to an HTTP status + body for `/api/health`.
///
/// Pure so the readiness contract is unit-testable without constructing the full
/// gateway runtime state. Returns `200 healthy` only when the database is
/// reachable *and* at least one LLM provider is configured; otherwise `503
/// unhealthy`, so load balancers stop routing to an instance that cannot serve.
pub fn readiness_response(
    db_ready: bool,
    provider_configured: bool,
) -> (StatusCode, HealthResponse) {
    if db_ready && provider_configured {
        (
            StatusCode::OK,
            HealthResponse {
                status: "healthy",
                channel: "gateway",
            },
        )
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            HealthResponse {
                status: "unhealthy",
                channel: "gateway",
            },
        )
    }
}

pub fn gateway_restart_already_in_progress_response() -> ActionResponse {
    ActionResponse::ok("Restart already in progress")
}

pub fn gateway_restart_accepted_response() -> ActionResponse {
    ActionResponse::ok("Restarting...")
}

pub fn cost_tracker_unavailable_status() -> StatusCode {
    StatusCode::SERVICE_UNAVAILABLE
}

pub fn format_daily_cost(cost: impl std::fmt::Display) -> String {
    format!("{cost:.4}")
}

pub fn format_model_cost(cost: impl std::fmt::Display) -> String {
    format!("{cost:.6}")
}

pub fn format_budget_limit_cents(cents: u64) -> String {
    format!("{:.2}", cents as f64 / 100.0)
}

pub fn model_usage_entry(
    model: impl Into<String>,
    input_tokens: u64,
    output_tokens: u64,
    cost: impl std::fmt::Display,
) -> ModelUsageEntry {
    ModelUsageEntry {
        model: model.into(),
        input_tokens,
        output_tokens,
        cost: format_model_cost(cost),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayRuntimeStatusInput {
    pub revision: u64,
    pub primary_model: String,
    pub cheap_model: Option<String>,
    pub routing_enabled: bool,
    pub routing_mode: String,
    pub primary_provider: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug)]
pub struct GatewayStatusResponseInput {
    pub sse_connections: u64,
    pub ws_connections: u64,
    pub uptime_secs: u64,
    pub daily_cost: Option<String>,
    pub actions_this_hour: Option<u64>,
    pub model_usage: Option<Vec<ModelUsageEntry>>,
    pub budget_limit_usd: Option<String>,
    pub hourly_action_limit: Option<u64>,
    pub runtime_status: Option<GatewayRuntimeStatusInput>,
    pub channel_setup: ChannelSetupStatus,
}

pub fn gateway_status_response(input: GatewayStatusResponseInput) -> GatewayStatusResponse {
    let runtime_status = input.runtime_status;
    GatewayStatusResponse {
        sse_connections: input.sse_connections,
        ws_connections: input.ws_connections,
        total_connections: input.sse_connections + input.ws_connections,
        uptime_secs: input.uptime_secs,
        daily_cost: input.daily_cost,
        actions_this_hour: input.actions_this_hour,
        model_usage: input.model_usage,
        budget_limit_usd: input.budget_limit_usd,
        hourly_action_limit: input.hourly_action_limit,
        runtime_revision: runtime_status.as_ref().map(|status| status.revision),
        active_model: runtime_status
            .as_ref()
            .map(|status| status.primary_model.clone()),
        active_cheap_model: runtime_status
            .as_ref()
            .and_then(|status| status.cheap_model.clone()),
        routing_enabled: runtime_status.as_ref().map(|status| status.routing_enabled),
        routing_mode: runtime_status
            .as_ref()
            .map(|status| status.routing_mode.clone()),
        primary_provider: runtime_status
            .as_ref()
            .and_then(|status| status.primary_provider.clone()),
        runtime_reload_error: runtime_status.and_then(|status| status.last_error),
        channel_setup: input.channel_setup,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupFieldStatus {
    pub name: String,
    pub present: bool,
}

impl SetupFieldStatus {
    pub fn new(name: impl Into<String>, present: bool) -> Self {
        Self {
            name: name.into(),
            present,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeLifecycleSetupStatusInput {
    pub enabled: bool,
    pub available: bool,
    pub required_fields: Vec<SetupFieldStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailSetupStatusInput {
    pub enabled: bool,
    pub project_id: Option<String>,
    pub subscription_id: Option<String>,
    pub topic_id: Option<String>,
    pub oauth_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NostrSetupStatusInput {
    pub enabled: bool,
    pub private_key_present: bool,
    pub owner_configured: bool,
    pub social_dm_enabled: bool,
    pub relay_count: Option<usize>,
    pub connected_relay_count: Option<usize>,
    pub public_key_hex: Option<String>,
    pub public_key_npub: Option<String>,
    pub owner_pubkey_hex: Option<String>,
    pub owner_pubkey_npub: Option<String>,
    pub invalid_private_key: bool,
}

fn value_present(value: &Option<String>) -> bool {
    value.as_ref().is_some_and(|value| !value.trim().is_empty())
}

pub fn build_native_lifecycle_setup_status(
    input: NativeLifecycleSetupStatusInput,
) -> PartialChannelSetupStatus {
    let mut missing_fields = Vec::new();
    if input.enabled {
        if !input.available {
            missing_fields.push("build_feature".to_string());
        }
        for field in &input.required_fields {
            if !field.present {
                missing_fields.push(field.name.clone());
            }
        }
    }

    PartialChannelSetupStatus {
        enabled: input.enabled,
        configured: input.enabled && input.available && missing_fields.is_empty(),
        missing_fields,
        needs_oauth: false,
        needs_private_key: input
            .required_fields
            .iter()
            .any(|field| field.name.contains("key")),
        owner_configured: false,
        tool_ready: input.enabled && input.available,
        control_ready: input.enabled && input.available,
        social_dm_enabled: false,
        relay_count: None,
        connected_relay_count: None,
        relay_health: None,
        public_key_hex: None,
        public_key_npub: None,
        owner_pubkey_hex: None,
        owner_pubkey_npub: None,
        invalid_private_key: false,
    }
}

pub fn build_gmail_setup_status(input: GmailSetupStatusInput) -> PartialChannelSetupStatus {
    let mut missing_fields = Vec::new();
    if input.enabled {
        if !value_present(&input.project_id) {
            missing_fields.push("project_id".to_string());
        }
        if !value_present(&input.subscription_id) {
            missing_fields.push("subscription_id".to_string());
        }
        if !value_present(&input.topic_id) {
            missing_fields.push("topic_id".to_string());
        }
    }

    let needs_oauth = input.enabled && missing_fields.is_empty() && input.oauth_token.is_none();

    PartialChannelSetupStatus {
        enabled: input.enabled,
        configured: input.enabled && missing_fields.is_empty() && !needs_oauth,
        missing_fields,
        needs_oauth,
        needs_private_key: false,
        owner_configured: false,
        tool_ready: false,
        control_ready: false,
        social_dm_enabled: false,
        relay_count: None,
        connected_relay_count: None,
        relay_health: None,
        public_key_hex: None,
        public_key_npub: None,
        owner_pubkey_hex: None,
        owner_pubkey_npub: None,
        invalid_private_key: false,
    }
}

pub fn build_nostr_setup_status(input: NostrSetupStatusInput) -> PartialChannelSetupStatus {
    let mut missing_fields = Vec::new();
    if input.enabled && !input.private_key_present {
        missing_fields.push("private_key".to_string());
    }
    if input.enabled && input.private_key_present && !input.owner_configured {
        missing_fields.push("owner_pubkey".to_string());
    }
    if input.enabled && input.invalid_private_key {
        missing_fields.push("private_key".to_string());
    }

    let needs_private_key =
        input.enabled && (!input.private_key_present || input.invalid_private_key);

    let status = PartialChannelSetupStatus {
        enabled: input.enabled,
        configured: input.enabled && input.private_key_present,
        missing_fields,
        needs_oauth: false,
        needs_private_key,
        owner_configured: input.owner_configured,
        tool_ready: input.enabled && input.private_key_present,
        control_ready: input.enabled && input.private_key_present && input.owner_configured,
        social_dm_enabled: input.social_dm_enabled,
        relay_count: input.relay_count,
        connected_relay_count: input.connected_relay_count,
        relay_health: None,
        public_key_hex: input.public_key_hex,
        public_key_npub: input.public_key_npub,
        owner_pubkey_hex: input.owner_pubkey_hex,
        owner_pubkey_npub: input.owner_pubkey_npub,
        invalid_private_key: input.invalid_private_key,
    };

    finalize_nostr_setup_status(status, input.owner_configured)
}

pub fn nostr_relay_health(
    enabled: bool,
    connected_relay_count: Option<usize>,
    invalid_private_key: bool,
) -> String {
    match (enabled, connected_relay_count, invalid_private_key) {
        (_, _, true) => "invalid_private_key".to_string(),
        (false, _, _) => "disabled".to_string(),
        (true, Some(count), _) if count > 0 => format!("connected:{count}"),
        (true, Some(_), _) => "configured_not_connected".to_string(),
        (true, None, _) => "configured".to_string(),
    }
}

pub fn finalize_nostr_setup_status(
    mut status: PartialChannelSetupStatus,
    owner_configured: bool,
) -> PartialChannelSetupStatus {
    status.tool_ready =
        status.enabled && status.public_key_hex.is_some() && !status.invalid_private_key;
    status.configured = status.tool_ready;
    status.control_ready = status.tool_ready && owner_configured;
    status.relay_health = Some(nostr_relay_health(
        status.enabled,
        status.connected_relay_count,
        status.invalid_private_key,
    ));
    status
}

pub fn build_cache_stats_response(
    hits: u64,
    misses: u64,
    evictions: u64,
    size_bytes: usize,
    hit_rate: f64,
) -> CacheStatsResponse {
    CacheStatsResponse {
        hits,
        misses,
        evictions,
        size_bytes,
        size: size_bytes,
        hit_rate,
        reason: None,
    }
}

pub fn unavailable_cache_stats_response(reason: impl Into<String>) -> CacheStatsResponse {
    CacheStatsResponse {
        hits: 0,
        misses: 0,
        evictions: 0,
        size_bytes: 0,
        size: 0,
        hit_rate: 0.0,
        reason: Some(reason.into()),
    }
}

pub fn status_update_to_sse_event(status: StatusUpdate, thread_id: Option<String>) -> SseEvent {
    match status {
        StatusUpdate::Thinking(msg) => SseEvent::Thinking {
            message: msg,
            thread_id,
        },
        StatusUpdate::ToolStarted { name, .. } => SseEvent::ToolStarted { name, thread_id },
        StatusUpdate::ToolCompleted { name, success, .. } => SseEvent::ToolCompleted {
            name,
            success,
            thread_id,
        },
        StatusUpdate::ToolResult {
            name,
            preview,
            artifacts,
        } => SseEvent::ToolResult {
            name,
            preview,
            artifacts,
            thread_id,
        },
        StatusUpdate::StreamChunk(content) => SseEvent::StreamChunk { content, thread_id },
        StatusUpdate::Status(msg) => SseEvent::Status {
            message: msg,
            thread_id,
        },
        StatusUpdate::ContextCompactionStarted { used, limit } => SseEvent::Status {
            message: format!("Compacting context ({used}/{limit} tokens) and retrying"),
            thread_id,
        },
        StatusUpdate::AdvisorConsultationStarted { .. } => SseEvent::Status {
            message: "Consulting the advisor lane".to_string(),
            thread_id,
        },
        StatusUpdate::SelfRepairStarted {
            repair_type,
            target_id,
            ..
        } => SseEvent::Status {
            message: format!("Self-repair: {repair_type} {target_id}"),
            thread_id,
        },
        StatusUpdate::SelfRepairCompleted {
            repair_type,
            target_id,
            success,
            ..
        } => SseEvent::Status {
            message: format!(
                "Self-repair {}: {repair_type} {target_id}",
                if success { "succeeded" } else { "failed" }
            ),
            thread_id,
        },
        StatusUpdate::Plan { entries } => SseEvent::PlanUpdate { entries, thread_id },
        StatusUpdate::Usage {
            input_tokens,
            output_tokens,
            cost_usd,
            model,
        } => SseEvent::UsageUpdate {
            input_tokens,
            output_tokens,
            cost_usd,
            model,
            thread_id,
        },
        StatusUpdate::JobStarted {
            job_id,
            title,
            browse_url,
        } => SseEvent::JobStarted {
            job_id,
            title,
            browse_url,
        },
        StatusUpdate::ApprovalNeeded {
            request_id,
            tool_name,
            description,
            parameters,
        } => SseEvent::ApprovalNeeded {
            request_id,
            tool_name,
            description,
            parameters: serde_json::to_string_pretty(&parameters)
                .unwrap_or_else(|_| parameters.to_string()),
            thread_id,
        },
        StatusUpdate::AuthRequired {
            extension_name,
            instructions,
            auth_url,
            setup_url,
            auth_mode,
            auth_status,
            shared_auth_provider,
            missing_scopes,
            thread_id: auth_thread_id,
        } => SseEvent::AuthRequired {
            extension_name,
            instructions,
            auth_url,
            setup_url,
            auth_mode,
            auth_status,
            shared_auth_provider,
            missing_scopes,
            thread_id: auth_thread_id.or(thread_id),
        },
        StatusUpdate::AuthCompleted {
            extension_name,
            success,
            message,
            auth_mode,
            auth_status,
            shared_auth_provider,
            missing_scopes,
            thread_id: auth_thread_id,
        } => SseEvent::AuthCompleted {
            extension_name,
            success,
            message,
            auth_mode,
            auth_status,
            shared_auth_provider,
            missing_scopes,
            thread_id: auth_thread_id.or(thread_id),
        },
        StatusUpdate::CredentialPrompt {
            prompt_id,
            secret_name,
            provider,
            reason,
            thread_id: prompt_thread_id,
        } => SseEvent::CredentialPrompt {
            prompt_id,
            secret_name,
            provider,
            reason,
            thread_id: prompt_thread_id.or(thread_id),
        },
        StatusUpdate::Error { message, code } => SseEvent::Status {
            message: format!(
                "[error{}] {}",
                code.as_ref().map(|c| format!(": {c}")).unwrap_or_default(),
                message
            ),
            thread_id,
        },
        StatusUpdate::CanvasAction(ref action) => {
            let (action_name, panel_id, content) = match action {
                thinclaw_tools_core::CanvasAction::Show {
                    panel_id,
                    title,
                    components,
                    ..
                } => (
                    "show",
                    panel_id.clone(),
                    Some(serde_json::json!({
                        "title": title,
                        "components": components,
                    })),
                ),
                thinclaw_tools_core::CanvasAction::Update {
                    panel_id,
                    components,
                } => (
                    "update",
                    panel_id.clone(),
                    Some(serde_json::json!({
                        "components": components,
                    })),
                ),
                thinclaw_tools_core::CanvasAction::Dismiss { panel_id } => {
                    ("dismiss", panel_id.clone(), None)
                }
                thinclaw_tools_core::CanvasAction::Notify {
                    message,
                    level,
                    duration_secs,
                } => (
                    "notify",
                    String::new(),
                    Some(serde_json::json!({
                        "message": message,
                        "level": format!("{:?}", level).to_lowercase(),
                        "duration_secs": duration_secs,
                    })),
                ),
            };
            SseEvent::CanvasUpdate {
                panel_id,
                action: action_name.to_string(),
                content,
            }
        }
        StatusUpdate::AgentMessage {
            content,
            message_type,
        } => {
            let prefix = match message_type.as_str() {
                "warning" => "⚠️ ",
                "question" => "❓ ",
                _ => "",
            };
            SseEvent::Response {
                content: format!("{}{}", prefix, content),
                thread_id: thread_id.unwrap_or_default(),
                attachments: Vec::new(),
            }
        }
        StatusUpdate::LifecycleStart { run_id } => SseEvent::Status {
            message: format!("{{\"lifecycle\":\"start\",\"runId\":\"{}\"}}", run_id),
            thread_id,
        },
        StatusUpdate::LifecycleEnd { run_id, phase } => SseEvent::Status {
            message: format!(
                "{{\"lifecycle\":\"end\",\"runId\":\"{}\",\"phase\":\"{}\"}}",
                run_id, phase
            ),
            thread_id,
        },
        StatusUpdate::SubagentSpawned {
            agent_id,
            name,
            task,
            task_packet,
            allowed_tools,
            allowed_skills,
            memory_mode,
            tool_mode,
            skill_mode,
        } => SseEvent::SubagentSpawned {
            agent_id,
            name,
            task,
            task_packet,
            allowed_tools,
            allowed_skills,
            memory_mode,
            tool_mode,
            skill_mode,
            timestamp: chrono::Utc::now().to_rfc3339(),
            thread_id,
        },
        StatusUpdate::SubagentProgress {
            agent_id,
            message,
            category,
        } => SseEvent::SubagentProgress {
            agent_id,
            message,
            category,
            timestamp: chrono::Utc::now().to_rfc3339(),
            thread_id,
        },
        StatusUpdate::SubagentCompleted {
            agent_id,
            name,
            success,
            response,
            duration_ms,
            iterations,
            task_packet,
            allowed_tools,
            allowed_skills,
            memory_mode,
            tool_mode,
            skill_mode,
        } => SseEvent::SubagentCompleted {
            agent_id,
            name,
            success,
            response,
            duration_ms,
            iterations,
            task_packet,
            allowed_tools,
            allowed_skills,
            memory_mode,
            tool_mode,
            skill_mode,
            timestamp: chrono::Utc::now().to_rfc3339(),
            thread_id,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_partial_status() -> PartialChannelSetupStatus {
        PartialChannelSetupStatus {
            enabled: false,
            configured: false,
            missing_fields: Vec::new(),
            needs_oauth: false,
            needs_private_key: false,
            owner_configured: false,
            tool_ready: false,
            control_ready: false,
            social_dm_enabled: false,
            relay_count: None,
            connected_relay_count: None,
            relay_health: None,
            public_key_hex: None,
            public_key_npub: None,
            owner_pubkey_hex: None,
            owner_pubkey_npub: None,
            invalid_private_key: false,
        }
    }

    fn empty_channel_setup_status() -> ChannelSetupStatus {
        ChannelSetupStatus {
            slack: empty_partial_status(),
            telegram: empty_partial_status(),
            gmail: empty_partial_status(),
            apple_mail: empty_partial_status(),
            nostr: empty_partial_status(),
            matrix: empty_partial_status(),
            voice_call: empty_partial_status(),
            apns: empty_partial_status(),
            browser_push: empty_partial_status(),
        }
    }

    #[test]
    fn health_response_preserves_existing_shape() {
        assert_eq!(
            serde_json::to_value(health_response()).unwrap(),
            serde_json::json!({
                "status": "healthy",
                "channel": "gateway",
            })
        );
    }

    #[test]
    fn readiness_503_when_db_down() {
        let (code, body) = readiness_response(false, true);
        assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body.status, "unhealthy");
    }

    #[test]
    fn readiness_503_when_no_provider() {
        let (code, body) = readiness_response(true, false);
        assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body.status, "unhealthy");
    }

    #[test]
    fn readiness_200_when_all_ready() {
        let (code, body) = readiness_response(true, true);
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body.status, "healthy");
    }

    #[test]
    fn gateway_restart_responses_preserve_existing_messages() {
        assert_eq!(
            serde_json::to_value(gateway_restart_already_in_progress_response()).unwrap(),
            serde_json::json!({
                "success": true,
                "message": "Restart already in progress",
            })
        );
        assert_eq!(
            serde_json::to_value(gateway_restart_accepted_response()).unwrap(),
            serde_json::json!({
                "success": true,
                "message": "Restarting...",
            })
        );
    }

    #[test]
    fn cost_tracker_unavailable_status_uses_service_unavailable() {
        assert_eq!(
            cost_tracker_unavailable_status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn gateway_status_response_formats_costs_and_runtime_fields() {
        let response = gateway_status_response(GatewayStatusResponseInput {
            sse_connections: 2,
            ws_connections: 3,
            uptime_secs: 10,
            daily_cost: Some(format_daily_cost(1.23456)),
            actions_this_hour: Some(4),
            model_usage: Some(vec![model_usage_entry("gpt", 5, 6, 0.1234567)]),
            budget_limit_usd: Some(format_budget_limit_cents(1234)),
            hourly_action_limit: Some(7),
            runtime_status: Some(GatewayRuntimeStatusInput {
                revision: 8,
                primary_model: "gpt".to_string(),
                cheap_model: Some("mini".to_string()),
                routing_enabled: true,
                routing_mode: "smart".to_string(),
                primary_provider: Some("openai".to_string()),
                last_error: None,
            }),
            channel_setup: empty_channel_setup_status(),
        });

        assert_eq!(response.total_connections, 5);
        assert_eq!(response.daily_cost.as_deref(), Some("1.2346"));
        assert_eq!(response.budget_limit_usd.as_deref(), Some("12.34"));
        assert_eq!(
            response.model_usage.unwrap()[0].cost,
            "0.123457".to_string()
        );
        assert_eq!(response.runtime_revision, Some(8));
        assert_eq!(response.routing_mode.as_deref(), Some("smart"));
    }

    #[test]
    fn native_lifecycle_status_reports_missing_setup_fields() {
        let status = build_native_lifecycle_setup_status(NativeLifecycleSetupStatusInput {
            enabled: true,
            available: true,
            required_fields: vec![
                SetupFieldStatus::new("homeserver", false),
                SetupFieldStatus::new("access_token", false),
            ],
        });

        assert!(status.enabled);
        assert!(!status.configured);
        assert!(
            status
                .missing_fields
                .iter()
                .any(|field| field == "homeserver")
        );
        assert!(
            status
                .missing_fields
                .iter()
                .any(|field| field == "access_token")
        );
    }

    #[test]
    fn native_lifecycle_status_reports_ready_when_required_fields_are_present() {
        let status = build_native_lifecycle_setup_status(NativeLifecycleSetupStatusInput {
            enabled: true,
            available: true,
            required_fields: vec![
                SetupFieldStatus::new("team_id", true),
                SetupFieldStatus::new("key_id", true),
                SetupFieldStatus::new("bundle_id", true),
                SetupFieldStatus::new("private_key", true),
                SetupFieldStatus::new("registration_secret", true),
            ],
        });

        assert!(status.enabled);
        assert!(status.configured);
        assert!(status.missing_fields.is_empty());
        assert!(status.needs_private_key);
    }

    #[test]
    fn native_lifecycle_status_reports_unavailable_build_feature_when_enabled() {
        let status = build_native_lifecycle_setup_status(NativeLifecycleSetupStatusInput {
            enabled: true,
            available: false,
            required_fields: Vec::new(),
        });

        assert!(status.enabled);
        assert!(!status.configured);
        assert_eq!(status.missing_fields, vec!["build_feature".to_string()]);
        assert!(!status.tool_ready);
        assert!(!status.control_ready);
    }

    #[test]
    fn native_lifecycle_status_ignores_missing_fields_when_disabled() {
        let status = build_native_lifecycle_setup_status(NativeLifecycleSetupStatusInput {
            enabled: false,
            available: true,
            required_fields: vec![SetupFieldStatus::new("bot_token", false)],
        });

        assert!(!status.enabled);
        assert!(!status.configured);
        assert!(status.missing_fields.is_empty());
    }

    #[test]
    fn gmail_status_requires_oauth_after_static_fields_are_present() {
        let status = build_gmail_setup_status(GmailSetupStatusInput {
            enabled: true,
            project_id: Some("project".to_string()),
            subscription_id: Some("subscription".to_string()),
            topic_id: Some("topic".to_string()),
            oauth_token: None,
        });

        assert!(status.enabled);
        assert!(!status.configured);
        assert!(status.needs_oauth);
        assert!(status.missing_fields.is_empty());
    }

    #[test]
    fn gmail_status_trims_static_fields_before_marking_present() {
        let status = build_gmail_setup_status(GmailSetupStatusInput {
            enabled: true,
            project_id: Some(" ".to_string()),
            subscription_id: Some("subscription".to_string()),
            topic_id: None,
            oauth_token: Some(String::new()),
        });

        assert!(status.enabled);
        assert!(!status.configured);
        assert!(!status.needs_oauth);
        assert_eq!(
            status.missing_fields,
            vec!["project_id".to_string(), "topic_id".to_string()]
        );
    }

    #[test]
    fn nostr_status_marks_missing_private_key_when_enabled() {
        let status = build_nostr_setup_status(NostrSetupStatusInput {
            enabled: true,
            private_key_present: false,
            owner_configured: false,
            social_dm_enabled: true,
            relay_count: Some(2),
            connected_relay_count: None,
            public_key_hex: None,
            public_key_npub: None,
            owner_pubkey_hex: None,
            owner_pubkey_npub: None,
            invalid_private_key: false,
        });

        assert!(status.enabled);
        assert!(!status.configured);
        assert!(status.needs_private_key);
        assert_eq!(status.missing_fields, vec!["private_key".to_string()]);
        assert_eq!(status.relay_health.as_deref(), Some("configured"));
    }

    #[test]
    fn nostr_status_marks_missing_owner_when_private_key_exists() {
        let status = build_nostr_setup_status(NostrSetupStatusInput {
            enabled: true,
            private_key_present: true,
            owner_configured: false,
            social_dm_enabled: false,
            relay_count: Some(1),
            connected_relay_count: Some(1),
            public_key_hex: Some("pub".to_string()),
            public_key_npub: Some("npub".to_string()),
            owner_pubkey_hex: None,
            owner_pubkey_npub: None,
            invalid_private_key: false,
        });

        assert!(status.tool_ready);
        assert!(!status.control_ready);
        assert_eq!(status.missing_fields, vec!["owner_pubkey".to_string()]);
        assert_eq!(status.relay_health.as_deref(), Some("connected:1"));
    }

    #[test]
    fn nostr_status_marks_invalid_private_key() {
        let status = build_nostr_setup_status(NostrSetupStatusInput {
            enabled: true,
            private_key_present: true,
            owner_configured: false,
            social_dm_enabled: false,
            relay_count: None,
            connected_relay_count: Some(0),
            public_key_hex: None,
            public_key_npub: None,
            owner_pubkey_hex: None,
            owner_pubkey_npub: None,
            invalid_private_key: true,
        });

        assert!(status.invalid_private_key);
        assert!(status.needs_private_key);
        assert!(!status.tool_ready);
        assert_eq!(
            status.missing_fields,
            vec!["owner_pubkey".to_string(), "private_key".to_string()]
        );
        assert_eq!(status.relay_health.as_deref(), Some("invalid_private_key"));
    }

    #[test]
    fn nostr_relay_health_normalizes_status_labels() {
        assert_eq!(nostr_relay_health(false, None, false), "disabled");
        assert_eq!(nostr_relay_health(true, Some(2), false), "connected:2");
        assert_eq!(
            nostr_relay_health(true, Some(0), false),
            "configured_not_connected"
        );
        assert_eq!(nostr_relay_health(true, None, false), "configured");
        assert_eq!(
            nostr_relay_health(true, Some(2), true),
            "invalid_private_key"
        );
    }

    #[test]
    fn cache_stats_response_duplicates_size_for_compatibility() {
        let response = build_cache_stats_response(3, 2, 1, 4096, 0.6);

        assert_eq!(response.hits, 3);
        assert_eq!(response.misses, 2);
        assert_eq!(response.evictions, 1);
        assert_eq!(response.size_bytes, 4096);
        assert_eq!(response.size, 4096);
        assert_eq!(response.hit_rate, 0.6);
        assert_eq!(response.reason, None);
    }

    #[test]
    fn unavailable_cache_stats_response_zeroes_counts_and_reports_reason() {
        let response = unavailable_cache_stats_response("cache unavailable");

        assert_eq!(response.hits, 0);
        assert_eq!(response.misses, 0);
        assert_eq!(response.evictions, 0);
        assert_eq!(response.size_bytes, 0);
        assert_eq!(response.size, 0);
        assert_eq!(response.hit_rate, 0.0);
        assert_eq!(response.reason.as_deref(), Some("cache unavailable"));
    }
}
