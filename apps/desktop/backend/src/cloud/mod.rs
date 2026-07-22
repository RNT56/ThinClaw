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
pub mod live_sync;
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
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::collections::HashMap;
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
#[derive(Default)]
pub enum StorageMode {
    /// All data stored locally on this device (default).
    #[default]
    Local,
    /// Data encrypted and stored in the cloud. Local is a cache.
    Cloud {
        provider_type: String,
        provider_name: String,
    },
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
    pub sync_active: bool,
    pub sync_error: Option<String>,
    pub has_recovery_key: bool,
    pub migration_in_progress: bool,
}

#[derive(Debug, Default)]
pub(crate) struct CloudSyncTelemetry {
    pub(crate) active: bool,
    pub(crate) last_success_at: Option<i64>,
    pub(crate) last_error: Option<String>,
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
    /// Most recent successful, provider-reported connection/quota status.
    /// A configured provider is not considered connected until an operation
    /// has actually verified it during this process lifetime.
    provider_status: Option<provider::CloudStatus>,
    /// Encryption master key (loaded from Keychain)
    master_key: Option<MasterKey>,
    /// Whether a migration is currently running
    migration_in_progress: bool,
    /// Cancellation flag for active migration
    cancel_flag: Option<Arc<RwLock<bool>>>,
    /// Handles for the live-sync background tasks (upload worker + sync engine).
    /// `None` whenever the app is in local mode.
    sync_handles: Option<live_sync::SyncHandles>,
    /// Observable health of the background sync pipeline.
    sync_telemetry: Arc<RwLock<CloudSyncTelemetry>>,
    /// Backend-only PKCE callback flows. Keeping these here prevents the PKCE
    /// verifier and CSRF state from crossing the renderer IPC boundary.
    oauth_flows: HashMap<String, oauth::PendingOAuthFlow>,
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
                provider_status: None,
                master_key: None,
                migration_in_progress: false,
                cancel_flag: None,
                sync_handles: None,
                sync_telemetry: Arc::new(RwLock::new(CloudSyncTelemetry::default())),
                oauth_flows: HashMap::new(),
            }),
        }
    }

    /// Initialize from database state (called on app launch).
    pub async fn init_from_db(&self, pool: &SqlitePool) -> Result<(), String> {
        // Read persisted state without holding the manager lock across SQLite,
        // Keychain, provider construction, or any other fallible boundary.
        let mode_row: Option<(String,)> =
            sqlx::query_as("SELECT value FROM cloud_config WHERE key = 'mode'")
                .fetch_optional(pool)
                .await
                .map_err(|e| format!("Failed to read cloud_config: {}", e))?;
        let mode = match mode_row {
            Some((mode_json,)) => serde_json::from_str::<StorageMode>(&mode_json)
                .map_err(|error| format!("Persisted cloud mode is invalid: {error}"))?,
            None => StorageMode::Local,
        };

        // A tested provider configuration is useful in Local mode too: users
        // commonly configure it, restart, and migrate later. Previously that
        // provider disappeared from runtime state until they configured it again.
        let config_row: Option<(String,)> =
            sqlx::query_as("SELECT value FROM cloud_config WHERE key = 'provider_config'")
                .fetch_optional(pool)
                .await
                .map_err(|e| format!("Failed to read provider_config: {}", e))?;

        let mut restored_provider = None;
        let mut restored_config = None;
        let mut provider_restore_error = None;
        if let Some((config_json,)) = config_row {
            match serde_json::from_str::<CloudProviderConfig>(&config_json) {
                Ok(persisted_config) => {
                    let sanitized_config = persisted_config.sanitized_for_persistence();
                    let should_sanitize_persisted_config = sanitized_config != persisted_config;
                    let hydrated_config = hydrate_provider_credentials(persisted_config);
                    let provider_result = if matches!(
                        hydrated_config.provider_type.as_str(),
                        "gdrive" | "dropbox" | "onedrive"
                    ) {
                        Self::create_oauth_provider(&hydrated_config).and_then(|provider| {
                            provider.ok_or_else(|| {
                                "OAuth provider factory returned no provider".to_string()
                            })
                        })
                    } else {
                        provider::create_provider(&hydrated_config)
                            .map_err(|error| error.to_string())
                    };

                    match provider_result {
                        Ok(provider) => {
                            restored_provider = Some(Arc::from(provider));
                            info!(
                                "[cloud] Restored configured {} provider",
                                sanitized_config.provider_type
                            );
                        }
                        Err(error) => {
                            provider_restore_error = Some(format!(
                                "Failed to restore configured {} provider: {error}",
                                sanitized_config.provider_type
                            ));
                        }
                    }
                    restored_config = Some(sanitized_config.clone());

                    if should_sanitize_persisted_config {
                        let sanitized_json =
                            serde_json::to_string(&sanitized_config).map_err(|e| {
                                format!("Failed to serialize sanitized provider_config: {e}")
                            })?;
                        sqlx::query(
                            "INSERT OR REPLACE INTO cloud_config (key, value) VALUES ('provider_config', ?)",
                        )
                        .bind(sanitized_json)
                        .execute(pool)
                        .await
                        .map_err(|e| format!("Failed to sanitize provider_config: {e}"))?;
                        info!("[cloud] Sanitized persisted provider_config");
                    }
                }
                Err(error) => {
                    provider_restore_error = Some(format!(
                        "Persisted provider configuration is invalid: {error}"
                    ));
                }
            }
        }

        if let (StorageMode::Cloud { provider_type, .. }, Some(config)) =
            (&mode, restored_config.as_ref())
        {
            if provider_type != &config.provider_type {
                restored_provider = None;
                provider_restore_error = Some(format!(
                    "Cloud mode expects provider '{provider_type}' but persisted configuration is for '{}'",
                    config.provider_type
                ));
            }
        }

        let master_key = if matches!(&mode, StorageMode::Local) {
            None
        } else {
            match encryption::load_master_key_from_keychain() {
                Ok(Some(key)) => Some(key),
                Ok(None) => {
                    warn!("[cloud] Cloud mode active but no master key in Keychain");
                    None
                }
                Err(error) => {
                    warn!("[cloud] Failed to load master key: {}", error);
                    None
                }
            }
        };

        {
            let mut inner = self.inner.write().await;
            inner.mode = mode;
            inner.provider = restored_provider;
            inner.provider_config = restored_config;
            inner.provider_status = None;
            inner.master_key = master_key;
            info!("[cloud] Initialized: mode={:?}", inner.mode);
        }

        if let Some(error) = provider_restore_error {
            warn!("[cloud] {error}");
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
        let (
            mode,
            provider_connected,
            provider_name,
            storage_used,
            storage_available,
            has_recovery_key,
            migration_in_progress,
            telemetry,
        ) = {
            let inner = self.inner.read().await;
            (
                inner.mode.clone(),
                inner
                    .provider_status
                    .as_ref()
                    .is_some_and(|status| status.connected),
                inner
                    .provider_status
                    .as_ref()
                    .map(|status| status.provider_name.clone())
                    .or_else(|| {
                        inner
                            .provider
                            .as_ref()
                            .map(|provider| provider.name().to_string())
                    }),
                inner
                    .provider_status
                    .as_ref()
                    .map(|status| status.storage_used)
                    .unwrap_or(0),
                inner
                    .provider_status
                    .as_ref()
                    .and_then(|status| status.storage_available),
                inner.master_key.is_some(),
                inner.migration_in_progress,
                inner.sync_telemetry.clone(),
            )
        };
        let telemetry = telemetry.read().await;
        CloudManagerStatus {
            mode,
            provider_connected,
            provider_name,
            storage_used,
            storage_available,
            last_sync_at: telemetry.last_success_at,
            sync_active: telemetry.active,
            sync_error: telemetry.last_error.clone(),
            has_recovery_key,
            migration_in_progress,
        }
    }

    /// Configure and test a cloud provider connection.
    pub async fn configure_provider(
        &self,
        pool: &SqlitePool,
        config: CloudProviderConfig,
    ) -> Result<provider::CloudStatus, CloudError> {
        let provider = provider::create_provider(&config)?;
        let status = provider.test_connection().await?;
        self.commit_provider(pool, provider, config, status.clone())
            .await?;

        info!("[cloud] Provider configured: {}", status.provider_name);
        Ok(status)
    }

    /// Set a pre-constructed provider (used by OAuth flows).
    ///
    /// Unlike `configure_provider`, this skips the factory and takes
    /// an already-authenticated provider.
    pub async fn set_provider(
        &self,
        pool: &SqlitePool,
        provider: Box<dyn CloudProvider>,
        config: CloudProviderConfig,
        status: provider::CloudStatus,
    ) -> Result<(), CloudError> {
        self.commit_provider(pool, provider, config, status).await
    }

    async fn commit_provider(
        &self,
        pool: &SqlitePool,
        provider: Box<dyn CloudProvider>,
        config: CloudProviderConfig,
        status: provider::CloudStatus,
    ) -> Result<(), CloudError> {
        if !status.connected {
            return Err(CloudError::ConnectionFailed(format!(
                "{} did not report a live connection",
                status.provider_name
            )));
        }
        let mut inner = self.inner.write().await;
        if inner.migration_in_progress {
            return Err(CloudError::Provider(
                "Cannot replace cloud provider while a migration is in progress".into(),
            ));
        }
        if !matches!(&inner.mode, StorageMode::Local) || inner.sync_handles.is_some() {
            return Err(CloudError::Provider(
                "Cannot replace the cloud provider while cloud mode is active; migrate to local storage first"
                    .into(),
            ));
        }

        let sanitized = config.sanitized_for_persistence();
        let config_json = serde_json::to_string(&sanitized).map_err(|error| {
            CloudError::Provider(format!("Failed to serialize provider config: {error}"))
        })?;
        let previous_credentials = snapshot_provider_credentials(&config);
        let mut transaction = pool.begin().await.map_err(|error| {
            CloudError::Provider(format!(
                "Failed to begin provider config transaction: {error}"
            ))
        })?;
        sqlx::query(
            "INSERT OR REPLACE INTO cloud_config (key, value) VALUES ('provider_config', ?)",
        )
        .bind(config_json)
        .execute(&mut *transaction)
        .await
        .map_err(|error| {
            CloudError::Provider(format!("Failed to stage provider config: {error}"))
        })?;
        save_provider_credentials(&config).map_err(|error| {
            CloudError::Provider(format!("Failed to save provider credentials: {error}"))
        })?;
        if let Err(error) = transaction.commit().await {
            let rollback_error = restore_provider_credentials(&previous_credentials).err();
            return Err(CloudError::Provider(match rollback_error {
                Some(rollback_error) => format!(
                    "Failed to commit provider config: {error}; credential rollback also failed: {rollback_error}"
                ),
                None => format!("Failed to commit provider config: {error}"),
            }));
        }

        info!("[cloud] Provider committed: {}", provider.name());
        inner.provider = Some(Arc::from(provider));
        inner.provider_config = Some(sanitized);
        inner.provider_status = Some(status);
        Ok(())
    }

    /// Test the current provider connection.
    pub async fn test_connection(&self) -> Result<provider::CloudStatus, CloudError> {
        let provider = self
            .inner
            .read()
            .await
            .provider
            .clone()
            .ok_or_else(|| CloudError::Provider("No provider configured".into()))?;

        let result = provider.test_connection().await;
        let mut inner = self.inner.write().await;
        // Do not apply a late result to a provider that was replaced while the
        // network request was in flight.
        if inner
            .provider
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &provider))
        {
            inner.provider_status = result.as_ref().ok().cloned();
        }
        result
    }

    pub async fn provider_config(&self) -> Option<CloudProviderConfig> {
        self.inner.read().await.provider_config.clone()
    }

    /// Currently active cloud provider, if any (used by the live-sync worker).
    pub(crate) async fn active_provider(&self) -> Option<Arc<dyn CloudProvider>> {
        self.inner.read().await.provider.clone()
    }

    /// Current encryption master key, if loaded (used by the live-sync worker).
    pub(crate) async fn master_key(&self) -> Option<MasterKey> {
        self.inner.read().await.master_key.clone()
    }

    /// App data directory (local root for all files).
    pub(crate) async fn app_data_dir(&self) -> PathBuf {
        self.inner.read().await.app_data_dir.clone()
    }

    pub(crate) async fn sync_telemetry(&self) -> Arc<RwLock<CloudSyncTelemetry>> {
        self.inner.read().await.sync_telemetry.clone()
    }

    pub(crate) async fn register_oauth_flow(
        &self,
        flow_id: String,
        flow: oauth::PendingOAuthFlow,
    ) -> Result<(), String> {
        let mut inner = self.inner.write().await;
        inner.oauth_flows.retain(|_, pending| !pending.is_expired());
        // All bundled providers use the same fixed, registered loopback port,
        // so only one callback can be live at a time.
        if !inner.oauth_flows.is_empty() {
            return Err("Another cloud OAuth sign-in is already in progress".to_string());
        }
        inner.oauth_flows.insert(flow_id, flow);
        Ok(())
    }

    pub(crate) async fn take_oauth_flow(
        &self,
        flow_id: &str,
        provider: &str,
    ) -> Result<oauth::PendingOAuthFlow, String> {
        if flow_id.is_empty()
            || flow_id.len() > 128
            || !flow_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err("Invalid cloud OAuth flow identifier".to_string());
        }
        let mut inner = self.inner.write().await;
        inner.oauth_flows.retain(|_, pending| !pending.is_expired());
        let pending = inner
            .oauth_flows
            .remove(flow_id)
            .ok_or_else(|| "Cloud OAuth flow was not found or has expired".to_string())?;
        if !pending.provider_is(provider) {
            return Err("Cloud OAuth flow does not match the requested provider".to_string());
        }
        Ok(pending)
    }

    /// Whether the manager is currently in cloud mode.
    pub async fn is_cloud_mode(&self) -> bool {
        !matches!(self.inner.read().await.mode, StorageMode::Local)
    }

    /// Store the handles for the running live-sync tasks. Any previously stored
    /// handles are stopped first to avoid orphaning a worker/engine.
    pub(crate) async fn install_sync_handles(&self, handles: live_sync::SyncHandles) {
        let previous = {
            let mut inner = self.inner.write().await;
            inner.sync_handles.replace(handles)
        };
        if let Some(previous) = previous {
            warn!("[cloud] Replacing existing sync handles; stopping the previous worker/engine");
            previous.stop().await;
        }
    }

    /// Stop the live-sync worker + engine (cancels the engine, drops the upload
    /// channel so the worker drains and exits, then awaits both tasks).
    pub async fn stop_sync(&self) {
        let handles = self.inner.write().await.sync_handles.take();
        if let Some(handles) = handles {
            info!("[cloud] Stopping live sync");
            handles.stop().await;
        }
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
            // A successful download is staged, not activated in-process.
            // Startup applies the marker before opening databases and then
            // reads Local mode from the restored primary database.
        }

        match &result {
            Ok(()) => {
                info!(
                    "[cloud] Migration {} staged; restart required for activation",
                    migration_id
                );
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
                Ok(())
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
        use oauth::OAuthManager;

        let provider_type = config.provider_type.as_str();
        if !matches!(provider_type, "gdrive" | "dropbox" | "onedrive") {
            return Ok(None);
        }
        let oauth_config = oauth::config_for_provider(provider_type)
            .map_err(|error| format!("OAuth provider is not configured: {error}"))?;

        let oauth = OAuthManager::new(oauth_config)
            .map_err(|error| format!("Failed to initialize OAuth provider: {error}"))?;

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
    let access_key = cloud_provider_credential_key(config, "access_key_id");
    let secret_key = cloud_provider_credential_key(config, "secret_access_key");
    let marker_key = cloud_provider_credential_key(config, "managed");
    crate::thinclaw::config::keychain::set_keys(&[
        (&access_key, config.access_key_id.as_deref()),
        (&secret_key, config.secret_access_key.as_deref()),
        (&marker_key, Some("1")),
    ])
}

fn snapshot_provider_credentials(config: &CloudProviderConfig) -> Vec<(String, Option<String>)> {
    ["access_key_id", "secret_access_key", "managed"]
        .into_iter()
        .map(|field| {
            let key = cloud_provider_credential_key(config, field);
            let value = crate::thinclaw::config::keychain::get_key(&key);
            (key, value)
        })
        .collect()
}

fn restore_provider_credentials(snapshot: &[(String, Option<String>)]) -> Result<(), String> {
    let entries = snapshot
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_deref()))
        .collect::<Vec<_>>();
    crate::thinclaw::config::keychain::set_keys(&entries)
}

