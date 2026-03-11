//! Tauri commands for OpenClaw integration
//!
//! Split into focused submodules:
//! - `types`: Response/input structs
//! - `gateway`: Gateway lifecycle, status, diagnostics
//! - `keys`: API key management, secret toggles, cloud config
//! - `sessions`: Session CRUD, history, messaging, memory
//! - `rpc`: Skills, cron, config, system commands

use tauri::{AppHandle, Manager};
use tokio::sync::RwLock;

use super::config::OpenClawConfig;

mod gateway;
mod keys;
pub(crate) mod rpc;
mod sessions;
pub mod types;

// Re-export all public command functions
pub use gateway::*;
pub use keys::*;
pub use rpc::*;
pub use sessions::*;
pub use types::*;

/// OpenClaw manager state — manages config and workspace paths.
///
/// After Phase 3 migration, this struct is much simpler: it only holds
/// the config for key management and workspace path resolution.
/// The WS client, process management, and gateway lifecycle have been
/// replaced by `IronClawState`.
pub struct OpenClawManager {
    /// App handle for paths
    pub(crate) app: AppHandle,
    /// Configuration manager
    pub(crate) config: RwLock<Option<OpenClawConfig>>,
}

impl OpenClawManager {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            config: RwLock::new(None),
        }
    }

    /// Initialize config from app data dir
    pub async fn init_config(&self) -> Result<OpenClawConfig, String> {
        let app_data_dir = self.app.path().app_data_dir().map_err(|e| e.to_string())?;

        let config = OpenClawConfig::new(app_data_dir);
        config.ensure_dirs().map_err(|e| e.to_string())?;

        *self.config.write().await = Some(config.clone());
        Ok(config)
    }

    /// Get current config
    pub async fn get_config(&self) -> Option<OpenClawConfig> {
        self.config.read().await.clone()
    }
}
