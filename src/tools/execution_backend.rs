use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use uuid::Uuid;

use crate::agent::Scheduler;
use crate::channels::IncomingMessage;
use crate::channels::web::types::SseEvent;
#[cfg(test)]
use crate::config::helpers::lock_env;
use crate::context::ContextManager;
use crate::db::Database;
use crate::history::SandboxJobRecord;
use crate::platform::shell_launcher;
use crate::sandbox::{SandboxManager, SandboxPolicy};
use crate::sandbox_jobs::{SandboxChildRegistry, SandboxJobController, SandboxJobSpec};
use crate::sandbox_types::{
    ContainerJobManager, ContainerState, CredentialGrant, JobMode, PromptQueue,
};
use crate::tools::tool::ToolError;

pub const MAX_CAPTURED_OUTPUT_SIZE: usize = 64 * 1024;

/// Environment variables safe to forward from the parent process.
///
/// These are value-independent variables that do not need to reflect the
/// specific spawn context. Dynamic values like `PWD` are derived from the
/// effective working directory for each child process.
const SAFE_EXEC_ENV_VARS: &[&str] = &[
    // Core OS
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TERM",
    "COLORTERM",
    // Locale
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "LC_MESSAGES",
    // Temp directories
    "TMPDIR",
    "TMP",
    "TEMP",
    // XDG (Linux desktop/config paths)
    "XDG_RUNTIME_DIR",
    "XDG_DATA_HOME",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    // Rust toolchain
    "CARGO_HOME",
    "RUSTUP_HOME",
    // Node.js
    "NODE_PATH",
    "NPM_CONFIG_PREFIX",
    // Editor (for git commit, etc.)
    "EDITOR",
    "VISUAL",
    // Windows (no-ops on Unix, but needed if we ever run on Windows)
    "SystemRoot",
    "SYSTEMROOT",
    "ComSpec",
    "PATHEXT",
    "APPDATA",
    "LOCALAPPDATA",
    "USERPROFILE",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "WINDIR",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExecutionBackendKind {
    LocalHost,
    DockerSandbox,
    RemoteRunnerAdapter,
}

impl ExecutionBackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalHost => "local_host",
            Self::DockerSandbox => "docker_sandbox",
            Self::RemoteRunnerAdapter => "remote_runner_adapter",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkIsolationKind {
    None,
    Hard,
    BestEffort,
}

impl NetworkIsolationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Hard => "hard",
            Self::BestEffort => "best_effort",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeDescriptor {
    pub execution_backend: String,
    pub runtime_family: String,
    pub runtime_mode: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_isolation: Option<String>,
}

impl RuntimeDescriptor {
    pub fn logical_surface(
        execution_backend: impl Into<String>,
        runtime_family: impl Into<String>,
        runtime_mode: impl Into<String>,
        runtime_capabilities: Vec<String>,
        network_isolation: Option<impl Into<String>>,
    ) -> Self {
        Self {
            execution_backend: execution_backend.into(),
            runtime_family: runtime_family.into(),
            runtime_mode: runtime_mode.into(),
            runtime_capabilities,
            network_isolation: network_isolation.map(Into::into),
        }
    }

    pub fn execution_surface(
        backend: ExecutionBackendKind,
        runtime_mode: impl Into<String>,
        runtime_capabilities: Vec<String>,
        network_isolation: NetworkIsolationKind,
    ) -> Self {
        Self::logical_surface(
            backend.as_str(),
            "execution_backend",
            runtime_mode,
            runtime_capabilities,
            Some(network_isolation.as_str()),
        )
    }
}

pub fn interactive_chat_runtime_descriptor() -> RuntimeDescriptor {
    RuntimeDescriptor::logical_surface(
        "interactive_chat",
        "agent_surface",
        "interactive_chat",
        vec![
            "conversation_state".to_string(),
            "llm_turn".to_string(),
            "thread_history".to_string(),
        ],
        Some(NetworkIsolationKind::None.as_str()),
    )
}

pub fn routine_engine_runtime_descriptor() -> RuntimeDescriptor {
    RuntimeDescriptor::logical_surface(
        "routine_engine",
        "agent_surface",
        "routine_engine",
        vec![
            "routine_orchestration".to_string(),
            "scheduled_execution".to_string(),
        ],
        Some(NetworkIsolationKind::None.as_str()),
    )
}

pub fn subagent_executor_runtime_descriptor() -> RuntimeDescriptor {
    RuntimeDescriptor::logical_surface(
        "subagent_executor",
        "agent_surface",
        "subagent_executor",
        vec![
            "delegated_execution".to_string(),
            "llm_turn".to_string(),
            "task_isolation".to_string(),
        ],
        Some(NetworkIsolationKind::None.as_str()),
    )
}

pub fn experiment_runner_runtime_descriptor(backend_slug: &str) -> RuntimeDescriptor {
    RuntimeDescriptor::logical_surface(
        backend_slug.to_string(),
        "experiment_runner",
        format!("experiment_runner:{backend_slug}"),
        vec![
            "artifact_capture".to_string(),
            "benchmark_execution".to_string(),
            "remote_trial".to_string(),
        ],
        Some(NetworkIsolationKind::BestEffort.as_str()),
    )
}

pub fn local_job_runtime_descriptor() -> RuntimeDescriptor {
    RuntimeDescriptor::execution_surface(
        ExecutionBackendKind::LocalHost,
        "in_memory",
        vec![
            "job_orchestration".to_string(),
            "queue_tracking".to_string(),
        ],
        NetworkIsolationKind::None,
    )
}

pub fn sandbox_job_runtime_descriptor(mode: JobMode) -> RuntimeDescriptor {
    let mut runtime_capabilities = vec![
        "file_browse".to_string(),
        "follow_up_prompts".to_string(),
        "job_orchestration".to_string(),
        "persistent_workspace".to_string(),
        "streamed_events".to_string(),
    ];
    match mode {
        JobMode::Worker => {
            runtime_capabilities.push("agent_loop".to_string());
            runtime_capabilities.push("llm_proxy".to_string());
        }
        JobMode::ClaudeCode => {
            runtime_capabilities.push("agent_loop".to_string());
            runtime_capabilities.push("claude_cli".to_string());
        }
        JobMode::CodexCode => {
            runtime_capabilities.push("agent_loop".to_string());
            runtime_capabilities.push("codex_cli".to_string());
        }
    }
    RuntimeDescriptor::execution_surface(
        ExecutionBackendKind::DockerSandbox,
        mode.as_str(),
        runtime_capabilities,
        NetworkIsolationKind::Hard,
    )
}

#[derive(Debug, Clone)]
pub struct CommandExecutionRequest {
    pub command: String,
    pub workdir: PathBuf,
    pub timeout: Duration,
    pub extra_env: HashMap<String, String>,
    pub allow_network: bool,
}

#[derive(Debug, Clone)]
pub struct ScriptExecutionRequest {
    pub program: String,
    pub args: Vec<String>,
    pub workdir: PathBuf,
    pub timeout: Duration,
    pub extra_env: HashMap<String, String>,
    pub allow_network: bool,
}

#[derive(Debug, Clone)]
pub struct ProcessStartRequest {
    pub command: String,
    pub workdir: Option<PathBuf>,
    pub extra_env: HashMap<String, String>,
    pub kill_on_drop: bool,
}

#[derive(Debug)]
pub struct StartedProcess {
    pub child: Child,
    pub stdin: Option<ChildStdin>,
    pub stdout: Option<ChildStdout>,
    pub stderr: Option<ChildStderr>,
    pub backend: ExecutionBackendKind,
    pub runtime: RuntimeDescriptor,
}

#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub stdout: String,
    pub stderr: String,
    pub output: String,
    pub exit_code: i64,
    pub backend: ExecutionBackendKind,
    pub runtime: RuntimeDescriptor,
    pub duration: Duration,
}