pub(crate) fn hydrate_provider_credentials(mut config: CloudProviderConfig) -> CloudProviderConfig {
    let access_key = cloud_provider_credential_key(&config, "access_key_id");
    let secret_key = cloud_provider_credential_key(&config, "secret_access_key");
    let marker_key = cloud_provider_credential_key(&config, "managed");
    let managed = crate::thinclaw::config::keychain::get_key(&marker_key).as_deref() == Some("1");
    let mut found_legacy = false;

    if config.access_key_id.is_none() {
        config.access_key_id =
            crate::thinclaw::config::keychain::get_key(&access_key).or_else(|| {
                if managed {
                    None
                } else {
                    let value = crate::thinclaw::config::keychain::get_key(
                        &legacy_cloud_provider_credential_key(&config, "access_key_id"),
                    );
                    found_legacy |= value.is_some();
                    value
                }
            });
    }
    if config.secret_access_key.is_none() {
        config.secret_access_key =
            crate::thinclaw::config::keychain::get_key(&secret_key).or_else(|| {
                if managed {
                    None
                } else {
                    let value = crate::thinclaw::config::keychain::get_key(
                        &legacy_cloud_provider_credential_key(&config, "secret_access_key"),
                    );
                    found_legacy |= value.is_some();
                    value
                }
            });
    }

    if !managed && found_legacy {
        if let Err(error) = crate::thinclaw::config::keychain::set_keys(&[
            (&access_key, config.access_key_id.as_deref()),
            (&secret_key, config.secret_access_key.as_deref()),
            (&marker_key, Some("1")),
        ]) {
            warn!("[cloud] Failed to migrate legacy provider credentials: {error}");
        }
    }

    config
}

