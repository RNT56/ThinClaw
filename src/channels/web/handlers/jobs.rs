use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use uuid::Uuid;

use crate::channels::web::identity_helpers::GatewayRequestIdentity;
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;
use crate::context::{JobContext, JobState};
use crate::history::SandboxJobRecord;
use crate::sandbox_jobs::SandboxJobController;
use crate::sandbox_types::{ContainerHandle, ContainerState, CredentialGrant, PendingPrompt};
use thinclaw_gateway::web::jobs::{
    GatewayLocalJobDetailInput, GatewayLocalJobListInput, GatewaySandboxJobDetailInput,
    GatewaySandboxJobListInput, JobEventInfoInput, JobEventsResponse, JobPromptQueuedResponse,
    JobPromptRequest, JobRestartResponse, JobStatusActionResponse, JobSummaryCounts,
    JobTransitionProjection, ProjectFileEntryInput, SandboxContainerState,
    SandboxJobLookupProjection, SandboxJobSpecProjection, direct_job_scheduler_unavailable_error,
    elapsed_secs as gateway_elapsed_secs, job_database_unavailable_error, job_event_info,
    job_events_response, job_list_response, job_not_found_error,
    job_prompt_queue_unavailable_error, job_prompt_queued_response, job_restart_response,
    job_summary_response, local_job_detail_response, local_job_info,
    missing_job_prompt_content_error, parse_job_id, project_cannot_read_directory_error,
    project_cannot_read_file_error, project_dir_not_found_error, project_file_entry,
    project_file_not_found_error, project_file_path_required_error, project_file_read_response,
    project_files_response, project_forbidden_error, project_path_not_found_error,
    sandbox_job_detail_response, sandbox_job_info, sandbox_job_metadata_unavailable_error,
    sandbox_unavailable_error,
};

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

    fn live_state(&self) -> Option<SandboxContainerState> {
        self.live.as_ref().map(|handle| match handle.state {
            ContainerState::Creating => SandboxContainerState::Creating,
            ContainerState::Running => SandboxContainerState::Running,
            ContainerState::Stopped => SandboxContainerState::Stopped,
            ContainerState::Failed => SandboxContainerState::Failed,
        })
    }

    fn projection(&self) -> SandboxJobLookupProjection {
        SandboxJobLookupProjection {
            live_state: self.live_state(),
            live_created_at: self.live.as_ref().map(|handle| handle.created_at),
            live_completion_status: self
                .live
                .as_ref()
                .and_then(|handle| handle.completion_result.as_ref())
                .map(|result| result.status.clone()),
            live_completion_message: self
                .live
                .as_ref()
                .and_then(|handle| handle.completion_result.as_ref())
                .and_then(|result| result.message.clone()),
            stored_status: self.stored.as_ref().map(|job| job.status.clone()),
            stored_created_at: self.stored.as_ref().map(|job| job.created_at),
            stored_started_at: self.stored.as_ref().and_then(|job| job.started_at),
            stored_completed_at: self.stored.as_ref().and_then(|job| job.completed_at),
            stored_failure_reason: self
                .stored
                .as_ref()
                .and_then(|job| job.failure_reason.clone()),
            spec: self.spec().map(|spec| SandboxJobSpecProjection {
                title: spec.title.clone(),
                description: spec.description.clone(),
                principal_id: spec.principal_id.clone(),
                project_dir: spec.project_dir.clone(),
                mode: spec.mode,
                interactive: spec.interactive,
            }),
        }
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

#[utoipa::path(
    get,
    path = "/api/jobs",
    tag = "jobs",
    responses(
        (status = 200, description = "Direct and sandbox jobs owned by the caller", body = JobListResponse),
        (status = 401, description = "Missing or invalid gateway bearer token"),
    ),
    security(("gateway_token" = [])),
)]
pub(crate) async fn jobs_list_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
) -> Result<Json<JobListResponse>, (StatusCode, String)> {
    let direct_jobs = load_owned_direct_jobs(state.as_ref(), &request_identity).await?;
    let sandbox_jobs = load_owned_sandbox_jobs(state.as_ref(), &request_identity).await?;
    let mut jobs = Vec::new();
    for (job_id, job) in direct_jobs {
        jobs.push(local_job_info(GatewayLocalJobListInput {
            id: job_id,
            title: job.title,
            state: job.state.to_string(),
            user_id: job.user_id,
            created_at: job.created_at,
            started_at: job.started_at,
        }));
    }
    for (job_id, lookup) in sandbox_jobs {
        let projection = lookup.projection();
        let Some(spec) = projection.spec.as_ref() else {
            continue;
        };
        jobs.push(sandbox_job_info(GatewaySandboxJobListInput {
            id: job_id,
            title: spec.title.clone(),
            state: projection.ui_state(),
            user_id: spec.principal_id.clone(),
            created_at: projection.created_at().unwrap_or_else(chrono::Utc::now),
            started_at: projection.started_at(),
            mode: spec.mode,
        }));
    }

    Ok(Json(job_list_response(jobs)))
}

