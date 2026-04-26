use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::channels::web::identity_helpers::GatewayRequestIdentity;
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;
use crate::context::{JobContext, JobState};
use crate::history::{SandboxJobRecord, SandboxJobSummary};
use crate::sandbox_jobs::{SandboxJobController, normalize_sandbox_ui_state};
use crate::sandbox_types::{
    ContainerHandle, ContainerState, CredentialGrant, JobMode, PendingPrompt,
};
use crate::tools::execution_backend::{
    ExecutionBackendKind, RuntimeDescriptor, local_job_runtime_descriptor,
    sandbox_job_runtime_descriptor,
};

#[derive(Debug, Clone)]
struct ParsedJobMode {
    resolved: JobMode,
    unknown_raw: Option<String>,
}

fn runtime_descriptor_for_mode(parsed: &ParsedJobMode) -> RuntimeDescriptor {
    let mut descriptor = sandbox_job_runtime_descriptor(parsed.resolved);
    if parsed.unknown_raw.is_some() {
        descriptor.runtime_mode = "unknown".to_string();
    }
    descriptor
}

fn normalized_job_mode_for_response(parsed: &ParsedJobMode) -> Option<String> {
    if parsed.unknown_raw.is_some() {
        return Some("unknown".to_string());
    }
    match parsed.resolved {
        JobMode::Worker => None,
        JobMode::ClaudeCode => Some("claude_code".to_string()),
        JobMode::CodexCode => Some("codex_code".to_string()),
    }
}

#[derive(Deserialize)]
pub(crate) struct FilePathQuery {
    path: Option<String>,
}

#[derive(Debug, Clone, Default)]
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
                ContainerState::Creating => "creating".to_string(),
                ContainerState::Running => "running".to_string(),
                ContainerState::Stopped => handle
                    .completion_result
                    .as_ref()
                    .map(|result| result.status.clone())
                    .or_else(|| self.stored.as_ref().map(|job| job.status.clone()))
                    .unwrap_or_else(|| "completed".to_string()),
                ContainerState::Failed => handle
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
        self.stored
            .as_ref()
            .and_then(|job| job.started_at)
            .or_else(|| {
                self.live.as_ref().and_then(|handle| match handle.state {
                    ContainerState::Creating => None,
                    ContainerState::Running | ContainerState::Stopped | ContainerState::Failed => {
                        Some(handle.created_at)
                    }
                })
            })
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

    fn accepts_prompts(&self) -> bool {
        self.is_interactive()
            && self
                .live
                .as_ref()
                .map(|handle| {
                    matches!(
                        handle.state,
                        ContainerState::Creating | ContainerState::Running
                    )
                })
                .unwrap_or(false)
    }

    fn is_interactive(&self) -> bool {
        self.spec().map(|spec| spec.interactive).unwrap_or(false)
    }

    fn is_cancellable(&self) -> bool {
        matches!(self.status().as_str(), "creating" | "running")
    }

    fn project_dir(&self) -> Option<String> {
        self.spec()
            .and_then(|spec| spec.project_dir.clone())
            .filter(|path| !path.trim().is_empty())
    }
}

