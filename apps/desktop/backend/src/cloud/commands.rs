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

/// WebDAV provider configuration input from the frontend.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct WebDavConfigInput {
    pub endpoint: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub root: Option<String>,
}

/// SFTP provider configuration input from the frontend.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct SftpConfigInput {
    pub endpoint: String,
    pub username: Option<String>,
    /// Path to SSH private key (e.g. `~/.ssh/id_rsa`) or password
    pub key_or_password: Option<String>,
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

/// OAuth flow start result for the frontend.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct OAuthStartResult {
    /// The authorization URL to open in the user's browser.
    pub auth_url: String,
    /// The PKCE code verifier — must be passed back in `cloud_oauth_complete`.
    pub code_verifier: String,
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
    db: State<'_, sqlx::SqlitePool>,
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

    test_provider(cloud, db, provider_config).await
}

/// Configure and test iCloud Drive storage.
///
/// iCloud requires no configuration — it uses the native macOS container.
#[tauri::command]
#[specta::specta]
pub async fn cloud_test_icloud(
    cloud: State<'_, CloudManager>,
    db: State<'_, sqlx::SqlitePool>,
) -> Result<ConnectionTestResult, String> {
    let provider_config = CloudProviderConfig {
        provider_type: "icloud".to_string(),
        endpoint: None,
        bucket: None,
        region: None,
        access_key_id: None,
        secret_access_key: None,
        root: None,
    };

    test_provider(cloud, db, provider_config).await
}

/// Configure and test a WebDAV provider.
#[tauri::command]
#[specta::specta]
pub async fn cloud_test_webdav(
    cloud: State<'_, CloudManager>,
    db: State<'_, sqlx::SqlitePool>,
    config: WebDavConfigInput,
) -> Result<ConnectionTestResult, String> {
    let provider_config = CloudProviderConfig {
        provider_type: "webdav".to_string(),
        endpoint: Some(config.endpoint),
        bucket: None,
        region: None,
        access_key_id: config.username,
        secret_access_key: config.password,
        root: config.root,
    };

    test_provider(cloud, db, provider_config).await
}

/// Configure and test an SFTP provider.
#[tauri::command]
#[specta::specta]
pub async fn cloud_test_sftp(
    cloud: State<'_, CloudManager>,
    db: State<'_, sqlx::SqlitePool>,
    config: SftpConfigInput,
) -> Result<ConnectionTestResult, String> {
    let provider_config = CloudProviderConfig {
        provider_type: "sftp".to_string(),
        endpoint: Some(config.endpoint),
        bucket: None,
        region: None,
        access_key_id: config.username,
        secret_access_key: config.key_or_password,
        root: config.root,
    };

    test_provider(cloud, db, provider_config).await
}

/// Start the OAuth 2.0 PKCE flow for a cloud provider.
///
/// Returns the authorization URL and PKCE code verifier.
/// The frontend should:
/// 1. Open the `auth_url` in the system browser
/// 2. Listen for the redirect on `http://localhost:11434/callback` (or similar)
/// 3. Pass the received `code` + `code_verifier` to `cloud_oauth_complete()`
#[tauri::command]
#[specta::specta]
pub async fn cloud_oauth_start(provider: String) -> Result<OAuthStartResult, String> {
    use super::oauth::{OAuthConfig, OAuthManager};

    let client_id = match provider.as_str() {
        "gdrive" => std::env::var("GOOGLE_CLIENT_ID")
            .unwrap_or_else(|_| "thinclaw-desktop.apps.googleusercontent.com".to_string()),
        "dropbox" => std::env::var("DROPBOX_CLIENT_ID")
            .unwrap_or_else(|_| "thinclaw_desktop_app".to_string()),
        "onedrive" => std::env::var("ONEDRIVE_CLIENT_ID")
            .unwrap_or_else(|_| "thinclaw-desktop-app".to_string()),
        other => return Err(format!("OAuth not supported for provider: {}", other)),
    };

    let config = match provider.as_str() {
        "gdrive" => OAuthConfig::google_drive(client_id),
        "dropbox" => OAuthConfig::dropbox(client_id),
        "onedrive" => OAuthConfig::onedrive(client_id),
        _ => unreachable!(),
    };

    let oauth = OAuthManager::new(config);
    let (auth_url, code_verifier) = oauth.authorize_url();

    info!("[cloud/oauth] Started {} OAuth flow", provider);

    Ok(OAuthStartResult {
        auth_url,
        code_verifier,
    })
}