#[utoipa::path(
    get,
    path = "/api/jobs/summary",
    tag = "jobs",
    responses(
        (status = 200, description = "Job counts grouped by state", body = JobSummaryResponse),
        (status = 401, description = "Missing or invalid gateway bearer token"),
    ),
    security(("gateway_token" = [])),
)]
pub(crate) async fn jobs_summary_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
) -> Result<Json<JobSummaryResponse>, (StatusCode, String)> {
    let direct_jobs = load_owned_direct_jobs(state.as_ref(), &request_identity).await?;
    let sandbox_jobs = load_owned_sandbox_jobs(state.as_ref(), &request_identity).await?;
    let mut summary = JobSummaryCounts::default();
    for job in direct_jobs.values() {
        summary.record_direct_state(job.state.to_string());
    }
    for lookup in sandbox_jobs.values() {
        summary.record_sandbox_status(lookup.projection().status());
    }

    Ok(Json(job_summary_response(&summary)))
}

#[utoipa::path(
    get,
    path = "/api/jobs/{id}",
    tag = "jobs",
    params(("id" = String, Path, description = "Job UUID")),
    responses(
        (status = 200, description = "Job detail including state transitions", body = JobDetailResponse),
        (status = 400, description = "Malformed job id"),
        (status = 401, description = "Missing or invalid gateway bearer token"),
        (status = 404, description = "Job not found or not visible to this identity"),
    ),
    security(("gateway_token" = [])),
)]
pub(crate) async fn jobs_detail_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<JobDetailResponse>, (StatusCode, String)> {
    let job_id = parse_job_id(&id)?;

    if let Some(job) = load_owned_direct_job(state.as_ref(), &request_identity, job_id).await? {
        let transitions = job
            .transitions
            .iter()
            .map(|transition| JobTransitionProjection {
                from: transition.from.to_string(),
                to: transition.to.to_string(),
                timestamp: transition.timestamp,
                reason: transition.reason.clone(),
            })
            .collect();
        return Ok(Json(local_job_detail_response(
            GatewayLocalJobDetailInput {
                id: job_id,
                title: job.title,
                description: job.description,
                state: job.state.to_string(),
                user_id: job.user_id,
                created_at: job.created_at,
                started_at: job.started_at,
                completed_at: job.completed_at,
                elapsed_secs: gateway_elapsed_secs(
                    job.started_at,
                    job.completed_at,
                    chrono::Utc::now(),
                ),
                transitions,
            },
        )));
    }

    let Some(lookup) = load_owned_sandbox_job(state.as_ref(), &request_identity, job_id).await?
    else {
        return Err(job_not_found_error());
    };
    let projection = lookup.projection();
    let Some(spec) = projection.spec.as_ref() else {
        return Err(sandbox_job_metadata_unavailable_error());
    };

    let started_at = projection.started_at();
    let completed_at = projection.completed_at();
    let project_dir = projection.project_dir();
    let elapsed_secs = gateway_elapsed_secs(started_at, completed_at, chrono::Utc::now());
    Ok(Json(sandbox_job_detail_response(
        GatewaySandboxJobDetailInput {
            id: job_id,
            title: spec.title.clone(),
            description: spec.description.clone(),
            state: projection.ui_state(),
            user_id: spec.principal_id.clone(),
            created_at: projection.created_at().unwrap_or_else(chrono::Utc::now),
            started_at,
            completed_at,
            elapsed_secs,
            project_dir,
            mode: spec.mode,
            interactive: projection.accepts_prompts(),
            final_status: projection.status(),
            failure_reason: projection.failure_reason(),
        },
    )))
}