async fn load_owned_sandbox_jobs(
    state: &GatewayState,
    request_identity: &GatewayRequestIdentity,
) -> Result<HashMap<Uuid, SandboxJobLookup>, (StatusCode, String)> {
    let mut jobs = HashMap::<Uuid, SandboxJobLookup>::new();

    if let Some(store) = state.store.as_ref() {
        let stored_jobs = store
            .list_sandbox_jobs_for_actor(&request_identity.principal_id, &request_identity.actor_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        for job in stored_jobs {
            let job_id = job.id;
            jobs.entry(job_id).or_default().stored = Some(job);
        }
    }

    if let Some(job_manager) = state.job_manager.as_ref() {
        for handle in job_manager.list_jobs().await {
            if handle.spec.principal_id == request_identity.principal_id
                && handle.spec.actor_id == request_identity.actor_id
            {
                let job_id = handle.job_id;
                jobs.entry(job_id).or_default().live = Some(handle);
            }
        }
    }

    Ok(jobs)
}

async fn load_owned_sandbox_job(
    state: &GatewayState,
    request_identity: &GatewayRequestIdentity,
    job_id: Uuid,
) -> Result<Option<SandboxJobLookup>, (StatusCode, String)> {
    let mut lookup = SandboxJobLookup::default();
    let mut found = false;

    if let Some(job_manager) = state.job_manager.as_ref()
        && let Some(handle) = job_manager.get_handle(job_id).await
    {
        found = true;
        if handle.spec.principal_id != request_identity.principal_id
            || handle.spec.actor_id != request_identity.actor_id
        {
            return Ok(None);
        }
        lookup.live = Some(handle);
    }

    if let Some(store) = state.store.as_ref()
        && let Some(job) = store
            .get_sandbox_job(job_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    {
        found = true;
        if job.spec.principal_id != request_identity.principal_id
            || job.spec.actor_id != request_identity.actor_id
        {
            return Ok(None);
        }
        lookup.stored = Some(job);
    }

    if found { Ok(Some(lookup)) } else { Ok(None) }
}

async fn load_owned_direct_jobs(
    state: &GatewayState,
    request_identity: &GatewayRequestIdentity,
) -> Result<HashMap<Uuid, JobContext>, (StatusCode, String)> {
    let mut jobs = HashMap::<Uuid, JobContext>::new();

    if let Some(store) = state.store.as_ref() {
        let stored_jobs = store
            .list_jobs_for_actor(&request_identity.principal_id, &request_identity.actor_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        for job in stored_jobs {
            jobs.insert(job.job_id, job);
        }
    }

    if let Some(context_manager) = state.context_manager.as_ref() {
        for job_id in context_manager
            .all_jobs_for_actor(&request_identity.principal_id, &request_identity.actor_id)
            .await
        {
            if let Ok(job_ctx) = context_manager.get_context(job_id).await {
                jobs.insert(job_id, job_ctx);
            }
        }
    }

    Ok(jobs)
}

async fn load_owned_direct_job(
    state: &GatewayState,
    request_identity: &GatewayRequestIdentity,
    job_id: Uuid,
) -> Result<Option<JobContext>, (StatusCode, String)> {
    let mut job = None;

    if let Some(store) = state.store.as_ref()
        && let Some(stored) = store
            .get_job(job_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    {
        if stored.user_id != request_identity.principal_id
            || stored.owner_actor_id() != request_identity.actor_id
        {
            return Ok(None);
        }
        job = Some(stored);
    }

    if let Some(context_manager) = state.context_manager.as_ref()
        && let Ok(live) = context_manager.get_context(job_id).await
    {
        if live.user_id != request_identity.principal_id
            || live.owner_actor_id() != request_identity.actor_id
        {
            return Ok(None);
        }
        job = Some(live);
    }

    Ok(job)
}

fn browse_id_for_project_dir(project_dir: &str, job_id: Uuid) -> String {
    std::path::Path::new(project_dir)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| job_id.to_string())
}

fn local_job_elapsed_secs(job: &JobContext) -> Option<u64> {
    job.started_at.map(|start| {
        let end = job.completed_at.unwrap_or_else(chrono::Utc::now);
        (end - start).num_seconds().max(0) as u64
    })
}

fn local_job_transition_infos(job: &JobContext) -> Vec<TransitionInfo> {
    job.transitions
        .iter()
        .map(|transition| TransitionInfo {
            from: transition.from.to_string(),
            to: transition.to.to_string(),
            timestamp: transition.timestamp.to_rfc3339(),
            reason: transition.reason.clone(),
        })
        .collect()
}

pub(crate) async fn jobs_list_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
) -> Result<Json<JobListResponse>, (StatusCode, String)> {
    let direct_jobs = load_owned_direct_jobs(state.as_ref(), &request_identity).await?;
    let sandbox_jobs = load_owned_sandbox_jobs(state.as_ref(), &request_identity).await?;
    let mut jobs = Vec::new();
    for (job_id, job) in direct_jobs {
        let runtime = local_job_runtime_descriptor();
        jobs.push(JobInfo {
            id: job_id,
            title: job.title.clone(),
            state: job.state.to_string(),
            user_id: job.user_id.clone(),
            created_at: job.created_at.to_rfc3339(),
            started_at: job.started_at.map(|dt| dt.to_rfc3339()),
            execution_backend: Some(ExecutionBackendKind::LocalHost.as_str().to_string()),
            runtime_family: Some(runtime.runtime_family),
            runtime_mode: Some(runtime.runtime_mode),
            unknown_job_mode_raw: None,
        });
    }
    for (job_id, lookup) in sandbox_jobs {
        let Some(spec) = lookup.spec() else {
            continue;
        };
        let parsed_mode = ParsedJobMode {
            resolved: spec.mode,
            unknown_raw: None,
        };
        let runtime = runtime_descriptor_for_mode(&parsed_mode);
        jobs.push(JobInfo {
            id: job_id,
            title: spec.title.clone(),
            state: normalize_sandbox_ui_state(&lookup.status()).to_string(),
            user_id: spec.principal_id.clone(),
            created_at: lookup
                .created_at()
                .unwrap_or_else(chrono::Utc::now)
                .to_rfc3339(),
            started_at: lookup.started_at().map(|dt| dt.to_rfc3339()),
            execution_backend: Some(ExecutionBackendKind::DockerSandbox.as_str().to_string()),
            runtime_family: Some(runtime.runtime_family),
            runtime_mode: Some(runtime.runtime_mode),
            unknown_job_mode_raw: parsed_mode.unknown_raw,
        });
    }

    jobs.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    Ok(Json(JobListResponse { jobs }))
}

pub(crate) async fn jobs_summary_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
) -> Result<Json<JobSummaryResponse>, (StatusCode, String)> {
    let direct_jobs = load_owned_direct_jobs(state.as_ref(), &request_identity).await?;
    let sandbox_jobs = load_owned_sandbox_jobs(state.as_ref(), &request_identity).await?;
    let mut s = SandboxJobSummary::default();
    for job in direct_jobs.values() {
        s.total += 1;
        match job.state {
            JobState::Pending => s.creating += 1,
            JobState::InProgress => s.running += 1,
            JobState::Completed | JobState::Submitted | JobState::Accepted => s.completed += 1,
            JobState::Failed => s.failed += 1,
            JobState::Cancelled => s.cancelled += 1,
            JobState::Stuck => s.stuck += 1,
            JobState::Abandoned => s.failed += 1,
        }
    }
    for lookup in sandbox_jobs.values() {
        s.total += 1;
        match lookup.status().as_str() {
            "creating" => s.creating += 1,
            "running" => s.running += 1,
            "completed" => s.completed += 1,
            "failed" => s.failed += 1,
            "cancelled" => s.cancelled += 1,
            "interrupted" => s.interrupted += 1,
            "stuck" => s.stuck += 1,
            _ => {}
        }
    }

    Ok(Json(JobSummaryResponse {
        total: s.total,
        pending: s.creating,
        in_progress: s.running,
        completed: s.completed,
        failed: s.failed,
        cancelled: s.cancelled,
        interrupted: s.interrupted,
        stuck: s.stuck,
    }))
}

pub(crate) async fn jobs_detail_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<JobDetailResponse>, (StatusCode, String)> {
    let job_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid job ID".to_string()))?;

    if let Some(job) = load_owned_direct_job(state.as_ref(), &request_identity, job_id).await? {
        let runtime = local_job_runtime_descriptor();
        return Ok(Json(JobDetailResponse {
            id: job_id,
            title: job.title.clone(),
            description: job.description.clone(),
            state: job.state.to_string(),
            user_id: job.user_id.clone(),
            created_at: job.created_at.to_rfc3339(),
            started_at: job.started_at.map(|dt| dt.to_rfc3339()),
            completed_at: job.completed_at.map(|dt| dt.to_rfc3339()),
            elapsed_secs: local_job_elapsed_secs(&job),
            project_dir: None,
            browse_url: None,
            execution_backend: Some(ExecutionBackendKind::LocalHost.as_str().to_string()),
            runtime_family: Some(runtime.runtime_family),
            runtime_mode: Some(runtime.runtime_mode),
            runtime_capabilities: runtime.runtime_capabilities,
            network_isolation: runtime.network_isolation,
            job_mode: None,
            unknown_job_mode_raw: None,
            interactive: false,
            transitions: local_job_transition_infos(&job),
        }));
    }

    let Some(lookup) = load_owned_sandbox_job(state.as_ref(), &request_identity, job_id).await?
    else {
        return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
    };
    let Some(spec) = lookup.spec() else {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Sandbox job metadata unavailable".to_string(),
        ));
    };

    let started_at = lookup.started_at();
    let completed_at = lookup.completed_at();
    let project_dir = lookup.project_dir();
    let browse_url = project_dir.as_deref().map(|dir| {
        let browse_id = browse_id_for_project_dir(dir, job_id);
        format!("/projects/{}/", browse_id)
    });

    let elapsed_secs = started_at.map(|start| {
        let end = completed_at.unwrap_or_else(chrono::Utc::now);
        (end - start).num_seconds().max(0) as u64
    });

    let mut transitions = Vec::new();
    if let Some(started) = started_at {
        transitions.push(TransitionInfo {
            from: "creating".to_string(),
            to: "running".to_string(),
            timestamp: started.to_rfc3339(),
            reason: None,
        });
    }
    if let Some(completed) = completed_at {
        transitions.push(TransitionInfo {
            from: "running".to_string(),
            to: lookup.status(),
            timestamp: completed.to_rfc3339(),
            reason: lookup.failure_reason(),
        });
    }

    let parsed_mode = ParsedJobMode {
        resolved: spec.mode,
        unknown_raw: None,
    };
    let runtime = runtime_descriptor_for_mode(&parsed_mode);

    Ok(Json(JobDetailResponse {
        id: job_id,
        title: spec.title.clone(),
        description: spec.description.clone(),
        state: normalize_sandbox_ui_state(&lookup.status()).to_string(),
        user_id: spec.principal_id.clone(),
        created_at: lookup
            .created_at()
            .unwrap_or_else(chrono::Utc::now)
            .to_rfc3339(),
        started_at: started_at.map(|dt| dt.to_rfc3339()),
        completed_at: completed_at.map(|dt| dt.to_rfc3339()),
        elapsed_secs,
        project_dir,
        browse_url,
        execution_backend: Some(ExecutionBackendKind::DockerSandbox.as_str().to_string()),
        runtime_family: Some(runtime.runtime_family),
        runtime_mode: Some(runtime.runtime_mode.clone()),
        runtime_capabilities: runtime.runtime_capabilities,
        network_isolation: runtime.network_isolation,
        job_mode: normalized_job_mode_for_response(&parsed_mode),
        unknown_job_mode_raw: parsed_mode.unknown_raw,
        interactive: lookup.accepts_prompts(),
        transitions,
    }))
}

