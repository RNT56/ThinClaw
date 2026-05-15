//! Tauri commands for OpenClaw integration.
//!
//! The `openclaw_*` Tauri command names are kept as alpha compatibility IPC
//! even when the implementation delegates to embedded IronClaw internals.
//!
//! Split into focused submodules:
//! - `types`: Response/input structs
//! - `gateway`: Gateway lifecycle, status, diagnostics
//! - `keys`: API key management, secret toggles, cloud config
//! - `sessions`: Session CRUD, history, messaging, memory
//! - `rpc_skills`: Skills management (install, list, toggle)
//! - `rpc_routines`: Cron/routine CRUD, lint, audit, delete, toggle
//! - `rpc_config`: Config CRUD, settings toggles, autonomy, bootstrap, cloud model, system presence
//! - `rpc_jobs_autonomy`: Jobs and desktop autonomy surfaces
//! - `rpc_orchestration`: Sub-agent registry, session spawning, canvas, agent profiles
//! - `rpc_extensions`: Hooks, extensions, diagnostics, tools, pairing, compaction
//! - `rpc_dashboard`: Cost tracking, channel status, ClawHub, routing, Gmail, workspace
//! - `rpc_experiments_learning`: Experiment and learning review surfaces

use tauri::{AppHandle, Manager};
use tokio::sync::RwLock;

use super::config::OpenClawConfig;

mod gateway;
mod keys;
mod remote_provider_config;
mod rpc;
pub(crate) mod rpc_config;
pub(crate) mod rpc_dashboard;
pub(crate) mod rpc_experiments_learning;
pub(crate) mod rpc_extensions;
pub(crate) mod rpc_jobs_autonomy;
pub(crate) mod rpc_orchestration;
pub(crate) mod rpc_routines;
pub(crate) mod rpc_skills;
mod sessions;
pub mod types;

// Re-export all public command functions
pub use gateway::*;
pub use keys::*;
pub use rpc_config::*;
pub use rpc_dashboard::*;
pub use rpc_experiments_learning::*;
pub use rpc_extensions::*;
pub use rpc_jobs_autonomy::*;
pub use rpc_orchestration::*;
pub use rpc_routines::*;
pub use rpc_skills::*;
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
