use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

use crate::channels::web::handlers::nostr::reconcile_nostr_runtime;
use crate::channels::web::handlers::providers::reload_llm_runtime;
use crate::channels::web::identity_helpers::GatewayRequestIdentity;
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;

const REDACTED_SETTING_VALUE: &str = "[REDACTED]";

fn parse_timezone_setting_value(value: &serde_json::Value) -> Result<Option<String>, StatusCode> {
    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            if crate::timezone::parse_timezone(trimmed).is_some() {
                Ok(Some(trimmed.to_string()))
            } else {
                Err(StatusCode::BAD_REQUEST)
            }
        }
        _ => Err(StatusCode::BAD_REQUEST),
    }
}

fn is_sensitive_settings_key(key: &str) -> bool {
    matches!(
        key,
        "database_url"
            | "libsql_url"
            | "tunnel.ngrok_token"
            | "tunnel.cf_token"
            | "channels.discord_bot_token"
            | "channels.slack_bot_token"
            | "channels.slack_app_token"
            | "channels.gateway_auth_token"
            | "channels.nostr_private_key"
    )
}

fn redact_setting_value(key: &str, value: serde_json::Value) -> serde_json::Value {
    if is_sensitive_settings_key(key) {
        serde_json::Value::String(REDACTED_SETTING_VALUE.to_string())
    } else {
        value
    }
}

fn sanitize_imported_settings(
    settings: std::collections::HashMap<String, serde_json::Value>,
) -> std::collections::HashMap<String, serde_json::Value> {
    settings
        .into_iter()
        .filter(|(key, _)| !is_sensitive_settings_key(key))
        .collect()
}

fn normalize_telegram_transport_mode(value: Option<&str>) -> String {
    match value
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "" | "auto" | "automatic" | "webhook" => "auto".to_string(),
        "polling" | "poll" | "off" | "disabled" => "polling".to_string(),
        _ => "auto".to_string(),
    }
}

fn is_nostr_settings_key(key: &str) -> bool {
    key.starts_with("channels.nostr_") || key.starts_with("nostr_")
}