pub(crate) async fn jobs_cancel_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let job_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid job ID".to_string()))?;

    if let Some(job) = load_owned_direct_job(state.as_ref(), &request_identity, job_id).await? {
        if !job.state.is_active() {
            return Err((
                StatusCode::CONFLICT,
                format!("Cannot cancel job in state '{}'", job.state),
            ));
        }

        let mut cancelled = false;
        if let Some(scheduler) = state.scheduler.read().await.as_ref()
            && scheduler.is_running(job_id).await
        {
            scheduler
                .stop(job_id)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            cancelled = true;
        }

        if !cancelled
            && let Some(context_manager) = state.context_manager.as_ref()
            && context_manager.get_context(job_id).await.is_ok()
        {
            context_manager
                .update_context(job_id, |job_ctx| {
                    job_ctx
                        .transition_to(JobState::Cancelled, Some("Cancelled by user".to_string()))
                })
                .await
                .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
                .map_err(|error| (StatusCode::CONFLICT, error))?;

            if let Some(store) = state.store.as_ref()
                && let Ok(snapshot) = context_manager.get_context(job_id).await
                && let Err(error) = store.save_job(&snapshot).await
            {
                tracing::warn!(
                    job_id = %job_id,
                    "Failed to persist cancelled direct job from web handler: {}",
                    error
                );
            }
            cancelled = true;
        }

        if !cancelled {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "Direct job scheduler not available".to_string(),
            ));
        }

        return Ok(Json(serde_json::json!({
            "status": "cancelled",
            "job_id": job_id,
        })));
    }

    let Some(lookup) = load_owned_sandbox_job(state.as_ref(), &request_identity, job_id).await?
    else {
        return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
    };

    if !lookup.is_cancellable() {
        return Err((
            StatusCode::CONFLICT,
            format!("Cannot cancel job in state '{}'", lookup.status()),
        ));
    }

    SandboxJobController::new(
        state.store.clone(),
        state.job_manager.clone(),
        None,
        state.prompt_queue.clone(),
    )
    .cancel_job(job_id, "Cancelled by user")
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(Json(serde_json::json!({
        "status": "cancelled",
        "job_id": job_id,
    })))
}