#[derive(Debug, Clone)]
pub struct JobExecutionRequest {
    pub title: String,
    pub description: String,
    pub principal_id: String,
    pub actor_id: String,
    pub parent_job_id: Option<Uuid>,
    pub wait: bool,
    pub explicit_project_dir: Option<PathBuf>,
    pub mode: Option<JobMode>,
    pub metadata: serde_json::Value,
    pub allowed_tools: Option<Vec<String>>,
    pub allowed_skills: Option<Vec<String>>,
    pub tool_profile: Option<String>,
    pub credential_grants: Vec<CredentialGrant>,
    pub job_events_available: bool,
    pub job_prompt_available: bool,
    pub job_status_available: bool,
}

#[derive(Debug, Clone)]
pub struct JobExecutionResult {
    pub job_id: Uuid,
    pub status: String,
    pub runtime: RuntimeDescriptor,
    pub message: Option<String>,
    pub output: Option<String>,
    pub project_dir: Option<String>,
    pub browse_url: Option<String>,
}

#[derive(Clone)]
pub struct JobOrchestrationContext {
    context_manager: Arc<ContextManager>,
    job_manager: Option<Arc<ContainerJobManager>>,
    store: Option<Arc<dyn Database>>,
    scheduler: Option<Arc<Scheduler>>,
    event_tx: Option<tokio::sync::broadcast::Sender<(Uuid, SseEvent)>>,
    inject_tx: Option<tokio::sync::mpsc::Sender<IncomingMessage>>,
    prompt_queue: Option<PromptQueue>,
    sandbox_children: Option<Arc<SandboxChildRegistry>>,
}

impl JobOrchestrationContext {
    pub fn new(
        context_manager: Arc<ContextManager>,
        job_manager: Option<Arc<ContainerJobManager>>,
        store: Option<Arc<dyn Database>>,
        scheduler: Option<Arc<Scheduler>>,
        event_tx: Option<tokio::sync::broadcast::Sender<(Uuid, SseEvent)>>,
        inject_tx: Option<tokio::sync::mpsc::Sender<IncomingMessage>>,
        prompt_queue: Option<PromptQueue>,
        sandbox_children: Option<Arc<SandboxChildRegistry>>,
    ) -> Self {
        Self {
            context_manager,
            job_manager,
            store,
            scheduler,
            event_tx,
            inject_tx,
            prompt_queue,
            sandbox_children,
        }
    }

    pub fn claude_code_enabled(&self) -> bool {
        self.job_manager
            .as_ref()
            .map(|manager| manager.claude_code_enabled())
            .unwrap_or(false)
    }

    pub fn codex_code_enabled(&self) -> bool {
        self.job_manager
            .as_ref()
            .map(|manager| manager.codex_code_enabled())
            .unwrap_or(false)
    }

    fn persist_job(&self, job: SandboxJobRecord) {
        if let Some(store) = self.store.clone() {
            tokio::spawn(async move {
                if let Err(error) = store.save_sandbox_job(&job).await {
                    tracing::warn!(job_id = %job.id, "Failed to persist sandbox job: {}", error);
                }
            });
        }
    }

    fn sandbox_controller(&self) -> SandboxJobController {
        SandboxJobController::new(
            self.store.clone(),
            self.job_manager.clone(),
            self.event_tx.clone(),
            self.prompt_queue.clone(),
        )
    }

    fn build_local_job_metadata(&self, request: &JobExecutionRequest) -> serde_json::Value {
        let runtime = local_job_runtime_descriptor();
        let mut metadata = match request.metadata.as_object() {
            Some(obj) => obj.clone(),
            None => serde_json::Map::new(),
        };

        if !request.metadata.is_null() && !request.metadata.is_object() {
            metadata.insert("request_metadata".to_string(), request.metadata.clone());
        }
        if let Some(parent_job_id) = request.parent_job_id {
            metadata.insert(
                "parent_job_id".to_string(),
                serde_json::json!(parent_job_id),
            );
        }
        if let Some(allowed_tools) = request.allowed_tools.as_ref() {
            metadata.insert(
                "allowed_tools".to_string(),
                serde_json::json!(allowed_tools),
            );
        }
        if let Some(allowed_skills) = request.allowed_skills.as_ref() {
            metadata.insert(
                "allowed_skills".to_string(),
                serde_json::json!(allowed_skills),
            );
        }
        if let Some(tool_profile) = request.tool_profile.as_ref() {
            metadata.insert("tool_profile".to_string(), serde_json::json!(tool_profile));
        }
        metadata.insert(
            "execution_backend".to_string(),
            serde_json::json!(runtime.execution_backend),
        );
        metadata.insert(
            "runtime_family".to_string(),
            serde_json::json!(runtime.runtime_family),
        );
        metadata.insert(
            "runtime_mode".to_string(),
            serde_json::json!(runtime.runtime_mode),
        );
        metadata.insert(
            "runtime_capabilities".to_string(),
            serde_json::json!(runtime.runtime_capabilities),
        );
        metadata.insert(
            "network_isolation".to_string(),
            serde_json::json!(runtime.network_isolation),
        );

        serde_json::Value::Object(metadata)
    }

