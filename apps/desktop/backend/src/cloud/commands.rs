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
    pub sync_active: bool,
    pub sync_error: Option<String>,
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
            sync_active: s.sync_active,
            sync_error: s.sync_error,
            has_recovery_key: s.has_recovery_key,
            migration_in_progress: s.migration_in_progress,
        }
    }
}

/// S3 provider configuration input from the frontend.
#[derive(Clone, Serialize, Deserialize, specta::Type)]
pub struct S3ConfigInput {
    pub endpoint: Option<String>,
    pub bucket: String,
    pub region: Option<String>,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub root: Option<String>,
}

impl std::fmt::Debug for S3ConfigInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("S3ConfigInput")
            .field("endpoint_configured", &self.endpoint.is_some())
            .field("bucket", &self.bucket)
            .field("region", &self.region)
            .field("access_key_id", &crate::debug_redaction::Redacted)
            .field("secret_access_key", &crate::debug_redaction::Redacted)
            .field("root_configured", &self.root.is_some())
            .finish()
    }
}

/// WebDAV provider configuration input from the frontend.
#[derive(Clone, Serialize, Deserialize, specta::Type)]
pub struct WebDavConfigInput {
    pub endpoint: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub root: Option<String>,
}

impl std::fmt::Debug for WebDavConfigInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WebDavConfigInput")
            .field("endpoint_configured", &!self.endpoint.is_empty())
            .field(
                "username",
                &crate::debug_redaction::RedactedOption(&self.username),
            )
            .field(
                "password",
                &crate::debug_redaction::RedactedOption(&self.password),
            )
            .field("root_configured", &self.root.is_some())
            .finish()
    }
}

/// SFTP provider configuration input from the frontend.
#[derive(Clone, Serialize, Deserialize, specta::Type)]
pub struct SftpConfigInput {
    pub endpoint: String,
    pub username: Option<String>,
    /// Path to an SSH private key (e.g. `~/.ssh/id_rsa`).
    pub key_or_password: Option<String>,
    pub root: Option<String>,
}

impl std::fmt::Debug for SftpConfigInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SftpConfigInput")
            .field("endpoint_configured", &!self.endpoint.is_empty())
            .field(
                "username",
                &crate::debug_redaction::RedactedOption(&self.username),
            )
            .field(
                "key_or_password",
                &crate::debug_redaction::RedactedOption(&self.key_or_password),
            )
            .field("root_configured", &self.root.is_some())
            .finish()
    }
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
    /// Opaque, one-time handle for the backend-owned callback flow.
    pub flow_id: String,
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
/// Returns the authorization URL and an opaque backend flow handle.
/// The frontend should:
/// 1. Open the `auth_url` in the system browser
/// 2. Call `cloud_oauth_complete()` with the returned flow ID; the backend
///    receives and validates the loopback callback.
#[tauri::command]
#[specta::specta]
pub async fn cloud_oauth_start(
    cloud: State<'_, CloudManager>,
    provider: String,
) -> Result<OAuthStartResult, String> {
    use super::oauth::{config_for_provider, OAuthManager};

    let config = config_for_provider(&provider).map_err(|error| error.to_string())?;

    let oauth = OAuthManager::new(config).map_err(|error| error.to_string())?;
    let started = oauth
        .start_authorization(provider.clone())
        .await
        .map_err(|error| error.to_string())?;
    let auth_url = started.auth_url;
    let flow_id = started.flow_id;
    cloud
        .register_oauth_flow(flow_id.clone(), started.pending)
        .await?;

    info!("[cloud/oauth] Started {} OAuth flow", provider);

    Ok(OAuthStartResult { auth_url, flow_id })
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
    flow_id: String,
) -> Result<ConnectionTestResult, String> {
    use super::oauth::OAuthManager;
    use super::provider::CloudProvider;

    if !matches!(provider.as_str(), "gdrive" | "dropbox" | "onedrive") {
        return Err(format!("OAuth not supported for provider: {provider}"));
    }

    let callback = cloud
        .take_oauth_flow(&flow_id, &provider)
        .await?
        .wait()
        .await
        .map_err(|error| error.to_string())?;
    let oauth_config = callback.oauth_config;
    let oauth = OAuthManager::new(oauth_config.clone()).map_err(|error| error.to_string())?;
    let previous_tokens = oauth
        .load_tokens_from_keychain()
        .map_err(|error| format!("Failed to read existing OAuth tokens: {error}"))?;

    // Exchange code for tokens
    let tokens = oauth
        .exchange_code(&callback.code, callback.code_verifier.as_str())
        .await
        .map_err(|e| format!("OAuth token exchange failed: {}", e))?;

    // Save tokens to Keychain
    oauth
        .save_tokens_to_keychain(&tokens)
        .map_err(|e| format!("Failed to save OAuth tokens: {}", e))?;

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
                .set_provider(&db, boxed_provider, provider_config, status.clone())
                .await
                .map_err(|error| {
                    let rollback = OAuthManager::new(oauth_config)
                        .and_then(|manager| restore_oauth_tokens(&manager, previous_tokens.as_ref()));
                    match rollback {
                        Ok(()) => format!("Failed to commit OAuth provider: {error}"),
                        Err(rollback_error) => format!(
                            "Failed to commit OAuth provider: {error}; token rollback also failed: {rollback_error}"
                        ),
                    }
                })?;

            info!("[cloud/oauth] {} OAuth provider committed", provider);

            Ok(ConnectionTestResult {
                connected: status.connected,
                provider_name: status.provider_name,
                storage_used: status.storage_used as f64,
                storage_available: status.storage_available.map(|v| v as f64),
                error: None,
            })
        }
        Err(error) => {
            let rollback = OAuthManager::new(oauth_config)
                .and_then(|manager| restore_oauth_tokens(&manager, previous_tokens.as_ref()));
            let error = match rollback {
                Ok(()) => error.to_string(),
                Err(rollback_error) => {
                    format!("{error}; token rollback also failed: {rollback_error}")
                }
            };
            Ok(ConnectionTestResult {
                connected: false,
                provider_name: provider,
                storage_used: 0.0,
                storage_available: None,
                error: Some(error),
            })
        }
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
    use tauri::Manager;

    info!("[cloud] Frontend requested: migrate to cloud");
    cloud.migrate_to_cloud(app.clone(), &db).await?;

    // Migration succeeded → the manager is now in cloud mode. Activate the live
    // upload worker + sync engine + read-path download fallback so new writes
    // actually reach the cloud and a fresh device can pull them back.
    let file_store = app.state::<crate::file_store::FileStore>();
    match super::live_sync::start_live_sync(&file_store, &cloud, &db).await {
        Ok(handles) => cloud.install_sync_handles(handles).await,
        Err(e) => {
            // The data is already in the cloud and the mode is persisted; surface
            // the activation failure so the operator can retry (e.g. restart).
            tracing::warn!(
                "[cloud] Migration succeeded but live sync failed to start: {}",
                e
            );
            return Err(format!(
                "Migrated to cloud, but live sync failed to start: {}. Restart the app to retry.",
                e
            ));
        }
    }

    Ok(())
}