pub(crate) async fn jobs_restart_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let jm = state.job_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Sandbox not enabled".to_string(),
    ))?;

    let old_job_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid job ID".to_string()))?;

    let old_job = store
        .get_sandbox_job(old_job_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Job not found".to_string()))?;

    if old_job.spec.principal_id != request_identity.principal_id
        || old_job.spec.actor_id != request_identity.actor_id
    {
        return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
    }

    if old_job.status != "interrupted" && old_job.status != "failed" {
        return Err((
            StatusCode::CONFLICT,
            format!("Cannot restart job in state '{}'", old_job.status),
        ));
    }

    let new_job_id = Uuid::new_v4();
    let now = chrono::Utc::now();

    let record = crate::history::SandboxJobRecord {
        id: new_job_id,
        spec: old_job.spec.clone(),
        status: "creating".to_string(),
        success: None,
        failure_reason: None,
        created_at: now,
        started_at: None,
        completed_at: None,
        credential_grants_json: old_job.credential_grants_json.clone(),
    };
    store
        .save_sandbox_job(&record)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let credential_grants: Vec<CredentialGrant> =
        serde_json::from_str(&old_job.credential_grants_json).unwrap_or_else(|e| {
            tracing::warn!(
                job_id = %old_job.id,
                "Failed to deserialize credential grants from stored job: {}. Restarted job will have no credentials.",
                e
            );
            vec![]
        });

    jm.create_job(new_job_id, old_job.spec.clone(), credential_grants)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create container: {}", e),
            )
        })?;

    store
        .update_sandbox_job_status(new_job_id, "running", None, None, Some(now), None)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "status": "restarted",
        "old_job_id": old_job_id,
        "new_job_id": new_job_id,
    })))
}

