use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};
#[cfg(feature = "nostr")]
use nostr_sdk::{ToBech32, prelude::Keys};

use crate::channels::web::identity_helpers::GatewayRequestIdentity;
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;

pub(crate) async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy",
        channel: "gateway",
    })
}

pub(crate) async fn gateway_restart_handler(
    State(state): State<Arc<GatewayState>>,
) -> Json<ActionResponse> {
    if state
        .restart_requested
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
        )
        .is_err()
    {
        return Json(ActionResponse::ok("Restart already in progress"));
    }

    if let Some(tx) = state.shutdown_tx.write().await.take() {
        let _ = tx.send(());
        tracing::info!("Gateway restart requested via API");
    }

    Json(ActionResponse::ok("Restarting..."))
}

pub(crate) async fn gateway_status_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
) -> Json<GatewayStatusResponse> {
    let sse_connections = state.sse.connection_count();
    let ws_connections = state
        .ws_tracker
        .as_ref()
        .map(|t| t.connection_count())
        .unwrap_or(0);

    let uptime_secs = state.startup_time.elapsed().as_secs();
    let runtime_status = state.llm_runtime.as_ref().map(|runtime| runtime.status());
    let channel_setup =
        load_channel_setup_status(state.as_ref(), &request_identity.principal_id).await;

    let (daily_cost, actions_this_hour, model_usage, budget_limit_usd, hourly_action_limit) =
        if let Some(ref cg) = state.cost_guard {
            let cost = cg.daily_spend().await;
            let actions = cg.actions_this_hour().await;
            let usage = cg.model_usage().await;
            let models: Vec<ModelUsageEntry> = usage
                .into_iter()
                .map(|(model, tokens)| ModelUsageEntry {
                    model,
                    input_tokens: tokens.input_tokens,
                    output_tokens: tokens.output_tokens,
                    cost: format!("{:.6}", tokens.cost),
                })
                .collect();
            let budget = cg
                .daily_budget_cents()
                .map(|c| format!("{:.2}", c as f64 / 100.0));
            let rate_limit = cg.hourly_action_limit();
            (
                Some(format!("{:.4}", cost)),
                Some(actions),
                Some(models),
                budget,
                rate_limit,
            )
        } else {
            (None, None, None, None, None)
        };

    Json(GatewayStatusResponse {
        sse_connections,
        ws_connections,
        total_connections: sse_connections + ws_connections,
        uptime_secs,
        daily_cost,
        actions_this_hour,
        model_usage,
        budget_limit_usd,
        hourly_action_limit,
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
            .map(|status| status.routing_mode.as_str().to_string()),
        primary_provider: runtime_status
            .as_ref()
            .and_then(|status| status.primary_provider.clone()),
        runtime_reload_error: runtime_status.and_then(|status| status.last_error),
        channel_setup,
    })
}

async fn load_channel_setup_status(state: &GatewayState, user_id: &str) -> ChannelSetupStatus {
    let settings = if let Some(store) = state.store.as_ref()
        && let Ok(map) = store.get_all_settings(user_id).await
    {
        crate::settings::Settings::from_db_map(&map)
    } else {
        crate::settings::Settings::default()
    };

    let nostr_diagnostics = if let Some(channel_manager) = state.channel_manager.as_ref() {
        channel_manager.channel_diagnostics("nostr").await
    } else {
        None
    };

    ChannelSetupStatus {
        gmail: build_gmail_setup_status(&settings),
        nostr: build_nostr_setup_status(&settings, nostr_diagnostics.as_ref()),
    }
}