pub(crate) fn cloud_provider_credential_key(config: &CloudProviderConfig, field: &str) -> String {
    let mut hasher = Sha256::new();
    hash_credential_scope_field(&mut hasher, b"provider_type", Some(&config.provider_type));
    hash_credential_scope_field(&mut hasher, b"endpoint", config.endpoint.as_deref());
    hash_credential_scope_field(&mut hasher, b"bucket", config.bucket.as_deref());
    hash_credential_scope_field(&mut hasher, b"region", config.region.as_deref());
    hash_credential_scope_field(&mut hasher, b"root", config.root.as_deref());
    let digest = hex::encode(hasher.finalize());
    format!(
        "{CLOUD_CREDENTIAL_KEY_PREFIX}.v2.{}.{}.{}",
        credential_key_segment(&config.provider_type),
        digest,
        credential_key_segment(field),
    )
}

fn hash_credential_scope_field(hasher: &mut Sha256, name: &[u8], value: Option<&str>) {
    hasher.update((name.len() as u64).to_be_bytes());
    hasher.update(name);
    match value {
        Some(value) => {
            hasher.update([1]);
            hasher.update((value.len() as u64).to_be_bytes());
            hasher.update(value.as_bytes());
        }
        None => hasher.update([0]),
    }
}

fn legacy_cloud_provider_credential_key(config: &CloudProviderConfig, field: &str) -> String {
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
            .starts_with("cloud_provider.v2.s3."));
    }

    #[test]
    fn cloud_credential_scope_is_injective_for_former_normalization_collisions() {
        let mut first = credential_test_config();
        first.endpoint = Some("https://storage.example/a-b".to_string());
        let mut second = first.clone();
        second.endpoint = Some("https://storage.example/a_b".to_string());

        assert_eq!(
            legacy_cloud_provider_credential_key(&first, "secret_access_key"),
            legacy_cloud_provider_credential_key(&second, "secret_access_key")
        );
        assert_ne!(
            cloud_provider_credential_key(&first, "secret_access_key"),
            cloud_provider_credential_key(&second, "secret_access_key")
        );
    }

    #[test]
    fn cloud_credential_scope_includes_region_and_none_distinction() {
        let first = credential_test_config();
        let mut other_region = first.clone();
        other_region.region = Some("us-east-1".to_string());
        let mut empty_endpoint = first.clone();
        empty_endpoint.endpoint = Some(String::new());
        let mut no_endpoint = first.clone();
        no_endpoint.endpoint = None;

        assert_ne!(
            cloud_provider_credential_key(&first, "access_key_id"),
            cloud_provider_credential_key(&other_region, "access_key_id")
        );
        assert_ne!(
            cloud_provider_credential_key(&empty_endpoint, "access_key_id"),
            cloud_provider_credential_key(&no_endpoint, "access_key_id")
        );
    }
}
