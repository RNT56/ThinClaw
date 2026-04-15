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

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum JobMode {
        Worker,
        ClaudeCode,
        CodexCode,
    }

    impl JobMode {
        pub fn as_str(&self) -> &'static str {
            match self {
                Self::Worker => "worker",
                Self::ClaudeCode => "claude_code",
                Self::CodexCode => "codex_code",
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ContainerState {
        Creating,
        Running,
        Stopped,
        Failed,
    }

    #[derive(Debug, Clone)]
    pub struct ContainerJobConfig {
        pub claude_code_enabled: bool,
        pub codex_code_enabled: bool,
    }

    impl Default for ContainerJobConfig {
        fn default() -> Self {
            Self {
                claude_code_enabled: false,
                codex_code_enabled: false,
            }
        }
    }

    #[derive(Debug)]
    pub struct TokenStore;

    impl Default for TokenStore {
        fn default() -> Self {
            Self::new()
        }
    }

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
    pub struct ContainerJobManager {
        config: ContainerJobConfig,
    }

    impl ContainerJobManager {
        pub fn new(config: ContainerJobConfig, _token_store: TokenStore) -> Self {
            Self { config }
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

        pub async fn update_codex_code_settings(&self, _model: Option<String>) {}

        pub fn claude_code_enabled(&self) -> bool {
            self.config.claude_code_enabled
        }

        pub fn codex_code_enabled(&self) -> bool {
            self.config.codex_code_enabled
        }
    }
}

#[cfg(not(feature = "docker-sandbox"))]
pub use fallback::{
    CompletionResult, ContainerHandle, ContainerJobConfig, ContainerJobManager, ContainerState,
    CredentialGrant, JobMode, PendingPrompt, TokenStore,
};
