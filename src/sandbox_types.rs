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
    CompletionResult, ContainerCompletionClaim, ContainerHandle, ContainerJobConfig,
    ContainerJobManager, ContainerState, JobMode,
};

#[cfg(not(feature = "docker-sandbox"))]
mod fallback {
    use std::future::Future;

    pub use thinclaw_types::sandbox::{CredentialGrant, JobMode};
    use uuid::Uuid;

    const CONTAINER_UNAVAILABLE: &str =
        "container jobs are unavailable: ThinClaw was built without the docker-sandbox feature";

    fn container_unavailable<T>() -> Result<T, String> {
        Err(CONTAINER_UNAVAILABLE.to_string())
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
        pub runtime_scope: String,
        pub claude_code_enabled: bool,
        pub codex_code_enabled: bool,
        pub interactive_idle_timeout_secs: u64,
    }

    impl Default for ContainerJobConfig {
        fn default() -> Self {
            Self {
                runtime_scope: crate::runtime_lease::runtime_scope_id_for_path(
                    &crate::platform::resolve_data_dir(""),
                ),
                claude_code_enabled: false,
                codex_code_enabled: false,
                interactive_idle_timeout_secs:
                    crate::sandbox_jobs::DEFAULT_SANDBOX_IDLE_TIMEOUT_SECS,
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
        pub status: String,
        pub session_id: Option<String>,
        pub success: bool,
        pub message: Option<String>,
        pub iterations: u32,
    }

    #[derive(Debug, Clone)]
    pub struct ContainerCompletionClaim {
        pub first_completion: bool,
        pub result: CompletionResult,
        pub container_id: String,
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
            spec: crate::sandbox_jobs::SandboxJobSpec,
            _credential_grants: Vec<CredentialGrant>,
        ) -> Result<(), String> {
            crate::sandbox_jobs::validate_sandbox_job_spec(&spec)?;
            container_unavailable()
        }

        pub async fn stop_job(&self, _job_id: Uuid) -> Result<(), String> {
            container_unavailable()
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
            container_unavailable()
        }

        pub async fn claim_job_completion(
            &self,
            _job_id: Uuid,
            _result: CompletionResult,
        ) -> Result<ContainerCompletionClaim, String> {
            container_unavailable()
        }

        pub async fn cleanup_completed_job_container(&self, _job_id: Uuid, _container_id: &str) {}

        pub async fn list_jobs(&self) -> Vec<ContainerHandle> {
            Vec::new()
        }

        pub async fn shutdown_all(&self) {}

        pub fn interactive_idle_timeout_secs(&self) -> u64 {
            self.config.interactive_idle_timeout_secs
        }

        pub fn runtime_scope(&self) -> &str {
            &self.config.runtime_scope
        }

        pub fn stamp_job_spec(&self, spec: &mut crate::sandbox_jobs::SandboxJobSpec) {
            spec.runtime_scope = Some(self.config.runtime_scope.clone());
        }

        pub fn validate_job_spec(
            &self,
            _job_id: Uuid,
            spec: &crate::sandbox_jobs::SandboxJobSpec,
        ) -> Result<(), String> {
            crate::sandbox_jobs::validate_sandbox_job_spec(spec)
        }

        pub(crate) async fn run_owned_finalization<F>(&self, future: F) -> Result<(), String>
        where
            F: Future<Output = Result<(), String>> + Send + 'static,
        {
            future.await
        }

        pub async fn update_claude_code_settings(
            &self,
            _model: Option<Option<String>>,
            _max_turns: Option<Option<u32>>,
        ) -> Result<(), String> {
            container_unavailable()
        }

        pub async fn update_codex_code_settings(
            &self,
            _model: Option<String>,
        ) -> Result<(), String> {
            container_unavailable()
        }

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
    CompletionResult, ContainerCompletionClaim, ContainerHandle, ContainerJobConfig,
    ContainerJobManager, ContainerState, CredentialGrant, JobMode, PendingPrompt, TokenStore,
};

pub type PromptQueue = Arc<Mutex<HashMap<Uuid, VecDeque<PendingPrompt>>>>;

#[cfg(all(test, not(feature = "docker-sandbox")))]
mod tests {
    use super::*;

    fn valid_spec() -> crate::sandbox_jobs::SandboxJobSpec {
        crate::sandbox_jobs::SandboxJobSpec::new(
            "bounded job",
            "do the work",
            "principal",
            "actor",
            None,
            JobMode::Worker,
        )
    }

    #[test]
    fn fallback_manager_stamps_and_validates_specs() {
        let manager = ContainerJobManager::new(
            ContainerJobConfig {
                runtime_scope: "runtime_test".to_string(),
                ..ContainerJobConfig::default()
            },
            TokenStore::new(),
        );
        let job_id = Uuid::new_v4();
        let mut spec = valid_spec();

        manager.stamp_job_spec(&mut spec);
        assert_eq!(spec.runtime_scope.as_deref(), Some("runtime_test"));
        assert!(manager.validate_job_spec(job_id, &spec).is_ok());

        spec.title.clear();
        assert!(manager.validate_job_spec(job_id, &spec).is_err());
    }

    #[tokio::test]
    async fn fallback_manager_fails_closed_without_container_support() {
        let manager = ContainerJobManager::new(ContainerJobConfig::default(), TokenStore::new());
        let error = manager
            .create_job(Uuid::new_v4(), valid_spec(), Vec::new())
            .await
            .expect_err("reduced builds must not report a phantom container job");
        assert!(error.contains("without the docker-sandbox feature"));
    }

    #[tokio::test]
    async fn fallback_manager_runs_owned_finalization_to_completion() {
        let manager = ContainerJobManager::new(ContainerJobConfig::default(), TokenStore::new());
        assert_eq!(
            manager
                .run_owned_finalization(async { Ok::<_, String>(()) })
                .await,
            Ok(())
        );
    }
}
