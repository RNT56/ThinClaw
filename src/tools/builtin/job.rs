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
use thinclaw_tools::ports::{
    JobToolHostPort, ToolCreateJobRequest, ToolHostError, ToolJobActionRequest,
    ToolJobActionResult, ToolJobQuery, ToolJobSnapshot, ToolOperationScope,
    job_context_from_tool_scope,
};

pub struct RootJobToolHost {
    context_manager: Arc<ContextManager>,
    job_manager: Option<Arc<ContainerJobManager>>,
    store: Option<Arc<dyn Database>>,
    scheduler: Option<Arc<Scheduler>>,
    event_tx: Option<tokio::sync::broadcast::Sender<(Uuid, SseEvent)>>,
    inject_tx: Option<tokio::sync::mpsc::Sender<IncomingMessage>>,
    prompt_queue: Option<PromptQueue>,
    sandbox_children: Option<Arc<SandboxChildRegistry>>,
    secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
}

#[allow(clippy::too_many_arguments)]
pub fn root_job_tool_host(
    context_manager: Arc<ContextManager>,
    job_manager: Option<Arc<ContainerJobManager>>,
    store: Option<Arc<dyn Database>>,
    scheduler: Option<Arc<Scheduler>>,
    event_tx: Option<tokio::sync::broadcast::Sender<(Uuid, SseEvent)>>,
    inject_tx: Option<tokio::sync::mpsc::Sender<IncomingMessage>>,
    prompt_queue: Option<PromptQueue>,
    sandbox_children: Option<Arc<SandboxChildRegistry>>,
    secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
) -> Arc<dyn JobToolHostPort> {
    Arc::new(RootJobToolHost {
        context_manager,
        job_manager,
        store,
        scheduler,
        event_tx,
        inject_tx,
        prompt_queue,
        sandbox_children,
        secrets_store,
    })
}

fn tool_host_error_from_tool(error: ToolError) -> ToolHostError {
    ToolHostError::OperationFailed {
        reason: error.to_string(),
    }
}

async fn execute_root_job_tool<T>(
    tool: T,
    request: ToolJobActionRequest,
    title: &str,
) -> Result<ToolJobActionResult, ToolHostError>
where
    T: Tool,
{
    let ctx = job_context_from_tool_scope(request.scope, title);
    let output = tool
        .execute(request.params, &ctx)
        .await
        .map_err(tool_host_error_from_tool)?;
    Ok(ToolJobActionResult {
        output: output.result,
    })
}

impl RootJobToolHost {
    fn create_tool(&self) -> CreateJobTool {
        let mut tool = CreateJobTool::new(Arc::clone(&self.context_manager));
        if let Some(scheduler) = self.scheduler.clone() {
            tool = tool.with_scheduler(scheduler);
        }
        if let Some(job_manager) = self.job_manager.clone() {
            tool = tool.with_sandbox(job_manager, self.store.clone());
        }
        if let (Some(event_tx), Some(inject_tx)) = (self.event_tx.clone(), self.inject_tx.clone()) {
            tool = tool.with_monitor_deps(event_tx, inject_tx, self.prompt_queue.clone());
        }
        if let Some(children) = self.sandbox_children.clone() {
            tool = tool.with_sandbox_children(children);
        }
        if let Some(secrets) = self.secrets_store.clone() {
            tool = tool.with_secrets(secrets);
        }
        tool
    }

    fn list_tool(&self) -> ListJobsTool {
        ListJobsTool::new(Arc::clone(&self.context_manager))
            .with_sandbox(self.job_manager.clone(), self.store.clone())
    }

    fn status_tool(&self) -> JobStatusTool {
        JobStatusTool::new(Arc::clone(&self.context_manager))
            .with_sandbox(self.job_manager.clone(), self.store.clone())
    }

    fn cancel_tool(&self) -> CancelJobTool {
        let mut tool = CancelJobTool::new(Arc::clone(&self.context_manager));
        if let Some(scheduler) = self.scheduler.clone() {
            tool = tool.with_scheduler(scheduler);
        }
        tool.with_sandbox(self.job_manager.clone(), self.store.clone())
    }

    fn events_tool(&self) -> Result<JobEventsTool, ToolHostError> {
        let store = self
            .store
            .clone()
            .ok_or_else(|| ToolHostError::Unavailable {
                service: "job_events".to_string(),
            })?;
        Ok(JobEventsTool::new(
            store,
            Arc::clone(&self.context_manager),
            self.job_manager.clone(),
        ))
    }

    fn prompt_tool(&self) -> Result<JobPromptTool, ToolHostError> {
        let prompt_queue = self
            .prompt_queue
            .clone()
            .ok_or_else(|| ToolHostError::Unavailable {
                service: "job_prompt".to_string(),
            })?;
        Ok(
            JobPromptTool::new(prompt_queue, Arc::clone(&self.context_manager))
                .with_sandbox(self.job_manager.clone(), self.store.clone()),
        )
    }
}

#[async_trait]
impl JobToolHostPort for RootJobToolHost {
    async fn create_job(
        &self,
        _request: ToolCreateJobRequest,
    ) -> Result<ToolJobSnapshot, ToolHostError> {
        Err(ToolHostError::Unavailable {
            service: "job_create_structured".to_string(),
        })
    }