pub(crate) async fn jobs_prompt_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let prompt_queue = state.prompt_queue.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Container coding agents not configured".to_string(),
    ))?;

    let job_id: uuid::Uuid = id
        .parse()
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid job ID".to_string()))?;

    let Some(lookup) = load_owned_sandbox_job(state.as_ref(), &request_identity, job_id).await?
    else {
        return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
    };
    if !lookup.is_interactive() {
        return Err((
            StatusCode::CONFLICT,
            "This job does not accept follow-up prompts".to_string(),
        ));
    }
    if !lookup.accepts_prompts() {
        return Err((
            StatusCode::CONFLICT,
            "This job is no longer accepting prompts".to_string(),
        ));
    }

    let done = body.get("done").and_then(|v| v.as_bool()).unwrap_or(false);
    let content = body
        .get("content")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    if !done && content.as_deref().unwrap_or("").trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Missing 'content' field".to_string(),
        ));
    }
    let prompt = PendingPrompt { content, done };

    {
        let mut queue = prompt_queue.lock().await;
        queue.entry(job_id).or_default().push_back(prompt);
    }

    Ok(Json(serde_json::json!({
        "status": "queued",
        "job_id": job_id.to_string(),
    })))
}

pub(crate) async fn jobs_events_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let job_id: uuid::Uuid = id
        .parse()
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid job ID".to_string()))?;

    let job_exists = load_owned_direct_job(state.as_ref(), &request_identity, job_id)
        .await?
        .is_some()
        || load_owned_sandbox_job(state.as_ref(), &request_identity, job_id)
            .await?
            .is_some();
    if !job_exists {
        return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
    }

    let events = if let Some(store) = state.store.as_ref() {
        store
            .list_job_events(job_id, None)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    } else {
        Vec::new()
    };

    let events_json: Vec<serde_json::Value> = events
        .into_iter()
        .map(|e| {
            serde_json::json!({
                "id": e.id,
                "event_type": e.event_type,
                "data": e.data,
                "created_at": e.created_at.to_rfc3339(),
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "job_id": job_id.to_string(),
        "events": events_json,
    })))
}

