use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use crate::agent::Scheduler;
use crate::channels::IncomingMessage;
use crate::channels::web::types::SseEvent;
use crate::context::ContextManager;
use crate::db::Database;
use crate::history::SandboxJobRecord;
use crate::sandbox::{SandboxManager, SandboxPolicy};
use crate::sandbox_jobs::{SandboxChildRegistry, SandboxJobController, SandboxJobSpec};
use crate::sandbox_types::{
    ContainerJobManager, ContainerState, CredentialGrant, JobMode, PromptQueue,
};
use crate::tools::tool::ToolError;

pub use thinclaw_tools::execution::{
    CommandExecutionRequest, ExecutionBackendKind, ExecutionResult, MAX_CAPTURED_OUTPUT_SIZE,
    NetworkIsolationKind, ProcessStartRequest, RuntimeDescriptor, ScriptExecutionRequest,
    StartedProcess, experiment_runner_runtime_descriptor, host_local_network_deny_support,
    host_local_network_isolation, interactive_chat_runtime_descriptor,
    local_job_runtime_descriptor, routine_engine_runtime_descriptor,
    subagent_executor_runtime_descriptor, truncate_output,
};
use thinclaw_tools::execution::{
    LocalExecutionBackend as RootIndependentLocalExecutionBackend,
    LocalHostExecutionBackend as RootIndependentLocalHostExecutionBackend,
};

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
            metadata.insert("parent_job_id".to_string(), json!(parent_job_id));
        }
        if let Some(allowed_tools) = request.allowed_tools.as_ref() {
            metadata.insert("allowed_tools".to_string(), json!(allowed_tools));
        }
        if let Some(allowed_skills) = request.allowed_skills.as_ref() {
            metadata.insert("allowed_skills".to_string(), json!(allowed_skills));
        }
        if let Some(tool_profile) = request.tool_profile.as_ref() {
            metadata.insert("tool_profile".to_string(), json!(tool_profile));
        }
        metadata.insert(
            "execution_backend".to_string(),
            json!(runtime.execution_backend),
        );
        metadata.insert("runtime_family".to_string(), json!(runtime.runtime_family));
        metadata.insert("runtime_mode".to_string(), json!(runtime.runtime_mode));
        metadata.insert(
            "runtime_capabilities".to_string(),
            json!(runtime.runtime_capabilities),
        );
        metadata.insert(
            "network_isolation".to_string(),
            json!(runtime.network_isolation),
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

pub struct LocalHostExecutionBackend {
    inner: Arc<dyn RootIndependentLocalExecutionBackend>,
    job_orchestration: Option<Arc<JobOrchestrationContext>>,
}

impl LocalHostExecutionBackend {
    pub fn shared() -> Arc<dyn ExecutionBackend> {
        Arc::new(Self {
            inner: RootIndependentLocalHostExecutionBackend::shared(),
            job_orchestration: None,
        })
    }

    pub fn with_job_orchestration(
        job_orchestration: Arc<JobOrchestrationContext>,
    ) -> Arc<dyn ExecutionBackend> {
        Arc::new(Self {
            inner: RootIndependentLocalHostExecutionBackend::shared(),
            job_orchestration: Some(job_orchestration),
        })
    }
}

#[async_trait]
impl ExecutionBackend for LocalHostExecutionBackend {
    fn kind(&self) -> ExecutionBackendKind {
        self.inner.kind()
    }

    async fn run_shell(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<ExecutionResult, ToolError> {
        self.inner.run_shell(request).await
    }

    async fn start_process(
        &self,
        request: ProcessStartRequest,
    ) -> Result<StartedProcess, ToolError> {
        self.inner.start_process(request).await
    }

    async fn run_script(
        &self,
        request: ScriptExecutionRequest,
    ) -> Result<ExecutionResult, ToolError> {
        self.inner.run_script(request).await
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