    fn update_status(
        &self,
        job_id: Uuid,
        status: &str,
        success: Option<bool>,
        message: Option<String>,
        started_at: Option<chrono::DateTime<Utc>>,
        completed_at: Option<chrono::DateTime<Utc>>,
    ) {
        if let Some(store) = self.store.clone() {
            let status = status.to_string();
            tokio::spawn(async move {
                if let Err(error) = store
                    .update_sandbox_job_status(
                        job_id,
                        &status,
                        success,
                        message.as_deref(),
                        started_at,
                        completed_at,
                    )
                    .await
                {
                    tracing::warn!(job_id = %job_id, "Failed to update sandbox job status: {}", error);
                }
            });
        }
    }

    async fn run_local_job(
        &self,
        request: JobExecutionRequest,
    ) -> Result<JobExecutionResult, ToolError> {
        let runtime = local_job_runtime_descriptor();
        let metadata = self.build_local_job_metadata(&request);

        if let Some(scheduler) = self.scheduler.as_ref() {
            let job_id = scheduler
                .dispatch_job_for_identity(
                    &request.principal_id,
                    &request.actor_id,
                    &request.title,
                    &request.description,
                    Some(metadata),
                )
                .await
                .map_err(|error| ToolError::ExecutionFailed(error.to_string()))?;
            return Ok(JobExecutionResult {
                job_id,
                status: "started".to_string(),
                runtime,
                message: Some(format!("Scheduled job '{}'", request.title)),
                output: None,
                project_dir: None,
                browse_url: None,
            });
        }

        let job_id = self
            .context_manager
            .create_job_for_identity(
                &request.principal_id,
                &request.actor_id,
                &request.title,
                &request.description,
            )
            .await
            .map_err(|error| ToolError::ExecutionFailed(error.to_string()))?;
        if let Err(error) = self
            .context_manager
            .update_context(job_id, |ctx| {
                ctx.metadata = metadata.clone();
            })
            .await
        {
            tracing::warn!(
                job_id = %job_id,
                "Failed to attach metadata to fallback local job: {}",
                error
            );
        }
        Ok(JobExecutionResult {
            job_id,
            status: "pending".to_string(),
            runtime,
            message: Some(format!("Created job '{}'", request.title)),
            output: None,
            project_dir: None,
            browse_url: None,
        })
    }

    async fn run_sandbox_job(
        &self,
        request: JobExecutionRequest,
    ) -> Result<JobExecutionResult, ToolError> {
        let job_manager = self.job_manager.as_ref().ok_or_else(|| {
            ToolError::ExecutionFailed("sandbox job execution is unavailable".to_string())
        })?;
        let mode = request.mode.unwrap_or(JobMode::Worker);

        let job_id = Uuid::new_v4();
        let (project_dir, browse_id) = resolve_project_dir(request.explicit_project_dir, job_id)?;
        let project_dir_str = project_dir.display().to_string();
        let spec = SandboxJobSpec {
            title: request.title.clone(),
            description: request.description.clone(),
            principal_id: request.principal_id.clone(),
            actor_id: request.actor_id.clone(),
            project_dir: Some(project_dir_str.clone()),
            mode,
            interactive: !request.wait,
            idle_timeout_secs: job_manager.interactive_idle_timeout_secs(),
            parent_job_id: request.parent_job_id,
            metadata: request.metadata.clone(),
            allowed_tools: request.allowed_tools.clone(),
            allowed_skills: request.allowed_skills.clone(),
            tool_profile: request.tool_profile.clone(),
        };

        let credential_grants_json = match serde_json::to_string(&request.credential_grants) {
            Ok(json) => json,
            Err(error) => {
                tracing::warn!(
                    "Failed to serialize credential grants for job {}: {}. Grants will not survive a restart.",
                    job_id,
                    error
                );
                String::from("[]")
            }
        };

        self.persist_job(SandboxJobRecord {
            id: job_id,
            spec: spec.clone(),
            status: "creating".to_string(),
            success: None,
            failure_reason: None,
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
            credential_grants_json,
        });

        job_manager
            .create_job(job_id, spec.clone(), request.credential_grants)
            .await
            .map_err(|error| {
                self.update_status(
                    job_id,
                    "failed",
                    Some(false),
                    Some(error.to_string()),
                    None,
                    Some(Utc::now()),
                );
                ToolError::ExecutionFailed(format!("failed to create container: {}", error))
            })?;

        let now = Utc::now();
        self.update_status(job_id, "running", None, None, Some(now), None);

        if spec.interactive
            && let (Some(parent_job_id), Some(children)) =
                (spec.parent_job_id, self.sandbox_children.as_ref())
        {
            children.register_child(parent_job_id, job_id).await;
        }

        if !request.wait {
            if let (Some(event_tx), Some(inject_tx)) = (&self.event_tx, &self.inject_tx) {
                crate::agent::job_monitor::spawn_job_monitor(
                    job_id,
                    event_tx.subscribe(),
                    inject_tx.clone(),
                );
            }
            let mut hints = Vec::new();
            if request.job_events_available {
                hints.push("Use job_events to inspect streamed activity.".to_string());
            } else if request.job_status_available {
                hints.push("Use job_status to inspect progress.".to_string());
            }
            if request.job_prompt_available {
                hints.push(
                    "Use job_prompt to send follow-up instructions or done=true when wrapping up."
                        .to_string(),
                );
            }
            if hints.is_empty() {
                hints.push("Use the Jobs UI to inspect progress.".to_string());
            }
            return Ok(JobExecutionResult {
                job_id,
                status: "started".to_string(),
                runtime: sandbox_job_runtime_descriptor(mode),
                message: Some(format!("Container started. {}", hints.join(" "))),
                output: None,
                project_dir: Some(project_dir_str),
                browse_url: Some(format!("/projects/{}", browse_id)),
            });
        }

        let timeout = Duration::from_secs(600);
        let poll_interval = Duration::from_secs(2);
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            if tokio::time::Instant::now() > deadline {
                let _ = self
                    .sandbox_controller()
                    .finalize_job(
                        job_id,
                        "failed",
                        false,
                        Some("Timed out (10 minutes)".to_string()),
                        None,
                        0,
                    )
                    .await;
                job_manager.cleanup_job(job_id).await;
                return Err(ToolError::ExecutionFailed(
                    "container execution timed out (10 minutes)".to_string(),
                ));
            }

            match job_manager.get_handle(job_id).await {
                Some(handle) => match handle.state {
                    ContainerState::Running | ContainerState::Creating => {
                        tokio::time::sleep(poll_interval).await;
                    }
                    ContainerState::Stopped => {
                        let message = handle
                            .completion_result
                            .as_ref()
                            .and_then(|result| result.message.clone())
                            .unwrap_or_else(|| "Container job completed".to_string());
                        let success = handle
                            .completion_result
                            .as_ref()
                            .map(|result| result.success)
                            .unwrap_or(true);
                        job_manager.cleanup_job(job_id).await;

                        if success {
                            return Ok(JobExecutionResult {
                                job_id,
                                status: "completed".to_string(),
                                runtime: sandbox_job_runtime_descriptor(mode),
                                message: None,
                                output: Some(message),
                                project_dir: Some(project_dir_str),
                                browse_url: Some(format!("/projects/{}", browse_id)),
                            });
                        }

                        return Err(ToolError::ExecutionFailed(format!(
                            "container job failed: {}",
                            message
                        )));
                    }
                    ContainerState::Failed => {
                        let message = handle
                            .completion_result
                            .as_ref()
                            .and_then(|result| result.message.clone())
                            .unwrap_or_else(|| "unknown failure".to_string());
                        job_manager.cleanup_job(job_id).await;
                        return Err(ToolError::ExecutionFailed(format!(
                            "container job failed: {}",
                            message
                        )));
                    }
                },
                None => {
                    self.update_status(
                        job_id,
                        "completed",
                        Some(true),
                        None,
                        None,
                        Some(Utc::now()),
                    );
                    return Ok(JobExecutionResult {
                        job_id,
                        status: "completed".to_string(),
                        runtime: sandbox_job_runtime_descriptor(mode),
                        message: None,
                        output: Some("Container job completed".to_string()),
                        project_dir: Some(project_dir_str),
                        browse_url: Some(format!("/projects/{}", browse_id)),
                    });
                }
            }
        }
    }
}

