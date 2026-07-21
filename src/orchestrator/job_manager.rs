//! Container lifecycle management for sandboxed jobs.
//!
//! Extends the existing `SandboxManager` infrastructure to support persistent
//! containers with their own agent loops (as opposed to ephemeral per-command containers).

use std::collections::{HashMap, hash_map::Entry};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use bollard::models::{Mount, MountType};
use chrono::{DateTime, Utc};
use futures::future::join_all;
pub use thinclaw_types::sandbox::JobMode;
use tokio::sync::{Notify, RwLock, oneshot};
use tokio::task::JoinSet;
use uuid::Uuid;

use crate::error::OrchestratorError;
use crate::orchestrator::auth::{CredentialGrant, TokenStore};
use crate::sandbox::config::SandboxPolicy;
use crate::sandbox::network::{
    CONTAINER_SCOPE_LABEL, MANAGED_CONTAINER_LABEL, cleanup_sandbox_networks,
    managed_container_labels,
};
use crate::sandbox::proxy::{NetworkProxyBuilder, NoCredentialResolver, validate_domain_allowlist};
use crate::sandbox::relay::{
    RELAY_ORCHESTRATOR_PORT, RELAY_PROXY_PORT, RelayForward, SandboxNetworkRelay,
};
use crate::sandbox::{HttpProxy, connect_docker};
use crate::sandbox_jobs::{
    MAX_JOB_IDLE_TIMEOUT_SECS, MAX_JOB_POLICY_ITEM_BYTES, MAX_JOB_POLICY_ITEMS, SandboxJobSpec,
    validate_sandbox_job_spec,
};

const MAX_CODE_MODEL_BYTES: usize = 256;
const MAX_CLAUDE_CODE_TURNS: u32 = 1_000;

fn valid_code_model(model: &str) -> bool {
    !model.trim().is_empty()
        && model.len() <= MAX_CODE_MODEL_BYTES
        && !model.chars().any(char::is_control)
}

/// Configuration for the container job manager.
#[derive(Clone)]
pub struct ContainerJobConfig {
    /// Stable identifier for the runtime state directory. Used to scope
    /// managed-container labels and orphan cleanup across process restarts.
    pub runtime_scope: String,
    /// Docker image for worker containers.
    pub image: String,
    /// Default memory limit in MB.
    pub memory_limit_mb: u64,
    /// Default CPU shares.
    pub cpu_shares: u32,
    /// Port the orchestrator internal API listens on.
    pub orchestrator_port: u16,
    /// Hosts sandbox jobs may reach through the authenticated network proxy.
    pub network_allowlist: Vec<String>,
    /// Host proxy port (`0` asks the OS to allocate one).
    pub proxy_port: u16,
    /// Anthropic API key for Claude Code containers (read from ANTHROPIC_API_KEY).
    /// Takes priority over OAuth token.
    pub claude_code_api_key: Option<String>,
    /// OAuth access token extracted from the host's `claude login` session.
    /// Passed as CLAUDE_CODE_OAUTH_TOKEN to containers. Falls back to this
    /// when no ANTHROPIC_API_KEY is available.
    pub claude_code_oauth_token: Option<String>,
    /// Whether Claude Code mode is available for new jobs.
    pub claude_code_enabled: bool,
    /// Claude model to use in ClaudeCode mode.
    pub claude_code_model: String,
    /// Maximum turns for Claude Code.
    pub claude_code_max_turns: u32,
    /// Memory limit in MB for Claude Code containers (heavier than workers).
    pub claude_code_memory_limit_mb: u64,
    /// Allowed tool patterns for Claude Code (passed as CLAUDE_CODE_ALLOWED_TOOLS env var).
    pub claude_code_allowed_tools: Vec<String>,
    /// OpenAI API key for Codex containers (read from OPENAI_API_KEY).
    pub codex_code_api_key: Option<String>,
    /// Whether Codex mode is available for new jobs.
    pub codex_code_enabled: bool,
    /// Codex model to use in CodexCode mode.
    pub codex_code_model: String,
    /// Memory limit in MB for Codex containers.
    pub codex_code_memory_limit_mb: u64,
    /// Host directory containing Codex auth/config files to mount read-only.
    pub codex_code_home_dir: PathBuf,
    /// Default idle timeout for interactive sandbox jobs (seconds).
    pub interactive_idle_timeout_secs: u64,
}

impl std::fmt::Debug for ContainerJobConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ContainerJobConfig")
            .field("runtime_scope", &self.runtime_scope)
            .field("image", &self.image)
            .field("memory_limit_mb", &self.memory_limit_mb)
            .field("cpu_shares", &self.cpu_shares)
            .field("orchestrator_port", &self.orchestrator_port)
            .field("network_allowlist", &self.network_allowlist)
            .field("proxy_port", &self.proxy_port)
            .field(
                "claude_code_api_key",
                &self.claude_code_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "claude_code_oauth_token",
                &self.claude_code_oauth_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("claude_code_enabled", &self.claude_code_enabled)
            .field("claude_code_model", &self.claude_code_model)
            .field("claude_code_max_turns", &self.claude_code_max_turns)
            .field(
                "claude_code_memory_limit_mb",
                &self.claude_code_memory_limit_mb,
            )
            .field("claude_code_allowed_tools", &self.claude_code_allowed_tools)
            .field(
                "codex_code_api_key",
                &self.codex_code_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("codex_code_enabled", &self.codex_code_enabled)
            .field("codex_code_model", &self.codex_code_model)
            .field(
                "codex_code_memory_limit_mb",
                &self.codex_code_memory_limit_mb,
            )
            .field("codex_code_home_dir", &self.codex_code_home_dir)
            .field(
                "interactive_idle_timeout_secs",
                &self.interactive_idle_timeout_secs,
            )
            .finish()
    }
}

impl Default for ContainerJobConfig {
    fn default() -> Self {
        Self {
            runtime_scope: crate::runtime_lease::runtime_scope_id_for_path(
                &crate::platform::resolve_data_dir(""),
            ),
            image: "thinclaw-worker:latest".to_string(),
            memory_limit_mb: 2048,
            cpu_shares: 1024,
            orchestrator_port: 50051,
            network_allowlist: crate::sandbox::default_allowlist(),
            proxy_port: 0,
            claude_code_api_key: None,
            claude_code_oauth_token: None,
            claude_code_enabled: false,
            claude_code_model: crate::config::ClaudeCodeConfig::default().model,
            claude_code_max_turns: 50,
            claude_code_memory_limit_mb: 4096,
            claude_code_allowed_tools: crate::config::ClaudeCodeConfig::default().allowed_tools,
            codex_code_api_key: None,
            codex_code_enabled: false,
            codex_code_model: "gpt-5.3-codex".to_string(),
            codex_code_memory_limit_mb: 4096,
            codex_code_home_dir: crate::config::CodexCodeConfig::default().home_dir,
            interactive_idle_timeout_secs: crate::sandbox_jobs::DEFAULT_SANDBOX_IDLE_TIMEOUT_SECS,
        }
    }
}

/// State of a container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerState {
    Creating,
    Running,
    Stopped,
    Failed,
}

