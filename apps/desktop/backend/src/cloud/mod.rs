//! Cloud storage module — client-side encrypted cloud sync.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────┐
//! │  App (always reads/writes local)            │
//! │  ┌──────────┐  ┌────────────────────┐       │
//! │  │ SQLite   │  │ Files (local cache) │       │
//! │  └────┬─────┘  └────────┬───────────┘       │
//! │       │                 │                    │
//! │  ┌────▼─────────────────▼────┐              │
//! │  │     CloudManager          │              │
//! │  │  (encrypt → upload/dl)    │              │
//! │  └──────────┬────────────────┘              │
//! └─────────────┼────────────────────────────────┘
//!               │
//!      ┌────────▼────────┐
//!      │  CloudProvider   │
//!      │  (S3/iCloud/...) │
//!      └─────────────────┘
//! ```
//!
//! # Storage Mode
//!
//! The app runs in one of two modes (stored in `cloud_config` table):
//! - **Local** (default): all data on device, no cloud sync
//! - **Cloud**: data encrypted + uploaded; local is a cache
//!
//! Switching between modes triggers a full migration.
//!
//! # Modules
//!
//! - `provider` — `CloudProvider` trait + error types
//! - `providers/s3` — S3-compatible implementation via opendal
//! - `encryption` — AES-256-GCM + HKDF + Keychain integration
//! - `manifest` — Archive manifest (JSON index of all files)
//! - `snapshot` — SQLite VACUUM INTO snapshots
//! - `progress` — Migration progress events for frontend

pub mod app_nap;
pub mod commands;
pub mod encryption;
pub mod manifest;
pub mod migration;
pub mod network;
pub mod oauth;
pub mod progress;
pub mod provider;
pub mod providers;
pub mod snapshot;
pub mod sync;

#[cfg(test)]
mod integration_tests;

use encryption::MasterKey;
use provider::{CloudError, CloudProvider, CloudProviderConfig};

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::AppHandle;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

const CLOUD_CREDENTIAL_KEY_PREFIX: &str = "cloud_provider";

// ── Storage Mode ─────────────────────────────────────────────────────────────

/// The global storage mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageMode {
    /// All data stored locally on this device (default).
    Local,
    /// Data encrypted and stored in the cloud. Local is a cache.
    Cloud {
        provider_type: String,
        provider_name: String,
    },
}

impl Default for StorageMode {
    fn default() -> Self {
        Self::Local
    }
}

/// Status returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudManagerStatus {
    pub mode: StorageMode,
    pub provider_connected: bool,
    pub provider_name: Option<String>,
    pub storage_used: u64,
    pub storage_available: Option<u64>,
    pub last_sync_at: Option<i64>,
    pub has_recovery_key: bool,
    pub migration_in_progress: bool,
}

// ── CloudManager ─────────────────────────────────────────────────────────────

/// Managed Tauri state for cloud storage operations.
///
/// Holds the active provider, encryption key, and migration state.
pub struct CloudManager {
    inner: RwLock<CloudManagerInner>,
}

struct CloudManagerInner {
    /// App data directory (local root for all files)
    app_data_dir: PathBuf,
    /// Current storage mode
    mode: StorageMode,
    /// Active cloud provider (None in local mode)
    provider: Option<Arc<dyn CloudProvider>>,
    /// Provider configuration (for reconnections)
    provider_config: Option<CloudProviderConfig>,
    /// Encryption master key (loaded from Keychain)
    master_key: Option<MasterKey>,
    /// Whether a migration is currently running
    migration_in_progress: bool,
    /// Cancellation flag for active migration
    cancel_flag: Option<Arc<RwLock<bool>>>,
}

impl CloudManager {
    /// Create a new CloudManager.
    pub fn new(app_data_dir: PathBuf) -> Self {
        Self {
            inner: RwLock::new(CloudManagerInner {
                app_data_dir,
                mode: StorageMode::Local,
                provider: None,
                provider_config: None,
                master_key: None,
                migration_in_progress: false,
                cancel_flag: None,
            }),
        }
    }