#[derive(Debug)]
struct CollectedOutput {
    stdout: String,
    stderr: String,
    output: String,
    exit_code: i64,
}

#[async_trait]
pub trait ExecutionBackend: Send + Sync {
    fn kind(&self) -> ExecutionBackendKind;

    async fn run_shell(
        &self,
        _request: CommandExecutionRequest,
    ) -> Result<ExecutionResult, ToolError> {
        Err(ToolError::ExecutionFailed(format!(
            "{} does not support shell execution",
            self.kind().as_str()
        )))
    }

    async fn start_process(
        &self,
        _request: ProcessStartRequest,
    ) -> Result<StartedProcess, ToolError> {
        Err(ToolError::ExecutionFailed(format!(
            "{} does not support background process execution",
            self.kind().as_str()
        )))
    }

    async fn run_script(
        &self,
        _request: ScriptExecutionRequest,
    ) -> Result<ExecutionResult, ToolError> {
        Err(ToolError::ExecutionFailed(format!(
            "{} does not support script execution",
            self.kind().as_str()
        )))
    }

    async fn run_job(
        &self,
        _request: JobExecutionRequest,
    ) -> Result<JobExecutionResult, ToolError> {
        Err(ToolError::ExecutionFailed(format!(
            "{} does not support job execution",
            self.kind().as_str()
        )))
    }
}

#[derive(Default)]
pub struct LocalHostExecutionBackend {
    job_orchestration: Option<Arc<JobOrchestrationContext>>,
}

impl LocalHostExecutionBackend {
    pub fn shared() -> Arc<dyn ExecutionBackend> {
        Arc::new(Self::default())
    }

    pub fn with_job_orchestration(
        job_orchestration: Arc<JobOrchestrationContext>,
    ) -> Arc<dyn ExecutionBackend> {
        Arc::new(Self {
            job_orchestration: Some(job_orchestration),
        })
    }
}

#[async_trait]
impl ExecutionBackend for LocalHostExecutionBackend {
    fn kind(&self) -> ExecutionBackendKind {
        ExecutionBackendKind::LocalHost
    }

    async fn run_shell(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<ExecutionResult, ToolError> {
        let mut command = build_shell_command(&request.command, request.allow_network);
        configure_spawn(
            &mut command,
            &request.workdir,
            &request.extra_env,
            request.allow_network,
        );

        let mut child = command
            .spawn()
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to spawn command: {}", e)))?;

        let start = Instant::now();
        let collected = collect_child_output(&mut child, request.timeout).await?;
        let network_isolation = host_local_network_isolation(request.allow_network);
        Ok(ExecutionResult {
            stdout: collected.stdout,
            stderr: collected.stderr,
            output: collected.output,
            exit_code: collected.exit_code,
            backend: self.kind(),
            runtime: RuntimeDescriptor::execution_surface(
                self.kind(),
                "shell",
                vec![
                    "captured_output".to_string(),
                    "short_lived_command".to_string(),
                ],
                network_isolation,
            ),
            duration: start.elapsed(),
        })
    }

    async fn start_process(
        &self,
        request: ProcessStartRequest,
    ) -> Result<StartedProcess, ToolError> {
        let mut command = shell_launcher().tokio_command(&request.command);
        if let Some(workdir) = request.workdir.as_ref() {
            command.current_dir(workdir);
        }
        configure_stdio_command(
            &mut command,
            request.workdir.as_deref(),
            &request.extra_env,
            true,
            request.kill_on_drop,
        );

        let mut child = command
            .spawn()
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to spawn process: {}", e)))?;

        Ok(StartedProcess {
            stdin: child.stdin.take(),
            stdout: child.stdout.take(),
            stderr: child.stderr.take(),
            child,
            backend: self.kind(),
            runtime: RuntimeDescriptor::execution_surface(
                self.kind(),
                "process",
                vec![
                    "captured_output".to_string(),
                    "incremental_output".to_string(),
                    "interactive_stdin".to_string(),
                    "long_running_process".to_string(),
                ],
                NetworkIsolationKind::None,
            ),
        })
    }

    async fn run_script(
        &self,
        request: ScriptExecutionRequest,
    ) -> Result<ExecutionResult, ToolError> {
        let mut command =
            build_script_command(&request.program, &request.args, request.allow_network);
        configure_spawn(
            &mut command,
            &request.workdir,
            &request.extra_env,
            request.allow_network,
        );

        let mut child = command.spawn().map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to spawn {}: {}", request.program, e))
        })?;