impl std::fmt::Display for ContainerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Creating => write!(f, "creating"),
            Self::Running => write!(f, "running"),
            Self::Stopped => write!(f, "stopped"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

/// Handle to a running container job.
#[derive(Debug, Clone)]
pub struct ContainerHandle {
    pub job_id: Uuid,
    pub container_id: String,
    pub state: ContainerState,
    pub mode: JobMode,
    pub created_at: DateTime<Utc>,
    pub spec: SandboxJobSpec,
    /// Last status message reported by the worker (iteration count, progress, etc.).
    pub last_worker_status: Option<String>,
    /// Which iteration the worker is on (updated via status reports).
    pub worker_iteration: u32,
    /// Completion result from the worker (set when the worker reports done).
    pub completion_result: Option<CompletionResult>,
    // NOTE: auth_token is intentionally NOT in this struct.
    // It lives only in the TokenStore (never logged, serialized, or persisted).
}

/// Result reported by a worker on completion.
#[derive(Debug, Clone)]
pub struct CompletionResult {
    pub status: String,
    pub session_id: Option<String>,
    pub success: bool,
    pub message: Option<String>,
    pub iterations: u32,
}

/// Canonical first-writer-wins completion selected under the container map
/// lock. Every racing finalizer receives the same result and container ID, so
/// durable status/event persistence cannot diverge from the in-memory result.
#[derive(Debug, Clone)]
pub struct ContainerCompletionClaim {
    pub first_completion: bool,
    pub result: CompletionResult,
    pub container_id: String,
}

/// Validate that a project directory is under the supplied projects root.
///
/// Returns the canonicalized path if valid. Creates the base directory if
/// it doesn't exist (so the prefix check always runs).
///
/// # TOCTOU note
///
/// There is a time-of-check/time-of-use gap between `canonicalize()` here
/// and the actual Docker `binds.push()` in the caller. In a multi-tenant
/// system a malicious actor could swap a symlink after validation. This is
/// acceptable in ThinClaw's single-tenant design where the user controls
/// the filesystem.
fn validate_bind_mount_path(
    dir: &std::path::Path,
    projects_base: &std::path::Path,
    job_id: Uuid,
) -> Result<PathBuf, OrchestratorError> {
    let canonical = dir
        .canonicalize()
        .map_err(|e| OrchestratorError::ContainerCreationFailed {
            job_id,
            reason: format!(
                "failed to canonicalize project dir {}: {}",
                dir.display(),
                e
            ),
        })?;

    // Ensure the base exists so canonicalize always succeeds.
    std::fs::create_dir_all(projects_base).map_err(|e| {
        OrchestratorError::ContainerCreationFailed {
            job_id,
            reason: format!(
                "failed to create projects base {}: {}",
                projects_base.display(),
                e
            ),
        }
    })?;

    let canonical_base =
        projects_base
            .canonicalize()
            .map_err(|e| OrchestratorError::ContainerCreationFailed {
                job_id,
                reason: format!(
                    "failed to canonicalize projects base {}: {}",
                    projects_base.display(),
                    e
                ),
            })?;

    if !canonical.starts_with(&canonical_base) {
        return Err(OrchestratorError::ContainerCreationFailed {
            job_id,
            reason: format!(
                "project directory {} is outside allowed base {}",
                canonical.display(),
                canonical_base.display()
            ),
        });
    }

    Ok(canonical)
}

/// Manages the lifecycle of Docker containers for sandboxed job execution.
pub struct ContainerJobManager {
    config: ContainerJobConfig,
    token_store: TokenStore,
    pub(crate) containers: Arc<RwLock<HashMap<Uuid, ContainerHandle>>>,
    /// Cached Docker connection (created on first use).
    docker: Arc<RwLock<Option<bollard::Docker>>>,
    /// Authenticated host proxy used as the only external-egress path from the
    /// internal sandbox bridge.
    proxy: RwLock<Option<HttpProxy>>,
    /// Per-job relays and networks. Jobs never share a Docker bridge, so one
    /// worker cannot scan or deny service to another worker.
    relays: RwLock<HashMap<Uuid, SandboxNetworkRelay>>,
    /// Runtime-override for Claude Code model (updated via settings API).
    cc_model: RwLock<String>,
    /// Runtime-override for Claude Code max turns.
    cc_max_turns: RwLock<u32>,
    /// Runtime-override for Codex model.
    codex_model: RwLock<String>,
    accepting_jobs: AtomicBool,
    create_admission: std::sync::Mutex<()>,
    inflight_creates: AtomicUsize,
    inflight_create_notify: Notify,
    /// Cancellation-shielded terminal persistence/cleanup tasks. Callers may
    /// disappear with an HTTP request, but these tasks are owned and drained
    /// by the manager during shutdown.
    finalization_tasks: std::sync::Mutex<JoinSet<()>>,
    accepting_finalizations: AtomicBool,
}

struct InflightCreateGuard<'a> {
    manager: &'a ContainerJobManager,
}

impl Drop for InflightCreateGuard<'_> {
    fn drop(&mut self) {
        self.manager.inflight_creates.fetch_sub(1, Ordering::AcqRel);
        self.manager.inflight_create_notify.notify_waiters();
    }
}

struct ContainerCreateCancellationGuard {
    armed: bool,
    job_id: Uuid,
    container_name: String,
    token_store: TokenStore,
    containers: Arc<RwLock<HashMap<Uuid, ContainerHandle>>>,
    docker: Arc<RwLock<Option<bollard::Docker>>>,
}

