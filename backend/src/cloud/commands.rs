//! Tauri commands for cloud storage operations.
//!
//! These commands are the IPC interface between the frontend and the
//! `CloudManager` backend state.

use serde::{Deserialize, Serialize};
use tauri::State;
use tracing::info;

use super::provider::CloudProviderConfig;
use super::{CloudManager, CloudManagerStatus, StorageMode};

// ── Types for Specta/TypeScript ──────────────────────────────────────────────

/// Cloud status response for the frontend.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct CloudStatusResponse {
    pub mode: String,
    pub provider_connected: bool,
    pub provider_name: Option<String>,
    pub storage_used: f64,
    pub storage_available: Option<f64>,
    #[specta(type = Option<f64>)]
    pub last_sync_at: Option<i64>,
    pub has_recovery_key: bool,
    pub migration_in_progress: bool,
}

impl From<CloudManagerStatus> for CloudStatusResponse {
    fn from(s: CloudManagerStatus) -> Self {
        Self {
            mode: match &s.mode {
                StorageMode::Local => "local".to_string(),
                StorageMode::Cloud { provider_type, .. } => format!("cloud:{}", provider_type),
            },
            provider_connected: s.provider_connected,
            provider_name: s.provider_name,
            storage_used: s.storage_used as f64,
            storage_available: s.storage_available.map(|v| v as f64),
            last_sync_at: s.last_sync_at,
            has_recovery_key: s.has_recovery_key,
            migration_in_progress: s.migration_in_progress,
        }
    }
}

/// S3 provider configuration input from the frontend.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct S3ConfigInput {
    pub endpoint: Option<String>,
    pub bucket: String,
    pub region: Option<String>,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub root: Option<String>,
}

/// Connection test result for the frontend.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ConnectionTestResult {
    pub connected: bool,
    pub provider_name: String,
    pub storage_used: f64,
    pub storage_available: Option<f64>,
    pub error: Option<String>,
}

// ── Commands ─────────────────────────────────────────────────────────────────

/// Get the current cloud storage status.
#[tauri::command]
#[specta::specta]
pub async fn cloud_get_status(
    cloud: State<'_, CloudManager>,
) -> Result<CloudStatusResponse, String> {
    Ok(cloud.get_status().await.into())
}

/// Configure and test an S3-compatible cloud provider.
#[tauri::command]
#[specta::specta]
pub async fn cloud_test_connection(
    cloud: State<'_, CloudManager>,
    config: S3ConfigInput,
) -> Result<ConnectionTestResult, String> {
    let provider_config = CloudProviderConfig {
        provider_type: "s3".to_string(),
        endpoint: config.endpoint,
        bucket: Some(config.bucket),
        region: config.region,
        access_key_id: Some(config.access_key_id),
        secret_access_key: Some(config.secret_access_key),
        root: config.root,
    };

    match cloud.configure_provider(provider_config).await {
        Ok(status) => Ok(ConnectionTestResult {
            connected: status.connected,
            provider_name: status.provider_name,
            storage_used: status.storage_used as f64,
            storage_available: status.storage_available.map(|v| v as f64),
            error: None,
        }),
        Err(e) => Ok(ConnectionTestResult {
            connected: false,
            provider_name: "Unknown".to_string(),
            storage_used: 0.0,
            storage_available: None,
            error: Some(e.to_string()),
        }),
    }
}

/// Start migration from local to cloud storage.
#[tauri::command]
#[specta::specta]
pub async fn cloud_migrate_to_cloud(
    cloud: State<'_, CloudManager>,
    app: tauri::AppHandle,
    db: State<'_, sqlx::SqlitePool>,
) -> Result<(), String> {
    info!("[cloud] Frontend requested: migrate to cloud");
    cloud.migrate_to_cloud(app.clone(), &db).await
}

/// Start migration from cloud to local storage.
#[tauri::command]
#[specta::specta]
pub async fn cloud_migrate_to_local(
    cloud: State<'_, CloudManager>,
    app: tauri::AppHandle,
    db: State<'_, sqlx::SqlitePool>,
) -> Result<(), String> {
    info!("[cloud] Frontend requested: migrate to local");
    cloud.migrate_to_local(app.clone(), &db).await
}

/// Cancel an in-progress migration.
#[tauri::command]
#[specta::specta]
pub async fn cloud_cancel_migration(cloud: State<'_, CloudManager>) -> Result<(), String> {
    cloud.cancel_migration().await
}

/// Get the recovery key (base64-encoded master encryption key).
#[tauri::command]
#[specta::specta]
pub async fn cloud_get_recovery_key(cloud: State<'_, CloudManager>) -> Result<String, String> {
    cloud.get_recovery_key().await
}

/// Import a recovery key (for restoring on a new device).
#[tauri::command]
#[specta::specta]
pub async fn cloud_import_recovery_key(
    cloud: State<'_, CloudManager>,
    recovery_key: String,
) -> Result<(), String> {
    cloud.import_recovery_key(&recovery_key).await
}

/// Get storage breakdown by category (for the progress bar UI).
#[tauri::command]
#[specta::specta]
pub async fn cloud_get_storage_breakdown(
    app: tauri::AppHandle,
) -> Result<Vec<StorageCategory>, String> {
    use tauri::Manager;
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;

    let categories = vec![
        ("generated", "Generated Images", "generated/"),
        ("documents", "Documents", "documents/"),
        ("images", "Chat Images", "images/"),
        ("database", "Database", ""),
        ("ironclaw_db", "Agent Database", "ironclaw"),
        ("vectors", "Vector Indices", "vectors/"),
        ("previews", "Previews", "previews/"),
        ("openclaw", "Agent State", "openclaw/"),
    ];

    let mut result = Vec::new();
    for (id, label, subdir) in categories {
        let path = if subdir.is_empty() {
            app_data_dir.join("openclaw.db")
        } else if subdir == "ironclaw" {
            app_data_dir.join("ironclaw.db")
        } else {
            app_data_dir.join(subdir)
        };

        let size = if path.is_file() {
            tokio::fs::metadata(&path)
                .await
                .map(|m| m.len())
                .unwrap_or(0)
        } else if path.is_dir() {
            dir_size(&path).await
        } else {
            0
        };

        result.push(StorageCategory {
            id: id.to_string(),
            label: label.to_string(),
            size_bytes: size as f64,
        });
    }

    Ok(result)
}

/// A storage category for the breakdown UI.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct StorageCategory {
    pub id: String,
    pub label: String,
    pub size_bytes: f64,
}

/// Recursively calculate directory size.
async fn dir_size(path: &std::path::Path) -> u64 {
    let mut total = 0u64;
    if let Ok(mut entries) = tokio::fs::read_dir(path).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let metadata = entry.metadata().await.unwrap_or_else(|_| {
                std::fs::metadata(entry.path()).unwrap_or_else(|_| {
                    // Fallback: skip
                    return std::fs::metadata("/dev/null").unwrap();
                })
            });
            if metadata.is_file() {
                total += metadata.len();
            } else if metadata.is_dir() {
                total += Box::pin(dir_size(&entry.path())).await;
            }
        }
    }
    total
}