fn telegram_transport_runtime_updates(
    diagnostics: Option<&serde_json::Value>,
    transport_mode: &str,
) -> std::collections::HashMap<String, serde_json::Value> {
    let host_tunnel_url = diagnostics
        .and_then(|diag| diag.get("host_tunnel_url"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let host_webhook_capable = diagnostics
        .and_then(|diag| diag.get("host_webhook_capable"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let host_transport_reason = diagnostics
        .and_then(|diag| diag.get("host_transport_reason"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    let tunnel_url_value = if transport_mode == "auto" && host_webhook_capable {
        host_tunnel_url
            .clone()
            .map(serde_json::Value::String)
            .unwrap_or(serde_json::Value::Null)
    } else {
        serde_json::Value::Null
    };
    let transport_reason_value = match transport_mode {
        "polling" => serde_json::Value::String("operator forced polling".to_string()),
        _ => host_transport_reason
            .map(serde_json::Value::String)
            .unwrap_or(serde_json::Value::Null),
    };

    std::collections::HashMap::from([
        (
            "transport_preference".to_string(),
            serde_json::Value::String(transport_mode.to_string()),
        ),
        ("tunnel_url".to_string(), tunnel_url_value),
        ("transport_reason".to_string(), transport_reason_value),
    ])
}

pub(crate) async fn settings_list_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
) -> Result<Json<SettingsListResponse>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let rows = store
        .list_settings(&request_identity.principal_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to list settings: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let settings = rows
        .into_iter()
        .map(|r| SettingResponse {
            value: redact_setting_value(&r.key, r.value),
            key: r.key,
            updated_at: r.updated_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(SettingsListResponse { settings }))
}

pub(crate) async fn settings_get_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(key): Path<String>,
) -> Result<Json<SettingResponse>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let row = store
        .get_setting_full(&request_identity.principal_id, &key)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get setting '{}': {}", key, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(SettingResponse {
        value: redact_setting_value(&row.key, row.value),
        key: row.key,
        updated_at: row.updated_at.to_rfc3339(),
    }))
}

pub(crate) async fn settings_set_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(key): Path<String>,
    Json(body): Json<SettingWriteRequest>,
) -> Result<StatusCode, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    if is_sensitive_settings_key(&key) {
        tracing::warn!(
            key = %key,
            "Rejected settings write for sensitive key; use the secrets store instead"
        );
        return Err(StatusCode::FORBIDDEN);
    }

    if key == "user_timezone" {
        let timezone = parse_timezone_setting_value(&body.value)?;
        crate::timezone::apply_user_timezone_change(
            store,
            state.workspace.as_deref(),
            &request_identity.principal_id,
            timezone.as_deref(),
        )
        .await
        .map_err(|err| {
            tracing::error!("Failed to apply timezone update: {}", err);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
        return Ok(StatusCode::NO_CONTENT);
    }

    let cc_update: Option<(Option<String>, Option<u32>)> = match key.as_str() {
        "claude_code_model" => body.value.as_str().map(|v| (Some(v.to_string()), None)),
        "claude_code_max_turns" => body.value.as_u64().map(|n| (None, Some(n as u32))),
        _ => None,
    };
    let codex_update: Option<Option<String>> = match key.as_str() {
        "codex_code_model" => Some(body.value.as_str().map(|v| v.to_string())),
        _ => None,
    };

    let stream_mode_update: Option<(&'static str, crate::channels::StreamMode)> = match key.as_str()
    {
        "telegram_stream_mode" | "channels.telegram_stream_mode" => Some((
            "telegram",
            body.value
                .as_str()
                .map(crate::channels::StreamMode::from_str_value)
                .unwrap_or_default(),
        )),
        "discord_stream_mode" | "channels.discord_stream_mode" => Some((
            "discord",
            body.value
                .as_str()
                .map(crate::channels::StreamMode::from_str_value)
                .unwrap_or_default(),
        )),
        _ => None,
    };

    let telegram_session_mode_restart = matches!(
        key.as_str(),
        "telegram_subagent_session_mode" | "channels.telegram_subagent_session_mode"
    );
    let telegram_session_mode_update = telegram_session_mode_restart.then(|| {
        serde_json::Value::String(
            body.value
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("temp_topic")
                .to_string(),
        )
    });
    let telegram_transport_mode_update = match key.as_str() {
        "telegram_transport_mode" | "channels.telegram_transport_mode" => {
            Some(normalize_telegram_transport_mode(body.value.as_str()))
        }
        _ => None,
    };

    store
        .set_setting(&request_identity.principal_id, &key, &body.value)
        .await
        .map_err(|e| {
            tracing::error!("Failed to set setting '{}': {}", key, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    if let (Some(jm), Some((model, max_turns))) = (state.job_manager.clone(), cc_update) {
        tokio::spawn(async move {
            jm.update_claude_code_settings(model, max_turns).await;
        });
    }
    if let (Some(jm), Some(model)) = (state.job_manager.clone(), codex_update) {
        tokio::spawn(async move {
            jm.update_codex_code_settings(model).await;
        });
    }

    if let (Some(cm), Some((channel_name, mode))) =
        (state.channel_manager.clone(), stream_mode_update)
    {
        tokio::spawn(async move {
            cm.set_channel_stream_mode(channel_name, mode).await;
        });
    }

    if let (Some(cm), true) = (state.channel_manager.clone(), telegram_session_mode_restart) {
        tokio::spawn(async move {
            if let Some(mode) = telegram_session_mode_update {
                let updates =
                    std::collections::HashMap::from([("subagent_session_mode".to_string(), mode)]);
                if let Err(error) = cm.update_channel_runtime_config("telegram", updates).await {
                    tracing::warn!(
                        error = %error,
                        "Failed to stage Telegram subagent session mode update before restart"
                    );
                }
            }
            if let Err(error) = cm.restart_channel("telegram").await {
                tracing::warn!(
                    error = %error,
                    "Failed to hot-restart Telegram channel after subagent session mode update"
                );
            }
        });
    }

    if let (Some(cm), Some(transport_mode)) = (
        state.channel_manager.clone(),
        telegram_transport_mode_update,
    ) {
        tokio::spawn(async move {
            let diagnostics = cm.channel_diagnostics("telegram").await;
            let updates = telegram_transport_runtime_updates(diagnostics.as_ref(), &transport_mode);
            if let Err(error) = cm.update_channel_runtime_config("telegram", updates).await {
                tracing::warn!(
                    error = %error,
                    transport_mode = %transport_mode,
                    "Failed to stage Telegram transport update before restart"
                );
            }
            if let Err(error) = cm.reset_channel_connection_state("telegram").await {
                tracing::warn!(
                    error = %error,
                    "Failed to clear Telegram runtime fallback state before transport restart"
                );
            }
            if let Err(error) = cm.restart_channel("telegram").await {
                tracing::warn!(
                    error = %error,
                    transport_mode = %transport_mode,
                    "Failed to hot-restart Telegram channel after transport mode update"
                );
            }
        });
    }

    if key.starts_with("providers.")
        || matches!(
            key.as_str(),
            "llm_backend" | "selected_model" | "openai_compatible_base_url" | "ollama_base_url"
        )
    {
        reload_llm_runtime(state.as_ref()).await.map_err(|e| {
            tracing::error!(
                "Runtime reload failed after settings update '{}': {}",
                key,
                e
            );
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    }

    if is_nostr_settings_key(&key) {
        reconcile_nostr_runtime(state.as_ref(), &request_identity.principal_id)
            .await
            .map_err(|err| {
                tracing::error!(
                    "Nostr runtime reconcile failed after settings update '{}': {}",
                    key,
                    err
                );
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
    }

    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn settings_delete_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(key): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    if key == "user_timezone" {
        crate::timezone::apply_user_timezone_change(
            store,
            state.workspace.as_deref(),
            &request_identity.principal_id,
            None,
        )
        .await
        .map_err(|err| {
            tracing::error!("Failed to clear timezone setting: {}", err);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
        return Ok(StatusCode::NO_CONTENT);
    }

    store
        .delete_setting(&request_identity.principal_id, &key)
        .await
        .map_err(|e| {
            tracing::error!("Failed to delete setting '{}': {}", key, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    if let Some(cm) = state.channel_manager.clone() {
        match key.as_str() {
            "telegram_transport_mode" | "channels.telegram_transport_mode" => {
                tokio::spawn(async move {
                    let diagnostics = cm.channel_diagnostics("telegram").await;
                    let updates = telegram_transport_runtime_updates(diagnostics.as_ref(), "auto");
                    if let Err(error) = cm.update_channel_runtime_config("telegram", updates).await
                    {
                        tracing::warn!(
                            error = %error,
                            "Failed to restore Telegram auto transport before restart"
                        );
                    }
                    if let Err(error) = cm.reset_channel_connection_state("telegram").await {
                        tracing::warn!(
                            error = %error,
                            "Failed to clear Telegram runtime fallback state before restoring auto transport"
                        );
                    }
                    if let Err(error) = cm.restart_channel("telegram").await {
                        tracing::warn!(
                            error = %error,
                            "Failed to hot-restart Telegram channel after transport mode reset"
                        );
                    }
                });
            }
            "telegram_subagent_session_mode" | "channels.telegram_subagent_session_mode" => {
                tokio::spawn(async move {
                    let updates = std::collections::HashMap::from([(
                        "subagent_session_mode".to_string(),
                        serde_json::Value::String("temp_topic".to_string()),
                    )]);
                    if let Err(error) = cm.update_channel_runtime_config("telegram", updates).await
                    {
                        tracing::warn!(
                            error = %error,
                            "Failed to restore Telegram subagent session mode before restart"
                        );
                    }
                    if let Err(error) = cm.restart_channel("telegram").await {
                        tracing::warn!(
                            error = %error,
                            "Failed to hot-restart Telegram channel after subagent mode reset"
                        );
                    }
                });
            }
            _ => {}
        }
    }

    if key.starts_with("providers.")
        || matches!(
            key.as_str(),
            "llm_backend" | "selected_model" | "openai_compatible_base_url" | "ollama_base_url"
        )
    {
        reload_llm_runtime(state.as_ref()).await.map_err(|e| {
            tracing::error!(
                "Runtime reload failed after settings delete '{}': {}",
                key,
                e
            );
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    }

    if is_nostr_settings_key(&key) {
        reconcile_nostr_runtime(state.as_ref(), &request_identity.principal_id)
            .await
            .map_err(|err| {
                tracing::error!(
                    "Nostr runtime reconcile failed after settings delete '{}': {}",
                    key,
                    err
                );
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
    }

    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn settings_export_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
) -> Result<Json<SettingsExportResponse>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let settings = store
        .get_all_settings(&request_identity.principal_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to export settings: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let settings = settings
        .into_iter()
        .map(|(key, value)| {
            let value = redact_setting_value(&key, value);
            (key, value)
        })
        .collect();

    Ok(Json(SettingsExportResponse { settings }))
}

pub(crate) async fn webchat_presentation_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
) -> Result<Json<crate::config::WebChatPresentation>, StatusCode> {
    let webchat = if let Some(store) = state.store.as_ref() {
        let map = store
            .get_all_settings(&request_identity.principal_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to load webchat presentation settings: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
        let settings = crate::settings::Settings::from_db_map(&map);
        crate::config::WebChatConfig::from_settings(&settings)
    } else {
        crate::config::WebChatConfig::from_env()
    };

    Ok(Json(webchat.presentation_payload()))
}

pub(crate) async fn settings_import_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Json(body): Json<SettingsImportRequest>,
) -> Result<StatusCode, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let settings = sanitize_imported_settings(body.settings);
    let imported_timezone = match settings.get("user_timezone") {
        Some(value) => parse_timezone_setting_value(value)?,
        None => None,
    };
    store
        .set_all_settings(&request_identity.principal_id, &settings)
        .await
        .map_err(|e| {
            tracing::error!("Failed to import settings: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    if settings.contains_key("user_timezone") {
        crate::timezone::apply_user_timezone_change(
            store,
            state.workspace.as_deref(),
            &request_identity.principal_id,
            imported_timezone.as_deref(),
        )
        .await
        .map_err(|err| {
            tracing::error!("Failed to apply imported timezone setting: {}", err);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    }

    reload_llm_runtime(state.as_ref()).await.map_err(|e| {
        tracing::error!("Runtime reload failed after settings import: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    if settings.keys().any(|key| is_nostr_settings_key(key)) {
        reconcile_nostr_runtime(state.as_ref(), &request_identity.principal_id)
            .await
            .map_err(|err| {
                tracing::error!(
                    "Nostr runtime reconcile failed after settings import: {}",
                    err
                );
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
    }

    Ok(StatusCode::NO_CONTENT)
}