impl ContainerCreateCancellationGuard {
    fn new(manager: &ContainerJobManager, job_id: Uuid, container_name: String) -> Self {
        Self {
            armed: true,
            job_id,
            container_name,
            token_store: manager.token_store.clone(),
            containers: Arc::clone(&manager.containers),
            docker: Arc::clone(&manager.docker),
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ContainerCreateCancellationGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::error!(
                job_id = %self.job_id,
                "Container creation was cancelled outside a Tokio runtime"
            );
            return;
        };
        let job_id = self.job_id;
        let container_name = self.container_name.clone();
        let token_store = self.token_store.clone();
        let containers = Arc::clone(&self.containers);
        let docker_cache = Arc::clone(&self.docker);
        runtime.spawn(async move {
            token_store.revoke(job_id).await;
            {
                let mut handles = containers.write().await;
                if let Some(handle) = handles.get_mut(&job_id)
                    && handle.state == ContainerState::Creating
                    && handle.completion_result.is_none()
                {
                    handle.state = ContainerState::Stopped;
                    handle.completion_result = Some(CompletionResult {
                        status: "cancelled".to_string(),
                        session_id: None,
                        success: false,
                        message: Some("Container creation future was cancelled".to_string()),
                        iterations: handle.worker_iteration,
                    });
                }
            }

            // Docker may finish a create request after its Rust future was
            // dropped. Retry the deterministic name long enough to catch that
            // late commit without ever relying on a missing response ID.
            let Some(docker) = docker_cache.read().await.clone() else {
                return;
            };
            const RETRIES: usize = 10;
            for attempt in 0..RETRIES {
                match docker
                    .remove_container(
                        &container_name,
                        Some(
                            bollard::query_parameters::RemoveContainerOptionsBuilder::new()
                                .force(true)
                                .build(),
                        ),
                    )
                    .await
                {
                    Ok(()) => return,
                    Err(bollard::errors::Error::DockerResponseServerError {
                        status_code: 404,
                        ..
                    }) if attempt + 1 < RETRIES => {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                    Err(bollard::errors::Error::DockerResponseServerError {
                        status_code: 404,
                        ..
                    }) => return,
                    Err(error) if attempt + 1 < RETRIES => {
                        tracing::debug!(
                            %job_id,
                            %container_name,
                            %error,
                            "Retrying cancelled container creation cleanup"
                        );
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                    Err(error) => {
                        tracing::error!(
                            %job_id,
                            %container_name,
                            %error,
                            "Exhausted cancelled container creation cleanup retries"
                        );
                        return;
                    }
                }
            }
        });
    }
}

fn container_name(job_id: Uuid, mode: JobMode) -> String {
    match mode {
        JobMode::Worker => format!("thinclaw-worker-{job_id}"),
        JobMode::ClaudeCode => format!("thinclaw-claude-{job_id}"),
        JobMode::CodexCode => format!("thinclaw-codex-{job_id}"),
    }
}

fn bind_mount(
    job_id: Uuid,
    source: &Path,
    target: &str,
    read_only: bool,
) -> Result<Mount, OrchestratorError> {
    let source = source
        .to_str()
        .ok_or_else(|| OrchestratorError::ContainerCreationFailed {
            job_id,
            reason: format!("bind mount path for {target} is not valid UTF-8"),
        })?;
    Ok(Mount {
        target: Some(target.to_string()),
        source: Some(source.to_string()),
        typ: Some(MountType::BIND),
        read_only: Some(read_only),
        ..Default::default()
    })
}

fn docker_nano_cpus(cpu_shares: u32) -> i64 {
    i64::from(cpu_shares)
        .saturating_mul(1_000_000_000)
        .checked_div(1024)
        .unwrap_or(1_000_000_000)
        .max(10_000_000)
}

fn validate_credential_grants(
    job_id: Uuid,
    grants: &[CredentialGrant],
) -> Result<(), OrchestratorError> {
    const MAX_CREDENTIAL_GRANTS: usize = 20;
    if grants.len() > MAX_CREDENTIAL_GRANTS {
        return Err(OrchestratorError::ContainerCreationFailed {
            job_id,
            reason: format!(
                "too many credential grants ({}; maximum {MAX_CREDENTIAL_GRANTS})",
                grants.len()
            ),
        });
    }

    let mut environment_names = std::collections::HashSet::new();
    for grant in grants {
        if grant.secret_name.trim().is_empty()
            || grant.secret_name.len() > 256
            || grant.secret_name.chars().any(char::is_control)
        {
            return Err(OrchestratorError::ContainerCreationFailed {
                job_id,
                reason: "credential grant contains an invalid secret name".to_string(),
            });
        }
        thinclaw_tools::builtin::job::validate_env_var_name(&grant.env_var).map_err(|error| {
            OrchestratorError::ContainerCreationFailed {
                job_id,
                reason: format!("invalid credential environment variable: {error}"),
            }
        })?;
        if grant.env_var.starts_with("THINCLAW_")
            || matches!(
                grant.env_var.as_str(),
                "HTTP_PROXY"
                    | "HTTPS_PROXY"
                    | "ALL_PROXY"
                    | "NO_PROXY"
                    | "DOCKER_HOST"
                    | "GIT_CONFIG"
                    | "GIT_CONFIG_COUNT"
                    | "GIT_SSH"
                    | "GIT_SSH_COMMAND"
                    | "SSH_AUTH_SOCK"
                    | "SSL_CERT_DIR"
                    | "SSL_CERT_FILE"
            )
        {
            return Err(OrchestratorError::ContainerCreationFailed {
                job_id,
                reason: format!(
                    "credential environment variable '{}' is reserved",
                    grant.env_var
                ),
            });
        }
        if !environment_names.insert(grant.env_var.as_str()) {
            return Err(OrchestratorError::ContainerCreationFailed {
                job_id,
                reason: format!(
                    "credential environment variable '{}' is granted more than once",
                    grant.env_var
                ),
            });
        }
    }
    Ok(())
}

impl ContainerJobManager {
    pub fn new(config: ContainerJobConfig, token_store: TokenStore) -> Self {
        let model = config.claude_code_model.clone();
        let turns = config.claude_code_max_turns;
        let codex_model = config.codex_code_model.clone();
        Self {
            config,
            token_store,
            containers: Arc::new(RwLock::new(HashMap::new())),
            docker: Arc::new(RwLock::new(None)),
            proxy: RwLock::new(None),
            relays: RwLock::new(HashMap::new()),
            cc_model: RwLock::new(model),
            cc_max_turns: RwLock::new(turns),
            codex_model: RwLock::new(codex_model),
            accepting_jobs: AtomicBool::new(true),
            create_admission: std::sync::Mutex::new(()),
            inflight_creates: AtomicUsize::new(0),
            inflight_create_notify: Notify::new(),
            finalization_tasks: std::sync::Mutex::new(JoinSet::new()),
            accepting_finalizations: AtomicBool::new(true),
        }
    }

    pub(crate) async fn run_owned_finalization<F>(&self, future: F) -> Result<(), String>
    where
        F: Future<Output = Result<(), String>> + Send + 'static,
    {
        let (result_tx, result_rx) = oneshot::channel();
        {
            let mut tasks = self
                .finalization_tasks
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            while let Some(result) = tasks.try_join_next() {
                if let Err(error) = result
                    && !error.is_cancelled()
                {
                    tracing::warn!(%error, "Sandbox finalization task failed");
                }
            }
            if !self.accepting_finalizations.load(Ordering::Acquire) {
                return Err("sandbox finalization runtime is shutting down".to_string());
            }
            tasks.spawn(async move {
                let result = future.await;
                let _ = result_tx.send(result);
            });
        }

        result_rx
            .await
            .map_err(|_| "sandbox finalization task exited before returning a result".to_string())?
    }

    pub fn claude_code_enabled(&self) -> bool {
        self.config.claude_code_enabled
    }

    pub fn codex_code_enabled(&self) -> bool {
        self.config.codex_code_enabled
    }

    pub fn interactive_idle_timeout_secs(&self) -> u64 {
        self.config.interactive_idle_timeout_secs
    }

    pub fn runtime_scope(&self) -> &str {
        &self.config.runtime_scope
    }

    pub fn stamp_job_spec(&self, spec: &mut SandboxJobSpec) {
        spec.runtime_scope = Some(self.config.runtime_scope.clone());
    }

    pub fn validate_job_spec(
        &self,
        job_id: Uuid,
        spec: &SandboxJobSpec,
    ) -> Result<(), OrchestratorError> {
        validate_sandbox_job_spec(spec)
            .map_err(|reason| OrchestratorError::ContainerCreationFailed { job_id, reason })
    }

    async fn validate_runtime_config(
        &self,
        job_id: Uuid,
        mode: JobMode,
    ) -> Result<(), OrchestratorError> {
        if self.config.runtime_scope.is_empty()
            || self.config.runtime_scope.len() > 128
            || !self.config.runtime_scope.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            })
        {
            return Err(OrchestratorError::ContainerCreationFailed {
                job_id,
                reason: "sandbox runtime scope is invalid".to_string(),
            });
        }
        let memory_mb = match mode {
            JobMode::ClaudeCode => self.config.claude_code_memory_limit_mb,
            JobMode::CodexCode => self.config.codex_code_memory_limit_mb,
            JobMode::Worker => self.config.memory_limit_mb,
        };
        if memory_mb < 16
            || memory_mb
                .checked_mul(1024 * 1024)
                .and_then(|bytes| i64::try_from(bytes).ok())
                .is_none()
        {
            return Err(OrchestratorError::ContainerCreationFailed {
                job_id,
                reason: "sandbox memory limit is outside Docker's supported range".to_string(),
            });
        }
        if !(2..=262_144).contains(&self.config.cpu_shares) {
            return Err(OrchestratorError::ContainerCreationFailed {
                job_id,
                reason: "sandbox CPU shares must be between 2 and 262144".to_string(),
            });
        }
        if self.config.image.trim().is_empty()
            || self.config.image.len() > 512
            || self.config.image.chars().any(char::is_control)
        {
            return Err(OrchestratorError::ContainerCreationFailed {
                job_id,
                reason: "sandbox image name is empty, oversized, or invalid".to_string(),
            });
        }
        if !(1..=MAX_JOB_IDLE_TIMEOUT_SECS).contains(&self.config.interactive_idle_timeout_secs) {
            return Err(OrchestratorError::ContainerCreationFailed {
                job_id,
                reason: "sandbox interactive idle timeout is outside the supported range"
                    .to_string(),
            });
        }
        if self.config.orchestrator_port == 0 {
            return Err(OrchestratorError::ContainerCreationFailed {
                job_id,
                reason: "orchestrator port was not reserved".to_string(),
            });
        }
        validate_domain_allowlist(&self.config.network_allowlist)
            .map_err(|reason| OrchestratorError::ContainerCreationFailed { job_id, reason })?;
        if mode == JobMode::ClaudeCode && !self.config.claude_code_enabled {
            return Err(OrchestratorError::ContainerCreationFailed {
                job_id,
                reason: "Claude Code container mode is disabled".to_string(),
            });
        }
        if mode == JobMode::CodexCode && !self.config.codex_code_enabled {
            return Err(OrchestratorError::ContainerCreationFailed {
                job_id,
                reason: "Codex container mode is disabled".to_string(),
            });
        }
        let (claude_model, claude_max_turns) = if mode == JobMode::ClaudeCode {
            (
                Some(self.cc_model.read().await.clone()),
                Some(*self.cc_max_turns.read().await),
            )
        } else {
            (None, None)
        };
        if mode == JobMode::ClaudeCode
            && (!valid_code_model(claude_model.as_deref().unwrap_or_default())
                || !(1..=MAX_CLAUDE_CODE_TURNS).contains(&claude_max_turns.unwrap_or_default())
                || self.config.claude_code_allowed_tools.len() > MAX_JOB_POLICY_ITEMS
                || {
                    let mut unique = std::collections::HashSet::new();
                    self.config.claude_code_allowed_tools.iter().any(|tool| {
                        tool.trim().is_empty()
                            || tool.len() > MAX_JOB_POLICY_ITEM_BYTES
                            || tool.chars().any(char::is_control)
                            || tool.contains(',')
                            || !unique.insert(tool.as_str())
                    })
                })
        {
            return Err(OrchestratorError::ContainerCreationFailed {
                job_id,
                reason: "Claude Code sandbox configuration is invalid".to_string(),
            });
        }
        if mode == JobMode::CodexCode && !valid_code_model(self.codex_model.read().await.as_str()) {
            return Err(OrchestratorError::ContainerCreationFailed {
                job_id,
                reason: "Codex sandbox model configuration is invalid".to_string(),
            });
        }
        for (label, secret) in [
            (
                "Anthropic API key",
                self.config.claude_code_api_key.as_deref(),
            ),
            (
                "Claude OAuth token",
                self.config.claude_code_oauth_token.as_deref(),
            ),
            ("OpenAI API key", self.config.codex_code_api_key.as_deref()),
        ] {
            if secret.is_some_and(|secret| {
                secret.is_empty() || secret.len() > 64 * 1024 || secret.contains('\0')
            }) {
                return Err(OrchestratorError::ContainerCreationFailed {
                    job_id,
                    reason: format!("sandbox {label} is invalid or oversized"),
                });
            }
        }
        Ok(())
    }

    /// Update Claude Code settings at runtime (called by the settings API).
    ///
    /// The next Claude Code container spawned will use the updated values.
    /// Already-running containers keep their current settings.
    pub async fn update_claude_code_settings(
        &self,
        model: Option<Option<String>>,
        max_turns: Option<Option<u32>>,
    ) -> Result<(), String> {
        let next_model =
            model.map(|model| model.unwrap_or_else(|| self.config.claude_code_model.clone()));
        if next_model
            .as_deref()
            .is_some_and(|model| !valid_code_model(model))
        {
            return Err(
                "Claude Code model is empty, oversized, or contains control characters".to_string(),
            );
        }
        let next_turns = max_turns.map(|turns| turns.unwrap_or(self.config.claude_code_max_turns));
        if next_turns.is_some_and(|turns| !(1..=MAX_CLAUDE_CODE_TURNS).contains(&turns)) {
            return Err(format!(
                "Claude Code max turns must be between 1 and {MAX_CLAUDE_CODE_TURNS}"
            ));
        }

        if let Some(model) = next_model {
            *self.cc_model.write().await = model;
        }
        if let Some(turns) = next_turns {
            *self.cc_max_turns.write().await = turns;
        }
        // Read into locals to avoid holding RwLock guards across the
        // tracing::info! macro (format_args! temporaries are !Send).
        let current_model = self.cc_model.read().await.clone();
        let current_turns = *self.cc_max_turns.read().await;
        tracing::info!(
            model = %current_model,
            max_turns = current_turns,
            "Claude Code settings updated at runtime"
        );
        Ok(())
    }

