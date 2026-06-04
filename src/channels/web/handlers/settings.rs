use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

#[cfg(feature = "nostr")]
use crate::channels::web::handlers::nostr::reconcile_nostr_runtime;
use crate::channels::web::handlers::providers::reload_llm_runtime;
use crate::channels::web::identity_helpers::GatewayRequestIdentity;
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;
use thinclaw_gateway::web::settings::{
    GatewaySettingRow, channel_stream_mode_update, claude_code_settings_update,
    codex_code_model_update, is_nostr_settings_key, is_sensitive_settings_key,
    is_telegram_subagent_session_mode_key, is_telegram_transport_mode_key,
    parse_timezone_setting_value, requires_llm_runtime_reload, sanitize_imported_settings,
    sensitive_setting_write_forbidden_status, setting_not_found_status, setting_response_from_row,
    settings_export_response_from_map, settings_list_response_from_rows,
    settings_store_unavailable_status, telegram_default_transport_mode,
    telegram_subagent_session_mode_reset_updates, telegram_subagent_session_mode_update,
    telegram_transport_runtime_updates,
};

pub(crate) async fn settings_list_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
) -> Result<Json<SettingsListResponse>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or_else(settings_store_unavailable_status)?;
    let rows = store
        .list_settings(&request_identity.principal_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to list settings: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(settings_list_response_from_rows(
        rows.into_iter().map(|row| GatewaySettingRow {
            key: row.key,
            value: row.value,
            updated_at: row.updated_at.to_rfc3339(),
        }),
    )))
}

pub(crate) async fn settings_get_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(key): Path<String>,
) -> Result<Json<SettingResponse>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or_else(settings_store_unavailable_status)?;
    let row = store
        .get_setting_full(&request_identity.principal_id, &key)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get setting '{}': {}", key, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or_else(setting_not_found_status)?;

    Ok(Json(setting_response_from_row(GatewaySettingRow {
        key: row.key,
        value: row.value,
        updated_at: row.updated_at.to_rfc3339(),
    })))
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
        .ok_or_else(settings_store_unavailable_status)?;

    if is_sensitive_settings_key(&key) {
        tracing::warn!(
            key = %key,
            "Rejected settings write for sensitive key; use the secrets store instead"
        );
        return Err(sensitive_setting_write_forbidden_status());
    }

    if key == "user_timezone" {
        let timezone = parse_timezone_setting_value(&body.value, |value| {
            crate::timezone::parse_timezone(value).is_some()
        })?;
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

    let cc_update = claude_code_settings_update(&key, &body.value);
    let codex_update = codex_code_model_update(&key, &body.value);
    let stream_mode_update = channel_stream_mode_update(&key, &body.value);
    let telegram_session_mode_update = telegram_subagent_session_mode_update(&key, &body.value);
    let telegram_transport_mode_update = if is_telegram_transport_mode_key(&key) {
        Some(
            thinclaw_config::channels::normalize_telegram_transport_mode(
                body.value.as_str().unwrap_or_default(),
            ),
        )
    } else {
        None
    };

    store
        .set_setting(&request_identity.principal_id, &key, &body.value)
        .await
        .map_err(|e| {
            tracing::error!("Failed to set setting '{}': {}", key, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    if let (Some(jm), Some(update)) = (state.job_manager.clone(), cc_update) {
        tokio::spawn(async move {
            jm.update_claude_code_settings(update.model, update.max_turns)
                .await;
        });
    }
    if let (Some(jm), Some(model)) = (state.job_manager.clone(), codex_update) {
        tokio::spawn(async move {
            jm.update_codex_code_settings(model).await;
        });
    }

    if let (Some(cm), Some(update)) = (state.channel_manager.clone(), stream_mode_update) {
        tokio::spawn(async move {
            cm.set_channel_stream_mode(update.channel_name, update.mode)
                .await;
        });
    }

    if let (Some(cm), Some(mode)) = (state.channel_manager.clone(), telegram_session_mode_update) {
        tokio::spawn(async move {
            let updates = std::collections::HashMap::from([(
                "subagent_session_mode".to_string(),
                serde_json::Value::String(mode),
            )]);
            if let Err(error) = cm.update_channel_runtime_config("telegram", updates).await {
                tracing::warn!(
                    error = %error,
                    "Failed to stage Telegram subagent session mode update before restart"
                );
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

    if requires_llm_runtime_reload(&key) {
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
        #[cfg(feature = "nostr")]
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
        .ok_or_else(settings_store_unavailable_status)?;

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
        if is_telegram_transport_mode_key(&key) {
            tokio::spawn(async move {
                let diagnostics = cm.channel_diagnostics("telegram").await;
                let updates = telegram_transport_runtime_updates(
                    diagnostics.as_ref(),
                    telegram_default_transport_mode(),
                );
                if let Err(error) = cm.update_channel_runtime_config("telegram", updates).await {
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
        } else if is_telegram_subagent_session_mode_key(&key) {
            tokio::spawn(async move {
                let updates = telegram_subagent_session_mode_reset_updates();
                if let Err(error) = cm.update_channel_runtime_config("telegram", updates).await {
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
    }

    if requires_llm_runtime_reload(&key) {
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
        #[cfg(feature = "nostr")]
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
        .ok_or_else(settings_store_unavailable_status)?;
    let settings = store
        .get_all_settings(&request_identity.principal_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to export settings: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(settings_export_response_from_map(settings)))
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
        .ok_or_else(settings_store_unavailable_status)?;
    let settings = sanitize_imported_settings(body.settings);
    let imported_timezone = match settings.get("user_timezone") {
        Some(value) => parse_timezone_setting_value(value, |value| {
            crate::timezone::parse_timezone(value).is_some()
        })?,
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
        #[cfg(feature = "nostr")]
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