        let start = Instant::now();
        let collected = collect_child_output(&mut child, request.timeout).await?;
        let network_isolation = host_local_network_isolation(request.allow_network);
        Ok(ExecutionResult {
            stdout: collected.stdout,
            stderr: collected.stderr,
            output: collected.output,
            exit_code: collected.exit_code,
            backend: self.kind(),
            runtime: RuntimeDescriptor::execution_surface(
                self.kind(),
                "script",
                vec![
                    "captured_output".to_string(),
                    "language_runtime".to_string(),
                    "short_lived_command".to_string(),
                ],
                network_isolation,
            ),
            duration: start.elapsed(),
        })
    }

    async fn run_job(&self, request: JobExecutionRequest) -> Result<JobExecutionResult, ToolError> {
        let job_orchestration = self.job_orchestration.as_ref().ok_or_else(|| {
            ToolError::ExecutionFailed(
                "local host backend is not configured for job execution".to_string(),
            )
        })?;
        job_orchestration.run_local_job(request).await
    }
}

pub struct DockerSandboxExecutionBackend {
    sandbox: Option<Arc<SandboxManager>>,
    policy: Option<SandboxPolicy>,
    job_orchestration: Option<Arc<JobOrchestrationContext>>,
}

impl DockerSandboxExecutionBackend {
    pub fn from_sandbox(
        sandbox: Arc<SandboxManager>,
        policy: SandboxPolicy,
    ) -> Arc<dyn ExecutionBackend> {
        Arc::new(Self {
            sandbox: Some(sandbox),
            policy: Some(policy),
            job_orchestration: None,
        })
    }

    pub fn with_job_orchestration(
        job_orchestration: Arc<JobOrchestrationContext>,
    ) -> Arc<dyn ExecutionBackend> {
        Arc::new(Self {
            sandbox: None,
            policy: None,
            job_orchestration: Some(job_orchestration),
        })
    }
}

#[async_trait]
impl ExecutionBackend for DockerSandboxExecutionBackend {
    fn kind(&self) -> ExecutionBackendKind {
        ExecutionBackendKind::DockerSandbox
    }

    async fn run_shell(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<ExecutionResult, ToolError> {
        let sandbox = self.sandbox.as_ref().ok_or_else(|| {
            ToolError::ExecutionFailed(
                "docker sandbox backend is not configured for shell execution".to_string(),
            )
        })?;
        let policy = self.policy.ok_or_else(|| {
            ToolError::ExecutionFailed(
                "docker sandbox backend is not configured for shell execution".to_string(),
            )
        })?;
        let start = Instant::now();
        let result = tokio::time::timeout(request.timeout, async {
            sandbox
                .execute_with_policy_and_network(
                    &request.command,
                    &request.workdir,
                    policy,
                    request.extra_env.clone(),
                    request.allow_network,
                )
                .await
        })
        .await;

        match result {
            Ok(Ok(output)) => Ok(ExecutionResult {
                stdout: truncate_output(&output.stdout),
                stderr: truncate_output(&output.stderr),
                output: truncate_output(&output.output),
                exit_code: output.exit_code,
                backend: self.kind(),
                runtime: RuntimeDescriptor::execution_surface(
                    self.kind(),
                    "shell",
                    vec![
                        "captured_output".to_string(),
                        "short_lived_command".to_string(),
                    ],
                    if request.allow_network {
                        NetworkIsolationKind::None
                    } else {
                        NetworkIsolationKind::Hard
                    },
                ),
                duration: start.elapsed(),
            }),
            Ok(Err(e)) => Err(ToolError::ExecutionFailed(format!("Sandbox error: {}", e))),
            Err(_) => Err(ToolError::Timeout(request.timeout)),
        }
    }

    async fn run_script(
        &self,
        request: ScriptExecutionRequest,
    ) -> Result<ExecutionResult, ToolError> {
        let command =
            render_container_script_command(&request.program, &request.args, &request.workdir);
        self.run_shell(CommandExecutionRequest {
            command,
            workdir: request.workdir,
            timeout: request.timeout,
            extra_env: request.extra_env,
            allow_network: request.allow_network,
        })
        .await
    }

    async fn run_job(&self, request: JobExecutionRequest) -> Result<JobExecutionResult, ToolError> {
        let job_orchestration = self.job_orchestration.as_ref().ok_or_else(|| {
            ToolError::ExecutionFailed(
                "docker sandbox backend is not configured for job execution".to_string(),
            )
        })?;
        job_orchestration.run_sandbox_job(request).await
    }
}

#[derive(Debug, Default)]
pub struct RemoteRunnerAdapterExecutionBackend;

impl RemoteRunnerAdapterExecutionBackend {
    pub fn shared() -> Arc<dyn ExecutionBackend> {
        Arc::new(Self)
    }
}

#[async_trait]
impl ExecutionBackend for RemoteRunnerAdapterExecutionBackend {
    fn kind(&self) -> ExecutionBackendKind {
        ExecutionBackendKind::RemoteRunnerAdapter
    }
}

fn build_shell_command(command: &str, allow_network: bool) -> Command {
    let launcher = shell_launcher();

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let _ = allow_network;

    #[cfg(target_os = "macos")]
    if !allow_network && Path::new("/usr/bin/sandbox-exec").is_file() {
        let mut sandboxed = Command::new("/usr/bin/sandbox-exec");
        sandboxed.arg("-p").arg(macos_network_deny_profile());
        sandboxed.arg(launcher.program());
        sandboxed.args(launcher.prefix_args());
        sandboxed.arg(command);
        return sandboxed;
    }

    #[cfg(target_os = "linux")]
    if !allow_network && let Some(wrapper) = linux_bubblewrap_program() {
        let mut sandboxed = Command::new(wrapper);
        sandboxed
            .arg("--die-with-parent")
            .arg("--unshare-net")
            .arg("--bind")
            .arg("/")
            .arg("/")
            .arg("--proc")
            .arg("/proc")
            .arg("--")
            .arg(launcher.program());
        sandboxed.args(launcher.prefix_args());
        sandboxed.arg(command);
        return sandboxed;
    }

    launcher.tokio_command(command)
}

fn build_script_command(program: &str, args: &[String], allow_network: bool) -> Command {
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let _ = allow_network;

    #[cfg(target_os = "macos")]
    if !allow_network && Path::new("/usr/bin/sandbox-exec").is_file() {
        let mut sandboxed = Command::new("/usr/bin/sandbox-exec");
        sandboxed.arg("-p").arg(macos_network_deny_profile());
        sandboxed.arg(program);
        sandboxed.args(args);
        return sandboxed;
    }

    #[cfg(target_os = "linux")]
    if !allow_network && let Some(wrapper) = linux_bubblewrap_program() {
        let mut sandboxed = Command::new(wrapper);
        sandboxed
            .arg("--die-with-parent")
            .arg("--unshare-net")
            .arg("--bind")
            .arg("/")
            .arg("/")
            .arg("--proc")
            .arg("/proc")
            .arg("--")
            .arg(program);
        sandboxed.args(args);
        return sandboxed;
    }

    let mut command = Command::new(program);
    command.args(args);
    command
}

#[cfg(target_os = "macos")]
fn macos_network_deny_profile() -> &'static str {
    "(version 1) (allow default) (deny network*)"
}

