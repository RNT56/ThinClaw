//! Typed adapter seam for infrastructure shared by both Desktop product modes.
//!
//! The Direct AI Workbench and ThinClaw Agent Cockpit remain separate product
//! systems. This state only centralizes access to host-level services that are
//! already managed by Tauri so consumers can migrate one domain at a time
//! without constructing duplicate stores or reaching through unrelated state.

use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, State};

use crate::config::ConfigManager;
use crate::inference::{InferenceRouter, ModelProviderRegistry};
use crate::secret_store::SecretStore;
use crate::thinclaw::runtime_bridge::ThinClawRuntimeState;
use crate::thinclaw::ThinClawManager;

/// App-wide entry point for services that may be consumed by either mode.
///
/// Accessors return the existing Tauri-managed singleton. The seam owns no
/// duplicate cache, database pool, runtime, or provider registry.
#[derive(Clone)]
pub struct SharedServices {
    app: AppHandle,
}

impl SharedServices {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }

    pub fn app_handle(&self) -> &AppHandle {
        &self.app
    }

    pub fn secrets(&self) -> State<'_, SecretStore> {
        self.app.state::<SecretStore>()
    }

    pub fn settings(&self) -> State<'_, ConfigManager> {
        self.app.state::<ConfigManager>()
    }

    pub fn inference(&self) -> State<'_, InferenceRouter> {
        self.app.state::<InferenceRouter>()
    }

    pub fn models(&self) -> State<'_, ModelProviderRegistry> {
        self.app.state::<ModelProviderRegistry>()
    }

    pub fn direct_history(&self) -> State<'_, SqlitePool> {
        self.app.state::<SqlitePool>()
    }

    pub fn agent_config(&self) -> State<'_, ThinClawManager> {
        self.app.state::<ThinClawManager>()
    }

    pub fn agent_runtime(&self) -> State<'_, ThinClawRuntimeState> {
        self.app.state::<ThinClawRuntimeState>()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn seam_is_registered_after_every_required_singleton() {
        let source = include_str!("lib.rs");
        let seam = source
            .find("handle.manage(shared_services::SharedServices::new(handle.clone()));")
            .expect("SharedServices must be registered");

        for dependency in [
            "handle.manage(secret_store);",
            "handle.manage(inference_router);",
            "handle.manage(model_registry);",
            "handle.manage(pool);",
            "handle.manage(ironclaw_state);",
        ] {
            let position = source
                .find(dependency)
                .unwrap_or_else(|| panic!("missing shared-service dependency: {dependency}"));
            assert!(
                position < seam,
                "{dependency} must be managed before SharedServices"
            );
        }
    }
}
