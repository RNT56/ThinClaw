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
    provider: Option<Box<dyn CloudProvider>>,
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
                            match provider::create_provider(&config) {
                                Ok(provider) => {
                                    inner.provider = Some(provider);
                                    inner.provider_config = Some(config);
                                }
                                Err(e) => {
                                    warn!("[cloud] Failed to create provider on init: {}", e);
                                }
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
        inner.provider = Some(provider);
        inner.provider_config = Some(config);

        info!("[cloud] Provider configured: {}", status.provider_name);
        Ok(status)
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
        // Check prerequisites
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
        let cancel_flag;
        {
            let mut inner = self.inner.write().await;
            inner.migration_in_progress = true;
            let flag = Arc::new(RwLock::new(false));
            cancel_flag = flag.clone();
            inner.cancel_flag = Some(flag);
        }

        info!("[cloud] Starting local → cloud migration: {}", migration_id);

        // Execute the full migration flow
        let (app_data_dir, provider_ref, master_key_ref) = {
            let inner = self.inner.read().await;
            (
                inner.app_data_dir.clone(),
                // Safety: we checked provider.is_some() above
                &**inner.provider.as_ref().unwrap() as *const dyn CloudProvider,
                inner.master_key.as_ref().unwrap().clone(),
            )
        };
        // SAFETY: provider outlives this call (held in inner via RwLock, only
        // released after we finish). We need the raw ptr to avoid holding
        // the RwLock across the await.
        let result = unsafe {
            migration::run_to_cloud(
                app.clone(),
                pool,
                &app_data_dir,
                &*provider_ref,
                &master_key_ref,
                &migration_id,
                cancel_flag,
            )
            .await
        };

        // Clear migration flag + update mode on success
        {
            let mut inner = self.inner.write().await;
            inner.migration_in_progress = false;
            inner.cancel_flag = None;
            if result.is_ok() {
                inner.mode = StorageMode::Cloud {
                    provider_type: "s3".to_string(),
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
        let inner = self.inner.read().await;
        if inner.migration_in_progress {
            return Err("Migration already in progress".into());
        }
        if matches!(&inner.mode, StorageMode::Local) {
            return Err("Already in local mode".into());
        }
        drop(inner);

        // Ensure master key
        self.ensure_master_key().await?;

        let migration_id = uuid::Uuid::new_v4().to_string();
        let cancel_flag;
        {
            let mut inner = self.inner.write().await;
            inner.migration_in_progress = true;
            let flag = Arc::new(RwLock::new(false));
            cancel_flag = flag.clone();
            inner.cancel_flag = Some(flag);
        }

        info!("[cloud] Starting cloud → local migration: {}", migration_id);

        let (app_data_dir, provider_ref, master_key_ref) = {
            let inner = self.inner.read().await;
            (
                inner.app_data_dir.clone(),
                &**inner.provider.as_ref().unwrap() as *const dyn CloudProvider,
                inner.master_key.as_ref().unwrap().clone(),
            )
        };
        let result = unsafe {
            migration::run_to_local(
                app.clone(),
                pool,
                &app_data_dir,
                &*provider_ref,
                &master_key_ref,
                &migration_id,
                cancel_flag,
            )
            .await
        };

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
}
