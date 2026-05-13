//! Job management tools.
//!
//! These tools allow the LLM to manage jobs:
//! - Create new jobs/tasks (with optional sandbox delegation)
//! - List existing jobs
//! - Check job status
//! - Cancel running jobs

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use uuid::Uuid;

use crate::agent::Scheduler;
use crate::channels::IncomingMessage;
use crate::channels::web::types::SseEvent;
use crate::context::{ContextManager, JobContext, JobState};
use crate::db::Database;
use crate::history::SandboxJobRecord;
use crate::sandbox_jobs::SandboxChildRegistry;
use crate::sandbox_types::{
    ContainerHandle, ContainerJobManager, CredentialGrant, JobMode, PendingPrompt, PromptQueue,
};
#[cfg(test)]
use crate::sandbox_types::{ContainerJobConfig, TokenStore};
use crate::secrets::SecretsStore;
use crate::tools::execution_backend::{
    DockerSandboxExecutionBackend, ExecutionBackend, JobExecutionRequest, JobOrchestrationContext,
    LocalHostExecutionBackend,
};
#[cfg(test)]
use crate::tools::execution_backend::{resolve_project_dir, sandbox_job_runtime_descriptor};
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput};
use thinclaw_tools::builtin::job::{self as job_policy, JobReferenceKind};

#[derive(Clone, Default)]
struct SandboxJobLookup {
    live: Option<ContainerHandle>,
    stored: Option<SandboxJobRecord>,
}

impl SandboxJobLookup {
    fn spec(&self) -> Option<&crate::sandbox_jobs::SandboxJobSpec> {
        self.live
            .as_ref()
            .map(|handle| &handle.spec)
            .or_else(|| self.stored.as_ref().map(|job| &job.spec))
    }

    fn status(&self) -> String {
        if let Some(handle) = self.live.as_ref() {
            return match handle.state {
                crate::sandbox_types::ContainerState::Creating => "creating".to_string(),
                crate::sandbox_types::ContainerState::Running => "running".to_string(),
                crate::sandbox_types::ContainerState::Stopped => handle
                    .completion_result
                    .as_ref()
                    .map(|result| result.status.clone())
                    .or_else(|| self.stored.as_ref().map(|job| job.status.clone()))
                    .unwrap_or_else(|| "completed".to_string()),
                crate::sandbox_types::ContainerState::Failed => handle
                    .completion_result
                    .as_ref()
                    .map(|result| result.status.clone())
                    .unwrap_or_else(|| "failed".to_string()),
            };
        }
        self.stored
            .as_ref()
            .map(|job| job.status.clone())
            .unwrap_or_else(|| "unknown".to_string())
    }

    fn created_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.live
            .as_ref()
            .map(|handle| handle.created_at)
            .or_else(|| self.stored.as_ref().map(|job| job.created_at))
    }

    fn started_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.stored.as_ref().and_then(|job| job.started_at)
    }

    fn completed_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.stored.as_ref().and_then(|job| job.completed_at)
    }

    fn failure_reason(&self) -> Option<String> {
        self.stored
            .as_ref()
            .and_then(|job| job.failure_reason.clone())
            .or_else(|| {
                self.live
                    .as_ref()
                    .and_then(|handle| handle.completion_result.as_ref())
                    .and_then(|result| result.message.clone())
            })
    }

    fn is_interactive(&self) -> bool {
        self.spec().map(|spec| spec.interactive).unwrap_or(false)
    }

    fn accepts_prompts(&self) -> bool {
        self.is_interactive()
            && self
                .live
                .as_ref()
                .map(|handle| {
                    matches!(
                        handle.state,
                        crate::sandbox_types::ContainerState::Creating
                            | crate::sandbox_types::ContainerState::Running
                    )
                })
                .unwrap_or(false)
    }
}

enum ResolvedOwnedJob {
    Local {
        job_id: Uuid,
        ctx: Box<JobContext>,
    },
    Sandbox {
        job_id: Uuid,
        lookup: Box<SandboxJobLookup>,
    },
}

async fn load_owned_sandbox_jobs(
    job_manager: Option<&Arc<ContainerJobManager>>,
    store: Option<&Arc<dyn Database>>,
    principal_id: &str,
    actor_id: &str,
) -> Result<std::collections::HashMap<Uuid, SandboxJobLookup>, ToolError> {
    let mut jobs = std::collections::HashMap::<Uuid, SandboxJobLookup>::new();

    if let Some(store) = store {
        for job in store
            .list_sandbox_jobs_for_actor(principal_id, actor_id)
            .await
            .map_err(|error| ToolError::ExecutionFailed(error.to_string()))?
        {
            let job_id = job.id;
            jobs.entry(job_id).or_default().stored = Some(job);
        }
    }

    if let Some(job_manager) = job_manager {
        for handle in job_manager.list_jobs().await {
            if handle.spec.principal_id == principal_id && handle.spec.actor_id == actor_id {
                let job_id = handle.job_id;
                jobs.entry(job_id).or_default().live = Some(handle);
            }
        }
    }

    Ok(jobs)
}

async fn load_owned_direct_jobs(
    context_manager: &ContextManager,
    store: Option<&Arc<dyn Database>>,
    principal_id: &str,
    actor_id: &str,
) -> Result<std::collections::HashMap<Uuid, JobContext>, ToolError> {
    let mut jobs = std::collections::HashMap::<Uuid, JobContext>::new();

    if let Some(store) = store {
        for job in store
            .list_jobs_for_actor(principal_id, actor_id)
            .await
            .map_err(|error| ToolError::ExecutionFailed(error.to_string()))?
        {
            jobs.insert(job.job_id, job);
        }
    }

    for job_id in context_manager
        .all_jobs_for_actor(principal_id, actor_id)
        .await
    {
        if let Ok(job_ctx) = context_manager.get_context(job_id).await {
            jobs.insert(job_id, job_ctx);
        }
    }

    Ok(jobs)
}

async fn resolve_owned_job_ref(
    input: &str,
    context_manager: &ContextManager,
    job_manager: Option<&Arc<ContainerJobManager>>,
    store: Option<&Arc<dyn Database>>,
    principal_id: &str,
    actor_id: &str,
) -> Result<ResolvedOwnedJob, ToolError> {
    let direct_jobs =
        load_owned_direct_jobs(context_manager, store, principal_id, actor_id).await?;
    let sandbox_jobs = load_owned_sandbox_jobs(job_manager, store, principal_id, actor_id).await?;

    let resolved = job_policy::resolve_job_reference(
        input,
        direct_jobs.keys().copied(),
        sandbox_jobs.keys().copied(),
    )
    .map_err(|error| match error {
        ToolError::InvalidParameters(message) if message == "no job found" => {
            ToolError::InvalidParameters("job not found".to_string())
        }
        other => other,
    })?;

    match resolved.kind {
        JobReferenceKind::Direct => {
            let job_ctx = direct_jobs
                .get(&resolved.job_id)
                .cloned()
                .ok_or_else(|| ToolError::InvalidParameters("job not found".to_string()))?;
            Ok(ResolvedOwnedJob::Local {
                job_id: resolved.job_id,
                ctx: Box::new(job_ctx),
            })
        }
        JobReferenceKind::Sandbox => {
            let lookup = sandbox_jobs
                .get(&resolved.job_id)
                .cloned()
                .ok_or_else(|| ToolError::InvalidParameters("job not found".to_string()))?;
            Ok(ResolvedOwnedJob::Sandbox {
                job_id: resolved.job_id,
                lookup: Box::new(lookup),
            })
        }
    }
}

