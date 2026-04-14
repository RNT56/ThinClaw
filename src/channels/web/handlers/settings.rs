use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

use crate::channels::web::handlers::providers::reload_llm_runtime;
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;

const REDACTED_SETTING_VALUE: &str = "[REDACTED]";

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

pub(crate) async fn settings_list_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<SettingsListResponse>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let rows = store.list_settings(&state.user_id).await.map_err(|e| {
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
    Path(key): Path<String>,
) -> Result<Json<SettingResponse>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let row = store
        .get_setting_full(&state.user_id, &key)
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

    let cc_update: Option<(Option<String>, Option<u32>)> = match key.as_str() {
        "claude_code_model" => body.value.as_str().map(|v| (Some(v.to_string()), None)),
        "claude_code_max_turns" => body.value.as_u64().map(|n| (None, Some(n as u32))),
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

    store
        .set_setting(&state.user_id, &key, &body.value)
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

    if let (Some(cm), Some((channel_name, mode))) =
        (state.channel_manager.clone(), stream_mode_update)
    {
        tokio::spawn(async move {
            cm.set_channel_stream_mode(channel_name, mode).await;
        });
    }

    if let (Some(cm), true) = (state.channel_manager.clone(), telegram_session_mode_restart) {
        tokio::spawn(async move {
            if let Err(error) = cm.restart_channel("telegram").await {
                tracing::warn!(
                    error = %error,
                    "Failed to hot-restart Telegram channel after subagent session mode update"
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

    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn settings_delete_handler(
    State(state): State<Arc<GatewayState>>,
    Path(key): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    store
        .delete_setting(&state.user_id, &key)
        .await
        .map_err(|e| {
            tracing::error!("Failed to delete setting '{}': {}", key, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

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

    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn settings_export_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<SettingsExportResponse>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let settings = store.get_all_settings(&state.user_id).await.map_err(|e| {
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

pub(crate) async fn settings_import_handler(
    State(state): State<Arc<GatewayState>>,
    Json(body): Json<SettingsImportRequest>,
) -> Result<StatusCode, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let settings = sanitize_imported_settings(body.settings);
    store
        .set_all_settings(&state.user_id, &settings)
        .await
        .map_err(|e| {
            tracing::error!("Failed to import settings: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    reload_llm_runtime(state.as_ref()).await.map_err(|e| {
        tracing::error!("Runtime reload failed after settings import: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(StatusCode::NO_CONTENT)
}