/// Complete the OAuth 2.0 flow by exchanging the authorization code for tokens.
///
/// On success, the provider is configured and a connection test is performed.
#[tauri::command]
#[specta::specta]
pub async fn cloud_oauth_complete(
    cloud: State<'_, CloudManager>,
    db: State<'_, sqlx::SqlitePool>,
    provider: String,
    code: String,
    code_verifier: String,
) -> Result<ConnectionTestResult, String> {
    use super::oauth::{OAuthConfig, OAuthManager};
    use super::provider::CloudProvider;

    let client_id = match provider.as_str() {
        "gdrive" => std::env::var("GOOGLE_CLIENT_ID")
            .unwrap_or_else(|_| "thinclaw-desktop.apps.googleusercontent.com".to_string()),
        "dropbox" => std::env::var("DROPBOX_CLIENT_ID")
            .unwrap_or_else(|_| "thinclaw_desktop_app".to_string()),
        "onedrive" => std::env::var("ONEDRIVE_CLIENT_ID")
            .unwrap_or_else(|_| "thinclaw-desktop-app".to_string()),
        other => return Err(format!("OAuth not supported for provider: {}", other)),
    };

    let config = match provider.as_str() {
        "gdrive" => OAuthConfig::google_drive(client_id),
        "dropbox" => OAuthConfig::dropbox(client_id),
        "onedrive" => OAuthConfig::onedrive(client_id),
        _ => unreachable!(),
    };

    let oauth = OAuthManager::new(config);

    // Exchange code for tokens
    let tokens = oauth
        .exchange_code(&code, &code_verifier)
        .await
        .map_err(|e| format!("OAuth token exchange failed: {}", e))?;

    // Save tokens to Keychain
    oauth
        .save_tokens_to_keychain(&tokens)
        .map_err(|e| format!("Failed to save OAuth tokens: {}", e))?;

    info!(
        "[cloud/oauth] {} OAuth flow completed, tokens saved",
        provider
    );

    // Create the provider and test connection
    let boxed_provider: Box<dyn CloudProvider> = match provider.as_str() {
        "gdrive" => Box::new(super::providers::gdrive::GDriveProvider::new(oauth)),
        "dropbox" => Box::new(super::providers::dropbox::DropboxProvider::new(oauth)),
        "onedrive" => Box::new(super::providers::onedrive::OneDriveProvider::new(oauth)),
        _ => unreachable!(),
    };

    match boxed_provider.test_connection().await {
        Ok(status) => {
            // Store the provider in CloudManager
            let provider_config = CloudProviderConfig {
                provider_type: provider.clone(),
                endpoint: None,
                bucket: None,
                region: None,
                access_key_id: None,
                secret_access_key: None,
                root: None,
            };
            // Update CloudManager's inner state with this provider
            cloud
                .set_provider(boxed_provider, provider_config.clone())
                .await;
            persist_provider_config(&db, &provider_config).await?;

            Ok(ConnectionTestResult {
                connected: status.connected,
                provider_name: status.provider_name,
                storage_used: status.storage_used as f64,
                storage_available: status.storage_available.map(|v| v as f64),
                error: None,
            })
        }
        Err(e) => Ok(ConnectionTestResult {
            connected: false,
            provider_name: provider,
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
        ("thinclaw", "Agent State", "thinclaw/"),
    ];

    let mut result = Vec::new();
    for (id, label, subdir) in categories {
        let path = if subdir.is_empty() {
            app_data_dir.join("thinclaw.db")
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

// ── Private Helpers ──────────────────────────────────────────────────────────

/// Shared helper for testing a provider connection.
async fn test_provider(
    cloud: State<'_, CloudManager>,
    db: State<'_, sqlx::SqlitePool>,
    config: CloudProviderConfig,
) -> Result<ConnectionTestResult, String> {
    match cloud.configure_provider(config).await {
        Ok(status) => {
            if let Some(config) = cloud.provider_config().await {
                persist_provider_config(&db, &config).await?;
            }
            Ok(ConnectionTestResult {
                connected: status.connected,
                provider_name: status.provider_name,
                storage_used: status.storage_used as f64,
                storage_available: status.storage_available.map(|v| v as f64),
                error: None,
            })
        }
        Err(e) => Ok(ConnectionTestResult {
            connected: false,
            provider_name: "Unknown".to_string(),
            storage_used: 0.0,
            storage_available: None,
            error: Some(e.to_string()),
        }),
    }
}

async fn persist_provider_config(
    db: &sqlx::SqlitePool,
    config: &CloudProviderConfig,
) -> Result<(), String> {
    super::save_provider_credentials(config)?;
    let config_json = provider_config_json_for_persistence(config)?;
    sqlx::query("INSERT OR REPLACE INTO cloud_config (key, value) VALUES ('provider_config', ?)")
        .bind(config_json)
        .execute(db)
        .await
        .map_err(|e| format!("Failed to persist provider config: {}", e))?;
    Ok(())
}

fn provider_config_json_for_persistence(config: &CloudProviderConfig) -> Result<String, String> {
    serde_json::to_string(&config.sanitized_for_persistence())
        .map_err(|e| format!("Failed to serialize provider config: {}", e))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_config_persistence_sanitizes_secrets() {
        let config = CloudProviderConfig {
            provider_type: "webdav".to_string(),
            endpoint: Some("https://dav.example.com".to_string()),
            bucket: None,
            region: None,
            access_key_id: Some("alice".to_string()),
            secret_access_key: Some("password".to_string()),
            root: Some("thinclaw-desktop/".to_string()),
        };

        let json = provider_config_json_for_persistence(&config).unwrap();
        assert!(!json.contains("alice"));
        assert!(!json.contains("password"));

        let persisted: CloudProviderConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(persisted.endpoint, config.endpoint);
        assert_eq!(persisted.root, config.root);
        assert_eq!(persisted.access_key_id, None);
        assert_eq!(persisted.secret_access_key, None);
    }
}