    /// Update Codex settings at runtime (called by the settings API).
    ///
    /// The next Codex container spawned will use the updated model.
    pub async fn update_codex_code_settings(&self, model: Option<String>) -> Result<(), String> {
        let next_model = model.unwrap_or_else(|| self.config.codex_code_model.clone());
        if !valid_code_model(&next_model) {
            return Err(
                "Codex model is empty, oversized, or contains control characters".to_string(),
            );
        }
        *self.codex_model.write().await = next_model;
        let current_model = self.codex_model.read().await.clone();
        tracing::info!(
            model = %current_model,
            "Codex settings updated at runtime"
        );
        Ok(())
    }

    fn extend_mode_runtime(
        &self,
        job_id: Uuid,
        mode: JobMode,
        env_vec: &mut Vec<String>,
        mounts: &mut Vec<Mount>,
    ) -> Result<(), OrchestratorError> {
        if mode == JobMode::ClaudeCode {
            if let Some(ref api_key) = self.config.claude_code_api_key {
                env_vec.push(format!("ANTHROPIC_API_KEY={}", api_key));
            } else if let Some(ref oauth_token) = self.config.claude_code_oauth_token {
                env_vec.push(format!("CLAUDE_CODE_OAUTH_TOKEN={}", oauth_token));
            }
            if !self.config.claude_code_allowed_tools.is_empty() {
                env_vec.push(format!(
                    "CLAUDE_CODE_ALLOWED_TOOLS={}",
                    self.config.claude_code_allowed_tools.join(",")
                ));
            }
        }

        if mode == JobMode::CodexCode {
            if let Some(ref api_key) = self.config.codex_code_api_key {
                env_vec.push(format!("OPENAI_API_KEY={}", api_key));
            }
            env_vec.push("CODEX_HOME=/home/sandbox/.codex".to_string());

            if self.config.codex_code_home_dir.exists() {
                mounts.push(bind_mount(
                    job_id,
                    &self.config.codex_code_home_dir,
                    "/home/sandbox/.codex-host",
                    true,
                )?);
            }
        }
        Ok(())
    }

    async fn container_cmd(
        &self,
        job_id: Uuid,
        orchestrator_url: String,
        mode: JobMode,
    ) -> Vec<String> {
        match mode {
            JobMode::Worker => vec![
                "worker".to_string(),
                "--job-id".to_string(),
                job_id.to_string(),
                "--orchestrator-url".to_string(),
                orchestrator_url,
            ],
            JobMode::ClaudeCode => vec![
                "claude-bridge".to_string(),
                "--job-id".to_string(),
                job_id.to_string(),
                "--orchestrator-url".to_string(),
                orchestrator_url,
                "--max-turns".to_string(),
                self.cc_max_turns.read().await.to_string(),
                "--model".to_string(),
                self.cc_model.read().await.clone(),
            ],
            JobMode::CodexCode => vec![
                "codex-bridge".to_string(),
                "--job-id".to_string(),
                job_id.to_string(),
                "--orchestrator-url".to_string(),
                orchestrator_url,
                "--model".to_string(),
                self.codex_model.read().await.clone(),
            ],
        }
    }

    /// Get or create a Docker connection.
    ///
    /// Supports late-binding: if Docker was not running at startup, the first
    /// job request that needs Docker will trigger a fresh connection attempt.
    /// If a cached connection goes stale (Docker restarted), a ping failure
    /// triggers automatic reconnection.
    async fn docker(&self) -> Result<bollard::Docker, OrchestratorError> {
        // Fast path: reuse cached connection if it's still alive.
        {
            let guard = self.docker.read().await;
            if let Some(ref d) = *guard {
                if d.ping().await.is_ok() {
                    return Ok(d.clone());
                }
                // Cached connection is stale — fall through to reconnect.
                tracing::info!("Cached Docker connection stale, reconnecting...");
            }
        }

        // Slow path: connect (or reconnect) to Docker.
        let docker = connect_docker()
            .await
            .map_err(|e| OrchestratorError::Docker {
                reason: e.to_string(),
            })?;
        *self.docker.write().await = Some(docker.clone());
        tracing::info!("Docker connection established");
        Ok(docker)
    }

    /// Start or reuse the authenticated proxy for persistent jobs.
    async fn proxy_endpoint(&self) -> Result<(u16, String), OrchestratorError> {
        let mut proxy_guard = self.proxy.write().await;
        if proxy_guard.is_none() {
            let proxy = NetworkProxyBuilder::new()
                .with_allowlist(self.config.network_allowlist.clone())
                .with_credential_resolver(Arc::new(NoCredentialResolver))
                .with_policy(SandboxPolicy::ReadOnly)
                .build();
            *proxy_guard = Some(proxy);
        }

        let proxy = proxy_guard
            .as_ref()
            .ok_or_else(|| OrchestratorError::Docker {
                reason: "sandbox proxy was not retained".to_string(),
            })?;
        let address = proxy.start(self.config.proxy_port).await.map_err(|error| {
            OrchestratorError::Docker {
                reason: format!("failed to start sandbox network proxy: {error}"),
            }
        })?;
        Ok((address.port(), proxy.proxy_token()))
    }

    /// Create and start a new container for a job.
    ///
    /// The caller provides the `job_id` so it can be persisted to the database
    /// before the container is created. Credential grants are stored in the
    /// TokenStore and served on-demand via the `/credentials` endpoint.
    /// Returns the auth token for the worker.
    pub async fn create_job(
        &self,
        job_id: Uuid,
        mut spec: SandboxJobSpec,
        credential_grants: Vec<CredentialGrant>,
    ) -> Result<String, OrchestratorError> {
        let mode = spec.mode;
        self.validate_runtime_config(job_id, mode).await?;
        self.validate_job_spec(job_id, &spec)?;
        if let Some(scope) = spec.runtime_scope.as_deref()
            && scope != self.config.runtime_scope
        {
            return Err(OrchestratorError::ContainerCreationFailed {
                job_id,
                reason: "sandbox job belongs to a different runtime scope".to_string(),
            });
        }
        self.stamp_job_spec(&mut spec);
        validate_credential_grants(job_id, &credential_grants)?;
        {
            let _admission = self
                .create_admission
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !self.accepting_jobs.load(Ordering::Acquire) {
                return Err(OrchestratorError::InvalidContainerState {
                    job_id,
                    state: "container runtime is shutting down".to_string(),
                });
            }
            self.inflight_creates.fetch_add(1, Ordering::AcqRel);
        }
        let _inflight = InflightCreateGuard { manager: self };

        let docker_container_name = container_name(job_id, mode);
        let project_dir = spec.project_dir.as_ref().map(PathBuf::from);

        // Reserve the job ID before creating credentials or touching Docker.
        // A duplicate ID must never rotate the live worker token, overwrite
        // its handle, or leak a second untracked container.
        {
            let mut containers = self.containers.write().await;
            match containers.entry(job_id) {
                Entry::Occupied(existing) => {
                    return Err(OrchestratorError::InvalidContainerState {
                        job_id,
                        state: format!("job id is already reserved ({})", existing.get().state),
                    });
                }
                Entry::Vacant(entry) => {
                    entry.insert(ContainerHandle {
                        job_id,
                        container_id: String::new(),
                        state: ContainerState::Creating,
                        mode,
                        created_at: Utc::now(),
                        spec,
                        last_worker_status: None,
                        worker_iteration: 0,
                        completion_result: None,
                    });
                }
            }
        }
        let mut cancellation_guard =
            ContainerCreateCancellationGuard::new(self, job_id, docker_container_name.clone());

        if !self.accepting_jobs.load(Ordering::Acquire) {
            self.containers.write().await.remove(&job_id);
            cancellation_guard.disarm();
            return Err(OrchestratorError::InvalidContainerState {
                job_id,
                state: "container runtime began shutting down during creation".to_string(),
            });
        }

        // Generate auth token (stored in TokenStore, never logged)
        let token = self.token_store.create_token(job_id).await;

        // Store credential grants (revoked automatically when the token is revoked)
        self.token_store
            .store_grants(job_id, credential_grants)
            .await;

        if !self.accepting_jobs.load(Ordering::Acquire) {
            self.token_store.revoke(job_id).await;
            self.remove_creating_handle(job_id).await;
            cancellation_guard.disarm();
            return Err(OrchestratorError::InvalidContainerState {
                job_id,
                state: "container runtime began shutting down during creation".to_string(),
            });
        }

        // Run the actual container creation. On any failure, revoke the token
        // and remove the handle so we don't leak resources.
        match self
            .create_job_inner(job_id, &token, project_dir, mode, &docker_container_name)
            .await
        {
            Ok(()) => {
                cancellation_guard.disarm();
                Ok(token)
            }
            Err(e) => {
                self.token_store.revoke(job_id).await;
                // Preserve a terminal handle installed by a concurrent cancel
                // or completion; only discard our still-creating reservation.
                self.remove_creating_handle(job_id).await;
                cancellation_guard.disarm();
                Err(e)
            }
        }
    }

    async fn remove_creating_handle(&self, job_id: Uuid) {
        let mut containers = self.containers.write().await;
        if containers.get(&job_id).is_some_and(|handle| {
            handle.state == ContainerState::Creating && handle.completion_result.is_none()
        }) {
            containers.remove(&job_id);
        }
    }