    /// Initialize from database state (called on app launch).
    pub async fn init_from_db(&self, pool: &SqlitePool) -> Result<(), String> {
        // Read storage mode from cloud_config table
        let mode_row: Option<(String,)> =
            sqlx::query_as("SELECT value FROM cloud_config WHERE key = 'mode'")
                .fetch_optional(pool)
                .await
                .map_err(|e| format!("Failed to read cloud_config: {}", e))?;

        if let Some((mode_json,)) = mode_row {
            if let Ok(mode) = serde_json::from_str::<StorageMode>(&mode_json) {
                let mut inner = self.inner.write().await;
                inner.mode = mode;

                // If in cloud mode, try to load provider config and master key
                if !matches!(inner.mode, StorageMode::Local) {
                    // Load provider config
                    if let Some((config_json,)) = sqlx::query_as::<_, (String,)>(
                        "SELECT value FROM cloud_config WHERE key = 'provider_config'",
                    )
                    .fetch_optional(pool)
                    .await
                    .map_err(|e| format!("Failed to read provider_config: {}", e))?
                    {
                        if let Ok(config) =
                            serde_json::from_str::<CloudProviderConfig>(&config_json)
                        {
                            let config = hydrate_provider_credentials(config);
                            let sanitized_config = config.sanitized_for_persistence();
                            let should_sanitize_persisted_config = sanitized_config != config;
                            match provider::create_provider(&config) {
                                Ok(provider) => {
                                    inner.provider = Some(Arc::from(provider));
                                    inner.provider_config = Some(config.clone());
                                }
                                Err(_) => {
                                    // OAuth providers (gdrive, dropbox, onedrive) can't be
                                    // created from config alone — reconstruct from Keychain tokens.
                                    match Self::create_oauth_provider(&config) {
                                        Ok(Some(provider)) => {
                                            inner.provider = Some(Arc::from(provider));
                                            inner.provider_config = Some(config.clone());
                                            info!("[cloud] OAuth provider restored from Keychain");
                                        }
                                        Ok(None) => {
                                            warn!("[cloud] Non-OAuth provider failed to init");
                                        }
                                        Err(e) => {
                                            warn!(
                                                "[cloud] Failed to restore OAuth provider: {}",
                                                e
                                            );
                                        }
                                    }
                                }
                            }

                            if should_sanitize_persisted_config {
                                let sanitized_json = serde_json::to_string(&sanitized_config)
                                    .map_err(|e| {
                                        format!(
                                            "Failed to serialize sanitized provider_config: {}",
                                            e
                                        )
                                    })?;
                                sqlx::query(
                                    "INSERT OR REPLACE INTO cloud_config (key, value) VALUES ('provider_config', ?)",
                                )
                                .bind(sanitized_json)
                                .execute(pool)
                                .await
                                .map_err(|e| {
                                    format!("Failed to sanitize provider_config: {}", e)
                                })?;
                                info!("[cloud] Sanitized persisted provider_config");
                            }
                        }
                    }

                    // Load master key from Keychain
                    match encryption::load_master_key_from_keychain() {
                        Ok(Some(key)) => {
                            inner.master_key = Some(key);
                        }
                        Ok(None) => {
                            warn!("[cloud] Cloud mode active but no master key in Keychain");
                        }
                        Err(e) => {
                            warn!("[cloud] Failed to load master key: {}", e);
                        }
                    }
                }

                info!("[cloud] Initialized: mode={:?}", inner.mode);
            }
        }

        // Check for interrupted migrations
        let pending: Option<(String,)> =
            sqlx::query_as("SELECT id FROM cloud_migrations WHERE status = 'in_progress' LIMIT 1")
                .fetch_optional(pool)
                .await
                .map_err(|e| format!("Failed to check pending migrations: {}", e))?;

        if let Some((migration_id,)) = pending {
            warn!(
                "[cloud] Found interrupted migration: {}. User should resume or cancel.",
                migration_id
            );
        }

        Ok(())
    }

    /// Get the current status.
    pub async fn get_status(&self) -> CloudManagerStatus {
        let inner = self.inner.read().await;
        CloudManagerStatus {
            mode: inner.mode.clone(),
            provider_connected: inner.provider.is_some(),
            provider_name: inner.provider.as_ref().map(|p| p.name().to_string()),
            storage_used: 0, // Updated on test_connection
            storage_available: None,
            last_sync_at: None,
            has_recovery_key: inner.master_key.is_some(),
            migration_in_progress: inner.migration_in_progress,
        }
    }

    /// Configure and test a cloud provider connection.
    pub async fn configure_provider(
        &self,
        config: CloudProviderConfig,
    ) -> Result<provider::CloudStatus, CloudError> {
        let provider = provider::create_provider(&config)?;
        let status = provider.test_connection().await?;

        let mut inner = self.inner.write().await;
        if inner.migration_in_progress {
            return Err(CloudError::Provider(
                "Cannot replace cloud provider while a migration is in progress".into(),
            ));
        }

        inner.provider = Some(Arc::from(provider));
        inner.provider_config = Some(config);

        info!("[cloud] Provider configured: {}", status.provider_name);
        Ok(status)
    }