pub(crate) async fn job_files_list_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
    Query(query): Query<FilePathQuery>,
) -> Result<Json<ProjectFilesResponse>, (StatusCode, String)> {
    let job_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid job ID".to_string()))?;

    let Some(lookup) = load_owned_sandbox_job(state.as_ref(), &request_identity, job_id).await?
    else {
        return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
    };

    let base = std::path::PathBuf::from(
        lookup
            .project_dir()
            .ok_or((StatusCode::NOT_FOUND, "Project dir not found".to_string()))?,
    );
    let rel_path = query.path.as_deref().unwrap_or("");
    let target = base.join(rel_path);

    let canonical = target
        .canonicalize()
        .map_err(|_| (StatusCode::NOT_FOUND, "Path not found".to_string()))?;
    let base_canonical = base
        .canonicalize()
        .map_err(|_| (StatusCode::NOT_FOUND, "Project dir not found".to_string()))?;
    if !canonical.starts_with(&base_canonical) {
        return Err((StatusCode::FORBIDDEN, "Forbidden".to_string()));
    }

    let mut entries = Vec::new();
    let mut read_dir = tokio::fs::read_dir(&canonical)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Cannot read directory".to_string()))?;

    while let Ok(Some(entry)) = read_dir.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = entry
            .file_type()
            .await
            .map(|ft| ft.is_dir())
            .unwrap_or(false);
        let rel = if rel_path.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", rel_path, name)
        };
        entries.push(ProjectFileEntry {
            name,
            path: rel,
            is_dir,
        });
    }

    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));

    Ok(Json(ProjectFilesResponse { entries }))
}

pub(crate) async fn job_files_read_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
    Query(query): Query<FilePathQuery>,
) -> Result<Json<ProjectFileReadResponse>, (StatusCode, String)> {
    let job_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid job ID".to_string()))?;

    let Some(lookup) = load_owned_sandbox_job(state.as_ref(), &request_identity, job_id).await?
    else {
        return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
    };

    let path = query.path.as_deref().ok_or((
        StatusCode::BAD_REQUEST,
        "path parameter required".to_string(),
    ))?;

    let base = std::path::PathBuf::from(
        lookup
            .project_dir()
            .ok_or((StatusCode::NOT_FOUND, "Project dir not found".to_string()))?,
    );
    let file_path = base.join(path);

    let canonical = file_path
        .canonicalize()
        .map_err(|_| (StatusCode::NOT_FOUND, "File not found".to_string()))?;
    let base_canonical = base
        .canonicalize()
        .map_err(|_| (StatusCode::NOT_FOUND, "Project dir not found".to_string()))?;
    if !canonical.starts_with(&base_canonical) {
        return Err((StatusCode::FORBIDDEN, "Forbidden".to_string()));
    }

    let content = tokio::fs::read_to_string(&canonical)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Cannot read file".to_string()))?;

    Ok(Json(ProjectFileReadResponse {
        path: path.to_string(),
        content,
    }))
}

#[cfg(all(test, feature = "docker-sandbox"))]
mod tests {
    use super::*;

    use std::collections::{HashMap, VecDeque};

    use chrono::Utc;
    use tokio::sync::Mutex;

    use crate::channels::web::identity_helpers::GatewayAuthSource;
    use crate::channels::web::server::{PromptQueue, RateLimiter};
    use crate::channels::web::sse::SseManager;
    use crate::orchestrator::auth::TokenStore;
    use crate::sandbox_jobs::SandboxJobSpec;
    use crate::sandbox_types::{
        ContainerHandle as SandboxContainerHandle, ContainerJobConfig, ContainerJobManager,
        PendingPrompt,
    };

    fn test_identity() -> GatewayRequestIdentity {
        GatewayRequestIdentity::new("user-1", "actor-1", GatewayAuthSource::TrustedProxy, false)
    }