    async fn creation_is_active(&self, job_id: Uuid) -> bool {
        self.containers
            .read()
            .await
            .get(&job_id)
            .is_some_and(|handle| {
                handle.state == ContainerState::Creating && handle.completion_result.is_none()
            })
    }

    async fn remove_created_container(
        &self,
        docker: &bollard::Docker,
        job_id: Uuid,
        container_id: &str,
        reason: &'static str,
    ) {
        if let Err(error) = docker
            .remove_container(
                container_id,
                Some(
                    bollard::query_parameters::RemoveContainerOptionsBuilder::new()
                        .force(true)
                        .build(),
                ),
            )
            .await
        {
            tracing::warn!(
                %job_id,
                %container_id,
                %error,
                %reason,
                "Failed to remove a container abandoned during creation"
            );
        }
    }

    /// Inner implementation of container creation (separated for cleanup).
    async fn create_job_inner(
        &self,
        job_id: Uuid,
        token: &str,
        project_dir: Option<PathBuf>,
        mode: JobMode,
        container_name: &str,
    ) -> Result<(), OrchestratorError> {
        // Connect to Docker (reuses cached connection)
        let docker = self.docker().await?;
        if !self.accepting_jobs.load(Ordering::Acquire) || !self.creation_is_active(job_id).await {
            return Err(OrchestratorError::InvalidContainerState {
                job_id,
                state: "job was cancelled before Docker container creation".to_string(),
            });
        }

        // Each job gets its own host-gateway-isolated bridge and narrow relay.
        // Environment proxy variables are only client configuration; the
        // network topology is the actual egress and cross-job boundary.
        let (host_proxy_port, proxy_token) = self.proxy_endpoint().await?;
        let mut relay = Some(
            SandboxNetworkRelay::start(
                docker.clone(),
                &self.config.image,
                &self.config.runtime_scope,
                "persistent-relay",
                &[
                    RelayForward {
                        listen_port: RELAY_PROXY_PORT,
                        target_port: host_proxy_port,
                    },
                    RelayForward {
                        listen_port: RELAY_ORCHESTRATOR_PORT,
                        target_port: self.config.orchestrator_port,
                    },
                ],
            )
            .await
            .map_err(|error| OrchestratorError::Docker {
                reason: error.to_string(),
            })?,
        );
        let gateway_host = relay
            .as_ref()
            .map(|relay| relay.gateway_host().to_string())
            .ok_or_else(|| OrchestratorError::Docker {
                reason: "sandbox network relay was not retained".to_string(),
            })?;

        let orchestrator_url = format!("http://{gateway_host}:{RELAY_ORCHESTRATOR_PORT}");
        let proxy_url = format!("http://thinclaw:{proxy_token}@{gateway_host}:{RELAY_PROXY_PORT}");

        let mut env_vec = vec![
            format!("THINCLAW_WORKER_TOKEN={}", token),
            format!("THINCLAW_JOB_ID={}", job_id),
            format!("THINCLAW_ORCHESTRATOR_URL={}", orchestrator_url),
            format!("HTTP_PROXY={proxy_url}"),
            format!("HTTPS_PROXY={proxy_url}"),
            format!("http_proxy={proxy_url}"),
            format!("https_proxy={proxy_url}"),
            format!("NO_PROXY={gateway_host},localhost,127.0.0.1"),
            format!("no_proxy={gateway_host},localhost,127.0.0.1"),
        ];

        // Build volume mounts (validate project_dir stays within ~/.thinclaw/projects/)
        let mut mounts = Vec::new();
        let mut has_workspace_mount = false;
        if let Some(ref dir) = project_dir {
            let projects_base = crate::platform::resolve_data_dir("projects");
            let canonical = validate_bind_mount_path(dir, &projects_base, job_id)?;
            mounts.push(bind_mount(job_id, &canonical, "/workspace", false)?);
            has_workspace_mount = true;
            env_vec.push("THINCLAW_WORKSPACE=/workspace".to_string());
        }

        self.extend_mode_runtime(job_id, mode, &mut env_vec, &mut mounts)?;

        // Memory limit: container coding agents get more memory
        let memory_mb = match mode {
            JobMode::ClaudeCode => self.config.claude_code_memory_limit_mb,
            JobMode::CodexCode => self.config.codex_code_memory_limit_mb,
            JobMode::Worker => self.config.memory_limit_mb,
        };
        let memory_bytes = memory_mb
            .checked_mul(1024 * 1024)
            .and_then(|value| i64::try_from(value).ok())
            .ok_or_else(|| OrchestratorError::ContainerCreationFailed {
                job_id,
                reason: format!("memory limit {memory_mb} MiB exceeds Docker's signed limit"),
            })?;

        // Create the container
        use bollard::models::{ContainerCreateBody, HostConfig, ResourcesUlimits};
        use bollard::query_parameters::CreateContainerOptionsBuilder;

        let host_config = HostConfig {
            mounts: if mounts.is_empty() {
                None
            } else {
                Some(mounts)
            },
            memory: Some(memory_bytes),
            memory_swap: Some(memory_bytes),
            memory_swappiness: Some(0),
            cpu_shares: Some(self.config.cpu_shares as i64),
            nano_cpus: Some(docker_nano_cpus(self.config.cpu_shares)),
            pids_limit: Some(512),
            ulimits: Some(vec![ResourcesUlimits {
                name: Some("nofile".to_string()),
                soft: Some(4096),
                hard: Some(4096),
            }]),
            network_mode: Some(
                relay
                    .as_ref()
                    .map(|relay| relay.network_name().to_string())
                    .ok_or_else(|| OrchestratorError::Docker {
                        reason: "sandbox execution network was not retained".to_string(),
                    })?,
            ),
            cap_drop: Some(vec!["ALL".to_string()]),
            security_opt: Some(vec!["no-new-privileges:true".to_string()]),
            readonly_rootfs: Some(true),
            tmpfs: Some(
                [
                    (
                        "/tmp".to_string(),
                        "rw,nosuid,nodev,size=512M,mode=1777".to_string(),
                    ),
                    (
                        "/home/sandbox/.claude".to_string(),
                        "rw,nosuid,nodev,size=256M,uid=1000,gid=1000,mode=700".to_string(),
                    ),
                    (
                        "/home/sandbox/.codex".to_string(),
                        "rw,nosuid,nodev,size=256M,uid=1000,gid=1000,mode=700".to_string(),
                    ),
                    (
                        "/home/sandbox/.cache".to_string(),
                        "rw,nosuid,nodev,size=512M,uid=1000,gid=1000,mode=700".to_string(),
                    ),
                    (
                        "/home/sandbox/.config".to_string(),
                        "rw,nosuid,nodev,size=64M,uid=1000,gid=1000,mode=700".to_string(),
                    ),
                    (
                        "/home/sandbox/.local".to_string(),
                        "rw,nosuid,nodev,size=256M,uid=1000,gid=1000,mode=700".to_string(),
                    ),
                ]
                .into_iter()
                .chain((!has_workspace_mount).then(|| {
                    (
                        "/workspace".to_string(),
                        "rw,nosuid,nodev,size=1G,uid=1000,gid=1000,mode=700".to_string(),
                    )
                }))
                .collect(),
            ),
            ..Default::default()
        };

        // Build CMD based on mode
        let cmd = self.container_cmd(job_id, orchestrator_url, mode).await;

        let container_config = ContainerCreateBody {
            image: Some(self.config.image.clone()),
            cmd: Some(cmd),
            env: Some(env_vec),
            host_config: Some(host_config),
            user: Some("1000:1000".to_string()),
            working_dir: Some("/workspace".to_string()),
            labels: Some(managed_container_labels(
                &self.config.runtime_scope,
                "persistent",
                Some(job_id),
            )),
            ..Default::default()
        };

        let options = CreateContainerOptionsBuilder::new()
            .name(container_name)
            .build();

        let response = docker
            .create_container(Some(options), container_config)
            .await
            .map_err(|e| OrchestratorError::ContainerCreationFailed {
                job_id,
                reason: e.to_string(),
            })?;

        let container_id = response.id;

        if !self.accepting_jobs.load(Ordering::Acquire) || !self.creation_is_active(job_id).await {
            self.remove_created_container(
                &docker,
                job_id,
                &container_id,
                "creation was cancelled before container start",
            )
            .await;
            return Err(OrchestratorError::InvalidContainerState {
                job_id,
                state: "job was cancelled while its container was being created".to_string(),
            });
        }

        // Start the container
        if let Err(error) = docker.start_container(&container_id, None).await {
            if let Err(cleanup_error) = docker
                .remove_container(
                    &container_id,
                    Some(
                        bollard::query_parameters::RemoveContainerOptionsBuilder::new()
                            .force(true)
                            .build(),
                    ),
                )
                .await
            {
                tracing::warn!(
                    %job_id,
                    %container_id,
                    %cleanup_error,
                    "Failed to remove container after start failure"
                );
            }
            return Err(OrchestratorError::ContainerCreationFailed {
                job_id,
                reason: format!("failed to start container: {error}"),
            });
        }

        if !self.accepting_jobs.load(Ordering::Acquire) || !self.creation_is_active(job_id).await {
            self.remove_created_container(
                &docker,
                job_id,
                &container_id,
                "creation was cancelled after container start",
            )
            .await;
            return Err(OrchestratorError::InvalidContainerState {
                job_id,
                state: "job was cancelled while its container was starting".to_string(),
            });
        }

        // Publish the running state only if this exact reservation is still
        // live. A concurrent terminal transition wins and forces cleanup.
        let published = {
            let mut relays = self.relays.write().await;
            let mut containers = self.containers.write().await;
            if let Some(handle) = containers.get_mut(&job_id)
                && handle.state == ContainerState::Creating
                && handle.completion_result.is_none()
                && self.accepting_jobs.load(Ordering::Acquire)
                && relay.is_some()
            {
                handle.container_id = container_id.clone();
                handle.state = ContainerState::Running;
                if let Some(relay) = relay.take() {
                    relays.insert(job_id, relay);
                }
                true
            } else {
                false
            }
        };
        if !published {
            self.remove_created_container(
                &docker,
                job_id,
                &container_id,
                "terminal state won the container publication race",
            )
            .await;
            return Err(OrchestratorError::InvalidContainerState {
                job_id,
                state: "job reached a terminal state during container creation".to_string(),
            });
        }

        tracing::info!(
            job_id = %job_id,
            mode = %mode,
            "Created and started container job"
        );

        Ok(())
    }

