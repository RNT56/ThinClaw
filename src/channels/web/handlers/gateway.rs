use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};

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

    ChannelSetupStatus {
        gmail: build_gmail_setup_status(&settings),
        nostr: build_nostr_setup_status(&settings),
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
    }
}

fn build_nostr_setup_status(settings: &crate::settings::Settings) -> PartialChannelSetupStatus {
    let enabled =
        crate::config::helpers::parse_bool_env("NOSTR_ENABLED", settings.channels.nostr_enabled)
            .unwrap_or(settings.channels.nostr_enabled);
    let has_private_key = crate::config::helpers::optional_env("NOSTR_PRIVATE_KEY")
        .ok()
        .flatten()
        .or_else(|| {
            crate::config::helpers::optional_env("NOSTR_SECRET_KEY")
                .ok()
                .flatten()
        })
        .is_some();

    PartialChannelSetupStatus {
        enabled,
        configured: enabled && has_private_key,
        missing_fields: Vec::new(),
        needs_oauth: false,
        needs_private_key: enabled && !has_private_key,
    }
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
}

fn is_false(value: &bool) -> bool {
    !*value
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
