use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::api::repo_projects as repo_projects_api;
use crate::channels::web::identity_helpers::GatewayRequestIdentity;
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::SseEvent;
use crate::db::Database;
use crate::repo_projects::supervisor::RepoSupervisorWakeReason;
use thinclaw_gateway::web::api::{FeatureDisabledStatus, gateway_api_error_response};

fn repo_project_api_error(error: crate::api::ApiError) -> (StatusCode, String) {
    gateway_api_error_response(error, FeatureDisabledStatus::Forbidden)
}

fn repo_project_store(state: &GatewayState) -> Result<&Arc<dyn Database>, (StatusCode, String)> {
    state.store.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Repository project database is not available".to_string(),
        )
    })
}

fn parse_repo_project_id(id: &str) -> Result<Uuid, (StatusCode, String)> {
    Uuid::parse_str(id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid repository project ID".to_string(),
        )
    })
}

fn broadcast_project_response(
    state: &GatewayState,
    response: &repo_projects_api::RepoProjectCommandResponse,
) {
    if let Some(project) = response.project.as_ref() {
        let message = response
            .message
            .clone()
            .unwrap_or_else(|| "Repository project updated".to_string());
        state.sse.broadcast(SseEvent::RepoProjectUpdated {
            project_id: project.id.clone(),
            state: project.state.clone(),
            message: message.clone(),
        });
        state.sse.broadcast(SseEvent::RepoProjectEvent {
            project_id: project.id.clone(),
            event_type: "repo_project_updated".to_string(),
            message,
        });
    }
}

async fn wake_project_supervisor(
    state: &GatewayState,
    project_id: Uuid,
    reason: RepoSupervisorWakeReason,
) {
    let supervisor = state.repo_project_supervisor.read().await.clone();
    if let Some(supervisor) = supervisor
        && let Err(error) = supervisor.wake(Some(project_id), reason).await
    {
        tracing::warn!(
            project_id = %project_id,
            error = %error,
            "failed to wake repo project supervisor"
        );
    }
}

async fn wake_project_from_response(
    state: &GatewayState,
    response: &repo_projects_api::RepoProjectCommandResponse,
    reason: RepoSupervisorWakeReason,
) {
    let Some(project_id) = response
        .project
        .as_ref()
        .and_then(|project| Uuid::parse_str(&project.id).ok())
    else {
        return;
    };
    wake_project_supervisor(state, project_id, reason).await;
}

#[derive(Debug, Deserialize)]
pub(crate) struct RepoProjectEventsQuery {
    #[serde(default)]
    limit: Option<i64>,
}

pub(crate) async fn repo_projects_list_handler(
    State(state): State<Arc<GatewayState>>,
    _request_identity: GatewayRequestIdentity,
) -> Result<Json<repo_projects_api::RepoProjectsListResponse>, (StatusCode, String)> {
    let store = repo_project_store(&state)?;
    repo_projects_api::list_projects(store)
        .await
        .map(Json)
        .map_err(repo_project_api_error)
}

pub(crate) async fn repo_project_detail_handler(
    State(state): State<Arc<GatewayState>>,
    _request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<repo_projects_api::RepoProjectResponse>, (StatusCode, String)> {
    let store = repo_project_store(&state)?;
    let id = parse_repo_project_id(&id)?;
    repo_projects_api::get_project(store, id)
        .await
        .map(Json)
        .map_err(repo_project_api_error)
}

pub(crate) async fn repo_project_create_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Json(input): Json<repo_projects_api::RepoProjectCreateInput>,
) -> Result<
    (
        StatusCode,
        Json<repo_projects_api::RepoProjectCommandResponse>,
    ),
    (StatusCode, String),
> {
    let store = repo_project_store(&state)?;
    let response = repo_projects_api::create_project(store, &request_identity.principal_id, input)
        .await
        .map_err(repo_project_api_error)?;
    broadcast_project_response(&state, &response);
    wake_project_from_response(&state, &response, RepoSupervisorWakeReason::Manual).await;
    Ok((StatusCode::CREATED, Json(response)))
}

pub(crate) async fn repo_project_start_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<repo_projects_api::RepoProjectCommandResponse>, (StatusCode, String)> {
    let store = repo_project_store(&state)?;
    let id = parse_repo_project_id(&id)?;
    let response = repo_projects_api::start_project(store, &request_identity.principal_id, id)
        .await
        .map_err(repo_project_api_error)?;
    broadcast_project_response(&state, &response);
    wake_project_supervisor(&state, id, RepoSupervisorWakeReason::Manual).await;
    Ok(Json(response))
}