fn render_container_script_command(program: &str, args: &[String], workdir: &Path) -> String {
    std::iter::once(program.to_string())
        .chain(args.iter().cloned())
        .map(|value| map_host_path_to_container(&value, workdir))
        .map(|value| shell_quote(&value))
        .collect::<Vec<_>>()
        .join(" ")
}

fn projects_base() -> PathBuf {
    crate::platform::resolve_data_dir("projects")
}

pub fn resolve_project_dir(
    explicit: Option<PathBuf>,
    project_id: Uuid,
) -> Result<(PathBuf, String), ToolError> {
    let base = projects_base();
    std::fs::create_dir_all(&base).map_err(|error| {
        ToolError::ExecutionFailed(format!(
            "failed to create projects base {}: {}",
            base.display(),
            error
        ))
    })?;
    let canonical_base = base.canonicalize().map_err(|error| {
        ToolError::ExecutionFailed(format!("failed to canonicalize projects base: {}", error))
    })?;

    let canonical_dir = match explicit {
        Some(dir) => {
            let canonical = dir.canonicalize().map_err(|error| {
                ToolError::InvalidParameters(format!(
                    "explicit project dir {} does not exist or is inaccessible: {}",
                    dir.display(),
                    error
                ))
            })?;
            if !canonical.starts_with(&canonical_base) {
                return Err(ToolError::InvalidParameters(format!(
                    "project directory must be under {}",
                    canonical_base.display()
                )));
            }
            canonical
        }
        None => {
            let dir = canonical_base.join(project_id.to_string());
            std::fs::create_dir_all(&dir).map_err(|error| {
                ToolError::ExecutionFailed(format!(
                    "failed to create project dir {}: {}",
                    dir.display(),
                    error
                ))
            })?;
            dir.canonicalize().map_err(|error| {
                ToolError::ExecutionFailed(format!(
                    "failed to canonicalize project dir {}: {}",
                    dir.display(),
                    error
                ))
            })?
        }
    };

    let browse_id = canonical_dir
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| project_id.to_string());
    Ok((canonical_dir, browse_id))
}

fn map_host_path_to_container(value: &str, workdir: &Path) -> String {
    let path = Path::new(value);
    if path.is_absolute()
        && let Ok(relative) = path.strip_prefix(workdir)
    {
        let mapped = if relative.as_os_str().is_empty() {
            PathBuf::from("/workspace")
        } else {
            PathBuf::from("/workspace").join(relative)
        };
        return mapped.to_string_lossy().to_string();
    }
    value.to_string()
}

fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        "''".to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}

fn configure_spawn(
    command: &mut Command,
    workdir: &Path,
    extra_env: &HashMap<String, String>,
    allow_network: bool,
) {
    command.current_dir(workdir);
    configure_env(command, Some(workdir), extra_env, allow_network);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(false);
}

fn configure_stdio_command(
    command: &mut Command,
    workdir: Option<&Path>,
    extra_env: &HashMap<String, String>,
    allow_network: bool,
    kill_on_drop: bool,
) {
    configure_env(command, workdir, extra_env, allow_network);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(kill_on_drop);
}

fn configure_env(
    command: &mut Command,
    workdir: Option<&Path>,
    extra_env: &HashMap<String, String>,
    allow_network: bool,
) {
    command.env_clear();
    for var in SAFE_EXEC_ENV_VARS {
        if let Ok(value) = std::env::var(var) {
            command.env(var, value);
        }
    }
    command.envs(extra_env);
    if let Some(dir) = workdir {
        command.env("PWD", dir);
    } else if let Ok(dir) = std::env::current_dir() {
        command.env("PWD", dir);
    }
    if !allow_network {
        command.env("no_proxy", "*");
        command.env("NO_PROXY", "*");
    }
}

pub fn host_local_network_deny_support() -> NetworkIsolationKind {
    #[cfg(target_os = "macos")]
    if Path::new("/usr/bin/sandbox-exec").is_file() {
        return NetworkIsolationKind::Hard;
    }

    #[cfg(target_os = "linux")]
    if linux_bubblewrap_program().is_some() {
        return NetworkIsolationKind::Hard;
    }

    NetworkIsolationKind::BestEffort
}

pub fn host_local_network_isolation(allow_network: bool) -> NetworkIsolationKind {
    if allow_network {
        NetworkIsolationKind::None
    } else {
        host_local_network_deny_support()
    }
}

#[cfg(target_os = "linux")]
fn linux_bubblewrap_program() -> Option<&'static str> {
    if Path::new("/usr/bin/bwrap").is_file() {
        Some("/usr/bin/bwrap")
    } else if executable_in_path("bwrap") {
        Some("bwrap")
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
fn executable_in_path(binary: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(binary).is_file()))
}

async fn collect_child_output(
    child: &mut Child,
    timeout: Duration,
) -> Result<CollectedOutput, ToolError> {
    let stdout_handle = child.stdout.take();
    let stderr_handle = child.stderr.take();
    let result = tokio::time::timeout(timeout, async {
        let stdout_fut = async {
            if let Some(mut out) = stdout_handle {
                let mut buf = Vec::new();
                (&mut out)
                    .take(MAX_CAPTURED_OUTPUT_SIZE as u64)
                    .read_to_end(&mut buf)
                    .await
                    .ok();
                tokio::io::copy(&mut out, &mut tokio::io::sink()).await.ok();
                String::from_utf8_lossy(&buf).to_string()
            } else {
                String::new()
            }
        };

        let stderr_fut = async {
            if let Some(mut err) = stderr_handle {
                let mut buf = Vec::new();
                (&mut err)
                    .take(MAX_CAPTURED_OUTPUT_SIZE as u64)
                    .read_to_end(&mut buf)
                    .await
                    .ok();
                tokio::io::copy(&mut err, &mut tokio::io::sink()).await.ok();
                String::from_utf8_lossy(&buf).to_string()
            } else {
                String::new()
            }
        };

        let (stdout, stderr, wait_result) = tokio::join!(stdout_fut, stderr_fut, child.wait());
        let status = wait_result?;
        let code = status.code().unwrap_or(-1) as i64;
        let output = if stderr.is_empty() {
            stdout.clone()
        } else if stdout.is_empty() {
            stderr.clone()
        } else {
            format!("{}\n\n--- stderr ---\n{}", stdout, stderr)
        };

        Ok::<_, std::io::Error>(CollectedOutput {
            stdout: truncate_output(&stdout),
            stderr: truncate_output(&stderr),
            output: truncate_output(&output),
            exit_code: code,
        })
    })
    .await;

    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => Err(ToolError::ExecutionFailed(format!(
            "Command execution failed: {}",
            err
        ))),
        Err(_) => {
            let _ = child.kill().await;
            Err(ToolError::Timeout(timeout))
        }
    }
}

