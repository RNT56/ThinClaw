//! Shared sandbox-related types for builds with and without container backends.
//!
//! When `docker-sandbox` is enabled, these types come directly from the
//! orchestrator modules. Otherwise, lightweight local shims are used so the
//! rest of the code can compile without optional container dependencies.

#[cfg(feature = "docker-sandbox")]
pub use crate::orchestrator::TokenStore;
#[cfg(feature = "docker-sandbox")]
pub use crate::orchestrator::api::PendingPrompt;
#[cfg(feature = "docker-sandbox")]
pub use crate::orchestrator::auth::CredentialGrant;
#[cfg(feature = "docker-sandbox")]
pub use crate::orchestrator::job_manager::{
    CompletionResult, ContainerHandle, ContainerJobConfig, ContainerJobManager, ContainerState,
    JobMode,
};

#[cfg(not(feature = "docker-sandbox"))]
mod fallback {
    use serde::{Deserialize, Serialize};
    use std::path::PathBuf;
    use uuid::Uuid;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct CredentialGrant {
        pub secret_name: String,
        pub env_var: String,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub enum JobMode {
        Worker,
        ClaudeCode,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ContainerState {
        Creating,
        Running,
        Stopped,
        Failed,
    }

    #[derive(Debug)]
    pub struct ContainerJobConfig;

    impl Default for ContainerJobConfig {
        fn default() -> Self {
            Self
        }
    }

    #[derive(Debug)]
    pub struct TokenStore;

    impl TokenStore {
        pub fn new() -> Self {
            Self
        }
    }

    #[derive(Debug, Clone)]
    pub struct CompletionResult {
        pub success: bool,
        pub message: Option<String>,
    }

    #[derive(Debug, Clone)]
    pub struct ContainerHandle {
        pub state: ContainerState,
        pub completion_result: Option<CompletionResult>,
    }

    #[derive(Debug, Clone)]
    pub struct PendingPrompt {
        pub content: String,
        pub done: bool,
    }

    #[derive(Debug)]
    pub struct ContainerJobManager;

    impl ContainerJobManager {
        pub fn new(_config: ContainerJobConfig, _token_store: TokenStore) -> Self {
            Self
        }

        pub async fn create_job(
            &self,
            _job_id: Uuid,
            _task: &str,
            _project_dir: Option<PathBuf>,
            _mode: JobMode,
            _credential_grants: Vec<CredentialGrant>,
        ) -> Result<(), String> {
            Ok(())
        }

        pub async fn stop_job(&self, _job_id: Uuid) -> Result<(), String> {
            Ok(())
        }

        pub async fn cleanup_job(&self, _job_id: Uuid) {}

        pub async fn get_handle(&self, _job_id: Uuid) -> Option<ContainerHandle> {
            None
        }

        pub async fn update_claude_code_settings(
            &self,
            _model: Option<String>,
            _max_turns: Option<u32>,
        ) {
        }
    }
}

#[cfg(not(feature = "docker-sandbox"))]
pub use fallback::{
    CompletionResult, ContainerHandle, ContainerJobConfig, ContainerJobManager, ContainerState,
    CredentialGrant, JobMode, PendingPrompt, TokenStore,
};