pub(crate) async fn repo_project_plan_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<repo_projects_api::RepoProjectCommandResponse>, (StatusCode, String)> {
    let store = repo_project_store(&state)?;
    let id = parse_repo_project_id(&id)?;
    let response = repo_projects_api::plan_project(store, &request_identity.principal_id, id)
        .await
        .map_err(repo_project_api_error)?;
    broadcast_project_response(&state, &response);
    wake_project_supervisor(&state, id, RepoSupervisorWakeReason::Manual).await;
    Ok(Json(response))
}

pub(crate) async fn repo_project_pause_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<repo_projects_api::RepoProjectCommandResponse>, (StatusCode, String)> {
    let store = repo_project_store(&state)?;
    let id = parse_repo_project_id(&id)?;
    let response = repo_projects_api::pause_project(store, &request_identity.principal_id, id)
        .await
        .map_err(repo_project_api_error)?;
    broadcast_project_response(&state, &response);
    wake_project_supervisor(&state, id, RepoSupervisorWakeReason::Manual).await;
    Ok(Json(response))
}

pub(crate) async fn repo_project_resume_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<repo_projects_api::RepoProjectCommandResponse>, (StatusCode, String)> {
    let store = repo_project_store(&state)?;
    let id = parse_repo_project_id(&id)?;
    let response = repo_projects_api::resume_project(store, &request_identity.principal_id, id)
        .await
        .map_err(repo_project_api_error)?;
    broadcast_project_response(&state, &response);
    wake_project_supervisor(&state, id, RepoSupervisorWakeReason::Manual).await;
    Ok(Json(response))
}

pub(crate) async fn repo_project_cancel_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<repo_projects_api::RepoProjectCommandResponse>, (StatusCode, String)> {
    let store = repo_project_store(&state)?;
    let id = parse_repo_project_id(&id)?;
    let response = repo_projects_api::cancel_project(store, &request_identity.principal_id, id)
        .await
        .map_err(repo_project_api_error)?;
    broadcast_project_response(&state, &response);
    wake_project_supervisor(&state, id, RepoSupervisorWakeReason::Manual).await;
    Ok(Json(response))
}

pub(crate) async fn repo_project_approve_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
    Json(input): Json<repo_projects_api::RepoApprovalInput>,
) -> Result<Json<repo_projects_api::RepoProjectCommandResponse>, (StatusCode, String)> {
    let store = repo_project_store(&state)?;
    let id = parse_repo_project_id(&id)?;
    let response =
        repo_projects_api::approve_project(store, &request_identity.principal_id, id, input)
            .await
            .map_err(repo_project_api_error)?;
    broadcast_project_response(&state, &response);
    wake_project_supervisor(&state, id, RepoSupervisorWakeReason::Manual).await;
    Ok(Json(response))
}

pub(crate) async fn repo_project_enqueue_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
    Json(input): Json<repo_projects_api::RepoBacklogEnqueueInput>,
) -> Result<Json<repo_projects_api::RepoProjectCommandResponse>, (StatusCode, String)> {
    let store = repo_project_store(&state)?;
    let id = parse_repo_project_id(&id)?;
    let response =
        repo_projects_api::enqueue_task(store, &request_identity.principal_id, id, input)
            .await
            .map_err(repo_project_api_error)?;
    if let Some(project) = response.project.as_ref()
        && let Some(task) = project.backlog.first()
    {
        state.sse.broadcast(SseEvent::RepoTaskUpdated {
            project_id: project.id.clone(),
            task_id: task.id.clone(),
            state: task.state.clone(),
            message: "Repository project task queued".to_string(),
        });
    }
    broadcast_project_response(&state, &response);
    wake_project_supervisor(&state, id, RepoSupervisorWakeReason::Manual).await;
    Ok(Json(response))
}

pub(crate) async fn repo_project_events_handler(
    State(state): State<Arc<GatewayState>>,
    _request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
    Query(query): Query<RepoProjectEventsQuery>,
) -> Result<Json<repo_projects_api::RepoProjectEventsResponse>, (StatusCode, String)> {
    let store = repo_project_store(&state)?;
    let id = parse_repo_project_id(&id)?;
    repo_projects_api::list_events(store, id, query.limit.unwrap_or(100).clamp(1, 500))
        .await
        .map(Json)
        .map_err(repo_project_api_error)
}