    async fn cleanup_job_relay(&self, job_id: Uuid) {
        if let Some(mut relay) = self.relays.write().await.remove(&job_id) {
            relay.stop().await;
        }
    }

    /// Stop a running container job.
    pub async fn stop_job(&self, job_id: Uuid) -> Result<(), OrchestratorError> {
        let container_id = {
            let containers = self.containers.read().await;
            containers
                .get(&job_id)
                .map(|h| h.container_id.clone())
                .ok_or(OrchestratorError::ContainerNotFound { job_id })?
        };

        // Closing a job while Docker creation is still in flight is a valid
        // cancellation. Revocation and the state transition make every later
        // creation checkpoint fail closed; the creator removes any container
        // that Docker happens to return afterward.
        self.token_store.revoke(job_id).await;
        if container_id.is_empty() {
            if let Some(handle) = self.containers.write().await.get_mut(&job_id) {
                handle.state = ContainerState::Stopped;
                if handle.completion_result.is_none() {
                    handle.completion_result = Some(CompletionResult {
                        status: "cancelled".to_string(),
                        session_id: None,
                        success: false,
                        message: Some("Container job stopped".to_string()),
                        iterations: handle.worker_iteration,
                    });
                }
            }
            tracing::info!(job_id = %job_id, "Cancelled container job during creation");
            return Ok(());
        }

        // Revoke worker access before any potentially slow Docker operation.
        // Even if daemon cleanup times out, the container can no longer call
        // privileged orchestrator endpoints with its job token.
        match self.docker().await {
            Ok(docker) => {
                // Stop the container (10 second grace period)
                if let Err(e) = docker
                    .stop_container(
                        &container_id,
                        Some(
                            bollard::query_parameters::StopContainerOptionsBuilder::new()
                                .t(10)
                                .build(),
                        ),
                    )
                    .await
                {
                    tracing::warn!(job_id = %job_id, error = %e, "Failed to stop container (may already be stopped)");
                }

                if let Err(e) = docker
                    .remove_container(
                        &container_id,
                        Some(
                            bollard::query_parameters::RemoveContainerOptionsBuilder::new()
                                .force(true)
                                .build(),
                        ),
                    )
                    .await
                {
                    tracing::warn!(job_id = %job_id, error = %e, "Failed to remove container (may require startup cleanup)");
                }
            }
            Err(error) => tracing::warn!(
                %job_id,
                %error,
                "Docker unavailable while stopping job; startup cleanup will retry"
            ),
        }
        self.cleanup_job_relay(job_id).await;

        // Update state
        if let Some(handle) = self.containers.write().await.get_mut(&job_id) {
            handle.state = ContainerState::Stopped;
            if handle.completion_result.is_none() {
                handle.completion_result = Some(CompletionResult {
                    status: "cancelled".to_string(),
                    session_id: None,
                    success: false,
                    message: Some("Container job stopped".to_string()),
                    iterations: handle.worker_iteration,
                });
            }
        }

        tracing::info!(job_id = %job_id, "Stopped container job");

        Ok(())
    }

    /// Mark a job as complete with a result. The container is stopped but the
    /// handle is kept so `CreateJobTool` can read the completion message.
    pub async fn claim_job_completion(
        &self,
        job_id: Uuid,
        mut result: CompletionResult,
    ) -> Result<ContainerCompletionClaim, OrchestratorError> {
        result.status = thinclaw_types::sandbox::normalize_terminal_sandbox_status(
            &result.status,
            result.success,
        );
        result.success = result.status == "completed";
        // First terminal result wins. Later finalizers may retry Docker
        // cleanup, but cannot rewrite the user-visible completion outcome.
        let (first_completion, canonical_result, container_id) = {
            let mut containers = self.containers.write().await;
            let handle = containers
                .get_mut(&job_id)
                .ok_or(OrchestratorError::ContainerNotFound { job_id })?;
            let first_completion = handle.completion_result.is_none();
            if first_completion {
                handle.completion_result = Some(result);
            }
            handle.state = ContainerState::Stopped;
            (
                first_completion,
                handle.completion_result.clone().ok_or(
                    OrchestratorError::InvalidContainerState {
                        job_id,
                        state: "terminal result was not retained".to_string(),
                    },
                )?,
                handle.container_id.clone(),
            )
        };

        // Revoke worker access before returning the claim. Durable persistence
        // and Docker cleanup may be slow, but a terminal worker immediately
        // loses access to privileged orchestrator endpoints.
        self.token_store.revoke(job_id).await;

        Ok(ContainerCompletionClaim {
            first_completion,
            result: canonical_result,
            container_id,
        })
    }

    /// Best-effort idempotent Docker cleanup after a completion has been
    /// claimed. The terminal handle/result remains queryable even if Docker is
    /// unavailable; startup orphan cleanup provides a second recovery layer.
    pub async fn cleanup_completed_job_container(&self, job_id: Uuid, container_id: &str) {
        if !container_id.is_empty() {
            match self.docker().await {
                Ok(docker) => {
                    if let Err(e) = docker
                        .stop_container(
                            container_id,
                            Some(
                                bollard::query_parameters::StopContainerOptionsBuilder::new()
                                    .t(5)
                                    .build(),
                            ),
                        )
                        .await
                    {
                        tracing::warn!(job_id = %job_id, error = %e, "Failed to stop completed container");
                    }
                    if let Err(e) = docker
                        .remove_container(
                            container_id,
                            Some(
                                bollard::query_parameters::RemoveContainerOptionsBuilder::new()
                                    .force(true)
                                    .build(),
                            ),
                        )
                        .await
                    {
                        tracing::warn!(job_id = %job_id, error = %e, "Failed to remove completed container");
                    }
                }
                Err(e) => {
                    tracing::warn!(job_id = %job_id, error = %e, "Failed to connect to Docker for container cleanup");
                }
            }
        }
        self.cleanup_job_relay(job_id).await;
    }

    /// Claim completion, revoke access, and clean up Docker in one call for
    /// callers that do not need to coordinate durable result persistence.
    pub async fn complete_job(
        &self,
        job_id: Uuid,
        result: CompletionResult,
    ) -> Result<(), OrchestratorError> {
        let claim = self.claim_job_completion(job_id, result).await?;
        self.cleanup_completed_job_container(job_id, &claim.container_id)
            .await;
        tracing::info!(job_id = %job_id, "Completed container job");
        Ok(())
    }

    /// Remove a completed job handle from memory (called after result is read).
    pub async fn cleanup_job(&self, job_id: Uuid) {
        self.containers.write().await.remove(&job_id);
        self.cleanup_job_relay(job_id).await;
    }

    /// Update the worker-reported status for a job.
    pub async fn update_worker_status(
        &self,
        job_id: Uuid,
        message: Option<String>,
        iteration: u32,
    ) {
        if let Some(handle) = self.containers.write().await.get_mut(&job_id) {
            handle.last_worker_status = message;
            handle.worker_iteration = iteration;
        }
    }

    /// Get the handle for a job.
    pub async fn get_handle(&self, job_id: Uuid) -> Option<ContainerHandle> {
        self.containers.read().await.get(&job_id).cloned()
    }

    /// List all active container jobs.
    pub async fn list_jobs(&self) -> Vec<ContainerHandle> {
        self.containers.read().await.values().cloned().collect()
    }