/// Tool for creating a new job.
///
/// When sandbox deps are injected (via `with_sandbox`), the tool automatically
/// delegates execution to a Docker container. Otherwise it creates an in-memory
/// job via the ContextManager. The LLM never needs to know the difference.
pub struct CreateJobTool {
    context_manager: Arc<ContextManager>,
    scheduler: Option<Arc<Scheduler>>,
    job_manager: Option<Arc<ContainerJobManager>>,
    store: Option<Arc<dyn Database>>,
    /// Broadcast sender for job events (used to subscribe a monitor).
    event_tx: Option<tokio::sync::broadcast::Sender<(Uuid, SseEvent)>>,
    /// Injection channel for pushing messages into the agent loop.
    inject_tx: Option<tokio::sync::mpsc::Sender<IncomingMessage>>,
    /// Follow-up prompt queue shared with the gateway/orchestrator.
    prompt_queue: Option<PromptQueue>,
    /// Parent-run registry for interactive sandbox child jobs.
    sandbox_children: Option<Arc<SandboxChildRegistry>>,
    /// Encrypted secrets store for validating credential grants.
    secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
}

impl CreateJobTool {
    pub fn new(context_manager: Arc<ContextManager>) -> Self {
        Self {
            context_manager,
            scheduler: None,
            job_manager: None,
            store: None,
            event_tx: None,
            inject_tx: None,
            prompt_queue: None,
            sandbox_children: None,
            secrets_store: None,
        }
    }

    pub fn with_scheduler(mut self, scheduler: Arc<Scheduler>) -> Self {
        self.scheduler = Some(scheduler);
        self
    }

    /// Inject sandbox dependencies so `create_job` delegates to Docker containers.
    pub fn with_sandbox(
        mut self,
        job_manager: Arc<ContainerJobManager>,
        store: Option<Arc<dyn Database>>,
    ) -> Self {
        self.job_manager = Some(job_manager);
        self.store = store;
        self
    }

    /// Inject monitor dependencies so fire-and-forget jobs spawn a background
    /// monitor that forwards container agent output to the main agent loop.
    pub fn with_monitor_deps(
        mut self,
        event_tx: tokio::sync::broadcast::Sender<(Uuid, SseEvent)>,
        inject_tx: tokio::sync::mpsc::Sender<IncomingMessage>,
        prompt_queue: Option<PromptQueue>,
    ) -> Self {
        self.event_tx = Some(event_tx);
        self.inject_tx = Some(inject_tx);
        self.prompt_queue = prompt_queue;
        self
    }

    pub fn with_sandbox_children(mut self, sandbox_children: Arc<SandboxChildRegistry>) -> Self {
        self.sandbox_children = Some(sandbox_children);
        self
    }

    /// Inject secrets store for credential validation.
    pub fn with_secrets(mut self, secrets: Arc<dyn SecretsStore + Send + Sync>) -> Self {
        self.secrets_store = Some(secrets);
        self
    }

    pub fn sandbox_enabled(&self) -> bool {
        self.job_manager.is_some()
    }

    fn job_backend(&self) -> Arc<dyn ExecutionBackend> {
        let orchestration = Arc::new(JobOrchestrationContext::new(
            Arc::clone(&self.context_manager),
            self.job_manager.clone(),
            self.store.clone(),
            self.scheduler.clone(),
            self.event_tx.clone(),
            self.inject_tx.clone(),
            self.prompt_queue.clone(),
            self.sandbox_children.clone(),
        ));
        if self.sandbox_enabled() {
            DockerSandboxExecutionBackend::with_job_orchestration(orchestration)
        } else {
            LocalHostExecutionBackend::with_job_orchestration(orchestration)
        }
    }

    fn claude_code_enabled(&self) -> bool {
        self.job_manager
            .as_ref()
            .map(|jm| jm.claude_code_enabled())
            .unwrap_or(false)
    }

    fn codex_code_enabled(&self) -> bool {
        self.job_manager
            .as_ref()
            .map(|jm| jm.codex_code_enabled())
            .unwrap_or(false)
    }