fn build_gmail_setup_status(settings: &crate::settings::Settings) -> PartialChannelSetupStatus {
    let enabled =
        crate::config::helpers::parse_bool_env("GMAIL_ENABLED", settings.channels.gmail_enabled)
            .unwrap_or(settings.channels.gmail_enabled);
    let project_id = crate::config::helpers::optional_env("GMAIL_PROJECT_ID")
        .ok()
        .flatten()
        .or(settings.channels.gmail_project_id.clone())
        .unwrap_or_default();
    let subscription_id = crate::config::helpers::optional_env("GMAIL_SUBSCRIPTION_ID")
        .ok()
        .flatten()
        .or(settings.channels.gmail_subscription_id.clone())
        .unwrap_or_default();
    let topic_id = crate::config::helpers::optional_env("GMAIL_TOPIC_ID")
        .ok()
        .flatten()
        .or(settings.channels.gmail_topic_id.clone())
        .unwrap_or_default();

    let mut missing_fields = Vec::new();
    if enabled {
        if project_id.trim().is_empty() {
            missing_fields.push("project_id".to_string());
        }
        if subscription_id.trim().is_empty() {
            missing_fields.push("subscription_id".to_string());
        }
        if topic_id.trim().is_empty() {
            missing_fields.push("topic_id".to_string());
        }
    }

    let has_oauth_token = crate::config::helpers::optional_env("GMAIL_OAUTH_TOKEN")
        .ok()
        .flatten()
        .is_some();
    let needs_oauth = enabled && missing_fields.is_empty() && !has_oauth_token;

    PartialChannelSetupStatus {
        enabled,
        configured: enabled && missing_fields.is_empty() && !needs_oauth,
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

fn build_nostr_setup_status(
    settings: &crate::settings::Settings,
    diagnostics: Option<&serde_json::Value>,
) -> PartialChannelSetupStatus {
    let enabled =
        crate::config::helpers::parse_bool_env("NOSTR_ENABLED", settings.channels.nostr_enabled)
            .unwrap_or(settings.channels.nostr_enabled);
    let private_key = crate::config::helpers::optional_env("NOSTR_PRIVATE_KEY")
        .ok()
        .flatten()
        .or_else(|| {
            crate::config::helpers::optional_env("NOSTR_SECRET_KEY")
                .ok()
                .flatten()
        });

    let mut missing_fields = Vec::new();
    if enabled && private_key.is_none() {
        missing_fields.push("private_key".to_string());
    }

    #[cfg(feature = "nostr")]
    let resolved = crate::config::ChannelsConfig::resolve_nostr(settings)
        .ok()
        .flatten();
    #[cfg(feature = "nostr")]
    let owner_configured = resolved
        .as_ref()
        .and_then(|config| config.owner_pubkey.as_ref())
        .is_some();
    #[cfg(not(feature = "nostr"))]
    let owner_configured = false;
    if enabled && private_key.is_some() && !owner_configured {
        missing_fields.push("owner_pubkey".to_string());
    }

    #[cfg(feature = "nostr")]
    let social_dm_enabled = resolved
        .as_ref()
        .map(|config| config.social_dm_enabled)
        .unwrap_or(settings.channels.nostr_social_dm_enabled);
    #[cfg(not(feature = "nostr"))]
    let social_dm_enabled = settings.channels.nostr_social_dm_enabled;

    #[cfg(feature = "nostr")]
    let relay_count = resolved
        .as_ref()
        .map(|config| config.relays.len())
        .or_else(|| {
            diagnostics
                .and_then(|value| value.get("relay_count"))
                .and_then(|value| value.as_u64())
                .map(|value| value as usize)
        });
    #[cfg(not(feature = "nostr"))]
    let relay_count = diagnostics
        .and_then(|value| value.get("relay_count"))
        .and_then(|value| value.as_u64())
        .map(|value| value as usize);

    let mut status = PartialChannelSetupStatus {
        enabled,
        configured: enabled && private_key.is_some(),
        missing_fields,
        needs_oauth: false,
        needs_private_key: enabled && private_key.is_none(),
        owner_configured,
        tool_ready: enabled && private_key.is_some(),
        control_ready: enabled && private_key.is_some() && owner_configured,
        social_dm_enabled,
        relay_count,
        connected_relay_count: diagnostics
            .and_then(|value| value.get("connected_relay_count"))
            .and_then(|value| value.as_u64())
            .map(|value| value as usize),
        relay_health: None,
        public_key_hex: diagnostics
            .and_then(|value| value.get("public_key_hex"))
            .and_then(|value| value.as_str())
            .map(str::to_string),
        public_key_npub: diagnostics
            .and_then(|value| value.get("public_key_npub"))
            .and_then(|value| value.as_str())
            .map(str::to_string),
        owner_pubkey_hex: diagnostics
            .and_then(|value| value.get("owner_pubkey_hex"))
            .and_then(|value| value.as_str())
            .map(str::to_string),
        owner_pubkey_npub: diagnostics
            .and_then(|value| value.get("owner_pubkey_npub"))
            .and_then(|value| value.as_str())
            .map(str::to_string),
        invalid_private_key: false,
    };

    if (status.public_key_hex.is_none() || status.public_key_npub.is_none())
        && let Some(secret) = private_key.as_deref()
    {
        #[cfg(feature = "nostr")]
        match Keys::parse(secret) {
            Ok(keys) => {
                let public_key_hex = keys.public_key().to_hex();
                let public_key_npub = keys
                    .public_key()
                    .to_bech32()
                    .unwrap_or_else(|_| public_key_hex.clone());
                status.public_key_hex = Some(public_key_hex);
                status.public_key_npub = Some(public_key_npub);
            }
            Err(_) => {
                status.invalid_private_key = true;
                status.needs_private_key = true;
                if !status
                    .missing_fields
                    .iter()
                    .any(|field| field == "private_key")
                {
                    status.missing_fields.push("private_key".to_string());
                }
            }
        }
        #[cfg(not(feature = "nostr"))]
        {
            let _ = secret;
        }
    }

    #[cfg(feature = "nostr")]
    if (status.owner_pubkey_hex.is_none() || status.owner_pubkey_npub.is_none())
        && let Some(owner) = resolved
            .as_ref()
            .and_then(|config| config.owner_pubkey.as_ref())
    {
        status.owner_pubkey_hex = Some(owner.clone());
        if let Ok(parsed) = crate::channels::nostr_runtime::parse_public_key(owner) {
            status.owner_pubkey_npub = parsed.to_bech32().ok();
        }
    }

    status.tool_ready = enabled && status.public_key_hex.is_some() && !status.invalid_private_key;
    status.configured = status.tool_ready;
    status.control_ready = status.tool_ready && owner_configured;

    status.relay_health = Some(
        match (
            enabled,
            status.connected_relay_count,
            status.invalid_private_key,
        ) {
            (_, _, true) => "invalid_private_key".to_string(),
            (false, _, _) => "disabled".to_string(),
            (true, Some(count), _) if count > 0 => format!("connected:{count}"),
            (true, Some(_), _) => "configured_not_connected".to_string(),
            (true, None, _) => "configured".to_string(),
        },
    );

    status
}

#[derive(serde::Serialize)]
struct ModelUsageEntry {
    model: String,
    input_tokens: u64,
    output_tokens: u64,
    cost: String,
}

#[derive(serde::Serialize)]
pub(crate) struct GatewayStatusResponse {
    sse_connections: u64,
    ws_connections: u64,
    total_connections: u64,
    uptime_secs: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    daily_cost: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    actions_this_hour: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_usage: Option<Vec<ModelUsageEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    budget_limit_usd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hourly_action_limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    active_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    active_cheap_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    routing_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    routing_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    primary_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime_reload_error: Option<String>,
    channel_setup: ChannelSetupStatus,
}

#[derive(serde::Serialize)]
struct ChannelSetupStatus {
    gmail: PartialChannelSetupStatus,
    nostr: PartialChannelSetupStatus,
}

#[derive(serde::Serialize)]
struct PartialChannelSetupStatus {
    enabled: bool,
    configured: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    missing_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    needs_oauth: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    needs_private_key: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    owner_configured: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    tool_ready: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    control_ready: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    social_dm_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    relay_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    connected_relay_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    relay_health: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    public_key_hex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    public_key_npub: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owner_pubkey_hex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owner_pubkey_npub: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    invalid_private_key: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
#[cfg(feature = "nostr")]
mod tests {
    use super::build_nostr_setup_status;

    #[test]
    fn nostr_status_marks_missing_owner_when_secret_exists() {
        let mut settings = crate::settings::Settings::default();
        settings.channels.nostr_enabled = true;

        unsafe {
            std::env::remove_var("NOSTR_ENABLED");
            std::env::set_var(
                "NOSTR_PRIVATE_KEY",
                "1111111111111111111111111111111111111111111111111111111111111111",
            );
            std::env::remove_var("NOSTR_OWNER_PUBKEY");
            std::env::remove_var("NOSTR_SECRET_KEY");
        }

        let status = build_nostr_setup_status(&settings, None);
        assert!(status.enabled);
        assert!(status.tool_ready);
        assert!(!status.control_ready);
        assert!(
            status
                .missing_fields
                .iter()
                .any(|field| field == "owner_pubkey")
        );

        unsafe {
            std::env::remove_var("NOSTR_ENABLED");
            std::env::remove_var("NOSTR_PRIVATE_KEY");
            std::env::remove_var("NOSTR_SECRET_KEY");
        }
    }
}

pub(crate) async fn costs_summary_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<crate::llm::cost_tracker::CostSummary>, StatusCode> {
    let tracker = state
        .cost_tracker
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let now = chrono::Utc::now();
    let today = now.format("%Y-%m-%d").to_string();
    let this_month = now.format("%Y-%m").to_string();
    let guard = tracker.lock().await;
    Ok(Json(guard.summary(&today, &this_month)))
}

pub(crate) async fn costs_export_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<
    (
        StatusCode,
        [(axum::http::header::HeaderName, String); 2],
        String,
    ),
    StatusCode,
> {
    let tracker = state
        .cost_tracker
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let guard = tracker.lock().await;
    let csv = guard.export_csv();
    let filename = format!(
        "thinclaw-costs-{}.csv",
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    );
    Ok((
        StatusCode::OK,
        [
            (
                axum::http::header::CONTENT_TYPE,
                "text/csv; charset=utf-8".to_string(),
            ),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", filename),
            ),
        ],
        csv,
    ))
}

pub(crate) async fn costs_reset_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<StatusCode, StatusCode> {
    let tracker = state
        .cost_tracker
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    {
        let mut guard = tracker.lock().await;
        guard.clear();
    }
    if let Some(ref cg) = state.cost_guard {
        cg.reset().await;
    }
    if let Some(ref db) = state.store {
        let snapshot = tracker.lock().await.to_json();
        if let Err(e) = db.set_setting("default", "cost_entries", &snapshot).await {
            tracing::warn!("Failed to persist cleared cost entries: {}", e);
        }
    }
    Ok(StatusCode::NO_CONTENT)
}
