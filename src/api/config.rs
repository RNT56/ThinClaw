//! Config/settings API — get and set user preferences.
//!
//! Extracted from `channels/web/handlers/settings.rs`.

use std::sync::Arc;

use crate::channels::web::types::*;
use crate::db::Database;
use thinclaw_config::setting_not_found_message;
use thinclaw_gateway::web::settings::{
    GatewaySettingRow, is_sensitive_settings_key, setting_response_from_row,
    settings_export_response_from_map, settings_list_response_from_rows,
    validate_sandbox_code_setting, validate_setting_entry, validate_settings_key,
};

use super::error::{ApiError, ApiResult};

fn validate_settings_user_id(user_id: &str) -> ApiResult<()> {
    if user_id.trim().is_empty() || user_id.len() > 256 || user_id.chars().any(char::is_control) {
        return Err(ApiError::InvalidInput(
            "settings user id is malformed or oversized".to_string(),
        ));
    }
    Ok(())
}

fn validate_config_entry(key: &str, value: &serde_json::Value) -> ApiResult<()> {
    if is_sensitive_settings_key(key) {
        return Err(ApiError::InvalidInput(
            "plaintext sensitive settings are forbidden; use the encrypted secrets store"
                .to_string(),
        ));
    }
    validate_setting_entry(key, value)
        .and_then(|_| validate_sandbox_code_setting(key, value))
        .map_err(|_| ApiError::InvalidInput(format!("setting '{key}' is malformed or oversized")))
}

/// List all settings for a user.
pub async fn list_settings(
    store: &Arc<dyn Database>,
    user_id: &str,
) -> ApiResult<SettingsListResponse> {
    validate_settings_user_id(user_id)?;
    let rows = store
        .list_settings(user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let settings = rows.into_iter().map(|r| GatewaySettingRow {
        key: r.key,
        value: r.value,
        updated_at: r.updated_at.to_rfc3339(),
    });

    Ok(settings_list_response_from_rows(settings))
}

/// Get a single setting by key.
pub async fn get_setting(
    store: &Arc<dyn Database>,
    user_id: &str,
    key: &str,
) -> ApiResult<SettingResponse> {
    validate_settings_user_id(user_id)?;
    validate_settings_key(key)
        .map_err(|_| ApiError::InvalidInput("setting key is malformed or oversized".to_string()))?;
    let row = store
        .get_setting_full(user_id, key)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(setting_not_found_message(key)))?;

    Ok(setting_response_from_row(GatewaySettingRow {
        key: row.key,
        value: row.value,
        updated_at: row.updated_at.to_rfc3339(),
    }))
}

/// Set a setting value.
pub async fn set_setting(
    store: &Arc<dyn Database>,
    user_id: &str,
    key: &str,
    value: &serde_json::Value,
) -> ApiResult<()> {
    validate_settings_user_id(user_id)?;
    validate_config_entry(key, value)?;
    store
        .set_setting(user_id, key, value)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    invalidate_learning_cache_if_needed(store, user_id, key).await;
    Ok(())
}

/// Delete a setting.
pub async fn delete_setting(store: &Arc<dyn Database>, user_id: &str, key: &str) -> ApiResult<()> {
    validate_settings_user_id(user_id)?;
    validate_settings_key(key)
        .map_err(|_| ApiError::InvalidInput("setting key is malformed or oversized".to_string()))?;
    store
        .delete_setting(user_id, key)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    invalidate_learning_cache_if_needed(store, user_id, key).await;
    Ok(())
}

/// Generic settings writes that touch `learning.*` must invalidate the
/// ready-provider cache, or the pre-change provider keeps serving recall and
/// sync for up to the cache TTL after an operator changes it via the UI.
async fn invalidate_learning_cache_if_needed(store: &Arc<dyn Database>, user_id: &str, key: &str) {
    if key.starts_with("learning.") {
        crate::agent::learning::invalidate_provider_ready_cache(store, user_id).await;
    }
}

/// Export all settings.
pub async fn export_settings(
    store: &Arc<dyn Database>,
    user_id: &str,
) -> ApiResult<SettingsExportResponse> {
    validate_settings_user_id(user_id)?;
    let settings = store
        .get_all_settings(user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(settings_export_response_from_map(settings))
}

/// Import settings (bulk set).
pub async fn import_settings(
    store: &Arc<dyn Database>,
    user_id: &str,
    settings: &std::collections::HashMap<String, serde_json::Value>,
) -> ApiResult<()> {
    validate_settings_user_id(user_id)?;
    if settings.len() > 10_000 {
        return Err(ApiError::InvalidInput(
            "settings import contains more than 10000 entries".to_string(),
        ));
    }
    let mut total_bytes = 0usize;
    for (key, value) in settings {
        validate_config_entry(key, value)?;
        total_bytes = total_bytes.saturating_add(key.len()).saturating_add(
            serde_json::to_vec(value)
                .map_err(ApiError::Serialization)?
                .len(),
        );
        if total_bytes > 4 * 1024 * 1024 {
            return Err(ApiError::InvalidInput(
                "settings import exceeds the 4 MiB aggregate limit".to_string(),
            ));
        }
    }
    store
        .set_all_settings(user_id, settings)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if settings.keys().any(|key| key.starts_with("learning.")) {
        crate::agent::learning::invalidate_provider_ready_cache(store, user_id).await;
    }
    Ok(())
}

/// Reload secrets from the store into the config overlay.
///
/// Zero-downtime secret refresh: when a user updates an API key in ThinClaw Desktop's
/// UI, call this instead of stop→start. Re-reads all secrets from the store,
/// updates the injected vars overlay, and the next LLM call picks up the new
/// keys automatically.
///
/// Returns the number of secrets loaded.
pub async fn refresh_secrets(
    secrets: &dyn crate::secrets::SecretsStore,
    user_id: &str,
) -> ApiResult<usize> {
    let count = crate::config::refresh_secrets(secrets, user_id).await;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_boundary_rejects_plaintext_secret_namespace_and_bad_values() {
        assert!(validate_config_entry("agent.name", &serde_json::json!("ThinClaw")).is_ok());
        assert!(
            validate_config_entry("secret.OPENAI_API_KEY", &serde_json::json!("sk-plaintext"))
                .is_err()
        );
        assert!(
            validate_config_entry(
                "channels.gateway_auth_token",
                &serde_json::json!("plaintext-token")
            )
            .is_err()
        );
        assert!(validate_config_entry("agent..name", &serde_json::json!("bad")).is_err());
        assert!(validate_config_entry("codex_code_model", &serde_json::json!(42)).is_err());
    }

    #[test]
    fn settings_user_ids_are_bounded() {
        assert!(validate_settings_user_id("local_user").is_ok());
        assert!(validate_settings_user_id("bad\nuser").is_err());
        assert!(validate_settings_user_id(&"x".repeat(257)).is_err());
    }
}