    /// Close job admission, wait for in-flight creates to observe shutdown,
    /// revoke every worker token, and stop/remove all tracked containers.
    pub async fn shutdown_all(&self) {
        const CREATE_DRAIN_TIMEOUT: Duration = Duration::from_secs(45);
        const FINALIZATION_DRAIN_TIMEOUT: Duration = Duration::from_secs(90);
        const CONTAINER_STOP_TIMEOUT: Duration = Duration::from_secs(45);

        {
            let _admission = self
                .create_admission
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            self.accepting_jobs.store(false, Ordering::Release);
        }

        let mut finalization_tasks = {
            let mut tasks = self
                .finalization_tasks
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            self.accepting_finalizations.store(false, Ordering::Release);
            std::mem::take(&mut *tasks)
        };

        let drain_creates = async {
            loop {
                let notified = self.inflight_create_notify.notified();
                if self.inflight_creates.load(Ordering::Acquire) == 0 {
                    break;
                }
                notified.await;
            }
        };
        if tokio::time::timeout(CREATE_DRAIN_TIMEOUT, drain_creates)
            .await
            .is_err()
        {
            tracing::warn!(
                remaining = self.inflight_creates.load(Ordering::Acquire),
                timeout_secs = CREATE_DRAIN_TIMEOUT.as_secs(),
                "Timed out draining in-flight sandbox container creates"
            );
        }

        let drain_finalizations = async {
            while let Some(result) = finalization_tasks.join_next().await {
                if let Err(error) = result
                    && !error.is_cancelled()
                {
                    tracing::warn!(%error, "Sandbox finalization task failed during shutdown");
                }
            }
        };
        if tokio::time::timeout(FINALIZATION_DRAIN_TIMEOUT, drain_finalizations)
            .await
            .is_err()
        {
            finalization_tasks.abort_all();
            while finalization_tasks.join_next().await.is_some() {}
            tracing::warn!(
                timeout_secs = FINALIZATION_DRAIN_TIMEOUT.as_secs(),
                "Timed out draining sandbox finalization tasks"
            );
        }

        let jobs = self.list_jobs().await;
        join_all(jobs.into_iter().map(|handle| async move {
            let job_id = handle.job_id;
            self.token_store.revoke(job_id).await;
            let result = CompletionResult {
                status: "cancelled".to_string(),
                session_id: None,
                success: false,
                message: Some("Runtime shutdown".to_string()),
                iterations: handle.worker_iteration,
            };
            match tokio::time::timeout(CONTAINER_STOP_TIMEOUT, self.complete_job(job_id, result))
                .await
            {
                Ok(Ok(())) => {}
                Ok(Err(error)) => tracing::warn!(
                    %job_id,
                    %error,
                    "Failed to stop sandbox container during shutdown"
                ),
                Err(_) => tracing::warn!(
                    %job_id,
                    timeout_secs = CONTAINER_STOP_TIMEOUT.as_secs(),
                    "Timed out stopping sandbox container during shutdown"
                ),
            }
            self.cleanup_job(job_id).await;
        }))
        .await;

        let relays = std::mem::take(&mut *self.relays.write().await);
        join_all(relays.into_values().map(|mut relay| async move {
            relay.stop().await;
        }))
        .await;
        if let Some(proxy) = self.proxy.write().await.take() {
            proxy.stop().await;
        }
    }

    /// Get a reference to the token store.
    pub fn token_store(&self) -> &TokenStore {
        &self.token_store
    }

