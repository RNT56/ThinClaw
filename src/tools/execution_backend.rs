use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use uuid::Uuid;

use crate::agent::Scheduler;
use crate::channels::IncomingMessage;
use crate::channels::web::types::SseEvent;
use crate::context::ContextManager;
use crate::db::Database;
use crate::history::SandboxJobRecord;
use crate::sandbox::{SandboxManager, SandboxPolicy};
use crate::sandbox_jobs::{SandboxChildRegistry, SandboxJobController};
use crate::sandbox_types::{ContainerJobManager, ContainerState, JobMode, PromptQueue};
use crate::tools::tool::ToolError;

pub use thinclaw_tools::execution::{
    CommandExecutionRequest, ExecutionBackend, ExecutionBackendKind, ExecutionResult,
    JobExecutionOrchestrator, JobExecutionRequest, JobExecutionResult, MAX_CAPTURED_OUTPUT_SIZE,
    NetworkIsolationKind, ProcessStartRequest, RuntimeDescriptor, ScriptExecutionRequest,
    StartedProcess, build_local_job_metadata, build_sandbox_job_spec,
    credential_grants_restart_json, experiment_runner_runtime_descriptor,
    host_local_network_deny_support, host_local_network_isolation,
    interactive_chat_runtime_descriptor, local_job_runtime_descriptor,
    render_container_script_command, resolve_project_dir, routine_engine_runtime_descriptor,
    sandbox_job_runtime_descriptor, subagent_executor_runtime_descriptor, truncate_output,
};
use thinclaw_tools::execution::{
    LocalHostExecutionBackendWithJobs as RootIndependentLocalHostExecutionBackendWithJobs,
    SandboxJobExecutionBackend as RootIndependentSandboxJobExecutionBackend,
};

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
        let metadata = build_local_job_metadata(&request);

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
            return Ok(JobExecutionResult::local_started(job_id, &request.title));
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
        Ok(JobExecutionResult::local_pending(job_id, &request.title))
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
        let explicit_project_dir = request.explicit_project_dir.clone();
        let (project_dir, browse_id) = resolve_project_dir(explicit_project_dir, job_id)?;
        let project_dir_str = project_dir.display().to_string();
        let spec = build_sandbox_job_spec(
            &request,
            mode,
            project_dir_str.clone(),
            job_manager.interactive_idle_timeout_secs(),
        );

        let credential_grants_json = match credential_grants_restart_json(
            &request.credential_grants,
        ) {
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
            .create_job(job_id, spec.clone(), request.credential_grants.clone())
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
            return Ok(JobExecutionResult::sandbox_started(
                job_id,
                mode,
                &request,
                project_dir_str,
                &browse_id,
            ));
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
                            return Ok(JobExecutionResult::sandbox_completed(
                                job_id,
                                mode,
                                message,
                                project_dir_str,
                                &browse_id,
                            ));
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
                    return Ok(JobExecutionResult::sandbox_completed(
                        job_id,
                        mode,
                        "Container job completed".to_string(),
                        project_dir_str,
                        &browse_id,
                    ));
                }
            }
        }
    }
}

#[async_trait]
impl JobExecutionOrchestrator for JobOrchestrationContext {
    async fn run_local_job(
        &self,
        request: JobExecutionRequest,
    ) -> Result<JobExecutionResult, ToolError> {
        JobOrchestrationContext::run_local_job(self, request).await
    }

    async fn run_sandbox_job(
        &self,
        request: JobExecutionRequest,
    ) -> Result<JobExecutionResult, ToolError> {
        JobOrchestrationContext::run_sandbox_job(self, request).await
    }
}

pub struct LocalHostExecutionBackend {
    inner: Arc<dyn ExecutionBackend>,
}

impl LocalHostExecutionBackend {
    pub fn shared() -> Arc<dyn ExecutionBackend> {
        Arc::new(Self {
            inner: RootIndependentLocalHostExecutionBackendWithJobs::shared(),
        })
    }

    pub fn with_job_orchestration(
        job_orchestration: Arc<JobOrchestrationContext>,
    ) -> Arc<dyn ExecutionBackend> {
        Arc::new(Self {
            inner: RootIndependentLocalHostExecutionBackendWithJobs::with_job_orchestration(
                job_orchestration,
            ),
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
        self.inner.run_job(request).await
    }
}

pub struct DockerSandboxExecutionBackend {
    sandbox: Option<Arc<SandboxManager>>,
    policy: Option<SandboxPolicy>,
    job_backend: Option<Arc<dyn ExecutionBackend>>,
}

impl DockerSandboxExecutionBackend {
    pub fn from_sandbox(
        sandbox: Arc<SandboxManager>,
        policy: SandboxPolicy,
    ) -> Arc<dyn ExecutionBackend> {
        Arc::new(Self {
            sandbox: Some(sandbox),
            policy: Some(policy),
            job_backend: None,
        })
    }

    pub fn with_job_orchestration(
        job_orchestration: Arc<JobOrchestrationContext>,
    ) -> Arc<dyn ExecutionBackend> {
        Arc::new(Self {
            sandbox: None,
            policy: None,
            job_backend: Some(
                RootIndependentSandboxJobExecutionBackend::with_job_orchestration(
                    job_orchestration,
                ),
            ),
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
        let job_backend = self.job_backend.as_ref().ok_or_else(|| {
            ToolError::ExecutionFailed(
                "docker sandbox backend is not configured for job execution".to_string(),
            )
        })?;
        job_backend.run_job(request).await
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