    /// Set a pre-constructed provider (used by OAuth flows).
    ///
    /// Unlike `configure_provider`, this skips the factory and takes
    /// an already-authenticated provider.
    pub async fn set_provider(
        &self,
        provider: Box<dyn CloudProvider>,
        config: CloudProviderConfig,
    ) {
        let mut inner = self.inner.write().await;
        if inner.migration_in_progress {
            warn!("[cloud] Ignoring provider replacement while migration is in progress");
            return;
        }

        info!("[cloud] Provider set directly: {}", provider.name());
        inner.provider = Some(Arc::from(provider));
        inner.provider_config = Some(config);
    }

    /// Test the current provider connection.
    pub async fn test_connection(&self) -> Result<provider::CloudStatus, CloudError> {
        let inner = self.inner.read().await;
        let provider = inner
            .provider
            .as_ref()
            .ok_or_else(|| CloudError::Provider("No provider configured".into()))?;
        provider.test_connection().await
    }

    pub async fn provider_config(&self) -> Option<CloudProviderConfig> {
        self.inner.read().await.provider_config.clone()
    }

    /// Get the recovery key (base64-encoded master key).
    pub async fn get_recovery_key(&self) -> Result<String, String> {
        let inner = self.inner.read().await;
        match &inner.master_key {
            Some(key) => Ok(key.to_recovery_key()),
            None => Err("No encryption key found. Set up cloud storage first.".into()),
        }
    }

    /// Import a recovery key (for restoring on a new device).
    pub async fn import_recovery_key(&self, recovery_key: &str) -> Result<(), String> {
        let key = MasterKey::from_recovery_key(recovery_key)
            .map_err(|e| format!("Invalid recovery key: {}", e))?;

        encryption::save_master_key_to_keychain(&key)
            .map_err(|e| format!("Failed to save key to Keychain: {}", e))?;

        let mut inner = self.inner.write().await;
        inner.master_key = Some(key);

        info!("[cloud] Recovery key imported and saved to Keychain");
        Ok(())
    }

    /// Cancel an in-progress migration.
    pub async fn cancel_migration(&self) -> Result<(), String> {
        let inner = self.inner.read().await;
        if let Some(flag) = &inner.cancel_flag {
            *flag.write().await = true;
            info!("[cloud] Migration cancel requested");
            Ok(())
        } else {
            Err("No migration in progress".into())
        }
    }

    /// Migrate all data from local to cloud.
    ///
    /// This is the big one — see `cloud_storage_implementation.md` § 7.1.
    pub async fn migrate_to_cloud(&self, app: AppHandle, pool: &SqlitePool) -> Result<(), String> {
        {
            let inner = self.inner.read().await;
            if inner.migration_in_progress {
                return Err("Migration already in progress".into());
            }
            if inner.provider.is_none() {
                return Err("No cloud provider configured. Configure one first.".into());
            }
        }

        // Generate or load master key
        self.ensure_master_key().await?;

        // Mark migration as in-progress + set cancel flag
        let migration_id = uuid::Uuid::new_v4().to_string();
        let (app_data_dir, provider, master_key, provider_type, cancel_flag) = {
            let mut inner = self.inner.write().await;
            if inner.migration_in_progress {
                return Err("Migration already in progress".into());
            }

            let provider =
                inner.provider.as_ref().cloned().ok_or_else(|| {
                    "No cloud provider configured. Configure one first.".to_string()
                })?;
            let master_key = inner.master_key.as_ref().cloned().ok_or_else(|| {
                "No encryption key found. Set up cloud storage first.".to_string()
            })?;
            let provider_type = inner
                .provider_config
                .as_ref()
                .map(|c| c.provider_type.clone())
                .unwrap_or_else(|| "unknown".to_string());

            inner.migration_in_progress = true;
            let flag = Arc::new(RwLock::new(false));
            let cancel_flag = flag.clone();
            inner.cancel_flag = Some(flag);

            (
                inner.app_data_dir.clone(),
                provider,
                master_key,
                provider_type,
                cancel_flag,
            )
        };

        info!("[cloud] Starting local → cloud migration: {}", migration_id);

        // Execute the full migration flow
        let result = migration::run_to_cloud(
            app.clone(),
            pool,
            &app_data_dir,
            provider.as_ref(),
            &master_key,
            &provider_type,
            &migration_id,
            cancel_flag,
        )
        .await;

        // Clear migration flag + update mode on success
        {
            let mut inner = self.inner.write().await;
            inner.migration_in_progress = false;
            inner.cancel_flag = None;
            if result.is_ok() {
                let pt = inner
                    .provider_config
                    .as_ref()
                    .map(|c| c.provider_type.clone())
                    .unwrap_or_else(|| "s3".to_string());
                inner.mode = StorageMode::Cloud {
                    provider_type: pt,
                    provider_name: inner
                        .provider
                        .as_ref()
                        .map(|p| p.name().to_string())
                        .unwrap_or_default(),
                };
            }
        }

        match &result {
            Ok(()) => {
                info!("[cloud] Migration {} completed successfully", migration_id);
            }
            Err(e) => {
                error!("[cloud] Migration {} failed: {}", migration_id, e);
                let _ = migration::record_migration_failure(pool, &migration_id, e).await;
            }
        }

        result
    }