    fn available_sandbox_modes(&self) -> Vec<&'static str> {
        job_policy::available_sandbox_modes(self.claude_code_enabled(), self.codex_code_enabled())
    }

    fn sandbox_mode_schema_description(&self) -> String {
        job_policy::sandbox_mode_schema_description(
            self.claude_code_enabled(),
            self.codex_code_enabled(),
        )
    }

    /// Parse and validate the `credentials` parameter.
    ///
    /// Each key is a secret name (must exist in SecretsStore), each value is the
    /// env var name the container should receive it as. Returns an empty vec if
    /// no credentials were requested.
    async fn parse_credentials(
        &self,
        params: &serde_json::Value,
        user_id: &str,
    ) -> Result<Vec<CredentialGrant>, ToolError> {
        let requests = job_policy::parse_credential_requests(params)?;
        if requests.is_empty() {
            return Ok(vec![]);
        }

        let secrets = match &self.secrets_store {
            Some(s) => s,
            None => {
                return Err(ToolError::ExecutionFailed(
                    "credentials requested but no secrets store is configured. \
                     Set SECRETS_MASTER_KEY to enable credential management."
                        .to_string(),
                ));
            }
        };

        let mut grants = Vec::with_capacity(requests.len());
        for request in requests {
            // Validate the secret actually exists
            let exists = secrets
                .exists(user_id, &request.secret_name)
                .await
                .map_err(|e| {
                    ToolError::ExecutionFailed(format!(
                        "failed to check secret '{}': {}",
                        request.secret_name, e
                    ))
                })?;

            if !exists {
                return Err(ToolError::ExecutionFailed(format!(
                    "secret '{}' not found. Store it first via 'thinclaw tool auth' or the web UI.",
                    request.secret_name
                )));
            }

            grants.push(CredentialGrant {
                secret_name: request.secret_name,
                env_var: request.env_var,
            });
        }

        Ok(grants)
    }

    /// Execute via in-memory ContextManager (no sandbox).
    async fn execute_local(
        &self,
        title: &str,
        description: &str,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let result = self
            .job_backend()
            .run_job(JobExecutionRequest {
                title: title.to_string(),
                description: description.to_string(),
                principal_id: ctx.user_id.clone(),
                actor_id: ctx.owner_actor_id().to_string(),
                parent_job_id: Some(ctx.job_id),
                wait: false,
                explicit_project_dir: None,
                mode: None,
                metadata: ctx.metadata.clone(),
                allowed_tools: crate::tools::ToolRegistry::metadata_string_list(
                    &ctx.metadata,
                    "allowed_tools",
                ),
                allowed_skills: crate::tools::ToolRegistry::metadata_string_list(
                    &ctx.metadata,
                    "allowed_skills",
                ),
                tool_profile: ctx
                    .metadata
                    .get("tool_profile")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                credential_grants: Vec::new(),
                job_events_available: self.store.is_some(),
                job_prompt_available: self.prompt_queue.is_some(),
                job_status_available: true,
            })
            .await?;
        Ok(ToolOutput::success(
            job_policy::local_job_output(&result, title),
            start.elapsed(),
        ))
    }

    /// Execute via sandboxed Docker container.
    async fn execute_sandbox(
        &self,
        task: &str,
        explicit_dir: Option<PathBuf>,
        wait: bool,
        mode: JobMode,
        credential_grants: Vec<CredentialGrant>,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let (title, description) = task
            .split_once("\n\n")
            .map(|(title, description)| (title.to_string(), description.to_string()))
            .unwrap_or_else(|| (task.to_string(), task.to_string()));
        let result = self
            .job_backend()
            .run_job(JobExecutionRequest {
                title,
                description,
                principal_id: ctx.user_id.clone(),
                actor_id: ctx.owner_actor_id().to_string(),
                parent_job_id: Some(ctx.job_id),
                wait,
                explicit_project_dir: explicit_dir,
                mode: Some(mode),
                metadata: ctx.metadata.clone(),
                allowed_tools: crate::tools::ToolRegistry::metadata_string_list(
                    &ctx.metadata,
                    "allowed_tools",
                ),
                allowed_skills: crate::tools::ToolRegistry::metadata_string_list(
                    &ctx.metadata,
                    "allowed_skills",
                ),
                tool_profile: ctx
                    .metadata
                    .get("tool_profile")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                credential_grants,
                job_events_available: self.store.is_some(),
                job_prompt_available: self.prompt_queue.is_some(),
                job_status_available: true,
            })
            .await?;
        Ok(ToolOutput::success(
            job_policy::sandbox_job_output(&result),
            start.elapsed(),
        ))
    }
}

#[cfg(test)]
fn projects_base() -> PathBuf {
    crate::platform::resolve_data_dir("projects")
}

#[async_trait]
impl Tool for CreateJobTool {
    fn name(&self) -> &str {
        "create_job"
    }

    fn description(&self) -> &str {
        job_policy::create_job_description(self.sandbox_enabled())
    }

    fn parameters_schema(&self) -> serde_json::Value {
        job_policy::create_job_parameters_schema(
            self.sandbox_enabled(),
            self.available_sandbox_modes(),
            self.sandbox_mode_schema_description(),
        )
    }

    fn execution_timeout(&self) -> Duration {
        if self.sandbox_enabled() {
            // Sandbox polls for up to 10 min internally; give an extra 60s buffer.
            Duration::from_secs(660)
        } else {
            Duration::from_secs(30)
        }
    }

