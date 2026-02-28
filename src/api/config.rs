//! Config/settings API — get and set user preferences.
//!
//! Extracted from `channels/web/handlers/settings.rs`.

use std::sync::Arc;

use crate::channels::web::types::*;
use crate::db::Database;

use super::error::{ApiError, ApiResult};

/// List all settings for a user.
pub async fn list_settings(
    store: &Arc<dyn Database>,
    user_id: &str,
) -> ApiResult<SettingsListResponse> {
    let rows = store
        .list_settings(user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let settings = rows
        .into_iter()
        .map(|r| SettingResponse {
            key: r.key,
            value: r.value,
            updated_at: r.updated_at.to_rfc3339(),
        })
        .collect();

    Ok(SettingsListResponse { settings })
}

/// Get a single setting by key.
pub async fn get_setting(
    store: &Arc<dyn Database>,
    user_id: &str,
    key: &str,
) -> ApiResult<SettingResponse> {
    let row = store
        .get_setting_full(user_id, key)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(format!("Setting '{}' not found", key)))?;

    Ok(SettingResponse {
        key: row.key,
        value: row.value,
        updated_at: row.updated_at.to_rfc3339(),
    })
}

/// Set a setting value.
pub async fn set_setting(
    store: &Arc<dyn Database>,
    user_id: &str,
    key: &str,
    value: &serde_json::Value,
) -> ApiResult<()> {
    store
        .set_setting(user_id, key, value)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(())
}

/// Delete a setting.
pub async fn delete_setting(store: &Arc<dyn Database>, user_id: &str, key: &str) -> ApiResult<()> {
    store
        .delete_setting(user_id, key)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(())
}

/// Export all settings.
pub async fn export_settings(
    store: &Arc<dyn Database>,
    user_id: &str,
) -> ApiResult<SettingsExportResponse> {
    let settings = store
        .get_all_settings(user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(SettingsExportResponse { settings })
}

/// Import settings (bulk set).
pub async fn import_settings(
    store: &Arc<dyn Database>,
    user_id: &str,
    settings: &std::collections::HashMap<String, serde_json::Value>,
) -> ApiResult<()> {
    store
        .set_all_settings(user_id, settings)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(())
}