    async fn load_job(
        &self,
        _scope: ToolOperationScope,
        _job_id: Uuid,
    ) -> Result<Option<ToolJobSnapshot>, ToolHostError> {
        Err(ToolHostError::Unavailable {
            service: "job_load_structured".to_string(),
        })
    }

    async fn list_jobs(&self, _query: ToolJobQuery) -> Result<Vec<ToolJobSnapshot>, ToolHostError> {
        Err(ToolHostError::Unavailable {
            service: "job_list_structured".to_string(),
        })
    }

    async fn send_job_prompt(
        &self,
        _scope: ToolOperationScope,
        _job_id: Uuid,
        _content: Option<String>,
        _done: bool,
    ) -> Result<ToolJobSnapshot, ToolHostError> {
        Err(ToolHostError::Unavailable {
            service: "job_prompt_structured".to_string(),
        })
    }

    async fn create_job_action(
        &self,
        request: ToolJobActionRequest,
    ) -> Result<ToolJobActionResult, ToolHostError> {
        execute_root_job_tool(self.create_tool(), request, "create job").await
    }

    async fn list_jobs_action(
        &self,
        request: ToolJobActionRequest,
    ) -> Result<ToolJobActionResult, ToolHostError> {
        execute_root_job_tool(self.list_tool(), request, "list jobs").await
    }

    async fn job_status_action(
        &self,
        request: ToolJobActionRequest,
    ) -> Result<ToolJobActionResult, ToolHostError> {
        execute_root_job_tool(self.status_tool(), request, "job status").await
    }

    async fn cancel_job_action(
        &self,
        request: ToolJobActionRequest,
    ) -> Result<ToolJobActionResult, ToolHostError> {
        execute_root_job_tool(self.cancel_tool(), request, "cancel job").await
    }

    async fn job_events_action(
        &self,
        request: ToolJobActionRequest,
    ) -> Result<ToolJobActionResult, ToolHostError> {
        execute_root_job_tool(self.events_tool()?, request, "job events").await
    }

    async fn job_prompt_action(
        &self,
        request: ToolJobActionRequest,
    ) -> Result<ToolJobActionResult, ToolHostError> {
        execute_root_job_tool(self.prompt_tool()?, request, "job prompt").await
    }
}

#[derive(Clone, Default)]
struct SandboxJobLookup {
    live: Option<ContainerHandle>,
    stored: Option<SandboxJobRecord>,
}

impl SandboxJobLookup {
    fn live_state(&self) -> Option<job_policy::SandboxContainerState> {
        self.live.as_ref().map(|handle| match handle.state {
            crate::sandbox_types::ContainerState::Creating => {
                job_policy::SandboxContainerState::Creating
            }
            crate::sandbox_types::ContainerState::Running => {
                job_policy::SandboxContainerState::Running
            }
            crate::sandbox_types::ContainerState::Stopped => {
                job_policy::SandboxContainerState::Stopped
            }
            crate::sandbox_types::ContainerState::Failed => {
                job_policy::SandboxContainerState::Failed
            }
        })
    }

    fn spec(&self) -> Option<&crate::sandbox_jobs::SandboxJobSpec> {
        self.live
            .as_ref()
            .map(|handle| &handle.spec)
            .or_else(|| self.stored.as_ref().map(|job| &job.spec))
    }

    fn status(&self) -> String {
        job_policy::sandbox_lookup_status(job_policy::SandboxJobLookupStatusInput {
            live_state: self.live_state(),
            live_completion_status: self
                .live
                .as_ref()
                .and_then(|handle| handle.completion_result.as_ref())
                .map(|result| result.status.as_str()),
            stored_status: self.stored.as_ref().map(|job| job.status.as_str()),
        })
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
        job_policy::sandbox_lookup_failure_reason(
            self.stored
                .as_ref()
                .and_then(|job| job.failure_reason.as_deref()),
            self.live
                .as_ref()
                .and_then(|handle| handle.completion_result.as_ref())
                .and_then(|result| result.message.as_deref()),
        )
    }

    fn is_interactive(&self) -> bool {
        self.spec().map(|spec| spec.interactive).unwrap_or(false)
    }

    fn accepts_prompts(&self) -> bool {
        job_policy::sandbox_lookup_accepts_prompts(self.is_interactive(), self.live_state())
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
        let metadata_policy = job_policy::job_execution_metadata_policy(&ctx.metadata);
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
                allowed_tools: metadata_policy.allowed_tools,
                allowed_skills: metadata_policy.allowed_skills,
                tool_profile: metadata_policy.tool_profile,
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
        let metadata_policy = job_policy::job_execution_metadata_policy(&ctx.metadata);
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
                allowed_tools: metadata_policy.allowed_tools,
                allowed_skills: metadata_policy.allowed_skills,
                tool_profile: metadata_policy.tool_profile,
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
                    job_policy::sandbox_ui_status(&status),
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
                        status: job_policy::sandbox_ui_status(&lookup.status()).to_string(),
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
mod tests;