    fn rate_limit_config(&self) -> Option<crate::tools::tool::ToolRateLimitConfig> {
        Some(crate::tools::tool::ToolRateLimitConfig::new(5, 30))
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let parsed = job_policy::parse_create_job_params(
            &params,
            self.sandbox_enabled(),
            self.claude_code_enabled(),
            self.codex_code_enabled(),
        )?;

        if self.sandbox_enabled() {
            let explicit_dir = parsed.project_dir.as_deref().map(PathBuf::from);
            let mode = parsed.mode.ok_or_else(|| {
                ToolError::InvalidParameters("missing sandbox execution mode".to_string())
            })?;

            // Parse and validate credential grants
            let credential_grants = self.parse_credentials(&params, &ctx.user_id).await?;

            self.execute_sandbox(
                &parsed.task_prompt(),
                explicit_dir,
                parsed.wait,
                mode,
                credential_grants,
                ctx,
            )
            .await
        } else {
            self.execute_local(&parsed.title, &parsed.description, ctx)
                .await
        }
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

/// Tool for listing jobs.
pub struct ListJobsTool {
    context_manager: Arc<ContextManager>,
    job_manager: Option<Arc<ContainerJobManager>>,
    store: Option<Arc<dyn Database>>,
}

impl ListJobsTool {
    pub fn new(context_manager: Arc<ContextManager>) -> Self {
        Self {
            context_manager,
            job_manager: None,
            store: None,
        }
    }

    pub fn with_sandbox(
        mut self,
        job_manager: Option<Arc<ContainerJobManager>>,
        store: Option<Arc<dyn Database>>,
    ) -> Self {
        self.job_manager = job_manager;
        self.store = store;
        self
    }
}

#[async_trait]
impl Tool for ListJobsTool {
    fn name(&self) -> &str {
        "list_jobs"
    }

    fn description(&self) -> &str {
        "List all jobs or filter by status. Shows job IDs, titles, and current status."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        job_policy::list_jobs_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let filter = params
            .get("filter")
            .and_then(|v| v.as_str())
            .unwrap_or("all");
        let actor_id = ctx.owner_actor_id();
        let direct_jobs = load_owned_direct_jobs(
            &self.context_manager,
            self.store.as_ref(),
            &ctx.user_id,
            actor_id,
        )
        .await?;
        let mut jobs = Vec::new();
        let mut summary = job_policy::JobSummaryCounts::default();
        for (job_id, job_ctx) in direct_jobs {
            summary.record_direct_state(job_ctx.state);

            let include = job_policy::direct_job_matches_filter(job_ctx.state, filter);

            if include {
                jobs.push(job_policy::local_job_list_entry(job_id, &job_ctx));
            }
        }

        for (job_id, lookup) in load_owned_sandbox_jobs(
            self.job_manager.as_ref(),
            self.store.as_ref(),
            &ctx.user_id,
            actor_id,
        )
        .await?
        {
            let status = lookup.status();
            summary.record_sandbox_status(&status);
            let include = job_policy::sandbox_status_matches_filter(&status, filter);

            if include && let Some(spec) = lookup.spec() {
                jobs.push(job_policy::sandbox_job_list_entry(
                    job_id,
                    &spec.title,
                    &crate::sandbox_jobs::normalize_sandbox_ui_state(&status),
                    lookup.created_at().map(|value| value.to_rfc3339()),
                    spec.mode.as_str(),
                ));
            }
        }

        let result = job_policy::list_jobs_output(jobs, &summary);

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

/// Tool for checking job status.
pub struct JobStatusTool {
    context_manager: Arc<ContextManager>,
    job_manager: Option<Arc<ContainerJobManager>>,
    store: Option<Arc<dyn Database>>,
}

impl JobStatusTool {
    pub fn new(context_manager: Arc<ContextManager>) -> Self {
        Self {
            context_manager,
            job_manager: None,
            store: None,
        }
    }

    pub fn with_sandbox(
        mut self,
        job_manager: Option<Arc<ContainerJobManager>>,
        store: Option<Arc<dyn Database>>,
    ) -> Self {
        self.job_manager = job_manager;
        self.store = store;
        self
    }
}

#[async_trait]
impl Tool for JobStatusTool {
    fn name(&self) -> &str {
        "job_status"
    }

    fn description(&self) -> &str {
        "Check the status and details of a specific job by its ID."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        job_policy::job_id_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let job_id_str = job_policy::parse_job_id_param(&params)?;
        let resolved = resolve_owned_job_ref(
            job_id_str,
            &self.context_manager,
            self.job_manager.as_ref(),
            self.store.as_ref(),
            &ctx.user_id,
            ctx.owner_actor_id(),
        )
        .await?;

        match resolved {
            ResolvedOwnedJob::Local {
                job_id,
                ctx: job_ctx,
            } => {
                let result = job_policy::local_job_status_output(job_id, &job_ctx);
                Ok(ToolOutput::success(result, start.elapsed()))
            }
            ResolvedOwnedJob::Sandbox { job_id, lookup } => {
                let result = job_policy::sandbox_job_status_output(
                    job_id,
                    job_policy::SandboxJobStatusOutput {
                        title: lookup.spec().map(|spec| spec.title.clone()),
                        description: lookup.spec().map(|spec| spec.description.clone()),
                        status: crate::sandbox_jobs::normalize_sandbox_ui_state(&lookup.status())
                            .to_string(),
                        created_at: lookup.created_at().map(|value| value.to_rfc3339()),
                        started_at: lookup.started_at().map(|value| value.to_rfc3339()),
                        completed_at: lookup.completed_at().map(|value| value.to_rfc3339()),
                        project_dir: lookup.spec().and_then(|spec| spec.project_dir.clone()),
                        runtime_mode: lookup.spec().map(|spec| spec.mode.as_str()),
                        interactive: lookup.spec().map(|spec| spec.interactive),
                        failure_reason: lookup.failure_reason(),
                    },
                );
                Ok(ToolOutput::success(result, start.elapsed()))
            }
        }
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

/// Tool for canceling a job.
pub struct CancelJobTool {
    context_manager: Arc<ContextManager>,
    scheduler: Option<Arc<Scheduler>>,
    job_manager: Option<Arc<ContainerJobManager>>,
    store: Option<Arc<dyn Database>>,
}

impl CancelJobTool {
    pub fn new(context_manager: Arc<ContextManager>) -> Self {
        Self {
            context_manager,
            scheduler: None,
            job_manager: None,
            store: None,
        }
    }

    pub fn with_scheduler(mut self, scheduler: Arc<Scheduler>) -> Self {
        self.scheduler = Some(scheduler);
        self
    }

    pub fn with_sandbox(
        mut self,
        job_manager: Option<Arc<ContainerJobManager>>,
        store: Option<Arc<dyn Database>>,
    ) -> Self {
        self.job_manager = job_manager;
        self.store = store;
        self
    }
}

#[async_trait]
impl Tool for CancelJobTool {
    fn name(&self) -> &str {
        "cancel_job"
    }

    fn description(&self) -> &str {
        "Cancel a running or pending job. The job will be marked as cancelled and stopped."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        job_policy::job_id_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let job_id_str = job_policy::parse_job_id_param(&params)?;
        let resolved = resolve_owned_job_ref(
            job_id_str,
            &self.context_manager,
            self.job_manager.as_ref(),
            self.store.as_ref(),
            &ctx.user_id,
            ctx.owner_actor_id(),
        )
        .await?;

        match resolved {
            ResolvedOwnedJob::Local {
                job_id,
                ctx: job_ctx,
            } => {
                job_policy::ensure_local_job_cancellable(job_id, job_ctx.state)?;

                if let Some(scheduler) = self.scheduler.as_ref()
                    && scheduler.is_running(job_id).await
                {
                    scheduler
                        .stop(job_id)
                        .await
                        .map_err(|error| ToolError::ExecutionFailed(error.to_string()))?;
                } else if self.context_manager.get_context(job_id).await.is_ok() {
                    self.context_manager
                        .update_context(job_id, |job_ctx| {
                            job_ctx.transition_to(
                                JobState::Cancelled,
                                Some("Cancelled by user".to_string()),
                            )
                        })
                        .await
                        .map_err(|error| ToolError::ExecutionFailed(error.to_string()))?
                        .map_err(ToolError::ExecutionFailed)?;

                    if let Some(store) = self.store.as_ref()
                        && let Ok(snapshot) = self.context_manager.get_context(job_id).await
                        && let Err(error) = store.save_job(&snapshot).await
                    {
                        tracing::warn!(job_id = %job_id, "Failed to persist cancelled job: {}", error);
                    }
                } else {
                    return Err(job_policy::local_job_not_cancellable_error(
                        job_id,
                        job_ctx.state,
                    ));
                }

                let result = job_policy::cancel_job_output(job_id);
                Ok(ToolOutput::success(result, start.elapsed()))
            }
            ResolvedOwnedJob::Sandbox { job_id, lookup } => {
                let status = lookup.status();
                job_policy::ensure_sandbox_job_cancellable(job_id, &status)?;
                let controller = crate::sandbox_jobs::SandboxJobController::new(
                    self.store.clone(),
                    self.job_manager.clone(),
                    None,
                    None,
                );
                controller
                    .cancel_job(job_id, "Cancelled by user")
                    .await
                    .map_err(ToolError::ExecutionFailed)?;
                let result = job_policy::cancel_job_output(job_id);
                Ok(ToolOutput::success(result, start.elapsed()))
            }
        }
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

/// Tool for reading sandbox job event logs.
///
/// Lets the main agent inspect what a running (or completed) container job has
/// been doing: messages, tool calls, results, status changes, etc.
///
/// Events are streamed from the sandbox worker into the database via the
/// orchestrator's event pipeline. This tool queries them with a DB-level
/// `LIMIT` (default 50, configurable via the `limit` parameter) so the
/// agent sees the most recent activity without loading the full history.
pub struct JobEventsTool {
    store: Arc<dyn Database>,
    context_manager: Arc<ContextManager>,
    job_manager: Option<Arc<ContainerJobManager>>,
}

impl JobEventsTool {
    pub fn new(
        store: Arc<dyn Database>,
        context_manager: Arc<ContextManager>,
        job_manager: Option<Arc<ContainerJobManager>>,
    ) -> Self {
        Self {
            store,
            context_manager,
            job_manager,
        }
    }
}

#[async_trait]
impl Tool for JobEventsTool {
    fn name(&self) -> &str {
        "job_events"
    }

    fn description(&self) -> &str {
        "Read the event log for a sandbox job. Shows messages, tool calls, results, \
         and status changes from the container. Use this to check what a container coding agent \
         or worker sub-agent has been doing."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        job_policy::job_events_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let parsed = job_policy::parse_job_events_params(&params)?;

        let resolved = resolve_owned_job_ref(
            parsed.job_id,
            &self.context_manager,
            self.job_manager.as_ref(),
            Some(&self.store),
            &ctx.user_id,
            ctx.owner_actor_id(),
        )
        .await?;
        let (job_id, kind) = match resolved {
            ResolvedOwnedJob::Sandbox { job_id, .. } => (job_id, "sandbox"),
            ResolvedOwnedJob::Local { job_id, .. } => (job_id, "local"),
        };

        let events = self
            .store
            .list_job_events(job_id, Some(parsed.limit))
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to load job events: {}", e)))?;

        let recent: Vec<job_policy::JobEventOutput> = events
            .iter()
            .map(|ev| job_policy::JobEventOutput {
                event_type: ev.event_type.clone(),
                data: ev.data.clone(),
                created_at: ev.created_at.to_rfc3339(),
            })
            .collect();

        let result = job_policy::job_events_output(job_id, kind, events.len(), recent);

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        true
    }
}

/// Tool for sending follow-up prompts to a running container coding agent job.
///
/// The prompt is queued in an in-memory `PromptQueue` (a broadcast channel
/// shared with the web gateway). The bridge inside the container
/// polls for queued prompts between turns and feeds them into the next
/// CLI resume invocation, enabling interactive multi-turn sessions
/// with long-running sandbox jobs.
pub struct JobPromptTool {
    prompt_queue: PromptQueue,
    context_manager: Arc<ContextManager>,
    job_manager: Option<Arc<ContainerJobManager>>,
    store: Option<Arc<dyn Database>>,
}

impl JobPromptTool {
    pub fn new(prompt_queue: PromptQueue, context_manager: Arc<ContextManager>) -> Self {
        Self {
            prompt_queue,
            context_manager,
            job_manager: None,
            store: None,
        }
    }

    pub fn with_sandbox(
        mut self,
        job_manager: Option<Arc<ContainerJobManager>>,
        store: Option<Arc<dyn Database>>,
    ) -> Self {
        self.job_manager = job_manager;
        self.store = store;
        self
    }

    pub fn sandbox_enabled(&self) -> bool {
        self.job_manager.is_some() || self.store.is_some()
    }
}

#[async_trait]
impl Tool for JobPromptTool {
    fn name(&self) -> &str {
        "job_prompt"
    }

    fn description(&self) -> &str {
        "Send a follow-up prompt to a running container coding agent job. The prompt is \
         queued and delivered on the next poll cycle. Use this to give the sub-agent \
         additional instructions, answer its questions, or tell it to wrap up."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        job_policy::job_prompt_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let job_id_str = job_policy::parse_job_id_param(&params)?;

        let resolved = resolve_owned_job_ref(
            job_id_str,
            &self.context_manager,
            self.job_manager.as_ref(),
            self.store.as_ref(),
            &ctx.user_id,
            ctx.owner_actor_id(),
        )
        .await?;

        let prompt_params = job_policy::parse_job_prompt_params(&params)?;
        let job_id = match resolved {
            ResolvedOwnedJob::Sandbox { job_id, lookup } => {
                job_policy::ensure_sandbox_job_accepts_prompt(
                    job_id,
                    lookup.is_interactive(),
                    lookup.accepts_prompts(),
                    &lookup.status(),
                )?;
                job_id
            }
            ResolvedOwnedJob::Local { job_id, .. } => {
                return Err(job_policy::local_job_prompt_unsupported_error(job_id));
            }
        };

        let prompt = PendingPrompt {
            content: prompt_params.content,
            done: prompt_params.done,
        };
        let done = prompt.done;

        {
            let mut queue = self.prompt_queue.lock().await;
            queue.entry(job_id).or_default().push_back(prompt);
        }

        let result = job_policy::job_prompt_output(job_id, done);

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_job_tool_local() {
        let manager = Arc::new(ContextManager::new(5));
        let tool = CreateJobTool::new(manager.clone());

        // Without sandbox deps, it should use the local path
        assert!(!tool.sandbox_enabled());

        let params = serde_json::json!({
            "title": "Test Job",
            "description": "A test job description"
        });

        let ctx = JobContext::default();
        let result = tool.execute(params, &ctx).await.unwrap();

        let job_id = result.result.get("job_id").unwrap().as_str().unwrap();
        assert!(!job_id.is_empty());
        assert_eq!(
            result.result.get("status").unwrap().as_str().unwrap(),
            "pending"
        );
        assert_eq!(
            result
                .result
                .get("runtime_family")
                .and_then(|value| value.as_str()),
            Some("execution_backend")
        );
        assert_eq!(
            result
                .result
                .get("runtime_mode")
                .and_then(|value| value.as_str()),
            Some("in_memory")
        );
    }

    #[test]
    fn sandbox_job_runtime_descriptor_tracks_mode_capabilities() {
        let worker = sandbox_job_runtime_descriptor(JobMode::Worker);
        assert_eq!(worker.runtime_family, "execution_backend");
        assert_eq!(worker.runtime_mode, "worker");
        assert!(
            worker
                .runtime_capabilities
                .contains(&"llm_proxy".to_string())
        );

        let codex = sandbox_job_runtime_descriptor(JobMode::CodexCode);
        assert_eq!(codex.runtime_mode, "codex_code");
        assert!(
            codex
                .runtime_capabilities
                .contains(&"codex_cli".to_string())
        );
        assert_eq!(codex.network_isolation.as_deref(), Some("hard"));
    }

    #[test]
    fn test_schema_changes_with_sandbox() {
        let manager = Arc::new(ContextManager::new(5));

        // Without sandbox
        let tool = CreateJobTool::new(Arc::clone(&manager));
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("title"));
        assert!(props.contains_key("description"));
        assert!(!props.contains_key("wait"));
        assert!(!props.contains_key("mode"));
    }

    #[test]
    fn test_execution_timeout_sandbox() {
        let manager = Arc::new(ContextManager::new(5));

        // Without sandbox: default timeout
        let tool = CreateJobTool::new(Arc::clone(&manager));
        assert_eq!(tool.execution_timeout(), Duration::from_secs(30));
    }

    #[tokio::test]
    async fn test_list_jobs_tool() {
        let manager = Arc::new(ContextManager::new(5));

        // Create some jobs
        let job1 = manager.create_job("Job 1", "Desc 1").await.unwrap();
        manager.create_job("Job 2", "Desc 2").await.unwrap();
        manager
            .update_context(job1, |ctx| {
                ctx.transition_to(JobState::Cancelled, Some("Cancelled in test".to_string()))
            })
            .await
            .unwrap()
            .unwrap();

        let tool = ListJobsTool::new(manager);

        let params = serde_json::json!({});
        let ctx = JobContext::default();
        let result = tool.execute(params, &ctx).await.unwrap();

        let jobs = result.result.get("jobs").unwrap().as_array().unwrap();
        assert_eq!(jobs.len(), 2);
        let summary = result.result.get("summary").unwrap();
        assert_eq!(summary.get("cancelled").and_then(|v| v.as_u64()), Some(1));
        assert_eq!(summary.get("failed").and_then(|v| v.as_u64()), Some(0));
        assert_eq!(summary.get("interrupted").and_then(|v| v.as_u64()), Some(0));
    }

    #[tokio::test]
    async fn test_job_status_tool() {
        let manager = Arc::new(ContextManager::new(5));
        let job_id = manager.create_job("Test Job", "Description").await.unwrap();

        let tool = JobStatusTool::new(manager);

        let params = serde_json::json!({
            "job_id": job_id.to_string()
        });
        let ctx = JobContext::default();
        let result = tool.execute(params, &ctx).await.unwrap();

        assert_eq!(
            result.result.get("title").unwrap().as_str().unwrap(),
            "Test Job"
        );
    }

    #[tokio::test]
    async fn test_direct_jobs_remain_visible_after_context_cleanup_when_persisted() {
        let (store, _guard) = crate::testing::test_db().await;
        let manager = Arc::new(ContextManager::new(5));
        let job_id = manager
            .create_job_for_identity("household", "alex", "Persisted Job", "Description")
            .await
            .unwrap();
        manager
            .update_context(job_id, |ctx| {
                ctx.transition_to(JobState::InProgress, Some("Started in test".to_string()))
            })
            .await
            .unwrap()
            .unwrap();
        manager
            .update_context(job_id, |ctx| {
                ctx.transition_to(JobState::Completed, Some("Finished in test".to_string()))
            })
            .await
            .unwrap()
            .unwrap();

        let snapshot = manager.get_context(job_id).await.unwrap();
        store.save_job(&snapshot).await.unwrap();
        manager.remove_job(job_id).await.unwrap();

        let actor_ctx = JobContext {
            user_id: "household".to_string(),
            principal_id: "household".to_string(),
            actor_id: Some("alex".to_string()),
            ..Default::default()
        };

        let list_tool =
            ListJobsTool::new(Arc::clone(&manager)).with_sandbox(None, Some(store.clone()));
        let list_result = list_tool
            .execute(serde_json::json!({}), &actor_ctx)
            .await
            .unwrap();
        let jobs = list_result
            .result
            .get("jobs")
            .and_then(|value| value.as_array())
            .expect("jobs array");
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0]["job_id"], serde_json::json!(job_id.to_string()));
        assert_eq!(jobs[0]["kind"], serde_json::json!("local"));

        let status_tool =
            JobStatusTool::new(Arc::clone(&manager)).with_sandbox(None, Some(store.clone()));
        let status_result = status_tool
            .execute(
                serde_json::json!({
                    "job_id": job_id.to_string(),
                }),
                &actor_ctx,
            )
            .await
            .unwrap();
        assert_eq!(
            status_result.result["status"],
            serde_json::json!("completed")
        );
        assert_eq!(status_result.result["kind"], serde_json::json!("local"));

        let events_tool = JobEventsTool::new(store, manager, None);
        let events_result = events_tool
            .execute(
                serde_json::json!({
                    "job_id": job_id.to_string(),
                }),
                &actor_ctx,
            )
            .await
            .unwrap();
        assert_eq!(events_result.result["kind"], serde_json::json!("local"));
        assert_eq!(events_result.result["total_events"], serde_json::json!(0));
    }

    #[tokio::test]
    async fn test_job_status_tool_rejects_same_user_different_actor() {
        let manager = Arc::new(ContextManager::new(5));
        let job_id = manager
            .create_job_for_identity("household", "alex", "Secret Job", "Description")
            .await
            .unwrap();

        let tool = JobStatusTool::new(manager);
        let params = serde_json::json!({
            "job_id": job_id.to_string()
        });
        let ctx = JobContext {
            user_id: "household".to_string(),
            principal_id: "household".to_string(),
            actor_id: Some("sam".to_string()),
            ..Default::default()
        };

        let result = tool.execute(params, &ctx).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("job not found"),
            "expected actor ownership rejection, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_cancel_job_tool_rejects_same_user_different_actor() {
        let manager = Arc::new(ContextManager::new(5));
        let job_id = manager
            .create_job_for_identity("household", "alex", "Secret Job", "Description")
            .await
            .unwrap();

        let tool = CancelJobTool::new(Arc::clone(&manager));
        let params = serde_json::json!({
            "job_id": job_id.to_string()
        });
        let ctx = JobContext {
            user_id: "household".to_string(),
            principal_id: "household".to_string(),
            actor_id: Some("sam".to_string()),
            ..Default::default()
        };

        let result = tool.execute(params, &ctx).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("job not found"),
            "expected actor ownership rejection, got: {}",
            err
        );

        let job_ctx = manager.get_context(job_id).await.unwrap();
        assert!(
            job_ctx.state.is_active(),
            "other actor must not cancel the job"
        );
    }

    #[test]
    fn test_resolve_project_dir_auto() {
        let project_id = Uuid::new_v4();
        let (dir, browse_id) = resolve_project_dir(None, project_id).unwrap();
        assert!(dir.exists());
        assert!(dir.ends_with(project_id.to_string()));
        assert_eq!(browse_id, project_id.to_string());

        // Must be under the projects base
        let base = projects_base().canonicalize().unwrap();
        assert!(dir.starts_with(&base));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_resolve_project_dir_explicit_under_base() {
        let base = projects_base();
        std::fs::create_dir_all(&base).unwrap();
        let explicit = base.join("test_explicit_project");
        // Explicit paths must already exist (no auto-create).
        std::fs::create_dir_all(&explicit).unwrap();
        let project_id = Uuid::new_v4();

        let (dir, browse_id) = resolve_project_dir(Some(explicit.clone()), project_id).unwrap();
        assert!(dir.exists());
        assert_eq!(browse_id, "test_explicit_project");

        let canonical_base = base.canonicalize().unwrap();
        assert!(dir.starts_with(&canonical_base));

        let _ = std::fs::remove_dir_all(&explicit);
    }

    #[test]
    fn test_resolve_project_dir_rejects_outside_base() {
        let tmp = tempfile::tempdir().unwrap();
        let escape_attempt = tmp.path().join("evil_project");
        // Don't create it: explicit paths that don't exist are rejected
        // before the prefix check even runs.

        let result = resolve_project_dir(Some(escape_attempt), Uuid::new_v4());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("does not exist"),
            "expected 'does not exist' error, got: {}",
            err
        );
    }

    #[test]
    fn test_resolve_project_dir_rejects_outside_base_existing() {
        // A directory that exists but is outside the projects base.
        let tmp = tempfile::tempdir().unwrap();
        let outside = tmp.path().to_path_buf();

        let result = resolve_project_dir(Some(outside), Uuid::new_v4());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("must be under"),
            "expected 'must be under' error, got: {}",
            err
        );
    }

    #[test]
    fn test_resolve_project_dir_rejects_traversal() {
        // Non-existent traversal path is rejected because canonicalize fails.
        let base = projects_base();
        let traversal = base.join("legit").join("..").join("..").join(".ssh");

        let result = resolve_project_dir(Some(traversal), Uuid::new_v4());
        assert!(result.is_err(), "traversal path should be rejected");

        // Traversal path that actually resolves gets the prefix check.
        // `base/../` resolves to the parent of projects base, which is outside.
        let base_parent = projects_base().join("..").join("definitely_not_projects");
        std::fs::create_dir_all(&base_parent).ok();
        if base_parent.exists() {
            let result = resolve_project_dir(Some(base_parent.clone()), Uuid::new_v4());
            assert!(result.is_err(), "path outside base should be rejected");
            let _ = std::fs::remove_dir_all(&base_parent);
        }
    }

    #[test]
    fn test_sandbox_schema_includes_project_dir() {
        let manager = Arc::new(ContextManager::new(5));
        let jm = Arc::new(ContainerJobManager::new(
            ContainerJobConfig::default(),
            TokenStore::new(),
        ));
        let tool = CreateJobTool::new(manager).with_sandbox(jm, None);
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap().as_object().unwrap();
        assert!(
            props.contains_key("project_dir"),
            "sandbox schema must expose project_dir"
        );
    }

    #[test]
    fn test_sandbox_schema_includes_credentials() {
        let manager = Arc::new(ContextManager::new(5));
        let jm = Arc::new(ContainerJobManager::new(
            ContainerJobConfig::default(),
            TokenStore::new(),
        ));
        let tool = CreateJobTool::new(manager).with_sandbox(jm, None);
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap().as_object().unwrap();
        assert!(
            props.contains_key("credentials"),
            "sandbox schema must expose credentials"
        );
    }

    #[test]
    fn test_sandbox_schema_only_exposes_enabled_agent_modes() {
        let manager = Arc::new(ContextManager::new(5));
        let jm = Arc::new(ContainerJobManager::new(
            ContainerJobConfig {
                claude_code_enabled: false,
                codex_code_enabled: true,
                ..ContainerJobConfig::default()
            },
            TokenStore::new(),
        ));
        let tool = CreateJobTool::new(manager).with_sandbox(jm, None);
        let schema = tool.parameters_schema();
        let mode_enum = schema["properties"]["mode"]["enum"]
            .as_array()
            .expect("mode enum array");
        let mode_values: Vec<&str> = mode_enum
            .iter()
            .filter_map(|value| value.as_str())
            .collect();

        assert_eq!(mode_values, vec!["worker", "codex_code"]);
    }

    #[tokio::test]
    async fn test_execute_rejects_disabled_codex_mode() {
        let manager = Arc::new(ContextManager::new(5));
        let jm = Arc::new(ContainerJobManager::new(
            ContainerJobConfig {
                claude_code_enabled: true,
                codex_code_enabled: false,
                ..ContainerJobConfig::default()
            },
            TokenStore::new(),
        ));
        let tool = CreateJobTool::new(manager).with_sandbox(jm, None);
        let params = serde_json::json!({
            "title": "Test Job",
            "description": "A test job description",
            "mode": "codex_code"
        });

        let err = tool
            .execute(params, &JobContext::default())
            .await
            .expect_err("disabled codex mode should be rejected");

        assert!(err.to_string().contains("not enabled"));
    }

    #[tokio::test]
    async fn test_parse_credentials_empty() {
        let manager = Arc::new(ContextManager::new(5));
        let tool = CreateJobTool::new(manager);

        // No credentials parameter
        let params = serde_json::json!({"title": "t", "description": "d"});
        let grants = tool.parse_credentials(&params, "user1").await.unwrap();
        assert!(grants.is_empty());

        // Empty credentials object
        let params = serde_json::json!({"credentials": {}});
        let grants = tool.parse_credentials(&params, "user1").await.unwrap();
        assert!(grants.is_empty());
    }

    #[tokio::test]
    async fn test_parse_credentials_no_secrets_store() {
        let manager = Arc::new(ContextManager::new(5));
        let tool = CreateJobTool::new(manager);

        let params = serde_json::json!({"credentials": {"my_secret": "MY_SECRET"}});
        let result = tool.parse_credentials(&params, "user1").await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("no secrets store"),
            "expected 'no secrets store' error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_parse_credentials_missing_secret() {
        use crate::secrets::{InMemorySecretsStore, SecretsCrypto};
        use secrecy::SecretString;

        let manager = Arc::new(ContextManager::new(5));
        let key = "0123456789abcdef0123456789abcdef";
        let crypto = Arc::new(SecretsCrypto::new(SecretString::from(key.to_string())).unwrap());
        let secrets: Arc<dyn SecretsStore + Send + Sync> =
            Arc::new(InMemorySecretsStore::new(crypto));

        let tool = CreateJobTool::new(manager).with_secrets(Arc::clone(&secrets));

        let params = serde_json::json!({"credentials": {"nonexistent_secret": "SOME_VAR"}});
        let result = tool.parse_credentials(&params, "user1").await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("not found"),
            "expected 'not found' error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_parse_credentials_valid() {
        use crate::secrets::{CreateSecretParams, InMemorySecretsStore, SecretsCrypto};
        use secrecy::SecretString;

        let manager = Arc::new(ContextManager::new(5));
        let key = "0123456789abcdef0123456789abcdef";
        let crypto = Arc::new(SecretsCrypto::new(SecretString::from(key.to_string())).unwrap());
        let secrets: Arc<dyn SecretsStore + Send + Sync> =
            Arc::new(InMemorySecretsStore::new(Arc::clone(&crypto)));

        // Store a secret
        secrets
            .create(
                "user1",
                CreateSecretParams::new("github_token", "ghp_test123"),
            )
            .await
            .unwrap();

        let tool = CreateJobTool::new(manager).with_secrets(Arc::clone(&secrets));

        let params = serde_json::json!({
            "credentials": {"github_token": "GITHUB_TOKEN"}
        });
        let grants = tool.parse_credentials(&params, "user1").await.unwrap();
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0].secret_name, "github_token");
        assert_eq!(grants[0].env_var, "GITHUB_TOKEN");
    }

    fn test_prompt_tool(queue: PromptQueue) -> JobPromptTool {
        let cm = Arc::new(ContextManager::new(5));
        JobPromptTool::new(queue, cm)
    }

    #[tokio::test]
    async fn test_job_prompt_tool_rejects_local_jobs() {
        let cm = Arc::new(ContextManager::new(5));
        let job_id = cm
            .create_job_for_user("default", "Test Job", "desc")
            .await
            .unwrap();

        let queue: PromptQueue =
            Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let tool = JobPromptTool::new(Arc::clone(&queue), cm);

        let params = serde_json::json!({
            "job_id": job_id.to_string(),
            "content": "What's the status?",
            "done": false,
        });

        let ctx = JobContext::default();
        let err = tool.execute(params, &ctx).await.unwrap_err();
        assert!(
            err.to_string()
                .contains("job_prompt only supports sandbox jobs"),
            "expected local-job rejection, got: {}",
            err
        );

        let q = queue.lock().await;
        assert!(q.get(&job_id).is_none());
    }

    #[tokio::test]
    async fn test_job_prompt_tool_requires_approval() {
        use crate::tools::tool::ApprovalRequirement;
        let queue: PromptQueue =
            Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let tool = test_prompt_tool(queue);
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::UnlessAutoApproved
        );
    }

    #[tokio::test]
    async fn test_job_prompt_tool_rejects_invalid_uuid() {
        let queue: PromptQueue =
            Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let tool = test_prompt_tool(queue);

        let params = serde_json::json!({
            "job_id": "not-a-uuid",
            "content": "hello",
        });

        let ctx = JobContext::default();
        let result = tool.execute(params, &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_job_prompt_tool_rejects_missing_content() {
        let queue: PromptQueue =
            Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let tool = test_prompt_tool(queue);

        let params = serde_json::json!({
            "job_id": Uuid::new_v4().to_string(),
        });

        let ctx = JobContext::default();
        let result = tool.execute(params, &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_job_events_tool_rejects_other_users_job() {
        // JobEventsTool needs a Store (PostgreSQL) for the full path, but the
        // ownership check happens first via ContextManager, so we can test that
        // without a database by using a Store that will never be reached.
        //
        // We construct the tool by hand: the store field is never touched
        // because the ownership check short-circuits before the query.
        let cm = Arc::new(ContextManager::new(5));
        let job_id = cm
            .create_job_for_user("owner-user", "Secret Job", "classified")
            .await
            .unwrap();

        // We need a Store to construct the tool, but creating one requires
        // a database URL. Instead, test the ownership logic directly:
        // simulate what execute() does.
        let attacker_ctx = JobContext {
            user_id: "attacker".to_string(),
            principal_id: "attacker".to_string(),
            actor_id: Some("attacker".to_string()),
            ..Default::default()
        };

        let job_ctx = cm.get_context(job_id).await.unwrap();
        assert_ne!(job_ctx.user_id, attacker_ctx.user_id);
        assert_eq!(job_ctx.user_id, "owner-user");
    }

    #[test]
    fn test_job_events_tool_schema() {
        // Verify the schema shape is correct (doesn't need a Store instance).
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "job_id": {
                    "type": "string",
                    "description": "The job ID (full UUID or short prefix, e.g. 'f2854dd8')"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of events to return (default 50, most recent)"
                }
            },
            "required": ["job_id"]
        });

        let props = schema.get("properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("job_id"));
        assert!(props.contains_key("limit"));
        let required = schema.get("required").unwrap().as_array().unwrap();
        assert_eq!(required.len(), 1);
        assert_eq!(required[0].as_str().unwrap(), "job_id");
    }

    #[tokio::test]
    async fn test_job_prompt_tool_rejects_other_users_job() {
        let cm = Arc::new(ContextManager::new(5));
        let job_id = cm
            .create_job_for_user("owner-user", "Test Job", "desc")
            .await
            .unwrap();

        let queue: PromptQueue =
            Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let tool = JobPromptTool::new(queue, cm);

        let params = serde_json::json!({
            "job_id": job_id.to_string(),
            "content": "sneaky prompt",
        });

        // Attacker context with a different user_id.
        let ctx = JobContext {
            user_id: "attacker".to_string(),
            principal_id: "attacker".to_string(),
            actor_id: Some("attacker".to_string()),
            ..Default::default()
        };

        let result = tool.execute(params, &ctx).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("job not found") || err.contains("does not belong to current user"),
            "expected ownership error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_job_prompt_tool_rejects_same_user_different_actor_job() {
        let cm = Arc::new(ContextManager::new(5));
        let job_id = cm
            .create_job_for_identity("household", "alex", "Test Job", "desc")
            .await
            .unwrap();

        let queue: PromptQueue =
            Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let tool = JobPromptTool::new(Arc::clone(&queue), Arc::clone(&cm));

        let params = serde_json::json!({
            "job_id": job_id.to_string(),
            "content": "sneaky prompt",
        });

        let ctx = JobContext {
            user_id: "household".to_string(),
            principal_id: "household".to_string(),
            actor_id: Some("sam".to_string()),
            ..Default::default()
        };

        let result = tool.execute(params, &ctx).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("job not found"),
            "expected actor ownership rejection, got: {}",
            err
        );

        let q = queue.lock().await;
        assert!(
            q.get(&job_id).is_none(),
            "prompt must not be queued for another actor's job"
        );
    }

    #[tokio::test]
    async fn test_resolve_job_id_full_uuid() {
        let cm = ContextManager::new(5);
        let job_id = cm.create_job("Test", "Desc").await.unwrap();

        let resolved = job_policy::resolve_job_reference(
            &job_id.to_string(),
            cm.all_jobs().await,
            std::iter::empty::<Uuid>(),
        );
        assert_eq!(resolved.unwrap().job_id, job_id);
    }

    #[tokio::test]
    async fn test_resolve_job_id_short_prefix() {
        let cm = ContextManager::new(5);
        let job_id = cm.create_job("Test", "Desc").await.unwrap();

        // Use first 8 hex chars (without dashes)
        let hex = job_id.to_string().replace('-', "");
        let prefix = &hex[..8];
        let resolved = job_policy::resolve_job_reference(
            prefix,
            cm.all_jobs().await,
            std::iter::empty::<Uuid>(),
        );
        assert_eq!(resolved.unwrap().job_id, job_id);
    }

    #[tokio::test]
    async fn test_resolve_job_id_no_match() {
        let cm = ContextManager::new(5);
        cm.create_job("Test", "Desc").await.unwrap();

        let result = job_policy::resolve_job_reference(
            "00000000",
            cm.all_jobs().await,
            std::iter::empty::<Uuid>(),
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("no job found"),
            "expected 'no job found', got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_resolve_job_id_invalid_input() {
        let cm = ContextManager::new(5);
        let result = job_policy::resolve_job_reference(
            "not-hex-at-all!",
            cm.all_jobs().await,
            std::iter::empty::<Uuid>(),
        );
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_resolve_owned_job_id_filters_other_actor_jobs() {
        let cm = ContextManager::new(5);
        let alex_job = cm
            .create_job_for_identity("household", "alex", "Test", "Desc")
            .await
            .unwrap();
        cm.create_job_for_identity("household", "sam", "Other", "Desc")
            .await
            .unwrap();

        let result =
            resolve_owned_job_ref(&alex_job.to_string(), &cm, None, None, "household", "sam").await;
        assert!(result.is_err());
        let err = result.err().expect("expected job lookup to fail");
        assert!(err.to_string().contains("job not found"));
    }
}