pub fn truncate_output(output: &str) -> String {
    if output.len() <= MAX_CAPTURED_OUTPUT_SIZE {
        output.to_string()
    } else {
        let half = MAX_CAPTURED_OUTPUT_SIZE / 2;
        let head_end = crate::util::floor_char_boundary(output, half);
        let tail_start =
            crate::util::floor_char_boundary(output, output.len().saturating_sub(half));
        format!(
            "{}\n\n... [truncated {} bytes] ...\n\n{}",
            &output[..head_end],
            output.len() - MAX_CAPTURED_OUTPUT_SIZE,
            &output[tail_start..]
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ContextManager;
    use crate::sandbox_types::JobMode;
    use std::ffi::OsString;
    use std::path::Path;
    use tokio::io::AsyncReadExt;

    #[cfg(unix)]
    const PWD_PROBE_SCRIPT: &str = "pwd; printf '%s\\n' \"$PWD\"";

    #[cfg(unix)]
    fn assert_pwd_probe_output(output: &str, workdir: &Path) {
        let expected = workdir.to_string_lossy();
        let lines: Vec<_> = output.lines().collect();
        assert_eq!(lines.len(), 2, "unexpected probe output: {output:?}");
        assert_eq!(lines[0], expected, "pwd output should match workdir");
        assert_eq!(lines[1], expected, "PWD should match workdir");
    }

    #[cfg(unix)]
    fn restore_env_var(name: &str, value: Option<OsString>) {
        match value {
            Some(value) => {
                // SAFETY: test-only, guarded by lock_env().
                unsafe { std::env::set_var(name, value) };
            }
            None => {
                // SAFETY: test-only, guarded by lock_env().
                unsafe { std::env::remove_var(name) };
            }
        }
    }

    #[cfg(unix)]
    struct EnvVarRestore {
        name: &'static str,
        value: Option<OsString>,
    }

    #[cfg(unix)]
    impl EnvVarRestore {
        fn capture(name: &'static str) -> Self {
            Self {
                name,
                value: std::env::var_os(name),
            }
        }
    }

    #[cfg(unix)]
    impl Drop for EnvVarRestore {
        fn drop(&mut self) {
            restore_env_var(self.name, self.value.take());
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn noninteractive_shell_closes_stdin_to_avoid_hanging_commands() {
        let backend = LocalHostExecutionBackend::shared();
        let result = backend
            .run_shell(CommandExecutionRequest {
                command: "cat".to_string(),
                workdir: std::env::current_dir().expect("cwd"),
                timeout: Duration::from_secs(2),
                extra_env: HashMap::new(),
                allow_network: false,
            })
            .await
            .expect("cat should observe EOF and exit");

        assert_eq!(result.exit_code, 0);
        assert!(result.output.trim().is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn local_backend_preserves_split_stdout_and_stderr() {
        let backend = LocalHostExecutionBackend::shared();
        let result = backend
            .run_shell(CommandExecutionRequest {
                command: "printf stdout_line; >&2 printf stderr_line".to_string(),
                workdir: std::env::current_dir().expect("cwd"),
                timeout: Duration::from_secs(2),
                extra_env: HashMap::new(),
                allow_network: false,
            })
            .await
            .expect("command should succeed");

        assert_eq!(result.stdout, "stdout_line");
        assert_eq!(result.stderr, "stderr_line");
        assert!(result.output.contains("stdout_line"));
        assert!(result.output.contains("stderr_line"));
    }

    #[cfg(unix)]
    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn local_backend_run_shell_sets_pwd_from_workdir() {
        let _env_guard = lock_env();
        let _restore_pwd = EnvVarRestore::capture("PWD");
        // SAFETY: test-only, single-threaded tokio runtime, guarded by lock_env().
        unsafe { std::env::set_var("PWD", "/definitely/stale") };

        let backend = LocalHostExecutionBackend::shared();
        let temp_dir = tempfile::tempdir().unwrap();
        let mut extra_env = HashMap::new();
        extra_env.insert("PWD".to_string(), "/also/stale".to_string());
        let result = backend
            .run_shell(CommandExecutionRequest {
                command: PWD_PROBE_SCRIPT.to_string(),
                workdir: temp_dir.path().to_path_buf(),
                timeout: Duration::from_secs(2),
                extra_env,
                allow_network: false,
            })
            .await
            .expect("shell command should succeed");

        assert_pwd_probe_output(&result.stdout, temp_dir.path());
    }

    #[cfg(unix)]
    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn local_backend_run_script_sets_pwd_from_workdir() {
        let _env_guard = lock_env();
        let _restore_pwd = EnvVarRestore::capture("PWD");
        // SAFETY: test-only, single-threaded tokio runtime, guarded by lock_env().
        unsafe { std::env::set_var("PWD", "/definitely/stale") };

        let backend = LocalHostExecutionBackend::shared();
        let temp_dir = tempfile::tempdir().unwrap();
        let mut extra_env = HashMap::new();
        extra_env.insert("PWD".to_string(), "/also/stale".to_string());
        let result = backend
            .run_script(ScriptExecutionRequest {
                program: "sh".to_string(),
                args: vec!["-lc".to_string(), PWD_PROBE_SCRIPT.to_string()],
                workdir: temp_dir.path().to_path_buf(),
                timeout: Duration::from_secs(2),
                extra_env,
                allow_network: false,
            })
            .await
            .expect("script command should succeed");

        assert_pwd_probe_output(&result.stdout, temp_dir.path());
    }

    #[cfg(unix)]
    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn local_backend_start_process_sets_pwd_from_workdir() {
        let _env_guard = lock_env();
        let _restore_pwd = EnvVarRestore::capture("PWD");
        // SAFETY: test-only, single-threaded tokio runtime, guarded by lock_env().
        unsafe { std::env::set_var("PWD", "/definitely/stale") };

        let backend = LocalHostExecutionBackend::shared();
        let temp_dir = tempfile::tempdir().unwrap();
        let mut extra_env = HashMap::new();
        extra_env.insert("PWD".to_string(), "/also/stale".to_string());
        let mut started = backend
            .start_process(ProcessStartRequest {
                command: PWD_PROBE_SCRIPT.to_string(),
                workdir: Some(temp_dir.path().to_path_buf()),
                extra_env,
                kill_on_drop: true,
            })
            .await
            .expect("process should start");

        let mut stdout = String::new();
        started
            .stdout
            .take()
            .expect("stdout should be captured")
            .read_to_string(&mut stdout)
            .await
            .expect("stdout should be readable");
        let status = started.child.wait().await.expect("process should exit");

        assert!(
            status.success(),
            "process should exit successfully: {status:?}"
        );
        assert_pwd_probe_output(&stdout, temp_dir.path());
    }

    #[tokio::test]
    async fn local_backend_supports_job_execution_when_job_orchestration_is_configured() {
        let context_manager = Arc::new(ContextManager::new(5));
        let backend = LocalHostExecutionBackend::with_job_orchestration(Arc::new(
            JobOrchestrationContext::new(context_manager, None, None, None, None, None, None, None),
        ));

        let result = backend
            .run_job(JobExecutionRequest {
                title: "Backend job".to_string(),
                description: "Created through the shared execution backend".to_string(),
                principal_id: "default".to_string(),
                actor_id: "default".to_string(),
                parent_job_id: None,
                wait: false,
                explicit_project_dir: None,
                mode: None,
                metadata: serde_json::json!({}),
                allowed_tools: None,
                allowed_skills: None,
                tool_profile: None,
                credential_grants: Vec::new(),
                job_events_available: false,
                job_prompt_available: false,
                job_status_available: true,
            })
            .await
            .expect("job execution should succeed");

        assert_eq!(result.status, "pending");
        assert_eq!(result.runtime.execution_backend, "local_host");
        assert_eq!(result.runtime.runtime_family, "execution_backend");
        assert_eq!(result.runtime.runtime_mode, "in_memory");
    }

    #[test]
    fn sandbox_job_runtime_descriptor_uses_shared_execution_backend_family() {
        let runtime = sandbox_job_runtime_descriptor(JobMode::CodexCode);
        assert_eq!(runtime.execution_backend, "docker_sandbox");
        assert_eq!(runtime.runtime_family, "execution_backend");
        assert_eq!(runtime.runtime_mode, "codex_code");
        assert!(
            runtime
                .runtime_capabilities
                .contains(&"job_orchestration".to_string())
        );
    }

    #[test]
    fn docker_script_command_maps_workspace_paths() {
        let workdir = PathBuf::from("/tmp/workspace");
        let rendered = render_container_script_command(
            "python3",
            &[
                "/tmp/workspace/.thinclaw_exec_test.py".to_string(),
                "--flag".to_string(),
            ],
            &workdir,
        );

        assert_eq!(
            rendered,
            "'python3' '/workspace/.thinclaw_exec_test.py' '--flag'"
        );
    }

    #[test]
    fn host_local_network_isolation_is_none_when_network_is_allowed() {
        assert_eq!(
            host_local_network_isolation(true),
            NetworkIsolationKind::None
        );
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn local_backend_denies_network_when_requested() {
        let backend = LocalHostExecutionBackend::shared();
        let result = backend
            .run_script(ScriptExecutionRequest {
                program: "python3".to_string(),
                args: vec![
                    "-c".to_string(),
                    "import socket; socket.create_connection(('1.1.1.1', 53), 1)".to_string(),
                ],
                workdir: std::env::current_dir().expect("cwd"),
                timeout: Duration::from_secs(5),
                extra_env: HashMap::new(),
                allow_network: false,
            })
            .await
            .expect("sandboxed invocation should complete");

        assert_ne!(result.exit_code, 0);
        assert!(
            result.stderr.contains("Operation not permitted")
                || result.output.contains("Operation not permitted")
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn local_backend_denies_network_when_requested() {
        if linux_bubblewrap_program().is_none() {
            eprintln!("skipping linux host network-deny test because bubblewrap is unavailable");
            return;
        }

        let backend = LocalHostExecutionBackend::shared();
        let result = backend
            .run_script(ScriptExecutionRequest {
                program: "python3".to_string(),
                args: vec![
                    "-c".to_string(),
                    "import socket; socket.create_connection(('1.1.1.1', 53), 1)".to_string(),
                ],
                workdir: std::env::current_dir().expect("cwd"),
                timeout: Duration::from_secs(5),
                extra_env: HashMap::new(),
                allow_network: false,
            })
            .await
            .expect("sandboxed invocation should complete");

        assert_ne!(result.exit_code, 0);
        assert_eq!(result.runtime.network_isolation.as_deref(), Some("hard"));
    }

    #[tokio::test]
    async fn docker_backend_run_shell_captures_stdout_end_to_end() {
        let mut config = crate::sandbox::SandboxConfig::default();
        config.enabled = true;
        config.policy = crate::sandbox::SandboxPolicy::WorkspaceWrite;
        config.image = "alpine:3.20".to_string();
        let sandbox = Arc::new(crate::sandbox::SandboxManager::new(config));
        if !sandbox.is_available().await {
            eprintln!("skipping docker shell test because sandbox is unavailable");
            return;
        }

        let backend = DockerSandboxExecutionBackend::from_sandbox(
            sandbox,
            crate::sandbox::SandboxPolicy::WorkspaceWrite,
        );
        let temp_dir = tempfile::tempdir().unwrap();
        let result = backend
            .run_shell(CommandExecutionRequest {
                command: "printf '{\"score\":1}\\n' > summary.json && echo benchmark-ok"
                    .to_string(),
                workdir: temp_dir.path().to_path_buf(),
                timeout: Duration::from_secs(60),
                extra_env: HashMap::new(),
                allow_network: false,
            })
            .await
            .expect("docker shell command should succeed");

        assert_eq!(result.exit_code, 0);
        assert!(
            result.output.contains("benchmark-ok"),
            "unexpected docker shell output: {:?}",
            result.output
        );
        assert_eq!(
            std::fs::read_to_string(temp_dir.path().join("summary.json")).unwrap(),
            "{\"score\":1}\n"
        );
    }
}
