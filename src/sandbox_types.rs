//! Shared sandbox-related types for builds with and without container backends.
//!
//! When `docker-sandbox` is enabled, these types come directly from the
//! orchestrator modules. Otherwise, lightweight local shims are used so the
//! rest of the code can compile without optional container dependencies.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use tokio::sync::Mutex;
use uuid::Uuid;

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
    pub use thinclaw_types::sandbox::{CredentialGrant, JobMode};
    use uuid::Uuid;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ContainerState {
        Creating,
        Running,
        Stopped,
        Failed,
    }

    #[derive(Debug, Clone, Default)]
    pub struct ContainerJobConfig {
        pub claude_code_enabled: bool,
        pub codex_code_enabled: bool,
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
        pub status: String,
        pub session_id: Option<String>,
        pub success: bool,
        pub message: Option<String>,
        pub iterations: u32,
    }

    #[derive(Debug, Clone)]
    pub struct ContainerHandle {
        pub job_id: Uuid,
        pub container_id: String,
        pub spec: crate::sandbox_jobs::SandboxJobSpec,
        pub state: ContainerState,
        pub mode: JobMode,
        pub created_at: chrono::DateTime<chrono::Utc>,
        pub last_worker_status: Option<String>,
        pub worker_iteration: u32,
        pub completion_result: Option<CompletionResult>,
    }

    #[derive(Debug, Clone)]
    pub struct PendingPrompt {
        pub content: Option<String>,
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
            _spec: crate::sandbox_jobs::SandboxJobSpec,
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

        pub async fn complete_job(
            &self,
            _job_id: Uuid,
            _result: CompletionResult,
        ) -> Result<(), String> {
            Ok(())
        }

        pub async fn list_jobs(&self) -> Vec<ContainerHandle> {
            Vec::new()
        }

        pub fn interactive_idle_timeout_secs(&self) -> u64 {
            crate::sandbox_jobs::DEFAULT_SANDBOX_IDLE_TIMEOUT_SECS
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

pub type PromptQueue = Arc<Mutex<HashMap<Uuid, VecDeque<PendingPrompt>>>>;