    /// Migrate all data from cloud to local.
    pub async fn migrate_to_local(&self, app: AppHandle, pool: &SqlitePool) -> Result<(), String> {
        {
            let inner = self.inner.read().await;
            if inner.migration_in_progress {
                return Err("Migration already in progress".into());
            }
            if matches!(&inner.mode, StorageMode::Local) {
                return Err("Already in local mode".into());
            }
        }

        // Ensure master key
        self.ensure_master_key().await?;

        let migration_id = uuid::Uuid::new_v4().to_string();
        let (app_data_dir, provider, master_key, cancel_flag) = {
            let mut inner = self.inner.write().await;
            if inner.migration_in_progress {
                return Err("Migration already in progress".into());
            }
            if matches!(&inner.mode, StorageMode::Local) {
                return Err("Already in local mode".into());
            }

            let provider =
                inner.provider.as_ref().cloned().ok_or_else(|| {
                    "No cloud provider configured. Configure one first.".to_string()
                })?;
            let master_key = inner.master_key.as_ref().cloned().ok_or_else(|| {
                "No encryption key found. Set up cloud storage first.".to_string()
            })?;

            inner.migration_in_progress = true;
            let flag = Arc::new(RwLock::new(false));
            let cancel_flag = flag.clone();
            inner.cancel_flag = Some(flag);

            (
                inner.app_data_dir.clone(),
                provider,
                master_key,
                cancel_flag,
            )
        };

        info!("[cloud] Starting cloud → local migration: {}", migration_id);

        let result = migration::run_to_local(
            app.clone(),
            pool,
            &app_data_dir,
            provider.as_ref(),
            &master_key,
            &migration_id,
            cancel_flag,
        )
        .await;

        {
            let mut inner = self.inner.write().await;
            inner.migration_in_progress = false;
            inner.cancel_flag = None;
            if result.is_ok() {
                inner.mode = StorageMode::Local;
            }
        }

        match &result {
            Ok(()) => {
                info!("[cloud] Migration {} completed", migration_id);
            }
            Err(e) => {
                error!("[cloud] Migration {} failed: {}", migration_id, e);
                let _ = migration::record_migration_failure(pool, &migration_id, e).await;
            }
        }

        result
    }

    // ── Private helpers ──────────────────────────────────────────────────────

    /// Ensure a master key exists (generate + store if needed).
    async fn ensure_master_key(&self) -> Result<(), String> {
        let mut inner = self.inner.write().await;
        if inner.master_key.is_some() {
            return Ok(());
        }

        // Try to load from Keychain
        match encryption::load_master_key_from_keychain() {
            Ok(Some(key)) => {
                inner.master_key = Some(key);
                return Ok(());
            }
            Ok(None) => {
                // Generate new key
                let key = MasterKey::generate();
                encryption::save_master_key_to_keychain(&key)
                    .map_err(|e| format!("Failed to save master key: {}", e))?;
                info!("[cloud] Generated and stored new master key");
                inner.master_key = Some(key);
                Ok(())
            }
            Err(e) => Err(format!("Keychain error: {}", e)),
        }
    }