pub(crate) async fn jobs_cancel_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<JobStatusActionResponse>, (StatusCode, String)> {
    let job_id = parse_job_id(&id)?;

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
            return Err(direct_job_scheduler_unavailable_error());
        }

        return Ok(Json(JobStatusActionResponse::new("cancelled", job_id)));
    }

    let Some(lookup) = load_owned_sandbox_job(state.as_ref(), &request_identity, job_id).await?
    else {
        return Err(job_not_found_error());
    };

    let projection = lookup.projection();
    if !projection.is_cancellable() {
        return Err((
            StatusCode::CONFLICT,
            format!("Cannot cancel job in state '{}'", projection.status()),
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

    Ok(Json(JobStatusActionResponse::new("cancelled", job_id)))
}

pub(crate) async fn jobs_restart_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<JobRestartResponse>, (StatusCode, String)> {
    let store = state
        .store
        .as_ref()
        .ok_or_else(job_database_unavailable_error)?;
    let jm = state
        .job_manager
        .as_ref()
        .ok_or_else(sandbox_unavailable_error)?;

    let old_job_id = parse_job_id(&id)?;

    let old_job = store
        .get_sandbox_job(old_job_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(job_not_found_error)?;

    if old_job.spec.principal_id != request_identity.principal_id
        || old_job.spec.actor_id != request_identity.actor_id
    {
        return Err(job_not_found_error());
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

    Ok(Json(job_restart_response(old_job_id, new_job_id)))
}

pub(crate) async fn jobs_prompt_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
    Json(body): Json<JobPromptRequest>,
) -> Result<Json<JobPromptQueuedResponse>, (StatusCode, String)> {
    let prompt_queue = state
        .prompt_queue
        .as_ref()
        .ok_or_else(job_prompt_queue_unavailable_error)?;

    let job_id = parse_job_id(&id)?;

    let Some(lookup) = load_owned_sandbox_job(state.as_ref(), &request_identity, job_id).await?
    else {
        return Err(job_not_found_error());
    };
    let projection = lookup.projection();
    if !projection.is_interactive() {
        return Err((
            StatusCode::CONFLICT,
            "This job does not accept follow-up prompts".to_string(),
        ));
    }
    if !projection.accepts_prompts() {
        return Err((
            StatusCode::CONFLICT,
            "This job is no longer accepting prompts".to_string(),
        ));
    }

    if !body.done && body.content.as_deref().unwrap_or("").trim().is_empty() {
        return Err(missing_job_prompt_content_error());
    }
    let prompt = PendingPrompt {
        content: body.content,
        done: body.done,
    };

    {
        let mut queue = prompt_queue.lock().await;
        queue.entry(job_id).or_default().push_back(prompt);
    }

    Ok(Json(job_prompt_queued_response(job_id)))
}

pub(crate) async fn jobs_events_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<JobEventsResponse>, (StatusCode, String)> {
    let job_id = parse_job_id(&id)?;

    let job_exists = load_owned_direct_job(state.as_ref(), &request_identity, job_id)
        .await?
        .is_some()
        || load_owned_sandbox_job(state.as_ref(), &request_identity, job_id)
            .await?
            .is_some();
    if !job_exists {
        return Err(job_not_found_error());
    }

    let events = if let Some(store) = state.store.as_ref() {
        store
            .list_job_events(job_id, None)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    } else {
        Vec::new()
    };

    let events = events
        .into_iter()
        .map(|event| {
            job_event_info(JobEventInfoInput {
                id: event.id,
                event_type: event.event_type,
                data: event.data,
                created_at: event.created_at,
            })
        })
        .collect();

    Ok(Json(job_events_response(job_id, events)))
}

pub(crate) async fn job_files_list_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
    Query(query): Query<FilePathQuery>,
) -> Result<Json<ProjectFilesResponse>, (StatusCode, String)> {
    let job_id = parse_job_id(&id)?;

    let Some(lookup) = load_owned_sandbox_job(state.as_ref(), &request_identity, job_id).await?
    else {
        return Err(job_not_found_error());
    };

    let base = std::path::PathBuf::from(
        lookup
            .projection()
            .project_dir()
            .ok_or_else(project_dir_not_found_error)?,
    );
    let rel_path = query.path.as_deref().unwrap_or("");
    let target = base.join(rel_path);

    let canonical = target
        .canonicalize()
        .map_err(|_| project_path_not_found_error())?;
    let base_canonical = base
        .canonicalize()
        .map_err(|_| project_dir_not_found_error())?;
    if !canonical.starts_with(&base_canonical) {
        return Err(project_forbidden_error());
    }

    let mut entries = Vec::new();
    let mut read_dir = tokio::fs::read_dir(&canonical)
        .await
        .map_err(|_| project_cannot_read_directory_error())?;

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
        entries.push(project_file_entry(ProjectFileEntryInput {
            name,
            path: rel,
            is_dir,
        }));
    }

    Ok(Json(project_files_response(entries)))
}

pub(crate) async fn job_files_read_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
    Query(query): Query<FilePathQuery>,
) -> Result<Json<ProjectFileReadResponse>, (StatusCode, String)> {
    let job_id = parse_job_id(&id)?;

    let Some(lookup) = load_owned_sandbox_job(state.as_ref(), &request_identity, job_id).await?
    else {
        return Err(job_not_found_error());
    };

    let path = query
        .path
        .as_deref()
        .ok_or_else(project_file_path_required_error)?;

    let base = std::path::PathBuf::from(
        lookup
            .projection()
            .project_dir()
            .ok_or_else(project_dir_not_found_error)?,
    );
    let file_path = base.join(path);

    let canonical = file_path
        .canonicalize()
        .map_err(|_| project_file_not_found_error())?;
    let base_canonical = base
        .canonicalize()
        .map_err(|_| project_dir_not_found_error())?;
    if !canonical.starts_with(&base_canonical) {
        return Err(project_forbidden_error());
    }

    let content = tokio::fs::read_to_string(&canonical)
        .await
        .map_err(|_| project_cannot_read_file_error())?;

    Ok(Json(project_file_read_response(path, content)))
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
        JobMode, PendingPrompt,
    };
    use crate::tools::execution_backend::ExecutionBackendKind;

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
            pair_complete_rate_limiter: RateLimiter::new(10, 300),
            registry_entries: Vec::new(),
            cost_guard: None,
            cost_tracker: None,
            metrics_registry: None,
            response_cache: None,
            routine_engine: Arc::new(std::sync::RwLock::new(None)),
            repo_project_supervisor: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
            startup_time: std::time::Instant::now(),
            restart_requested: std::sync::atomic::AtomicBool::new(false),
            secrets_store: None,
            channel_manager: None,
            hooks: None,
            device_registry: crate::channels::web::server::test_device_registry(),
            pending_approvals: std::sync::Arc::new(
                crate::channels::web::server::PendingApprovalsStore::in_memory(),
            ),
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
            Json(JobPromptRequest {
                content: None,
                done: true,
            }),
        )
        .await
        .expect("prompt handler should succeed");

        assert_eq!(response.status, "queued");
        assert_eq!(response.job_id, job_id.to_string());

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

        assert_eq!(response.job_id, job_id.to_string());
        assert!(response.events.is_empty());
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

        assert_eq!(response.status, "cancelled");
        assert_eq!(response.job_id, job_id);

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