    /// Clean up orphan containers from a previous process crash.
    ///
    /// Removes only explicitly managed containers whose stable state-directory
    /// scope matches this runtime. Name prefixes are intentionally insufficient:
    /// another ThinClaw installation may be using the same Docker daemon.
    pub async fn cleanup_orphan_containers(&self) {
        let docker = match self.docker().await {
            Ok(d) => d,
            Err(e) => {
                tracing::debug!("Skipping orphan container cleanup (no Docker): {}", e);
                return;
            }
        };

        use bollard::query_parameters::{
            ListContainersOptionsBuilder, RemoveContainerOptionsBuilder,
        };
        use std::collections::HashMap;

        // Ask Docker for managed containers, then enforce the stable scope
        // again client-side before deleting anything.
        let mut filters = HashMap::new();
        filters.insert(
            "label".to_string(),
            vec![format!("{MANAGED_CONTAINER_LABEL}=v1")],
        );
        let all_containers = match docker
            .list_containers(Some(
                ListContainersOptionsBuilder::new()
                    .all(true)
                    .filters(&filters)
                    .build(),
            ))
            .await
        {
            Ok(containers) => containers,
            Err(error) => {
                tracing::warn!(%error, "Failed to list managed sandbox containers for cleanup");
                return;
            }
        };

        let mut known_ids: std::collections::HashSet<_> = self
            .containers
            .read()
            .await
            .values()
            .map(|h| h.container_id.clone())
            .collect();
        known_ids.extend(
            self.relays
                .read()
                .await
                .values()
                .map(|relay| relay.container_id().to_string()),
        );

        let mut removed = 0u32;
        for container in &all_containers {
            if container
                .labels
                .as_ref()
                .and_then(|labels| labels.get(CONTAINER_SCOPE_LABEL))
                .map(String::as_str)
                != Some(self.config.runtime_scope.as_str())
            {
                continue;
            }
            let cid = match &container.id {
                Some(id) => id.clone(),
                None => continue,
            };

            if known_ids.contains(&cid) {
                continue;
            }

            let name = container
                .names
                .as_ref()
                .and_then(|n| n.first())
                .map(|n| n.trim_start_matches('/'))
                .unwrap_or("unknown");

            // Force remove (stops if running)
            match docker
                .remove_container(
                    &cid,
                    Some(RemoveContainerOptionsBuilder::new().force(true).build()),
                )
                .await
            {
                Ok(()) => {
                    removed += 1;
                    tracing::info!(
                        container = name,
                        "Removed orphan container from previous session"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        container = name,
                        error = %e,
                        "Failed to remove orphan container"
                    );
                }
            }
        }

        if removed > 0 {
            tracing::info!("Cleaned up {} orphan container(s)", removed);
        }
        match cleanup_sandbox_networks(&docker, &self.config.runtime_scope).await {
            Ok(networks) if networks > 0 => {
                tracing::info!(networks, "Cleaned up stale sandbox network(s)");
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "Failed to clean stale sandbox networks"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_container_job_config_default() {
        let config = ContainerJobConfig::default();
        assert_eq!(config.orchestrator_port, 50051);
        assert_eq!(config.memory_limit_mb, 2048);
    }

    #[test]
    fn sandbox_job_spec_validation_bounds_persisted_and_worker_input() {
        let manager = ContainerJobManager::new(ContainerJobConfig::default(), TokenStore::new());
        let job_id = Uuid::new_v4();
        let mut spec = SandboxJobSpec::new(
            "bounded job",
            "do the work",
            "principal",
            "actor",
            None,
            JobMode::Worker,
        );
        assert!(manager.validate_job_spec(job_id, &spec).is_ok());

        spec.idle_timeout_secs = u64::MAX;
        assert!(manager.validate_job_spec(job_id, &spec).is_err());
        spec.idle_timeout_secs = crate::sandbox_jobs::DEFAULT_SANDBOX_IDLE_TIMEOUT_SECS;
        spec.description = "x".repeat(crate::sandbox_jobs::MAX_JOB_DESCRIPTION_BYTES + 1);
        assert!(manager.validate_job_spec(job_id, &spec).is_err());
        spec.description = "valid".to_string();
        spec.metadata = serde_json::json!({
            "blob": "x".repeat(crate::sandbox_jobs::MAX_JOB_METADATA_BYTES)
        });
        assert!(manager.validate_job_spec(job_id, &spec).is_err());
    }

    #[test]
    fn test_container_state_display() {
        assert_eq!(ContainerState::Running.to_string(), "running");
        assert_eq!(ContainerState::Stopped.to_string(), "stopped");
    }

    #[test]
    fn credential_grants_reject_runtime_redirection_and_duplicates() {
        let job_id = Uuid::new_v4();
        let reserved = validate_credential_grants(
            job_id,
            &[CredentialGrant {
                secret_name: "token".to_string(),
                env_var: "HTTP_PROXY".to_string(),
            }],
        );
        assert!(reserved.is_err());

        let duplicate = validate_credential_grants(
            job_id,
            &[
                CredentialGrant {
                    secret_name: "one".to_string(),
                    env_var: "GITHUB_TOKEN".to_string(),
                },
                CredentialGrant {
                    secret_name: "two".to_string(),
                    env_var: "GITHUB_TOKEN".to_string(),
                },
            ],
        );
        assert!(duplicate.is_err());
    }

    #[test]
    fn test_validate_bind_mount_valid_path() {
        let root = tempfile::tempdir().unwrap();
        let base = root.path().join("projects");
        std::fs::create_dir_all(&base).unwrap();

        let test_dir = base.join("test_validate_bind");
        std::fs::create_dir_all(&test_dir).unwrap();

        let result = validate_bind_mount_path(&test_dir, &base, Uuid::new_v4());
        assert!(result.is_ok());
        let canonical = result.unwrap();
        assert!(canonical.starts_with(base.canonicalize().unwrap()));
    }

    #[test]
    fn test_validate_bind_mount_rejects_outside_base() {
        let root = tempfile::tempdir().unwrap();
        let base = root.path().join("projects");
        let tmp = tempfile::tempdir().unwrap();
        let outside = tmp.path().to_path_buf();

        let result = validate_bind_mount_path(&outside, &base, Uuid::new_v4());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("outside allowed base"),
            "expected 'outside allowed base', got: {}",
            err
        );
    }

    #[test]
    fn test_validate_bind_mount_rejects_nonexistent() {
        let root = tempfile::tempdir().unwrap();
        let base = root.path().join("projects");
        let nonexistent = PathBuf::from("/no/such/path/at/all");
        let result = validate_bind_mount_path(&nonexistent, &base, Uuid::new_v4());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("canonicalize"),
            "expected canonicalize error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_update_worker_status() {
        let store = TokenStore::new();
        let mgr = ContainerJobManager::new(ContainerJobConfig::default(), store);
        let job_id = Uuid::new_v4();

        // Insert a handle
        {
            let mut containers = mgr.containers.write().await;
            containers.insert(
                job_id,
                ContainerHandle {
                    job_id,
                    container_id: "test".to_string(),
                    state: ContainerState::Running,
                    mode: JobMode::Worker,
                    created_at: chrono::Utc::now(),
                    spec: SandboxJobSpec::new(
                        "test job",
                        "test job",
                        "default",
                        "default",
                        None,
                        JobMode::Worker,
                    ),
                    last_worker_status: None,
                    worker_iteration: 0,
                    completion_result: None,
                },
            );
        }

        mgr.update_worker_status(job_id, Some("Iteration 3".to_string()), 3)
            .await;

        let handle = mgr.get_handle(job_id).await.unwrap();
        assert_eq!(handle.worker_iteration, 3);
        assert_eq!(handle.last_worker_status.as_deref(), Some("Iteration 3"));
    }

    #[tokio::test]
    async fn finalization_survives_calling_request_cancellation_and_is_drained() {
        let manager = Arc::new(ContainerJobManager::new(
            ContainerJobConfig::default(),
            TokenStore::new(),
        ));
        let completed = Arc::new(AtomicBool::new(false));
        let (started_tx, started_rx) = oneshot::channel();
        let caller_manager = Arc::clone(&manager);
        let caller_completed = Arc::clone(&completed);
        let caller = tokio::spawn(async move {
            caller_manager
                .run_owned_finalization(async move {
                    let _ = started_tx.send(());
                    tokio::time::sleep(Duration::from_millis(25)).await;
                    caller_completed.store(true, Ordering::Release);
                    Ok(())
                })
                .await
        });

        started_rx.await.expect("finalization should start");
        caller.abort();
        let _ = caller.await;
        manager.shutdown_all().await;

        assert!(completed.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn complete_job_revokes_worker_token_before_cleanup() {
        let token_store = TokenStore::new();
        let mgr = ContainerJobManager::new(ContainerJobConfig::default(), token_store.clone());
        let job_id = Uuid::new_v4();
        let token = token_store.create_token(job_id).await;
        mgr.containers.write().await.insert(
            job_id,
            ContainerHandle {
                job_id,
                container_id: String::new(),
                state: ContainerState::Running,
                mode: JobMode::Worker,
                created_at: Utc::now(),
                spec: SandboxJobSpec::new(
                    "test job",
                    "test job",
                    "default",
                    "default",
                    None,
                    JobMode::Worker,
                ),
                last_worker_status: None,
                worker_iteration: 0,
                completion_result: None,
            },
        );

        mgr.complete_job(
            job_id,
            CompletionResult {
                status: "completed".to_string(),
                session_id: None,
                success: true,
                message: None,
                iterations: 0,
            },
        )
        .await
        .expect("completion should succeed without a container id");

        assert!(!token_store.validate(job_id, &token).await);
    }

    #[tokio::test]
    async fn shutdown_closes_container_job_admission() {
        let mgr = ContainerJobManager::new(ContainerJobConfig::default(), TokenStore::new());
        mgr.shutdown_all().await;

        let job_id = Uuid::new_v4();
        let error = mgr
            .create_job(
                job_id,
                SandboxJobSpec::new(
                    "late job",
                    "late job",
                    "default",
                    "default",
                    None,
                    JobMode::Worker,
                ),
                Vec::new(),
            )
            .await
            .expect_err("shutdown must reject late container jobs");
        assert!(matches!(
            error,
            OrchestratorError::InvalidContainerState { .. }
        ));
    }

    #[tokio::test]
    async fn duplicate_job_id_cannot_rotate_token_or_overwrite_handle() {
        let token_store = TokenStore::new();
        let mgr = ContainerJobManager::new(ContainerJobConfig::default(), token_store.clone());
        let job_id = Uuid::new_v4();
        let original_token = token_store.create_token(job_id).await;
        mgr.containers.write().await.insert(
            job_id,
            ContainerHandle {
                job_id,
                container_id: "existing-container".to_string(),
                state: ContainerState::Running,
                mode: JobMode::Worker,
                created_at: Utc::now(),
                spec: SandboxJobSpec::new(
                    "existing job",
                    "existing job",
                    "default",
                    "default",
                    None,
                    JobMode::Worker,
                ),
                last_worker_status: None,
                worker_iteration: 0,
                completion_result: None,
            },
        );

        let error = mgr
            .create_job(
                job_id,
                SandboxJobSpec::new(
                    "duplicate job",
                    "duplicate job",
                    "default",
                    "default",
                    None,
                    JobMode::Worker,
                ),
                Vec::new(),
            )
            .await
            .expect_err("a duplicate job id must fail before touching Docker");

        assert!(matches!(
            error,
            OrchestratorError::InvalidContainerState { .. }
        ));
        assert!(token_store.validate(job_id, &original_token).await);
        assert_eq!(
            mgr.get_handle(job_id).await.unwrap().container_id,
            "existing-container"
        );
    }

    #[test]
    fn test_extend_mode_runtime_adds_codex_env_and_mount() {
        let codex_home = tempfile::tempdir().unwrap();
        let config = ContainerJobConfig {
            codex_code_enabled: true,
            codex_code_api_key: Some("sk-test".to_string()),
            codex_code_home_dir: codex_home.path().to_path_buf(),
            ..ContainerJobConfig::default()
        };
        let mgr = ContainerJobManager::new(config, TokenStore::new());
        let mut env_vec = Vec::new();
        let mut mounts = Vec::new();

        mgr.extend_mode_runtime(
            Uuid::new_v4(),
            JobMode::CodexCode,
            &mut env_vec,
            &mut mounts,
        )
        .unwrap();

        assert!(
            env_vec
                .iter()
                .any(|entry| entry == "OPENAI_API_KEY=sk-test")
        );
        assert!(
            env_vec
                .iter()
                .any(|entry| entry == "CODEX_HOME=/home/sandbox/.codex")
        );
        assert_eq!(mounts.len(), 1);
        assert_eq!(
            mounts[0].target.as_deref(),
            Some("/home/sandbox/.codex-host")
        );
        assert_eq!(mounts[0].source.as_deref(), codex_home.path().to_str());
        assert_eq!(mounts[0].read_only, Some(true));
    }

    #[tokio::test]
    async fn test_codex_container_command_uses_cached_model_and_resets_to_default() {
        let job_id = Uuid::new_v4();
        let config = ContainerJobConfig {
            codex_code_enabled: true,
            codex_code_model: "gpt-5.3-codex".to_string(),
            ..ContainerJobConfig::default()
        };
        let mgr = ContainerJobManager::new(config, TokenStore::new());

        mgr.update_codex_code_settings(Some("gpt-5.4".to_string()))
            .await
            .unwrap();
        let updated = mgr
            .container_cmd(
                job_id,
                "http://orchestrator".to_string(),
                JobMode::CodexCode,
            )
            .await;
        assert_eq!(
            updated,
            vec![
                "codex-bridge".to_string(),
                "--job-id".to_string(),
                job_id.to_string(),
                "--orchestrator-url".to_string(),
                "http://orchestrator".to_string(),
                "--model".to_string(),
                "gpt-5.4".to_string(),
            ]
        );

        mgr.update_codex_code_settings(None).await.unwrap();
        let reset = mgr
            .container_cmd(
                job_id,
                "http://orchestrator".to_string(),
                JobMode::CodexCode,
            )
            .await;
        assert_eq!(reset.last().map(String::as_str), Some("gpt-5.3-codex"));
    }

    #[tokio::test]
    async fn runtime_code_settings_reject_invalid_values_and_support_resets() {
        let config = ContainerJobConfig {
            claude_code_model: "claude-default".to_string(),
            claude_code_max_turns: 50,
            codex_code_model: "codex-default".to_string(),
            ..ContainerJobConfig::default()
        };
        let mgr = ContainerJobManager::new(config, TokenStore::new());

        assert!(
            mgr.update_claude_code_settings(Some(Some("\n".to_string())), None)
                .await
                .is_err()
        );
        assert!(
            mgr.update_claude_code_settings(None, Some(Some(0)))
                .await
                .is_err()
        );
        assert!(
            mgr.update_codex_code_settings(Some(String::new()))
                .await
                .is_err()
        );

        mgr.update_claude_code_settings(Some(Some("claude-updated".to_string())), Some(Some(12)))
            .await
            .unwrap();
        mgr.update_claude_code_settings(Some(None), Some(None))
            .await
            .unwrap();
        let command = mgr
            .container_cmd(
                Uuid::new_v4(),
                "http://orchestrator".to_string(),
                JobMode::ClaudeCode,
            )
            .await;
        assert_eq!(
            command
                .windows(2)
                .find(|pair| pair[0] == "--model")
                .map(|pair| pair[1].as_str()),
            Some("claude-default")
        );
        assert_eq!(
            command
                .windows(2)
                .find(|pair| pair[0] == "--max-turns")
                .map(|pair| pair[1].as_str()),
            Some("50")
        );
    }
}