    fn test_gateway_state(
        store: Option<Arc<dyn crate::db::Database>>,
        job_manager: Option<Arc<ContainerJobManager>>,
        prompt_queue: Option<PromptQueue>,
    ) -> Arc<GatewayState> {
        Arc::new(GatewayState {
            msg_tx: tokio::sync::RwLock::new(None),
            sse: SseManager::new(),
            workspace: None,
            session_manager: None,
            log_broadcaster: None,
            log_level_handle: None,
            extension_manager: None,
            tool_registry: None,
            store,
            job_manager,
            prompt_queue,
            context_manager: None,
            scheduler: tokio::sync::RwLock::new(None),
            user_id: "gateway-user".to_string(),
            actor_id: "gateway-actor".to_string(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: None,
            llm_provider: None,
            llm_runtime: None,
            skill_registry: None,
            skill_catalog: None,
            skill_remote_hub: None,
            skill_quarantine: None,
            chat_rate_limiter: RateLimiter::new(30, 60),
            registry_entries: Vec::new(),
            cost_guard: None,
            cost_tracker: None,
            routine_engine: None,
            startup_time: std::time::Instant::now(),
            restart_requested: std::sync::atomic::AtomicBool::new(false),
            secrets_store: None,
            channel_manager: None,
        })
    }

    fn sandbox_spec(
        title: &str,
        description: &str,
        project_dir: Option<String>,
        interactive: bool,
    ) -> SandboxJobSpec {
        let mut spec = SandboxJobSpec::new(
            title,
            description,
            "user-1",
            "actor-1",
            project_dir,
            JobMode::Worker,
        );
        spec.interactive = interactive;
        spec
    }

    async fn insert_live_job(
        job_manager: &Arc<ContainerJobManager>,
        job_id: Uuid,
        state: ContainerState,
        spec: SandboxJobSpec,
    ) {
        let mode = spec.mode;
        job_manager.containers.write().await.insert(
            job_id,
            ContainerHandle {
                job_id,
                container_id: String::new(),
                state,
                mode,
                created_at: Utc::now(),
                spec,
                last_worker_status: None,
                worker_iteration: 0,
                completion_result: None,
            },
        );
    }

    #[tokio::test]
    async fn jobs_list_handler_includes_live_sandbox_job_without_store_record() {
        let job_manager = Arc::new(ContainerJobManager::new(
            ContainerJobConfig::default(),
            TokenStore::new(),
        ));
        let job_id = Uuid::new_v4();
        insert_live_job(
            &job_manager,
            job_id,
            ContainerState::Running,
            sandbox_spec("Live Job", "live sandbox description", None, true),
        )
        .await;
        let state = test_gateway_state(None, Some(Arc::clone(&job_manager)), None);

        let Json(response) = jobs_list_handler(State(state), test_identity())
            .await
            .expect("jobs list should succeed");

        assert_eq!(response.jobs.len(), 1);
        let job = &response.jobs[0];
        assert_eq!(job.id, job_id);
        assert_eq!(job.title, "Live Job");
        assert_eq!(job.state, "in_progress");
        assert_eq!(
            job.execution_backend.as_deref(),
            Some(ExecutionBackendKind::DockerSandbox.as_str())
        );
        assert_eq!(job.runtime_mode.as_deref(), Some("worker"));
        assert_eq!(job.user_id, "user-1");
    }

    #[tokio::test]
    async fn jobs_detail_handler_reads_live_sandbox_job_without_store_record() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let project_dir = tempdir.path().to_string_lossy().to_string();
        let job_manager = Arc::new(ContainerJobManager::new(
            ContainerJobConfig::default(),
            TokenStore::new(),
        ));
        let job_id = Uuid::new_v4();
        insert_live_job(
            &job_manager,
            job_id,
            ContainerState::Running,
            sandbox_spec(
                "Detail Job",
                "detail comes from the live sandbox spec",
                Some(project_dir.clone()),
                true,
            ),
        )
        .await;
        let state = test_gateway_state(None, Some(Arc::clone(&job_manager)), None);

        let Json(response) =
            jobs_detail_handler(State(state), test_identity(), Path(job_id.to_string()))
                .await
                .expect("job detail should succeed");

        assert_eq!(response.id, job_id);
        assert_eq!(response.title, "Detail Job");
        assert_eq!(
            response.description,
            "detail comes from the live sandbox spec"
        );
        assert_eq!(response.state, "in_progress");
        assert_eq!(response.project_dir.as_deref(), Some(project_dir.as_str()));
        assert!(response.interactive);
        assert_eq!(
            response.execution_backend.as_deref(),
            Some(ExecutionBackendKind::DockerSandbox.as_str())
        );
        assert_eq!(response.runtime_mode.as_deref(), Some("worker"));
        assert_eq!(response.job_mode, None);
        assert_eq!(response.transitions.len(), 1);
    }

    #[tokio::test]
    async fn jobs_prompt_handler_queues_done_only_prompt_for_live_interactive_job() {
        let prompt_queue: PromptQueue =
            Arc::new(Mutex::new(HashMap::<Uuid, VecDeque<PendingPrompt>>::new()));
        let job_manager = Arc::new(ContainerJobManager::new(
            ContainerJobConfig::default(),
            TokenStore::new(),
        ));
        let job_id = Uuid::new_v4();
        insert_live_job(
            &job_manager,
            job_id,
            ContainerState::Running,
            sandbox_spec("Prompt Job", "accept follow-up prompts", None, true),
        )
        .await;
        let state = test_gateway_state(
            None,
            Some(Arc::clone(&job_manager)),
            Some(Arc::clone(&prompt_queue)),
        );

        let Json(response) = jobs_prompt_handler(
            State(state),
            test_identity(),
            Path(job_id.to_string()),
            Json(serde_json::json!({ "done": true })),
        )
        .await
        .expect("prompt handler should succeed");

        assert_eq!(
            response.get("status").and_then(|v| v.as_str()),
            Some("queued")
        );

        let queued = prompt_queue
            .lock()
            .await
            .get(&job_id)
            .and_then(|entries| entries.front().cloned())
            .expect("prompt should be queued");
        assert_eq!(queued.content, None);
        assert!(queued.done);
    }