/// Start migration from cloud to local storage.
#[tauri::command]
#[specta::specta]
pub async fn cloud_migrate_to_local(
    cloud: State<'_, CloudManager>,
    app: tauri::AppHandle,
    db: State<'_, sqlx::SqlitePool>,
) -> Result<(), String> {
    use tauri::Manager;

    info!("[cloud] Frontend requested: migrate to local");
    // Freeze the cloud upload worker before reading the archive so it cannot
    // mutate objects underneath manifest verification. The FileStore retains
    // the disconnected sender, causing writes to fail before local mutation.
    cloud.stop_sync().await;
    if let Err(error) = cloud.migrate_to_local(app.clone(), &db).await {
        let file_store = app.state::<crate::file_store::FileStore>();
        match super::live_sync::start_live_sync(&file_store, &cloud, &db).await {
            Ok(handles) => cloud.install_sync_handles(handles).await,
            Err(restart_error) => {
                return Err(format!(
                    "{error}; additionally failed to resume cloud sync: {restart_error}"
                ));
            }
        }
        return Err(error);
    }

    info!("[cloud] Restore is authenticated and staged; restarting for atomic activation");
    app.restart()
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

    let categories = [
        ("generated", "Generated Images", "generated/"),
        ("documents", "Documents", "documents/"),
        ("images", "Chat Images", "images/"),
        ("database", "Database", ""),
        ("thinclaw_runtime_db", "Agent Database", "thinclaw-runtime"),
        ("vectors", "Vector Indices", "vectors/"),
        ("previews", "Previews", "previews/"),
        ("thinclaw", "Agent State", "thinclaw/"),
    ];

    let mut result = Vec::new();
    for (id, label, subdir) in categories {
        let path = if subdir.is_empty() {
            app_data_dir.join("thinclaw.db")
        } else if subdir == "thinclaw-runtime" {
            let runtime_db = app_data_dir.join("thinclaw-runtime.db");
            if runtime_db.exists() {
                runtime_db
            } else {
                app_data_dir.join("ironclaw.db")
            }
        } else {
            app_data_dir.join(subdir)
        };

        let size = if id == "database" || id == "thinclaw_runtime_db" {
            sqlite_file_set_size(&path).await?
        } else {
            path_size(&path).await?
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
    match cloud.configure_provider(&db, config).await {
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

#[cfg(test)]
fn provider_config_json_for_persistence(config: &CloudProviderConfig) -> Result<String, String> {
    serde_json::to_string(&config.sanitized_for_persistence())
        .map_err(|e| format!("Failed to serialize provider config: {}", e))
}

fn restore_oauth_tokens(
    oauth: &super::oauth::OAuthManager,
    previous: Option<&super::oauth::OAuthTokens>,
) -> Result<(), super::provider::CloudError> {
    match previous {
        Some(tokens) => oauth.save_tokens_to_keychain(tokens),
        None => oauth.delete_tokens_from_keychain(),
    }
}

const MAX_STORAGE_BREAKDOWN_ENTRIES: usize = 100_000;

/// Calculate a path's logical size without following symbolic links.
async fn path_size(path: &std::path::Path) -> Result<u64, String> {
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(format!(
                "Failed to inspect storage path '{}': {error}",
                path.display()
            ))
        }
    };

    if metadata.file_type().is_symlink() {
        Ok(0)
    } else if metadata.is_file() {
        Ok(metadata.len())
    } else if metadata.is_dir() {
        dir_size_with_limit(path, MAX_STORAGE_BREAKDOWN_ENTRIES).await
    } else {
        Ok(0)
    }
}

/// Iteratively calculate directory size. Symlinks and special files are not
/// followed, traversal is bounded, and I/O failures are reported to the UI
/// instead of being converted into a plausible but incorrect zero.
async fn dir_size_with_limit(path: &std::path::Path, max_entries: usize) -> Result<u64, String> {
    let mut total = 0u64;
    let mut visited = 0usize;
    let mut pending = vec![path.to_path_buf()];

    while let Some(directory) = pending.pop() {
        let mut entries = tokio::fs::read_dir(&directory).await.map_err(|error| {
            format!(
                "Failed to read storage directory '{}': {error}",
                directory.display()
            )
        })?;
        loop {
            let entry = entries.next_entry().await.map_err(|error| {
                format!(
                    "Failed while reading storage directory '{}': {error}",
                    directory.display()
                )
            })?;
            let Some(entry) = entry else { break };

            visited = visited
                .checked_add(1)
                .ok_or_else(|| "Storage traversal entry counter overflowed".to_string())?;
            if visited > max_entries {
                return Err(format!(
                    "Storage breakdown exceeded the {max_entries}-entry safety limit"
                ));
            }

            let entry_path = entry.path();
            let metadata = match tokio::fs::symlink_metadata(&entry_path).await {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(format!(
                        "Failed to inspect storage entry '{}': {error}",
                        entry_path.display()
                    ))
                }
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_file() {
                total = total
                    .checked_add(metadata.len())
                    .ok_or_else(|| "Storage breakdown byte total overflowed".to_string())?;
            } else if metadata.is_dir() {
                pending.push(entry_path);
            }
        }
    }

    Ok(total)
}

/// Include SQLite's live WAL and shared-memory sidecars in its footprint.
async fn sqlite_file_set_size(database: &std::path::Path) -> Result<u64, String> {
    let mut total = path_size(database).await?;
    for suffix in ["-wal", "-shm"] {
        let mut sidecar = database.as_os_str().to_os_string();
        sidecar.push(suffix);
        total = total
            .checked_add(path_size(std::path::Path::new(&sidecar)).await?)
            .ok_or_else(|| "SQLite storage byte total overflowed".to_string())?;
    }
    Ok(total)
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

    #[tokio::test]
    async fn directory_size_is_bounded_and_counts_nested_regular_files() {
        let temp = tempfile::tempdir().unwrap();
        tokio::fs::create_dir(temp.path().join("nested"))
            .await
            .unwrap();
        tokio::fs::write(temp.path().join("one"), b"123")
            .await
            .unwrap();
        tokio::fs::write(temp.path().join("nested/two"), b"4567")
            .await
            .unwrap();

        assert_eq!(dir_size_with_limit(temp.path(), 3).await.unwrap(), 7);
        assert!(dir_size_with_limit(temp.path(), 2).await.is_err());
    }

    #[tokio::test]
    async fn sqlite_size_includes_wal_and_shm_files() {
        let temp = tempfile::tempdir().unwrap();
        let database = temp.path().join("data.db");
        tokio::fs::write(&database, [0u8; 2]).await.unwrap();
        tokio::fs::write(temp.path().join("data.db-wal"), [0u8; 3])
            .await
            .unwrap();
        tokio::fs::write(temp.path().join("data.db-shm"), [0u8; 5])
            .await
            .unwrap();

        assert_eq!(sqlite_file_set_size(&database).await.unwrap(), 10);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn directory_size_does_not_follow_symlinks() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        tokio::fs::write(outside.path().join("outside"), [0u8; 64])
            .await
            .unwrap();
        symlink(outside.path(), temp.path().join("linked")).unwrap();
        tokio::fs::write(temp.path().join("inside"), [0u8; 7])
            .await
            .unwrap();

        assert_eq!(dir_size_with_limit(temp.path(), 2).await.unwrap(), 7);
    }
}