    /// Try to create an OAuth provider from Keychain tokens.
    ///
    /// Returns `Ok(None)` if the provider type isn't an OAuth provider.
    /// Returns `Ok(Some(provider))` if tokens were found and provider created.
    fn create_oauth_provider(
        config: &CloudProviderConfig,
    ) -> Result<Option<Box<dyn CloudProvider>>, String> {
        use oauth::{OAuthConfig, OAuthManager};

        let provider_type = config.provider_type.as_str();

        let client_id = match provider_type {
            "gdrive" => std::env::var("GOOGLE_CLIENT_ID")
                .unwrap_or_else(|_| "thinclaw-desktop.apps.googleusercontent.com".to_string()),
            "dropbox" => std::env::var("DROPBOX_CLIENT_ID")
                .unwrap_or_else(|_| "thinclaw_desktop_app".to_string()),
            "onedrive" => std::env::var("ONEDRIVE_CLIENT_ID")
                .unwrap_or_else(|_| "thinclaw-desktop-app".to_string()),
            _ => return Ok(None), // Not an OAuth provider
        };

        let oauth_config = match provider_type {
            "gdrive" => OAuthConfig::google_drive(client_id),
            "dropbox" => OAuthConfig::dropbox(client_id),
            "onedrive" => OAuthConfig::onedrive(client_id),
            _ => unreachable!(),
        };

        let oauth = OAuthManager::new(oauth_config);

        // Check if tokens exist in Keychain
        match oauth.load_tokens_from_keychain() {
            Ok(Some(_)) => { /* tokens exist, proceed */ }
            Ok(None) => {
                return Err(format!(
                    "No OAuth tokens in Keychain for {}. Please re-authenticate.",
                    provider_type
                ));
            }
            Err(e) => {
                return Err(format!(
                    "Failed to load OAuth tokens for {}: {}",
                    provider_type, e
                ));
            }
        }

        let provider: Box<dyn CloudProvider> = match provider_type {
            "gdrive" => Box::new(providers::gdrive::GDriveProvider::new(oauth)),
            "dropbox" => Box::new(providers::dropbox::DropboxProvider::new(oauth)),
            "onedrive" => Box::new(providers::onedrive::OneDriveProvider::new(oauth)),
            _ => unreachable!(),
        };

        Ok(Some(provider))
    }
}

pub(crate) fn save_provider_credentials(config: &CloudProviderConfig) -> Result<(), String> {
    if let Some(access_key_id) = config.access_key_id.as_deref() {
        crate::openclaw::config::keychain::set_key(
            &cloud_provider_credential_key(config, "access_key_id"),
            Some(access_key_id),
        )?;
    }
    if let Some(secret_access_key) = config.secret_access_key.as_deref() {
        crate::openclaw::config::keychain::set_key(
            &cloud_provider_credential_key(config, "secret_access_key"),
            Some(secret_access_key),
        )?;
    }

    Ok(())
}

pub(crate) fn hydrate_provider_credentials(mut config: CloudProviderConfig) -> CloudProviderConfig {
    if config.access_key_id.is_none() {
        config.access_key_id = crate::openclaw::config::keychain::get_key(
            &cloud_provider_credential_key(&config, "access_key_id"),
        );
    }
    if config.secret_access_key.is_none() {
        config.secret_access_key = crate::openclaw::config::keychain::get_key(
            &cloud_provider_credential_key(&config, "secret_access_key"),
        );
    }

    config
}

pub(crate) fn cloud_provider_credential_key(config: &CloudProviderConfig, field: &str) -> String {
    let endpoint = credential_key_segment(config.endpoint.as_deref().unwrap_or("default"));
    let bucket = credential_key_segment(config.bucket.as_deref().unwrap_or("default"));
    let root = credential_key_segment(config.root.as_deref().unwrap_or("default"));
    format!(
        "{CLOUD_CREDENTIAL_KEY_PREFIX}.{}.{}.{}.{}.{}",
        credential_key_segment(&config.provider_type),
        endpoint,
        bucket,
        root,
        field
    )
}

fn credential_key_segment(value: &str) -> String {
    let mut segment = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            segment.push(ch.to_ascii_lowercase());
        } else {
            segment.push('_');
        }
    }
    let trimmed = segment.trim_matches('_');
    if trimmed.is_empty() {
        "default".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credential_test_config() -> CloudProviderConfig {
        CloudProviderConfig {
            provider_type: "s3".to_string(),
            endpoint: Some("https://s3.example.com".to_string()),
            bucket: Some("thin-bucket".to_string()),
            region: Some("eu-central-1".to_string()),
            access_key_id: Some("AKIA".to_string()),
            secret_access_key: Some("secret".to_string()),
            root: Some("thinclaw-desktop/".to_string()),
        }
    }

    #[test]
    fn cloud_credential_key_is_stable_without_secret_fields() {
        let full = credential_test_config();
        let sanitized = full.sanitized_for_persistence();

        assert_eq!(
            cloud_provider_credential_key(&full, "secret_access_key"),
            cloud_provider_credential_key(&sanitized, "secret_access_key")
        );
        assert!(cloud_provider_credential_key(&full, "access_key_id")
            .starts_with("cloud_provider.s3.https___s3_example_com.thin_bucket"));
    }
}