    #[tokio::test]
    async fn jobs_events_handler_allows_live_sandbox_job_without_store_record() {
        let job_manager = Arc::new(ContainerJobManager::new(
            ContainerJobConfig::default(),
            TokenStore::new(),
        ));
        let job_id = Uuid::new_v4();
        insert_live_job(
            &job_manager,
            job_id,
            ContainerState::Running,
            sandbox_spec("Events Job", "live-only job events", None, true),
        )
        .await;
        let state = test_gateway_state(None, Some(Arc::clone(&job_manager)), None);

        let Json(response) =
            jobs_events_handler(State(state), test_identity(), Path(job_id.to_string()))
                .await
                .expect("events handler should succeed");

        assert_eq!(
            response.get("job_id").and_then(|value| value.as_str()),
            Some(job_id.to_string().as_str())
        );
        assert_eq!(
            response
                .get("events")
                .and_then(|value| value.as_array())
                .map(Vec::len),
            Some(0)
        );
    }

    #[tokio::test]
    async fn job_files_handlers_use_live_sandbox_project_dir_without_store_record() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        std::fs::create_dir(root.join("nested")).expect("create nested dir");
        std::fs::write(root.join("alpha.txt"), "hello from sandbox").expect("write alpha file");

        let job_manager = Arc::new(ContainerJobManager::new(
            ContainerJobConfig::default(),
            TokenStore::new(),
        ));
        let job_id = Uuid::new_v4();
        insert_live_job(
            &job_manager,
            job_id,
            ContainerState::Running,
            sandbox_spec(
                "Files Job",
                "inspect project files",
                Some(root.to_string_lossy().to_string()),
                true,
            ),
        )
        .await;
        let state = test_gateway_state(None, Some(Arc::clone(&job_manager)), None);
        let identity = test_identity();

        let Json(list_response) = job_files_list_handler(
            State(Arc::clone(&state)),
            identity.clone(),
            Path(job_id.to_string()),
            Query(FilePathQuery { path: None }),
        )
        .await
        .expect("file listing should succeed");

        assert_eq!(list_response.entries.len(), 2);
        assert_eq!(list_response.entries[0].name, "nested");
        assert!(list_response.entries[0].is_dir);
        assert_eq!(list_response.entries[1].name, "alpha.txt");
        assert!(!list_response.entries[1].is_dir);

        let Json(read_response) = job_files_read_handler(
            State(state),
            identity,
            Path(job_id.to_string()),
            Query(FilePathQuery {
                path: Some("alpha.txt".to_string()),
            }),
        )
        .await
        .expect("file read should succeed");

        assert_eq!(read_response.path, "alpha.txt");
        assert_eq!(read_response.content, "hello from sandbox");
    }

    #[tokio::test]
    async fn jobs_cancel_handler_cancels_live_sandbox_job_without_store_record() {
        let job_manager = Arc::new(ContainerJobManager::new(
            ContainerJobConfig::default(),
            TokenStore::new(),
        ));
        let job_id = Uuid::new_v4();
        insert_live_job(
            &job_manager,
            job_id,
            ContainerState::Running,
            sandbox_spec("Cancel Job", "cancel me live-only", None, true),
        )
        .await;
        let state = test_gateway_state(None, Some(Arc::clone(&job_manager)), None);

        let Json(response) =
            jobs_cancel_handler(State(state), test_identity(), Path(job_id.to_string()))
                .await
                .expect("cancel should succeed");

        assert_eq!(
            response.get("status").and_then(|value| value.as_str()),
            Some("cancelled")
        );

        let handle: SandboxContainerHandle = job_manager
            .get_handle(job_id)
            .await
            .expect("job handle should still exist");
        assert!(matches!(handle.state, ContainerState::Stopped));
        let result = handle
            .completion_result
            .as_ref()
            .expect("completion result should be stored");
        assert_eq!(result.status, "cancelled");
        assert!(!result.success);
    }
}