pub(crate) async fn repo_project_merge_gates_handler(
    State(state): State<Arc<GatewayState>>,
    _request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<repo_projects_api::RepoProjectMergeGatesResponse>, (StatusCode, String)> {
    let store = repo_project_store(&state)?;
    let id = parse_repo_project_id(&id)?;
    repo_projects_api::list_merge_gates(store, id)
        .await
        .map(Json)
        .map_err(repo_project_api_error)
}

// ── Setup / credentials / GitHub connector ──────────────────────────────

type SharedSecrets = Arc<dyn crate::secrets::SecretsStore + Send + Sync>;

fn repo_project_secrets(state: &GatewayState) -> Result<&SharedSecrets, (StatusCode, String)> {
    state.secrets_store.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Secrets store is not available; cannot manage GitHub credentials".to_string(),
        )
    })
}

pub(crate) async fn repo_project_readiness_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
) -> Result<Json<repo_projects_api::RepoProjectsReadiness>, (StatusCode, String)> {
    let store = repo_project_store(&state)?;
    repo_projects_api::repo_projects_readiness(
        store,
        state.secrets_store.as_ref(),
        &request_identity.principal_id,
    )
    .await
    .map(Json)
    .map_err(repo_project_api_error)
}

pub(crate) async fn repo_project_setup_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Json(input): Json<repo_projects_api::RepoProjectsConfigureInput>,
) -> Result<Json<repo_projects_api::RepoProjectsReadiness>, (StatusCode, String)> {
    let store = repo_project_store(&state)?;
    let readiness = repo_projects_api::configure_supervisor(
        store,
        state.secrets_store.as_ref(),
        &request_identity.principal_id,
        input,
    )
    .await
    .map_err(repo_project_api_error)?;
    state.sse.broadcast(SseEvent::RepoProjectEvent {
        project_id: String::new(),
        event_type: "repo_project_setup".to_string(),
        message: "Repository project supervisor configuration updated".to_string(),
    });
    Ok(Json(readiness))
}

pub(crate) async fn repo_project_credential_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Json(input): Json<repo_projects_api::RepoCredentialInput>,
) -> Result<Json<repo_projects_api::RepoCredentialStored>, (StatusCode, String)> {
    let secrets = repo_project_secrets(&state)?;
    repo_projects_api::store_repo_credential(
        secrets,
        &request_identity.principal_id,
        input.name,
        input.value,
    )
    .await
    .map(Json)
    .map_err(repo_project_api_error)
}

pub(crate) async fn repo_project_connectable_repos_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
) -> Result<Json<repo_projects_api::ConnectableReposResponse>, (StatusCode, String)> {
    let store = repo_project_store(&state)?;
    let secrets = repo_project_secrets(&state)?;
    repo_projects_api::list_connectable_repos(store, secrets, &request_identity.principal_id)
        .await
        .map(Json)
        .map_err(repo_project_api_error)
}

pub(crate) async fn repo_project_connect_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Json(input): Json<repo_projects_api::RepoConnectInput>,
) -> Result<Json<repo_projects_api::RepoConnectResponse>, (StatusCode, String)> {
    let store = repo_project_store(&state)?;
    let secrets = repo_project_secrets(&state)?;
    let response =
        repo_projects_api::connect_repos(store, secrets, &request_identity.principal_id, input)
            .await
            .map_err(repo_project_api_error)?;
    for full_name in &response.connected {
        state.sse.broadcast(SseEvent::RepoProjectEvent {
            project_id: String::new(),
            event_type: "repo_project_connected".to_string(),
            message: format!("Connected {full_name}"),
        });
    }
    Ok(Json(response))
}

pub(crate) async fn repo_project_enroll_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
    Json(input): Json<repo_projects_api::RepoEnrollInput>,
) -> Result<Json<repo_projects_api::RepoProjectCommandResponse>, (StatusCode, String)> {
    let store = repo_project_store(&state)?;
    let id = parse_repo_project_id(&id)?;
    let response =
        repo_projects_api::enroll_repo(store, &request_identity.principal_id, id, input)
            .await
            .map_err(repo_project_api_error)?;
    broadcast_project_response(&state, &response);
    wake_project_supervisor(&state, id, RepoSupervisorWakeReason::Manual).await;
    Ok(Json(response))
}
