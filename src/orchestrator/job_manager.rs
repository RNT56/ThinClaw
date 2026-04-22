//! Container lifecycle management for sandboxed jobs.
//!
//! Extends the existing `SandboxManager` infrastructure to support persistent
//! containers with their own agent loops (as opposed to ephemeral per-command containers).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::error::OrchestratorError;
use crate::orchestrator::auth::{CredentialGrant, TokenStore};
use crate::sandbox::connect_docker;
use crate::sandbox_jobs::SandboxJobSpec;

/// Which mode a sandbox container runs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobMode {
    /// Standard ThinClaw worker with proxied LLM calls.
    Worker,
    /// Claude Code bridge that spawns the `claude` CLI directly.
    ClaudeCode,
    /// Codex bridge that spawns the `codex` CLI directly.
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

impl std::fmt::Display for JobMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Configuration for the container job manager.
#[derive(Debug, Clone)]
pub struct ContainerJobConfig {
    /// Docker image for worker containers.
    pub image: String,
    /// Default memory limit in MB.
    pub memory_limit_mb: u64,
    /// Default CPU shares.
    pub cpu_shares: u32,
    /// Port the orchestrator internal API listens on.
    pub orchestrator_port: u16,
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

impl Default for ContainerJobConfig {
    fn default() -> Self {
        Self {
            image: "thinclaw-worker:latest".to_string(),
            memory_limit_mb: 2048,
            cpu_shares: 1024,
            orchestrator_port: 50051,
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

/// Validate that a project directory is under `~/.thinclaw/projects/`.
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

    let projects_base = crate::platform::resolve_data_dir("projects");

    // Ensure the base exists so canonicalize always succeeds.
    std::fs::create_dir_all(&projects_base).map_err(|e| {
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
    /// Runtime-override for Claude Code model (updated via settings API).
    cc_model: RwLock<String>,
    /// Runtime-override for Claude Code max turns.
    cc_max_turns: RwLock<u32>,
    /// Runtime-override for Codex model.
    codex_model: RwLock<String>,
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
            cc_model: RwLock::new(model),
            cc_max_turns: RwLock::new(turns),
            codex_model: RwLock::new(codex_model),
        }
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

    /// Update Claude Code settings at runtime (called by the settings API).
    ///
    /// The next Claude Code container spawned will use the updated values.
    /// Already-running containers keep their current settings.
    pub async fn update_claude_code_settings(&self, model: Option<String>, max_turns: Option<u32>) {
        if let Some(m) = model {
            *self.cc_model.write().await = m;
        }
        if let Some(t) = max_turns {
            *self.cc_max_turns.write().await = t;
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
    }

    /// Update Codex settings at runtime (called by the settings API).
    ///
    /// The next Codex container spawned will use the updated model.
    pub async fn update_codex_code_settings(&self, model: Option<String>) {
        let next_model = model.unwrap_or_else(|| self.config.codex_code_model.clone());
        *self.codex_model.write().await = next_model;
        let current_model = self.codex_model.read().await.clone();
        tracing::info!(
            model = %current_model,
            "Codex settings updated at runtime"
        );
    }

    fn extend_mode_runtime(
        &self,
        mode: JobMode,
        env_vec: &mut Vec<String>,
        binds: &mut Vec<String>,
    ) {
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
                binds.push(format!(
                    "{}:/home/sandbox/.codex-host:ro",
                    self.config.codex_code_home_dir.display()
                ));
            }
        }
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

    /// Create and start a new container for a job.
    ///
    /// The caller provides the `job_id` so it can be persisted to the database
    /// before the container is created. Credential grants are stored in the
    /// TokenStore and served on-demand via the `/credentials` endpoint.
    /// Returns the auth token for the worker.
    pub async fn create_job(
        &self,
        job_id: Uuid,
        spec: SandboxJobSpec,
        credential_grants: Vec<CredentialGrant>,
    ) -> Result<String, OrchestratorError> {
        let mode = spec.mode;
        let project_dir = spec.project_dir.as_ref().map(PathBuf::from);
        // Generate auth token (stored in TokenStore, never logged)
        let token = self.token_store.create_token(job_id).await;

        // Store credential grants (revoked automatically when the token is revoked)
        self.token_store
            .store_grants(job_id, credential_grants)
            .await;

        // Record the handle
        let handle = ContainerHandle {
            job_id,
            container_id: String::new(), // set after container creation
            state: ContainerState::Creating,
            mode,
            created_at: Utc::now(),
            spec,
            last_worker_status: None,
            worker_iteration: 0,
            completion_result: None,
        };
        self.containers.write().await.insert(job_id, handle);

        // Run the actual container creation. On any failure, revoke the token
        // and remove the handle so we don't leak resources.
        match self
            .create_job_inner(job_id, &token, project_dir, mode)
            .await
        {
            Ok(()) => Ok(token),
            Err(e) => {
                self.token_store.revoke(job_id).await;
                self.containers.write().await.remove(&job_id);
                Err(e)
            }
        }
    }

    /// Inner implementation of container creation (separated for cleanup).
    async fn create_job_inner(
        &self,
        job_id: Uuid,
        token: &str,
        project_dir: Option<PathBuf>,
        mode: JobMode,
    ) -> Result<(), OrchestratorError> {
        // Connect to Docker (reuses cached connection)
        let docker = self.docker().await?;

        // Build container configuration
        let orchestrator_host = if cfg!(target_os = "linux") {
            "172.17.0.1"
        } else {
            "host.docker.internal"
        };

        let orchestrator_url = format!(
            "http://{}:{}",
            orchestrator_host, self.config.orchestrator_port
        );

        let mut env_vec = vec![
            format!("THINCLAW_WORKER_TOKEN={}", token),
            format!("THINCLAW_JOB_ID={}", job_id),
            format!("THINCLAW_ORCHESTRATOR_URL={}", orchestrator_url),
        ];

        // Build volume mounts (validate project_dir stays within ~/.thinclaw/projects/)
        let mut binds = Vec::new();
        if let Some(ref dir) = project_dir {
            let canonical = validate_bind_mount_path(dir, job_id)?;
            binds.push(format!("{}:/workspace:rw", canonical.display()));
            env_vec.push("THINCLAW_WORKSPACE=/workspace".to_string());
        }

        self.extend_mode_runtime(mode, &mut env_vec, &mut binds);

        // Memory limit: container coding agents get more memory
        let memory_mb = match mode {
            JobMode::ClaudeCode => self.config.claude_code_memory_limit_mb,
            JobMode::CodexCode => self.config.codex_code_memory_limit_mb,
            JobMode::Worker => self.config.memory_limit_mb,
        };

        // Create the container
        use bollard::container::{Config, CreateContainerOptions};
        use bollard::models::HostConfig;

        let host_config = HostConfig {
            binds: if binds.is_empty() { None } else { Some(binds) },
            memory: Some((memory_mb * 1024 * 1024) as i64),
            cpu_shares: Some(self.config.cpu_shares as i64),
            network_mode: Some("bridge".to_string()),
            extra_hosts: Some(vec!["host.docker.internal:host-gateway".to_string()]),
            cap_drop: Some(vec!["ALL".to_string()]),
            cap_add: Some(vec!["CHOWN".to_string()]),
            security_opt: Some(vec!["no-new-privileges:true".to_string()]),
            tmpfs: Some(
                [("/tmp".to_string(), "size=512M".to_string())]
                    .into_iter()
                    .collect(),
            ),
            ..Default::default()
        };

        // Build CMD based on mode
        let cmd = self.container_cmd(job_id, orchestrator_url, mode).await;

        let container_config = Config {
            image: Some(self.config.image.clone()),
            cmd: Some(cmd),
            env: Some(env_vec),
            host_config: Some(host_config),
            user: Some("1000:1000".to_string()),
            working_dir: Some("/workspace".to_string()),
            ..Default::default()
        };

        let container_name = match mode {
            JobMode::Worker => format!("thinclaw-worker-{}", job_id),
            JobMode::ClaudeCode => format!("thinclaw-claude-{}", job_id),
            JobMode::CodexCode => format!("thinclaw-codex-{}", job_id),
        };
        let options = CreateContainerOptions {
            name: container_name,
            ..Default::default()
        };

        let response = docker
            .create_container(Some(options), container_config)
            .await
            .map_err(|e| OrchestratorError::ContainerCreationFailed {
                job_id,
                reason: e.to_string(),
            })?;

        let container_id = response.id;

        // Start the container
        docker
            .start_container::<String>(&container_id, None)
            .await
            .map_err(|e| OrchestratorError::ContainerCreationFailed {
                job_id,
                reason: format!("failed to start container: {}", e),
            })?;

        // Update handle with container ID
        if let Some(handle) = self.containers.write().await.get_mut(&job_id) {
            handle.container_id = container_id;
            handle.state = ContainerState::Running;
        }

        tracing::info!(
            job_id = %job_id,
            mode = %mode,
            "Created and started container job"
        );

        Ok(())
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

        if container_id.is_empty() {
            return Err(OrchestratorError::InvalidContainerState {
                job_id,
                state: "creating (no container ID yet)".to_string(),
            });
        }

        let docker = self.docker().await?;

        // Stop the container (10 second grace period)
        if let Err(e) = docker
            .stop_container(
                &container_id,
                Some(bollard::container::StopContainerOptions { t: 10 }),
            )
            .await
        {
            tracing::warn!(job_id = %job_id, error = %e, "Failed to stop container (may already be stopped)");
        }

        // Remove the container
        if let Err(e) = docker
            .remove_container(
                &container_id,
                Some(bollard::container::RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
        {
            tracing::warn!(job_id = %job_id, error = %e, "Failed to remove container (may require manual cleanup)");
        }

        // Update state
        if let Some(handle) = self.containers.write().await.get_mut(&job_id) {
            handle.state = ContainerState::Stopped;
        }

        // Revoke the auth token
        self.token_store.revoke(job_id).await;

        tracing::info!(job_id = %job_id, "Stopped container job");

        Ok(())
    }

    /// Mark a job as complete with a result. The container is stopped but the
    /// handle is kept so `CreateJobTool` can read the completion message.
    pub async fn complete_job(
        &self,
        job_id: Uuid,
        result: CompletionResult,
    ) -> Result<(), OrchestratorError> {
        // Store the result before stopping
        {
            let mut containers = self.containers.write().await;
            if let Some(handle) = containers.get_mut(&job_id) {
                handle.completion_result = Some(result);
                handle.state = ContainerState::Stopped;
            }
        }

        // Stop container and revoke token (but keep handle in map)
        let container_id = {
            let containers = self.containers.read().await;
            containers.get(&job_id).map(|h| h.container_id.clone())
        };
        if let Some(cid) = container_id
            && !cid.is_empty()
        {
            match self.docker().await {
                Ok(docker) => {
                    if let Err(e) = docker
                        .stop_container(
                            &cid,
                            Some(bollard::container::StopContainerOptions { t: 5 }),
                        )
                        .await
                    {
                        tracing::warn!(job_id = %job_id, error = %e, "Failed to stop completed container");
                    }
                    if let Err(e) = docker
                        .remove_container(
                            &cid,
                            Some(bollard::container::RemoveContainerOptions {
                                force: true,
                                ..Default::default()
                            }),
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
        self.token_store.revoke(job_id).await;

        tracing::info!(job_id = %job_id, "Completed container job");
        Ok(())
    }

    /// Remove a completed job handle from memory (called after result is read).
    pub async fn cleanup_job(&self, job_id: Uuid) {
        self.containers.write().await.remove(&job_id);
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

    /// Get a reference to the token store.
    pub fn token_store(&self) -> &TokenStore {
        &self.token_store
    }

    /// Clean up orphan containers from a previous process crash.
    ///
    /// Lists all Docker containers matching the `thinclaw-worker-*`,
    /// `thinclaw-claude-*`, or `thinclaw-codex-*` naming convention and removes any that are not
    /// tracked in the current in-memory handle map (which is empty at startup).
    ///
    /// This is fire-and-forget — errors are logged but never fatal.
    pub async fn cleanup_orphan_containers(&self) {
        let docker = match self.docker().await {
            Ok(d) => d,
            Err(e) => {
                tracing::debug!("Skipping orphan container cleanup (no Docker): {}", e);
                return;
            }
        };

        use bollard::container::{ListContainersOptions, RemoveContainerOptions};
        use std::collections::HashMap;

        // List all containers (including stopped) with our name prefix
        let mut filters = HashMap::new();
        filters.insert("name".to_string(), vec!["thinclaw-worker-".to_string()]);

        let worker_containers = docker
            .list_containers(Some(ListContainersOptions {
                all: true,
                filters: filters.clone(),
                ..Default::default()
            }))
            .await
            .unwrap_or_default();

        filters.insert("name".to_string(), vec!["thinclaw-claude-".to_string()]);
        let claude_containers = docker
            .list_containers(Some(ListContainersOptions {
                all: true,
                filters,
                ..Default::default()
            }))
            .await
            .unwrap_or_default();

        let mut codex_filters = HashMap::new();
        codex_filters.insert("name".to_string(), vec!["thinclaw-codex-".to_string()]);
        let codex_containers = docker
            .list_containers(Some(ListContainersOptions {
                all: true,
                filters: codex_filters,
                ..Default::default()
            }))
            .await
            .unwrap_or_default();

        let all_containers: Vec<_> = worker_containers
            .into_iter()
            .chain(claude_containers)
            .chain(codex_containers)
            .collect();

        if all_containers.is_empty() {
            return;
        }

        let known_ids: std::collections::HashSet<_> = self
            .containers
            .read()
            .await
            .values()
            .map(|h| h.container_id.clone())
            .collect();

        let mut removed = 0u32;
        for container in &all_containers {
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
                    Some(RemoveContainerOptions {
                        force: true,
                        ..Default::default()
                    }),
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
    fn test_container_state_display() {
        assert_eq!(ContainerState::Running.to_string(), "running");
        assert_eq!(ContainerState::Stopped.to_string(), "stopped");
    }

    #[test]
    fn test_validate_bind_mount_valid_path() {
        let base = dirs::home_dir().unwrap().join(".thinclaw").join("projects");
        std::fs::create_dir_all(&base).unwrap();

        let test_dir = base.join("test_validate_bind");
        std::fs::create_dir_all(&test_dir).unwrap();

        let result = validate_bind_mount_path(&test_dir, Uuid::new_v4());
        assert!(result.is_ok());
        let canonical = result.unwrap();
        assert!(canonical.starts_with(base.canonicalize().unwrap()));

        let _ = std::fs::remove_dir_all(&test_dir);
    }

    #[test]
    fn test_validate_bind_mount_rejects_outside_base() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tmp.path().to_path_buf();

        let result = validate_bind_mount_path(&outside, Uuid::new_v4());
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
        let nonexistent = PathBuf::from("/no/such/path/at/all");
        let result = validate_bind_mount_path(&nonexistent, Uuid::new_v4());
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
        let mut binds = Vec::new();

        mgr.extend_mode_runtime(JobMode::CodexCode, &mut env_vec, &mut binds);

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
        assert_eq!(binds.len(), 1);
        assert!(binds[0].ends_with(":/home/sandbox/.codex-host:ro"));
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
            .await;
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

        mgr.update_codex_code_settings(None).await;
        let reset = mgr
            .container_cmd(
                job_id,
                "http://orchestrator".to_string(),
                JobMode::CodexCode,
            )
            .await;
        assert_eq!(reset.last().map(String::as_str), Some("gpt-5.3-codex"));
    }
}
