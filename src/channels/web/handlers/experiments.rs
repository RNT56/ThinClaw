use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
};
use serde::Deserialize;
use uuid::Uuid;

use crate::api::experiments as experiments_api;
use crate::channels::web::handlers::providers::secret_exists;
use crate::channels::web::identity_helpers::{
    GatewayRequestIdentity, request_identity_with_overrides,
};
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;
use crate::db::Database;

#[derive(Debug, Deserialize, Default)]
pub(crate) struct ExperimentsQuery {
    #[serde(default)]
    user_id: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct ExperimentsLimitQuery {
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

fn experiment_api_error(error: crate::api::ApiError) -> (StatusCode, String) {
    match error {
        crate::api::ApiError::InvalidInput(message) => (StatusCode::BAD_REQUEST, message),
        crate::api::ApiError::SessionNotFound(message) => (StatusCode::NOT_FOUND, message),
        crate::api::ApiError::Unavailable(message) => (StatusCode::SERVICE_UNAVAILABLE, message),
        crate::api::ApiError::FeatureDisabled(message) => (StatusCode::FORBIDDEN, message),
        other => (StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
    }
}

fn experiment_lease_token(headers: &HeaderMap) -> Result<String, (StatusCode, String)> {
    if let Some(value) = headers
        .get("x-experiment-lease-token")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(value.to_string());
    }

    if let Some(value) = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(value.to_string());
    }

    Err((
        StatusCode::UNAUTHORIZED,
        "Missing experiment lease token".to_string(),
    ))
}

fn experiment_store(state: &GatewayState) -> Result<&Arc<dyn Database>, (StatusCode, String)> {
    state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))
}

fn status_to_sse_string<T: serde::Serialize>(status: &T) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|value| value.as_str().map(ToString::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

fn broadcast_campaign_update(
    state: &GatewayState,
    response: &experiments_api::ExperimentCampaignActionResponse,
) {
    state.sse.broadcast(SseEvent::ExperimentCampaignUpdated {
        campaign_id: response.campaign.id.to_string(),
        status: status_to_sse_string(&response.campaign.status),
        message: response.message.clone(),
    });
    if let Some(trial) = response.trial.as_ref() {
        state.sse.broadcast(SseEvent::ExperimentTrialUpdated {
            campaign_id: response.campaign.id.to_string(),
            trial_id: trial.id.to_string(),
            status: status_to_sse_string(&trial.status),
            message: trial
                .decision_reason
                .clone()
                .or(trial.summary.clone())
                .unwrap_or_else(|| response.message.clone()),
        });
    }
}

fn broadcast_experiment_opportunity_update(
    state: &GatewayState,
    opportunity_id: impl Into<String>,
    status: impl Into<String>,
    message: impl Into<String>,
) {
    state.sse.broadcast(SseEvent::ExperimentOpportunityUpdated {
        opportunity_id: opportunity_id.into(),
        status: status.into(),
        message: message.into(),
    });
}

fn research_gpu_cloud_backend(
    provider: &str,
) -> Option<crate::experiments::ExperimentRunnerBackend> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "runpod" => Some(crate::experiments::ExperimentRunnerBackend::Runpod),
        "vast" | "vast.ai" => Some(crate::experiments::ExperimentRunnerBackend::Vast),
        "lambda" => Some(crate::experiments::ExperimentRunnerBackend::Lambda),
        _ => None,
    }
}

fn research_gpu_cloud_info(
    provider: &str,
    connected: bool,
) -> Option<experiments_api::ExperimentGpuCloudProviderInfo> {
    let backend = research_gpu_cloud_backend(provider)?;
    Some(experiments_api::ExperimentGpuCloudProviderInfo {
        slug: backend.slug().to_string(),
        display_name: crate::experiments::adapters::gpu_cloud_display_name(backend).to_string(),
        backend,
        description: format!(
            "{} setup for outbound ThinClaw experiment runners.",
            crate::experiments::adapters::gpu_cloud_display_name(backend)
        ),
        signup_url: crate::experiments::adapters::gpu_cloud_signup_url(backend)
            .unwrap_or_default()
            .to_string(),
        docs_url: crate::experiments::adapters::gpu_cloud_docs_url(backend)
            .unwrap_or_default()
            .to_string(),
        secret_name: crate::experiments::adapters::gpu_cloud_secret_name(backend)
            .unwrap_or_default()
            .to_string(),
        connected,
        template_hint: Some(crate::experiments::adapters::gpu_cloud_template_hint(
            backend,
        )),
    })
}

type GpuCloudRunnerPayload = (
    experiments_api::CreateExperimentRunnerProfileRequest,
    Vec<String>,
    Option<serde_json::Value>,
);

fn research_gpu_cloud_default_runner_payload(
    backend: crate::experiments::ExperimentRunnerBackend,
    req: &ExperimentGpuCloudTemplateRequest,
) -> Result<GpuCloudRunnerPayload, (StatusCode, String)> {
    let runner_name = req
        .runner_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            crate::experiments::adapters::gpu_cloud_default_runner_name(backend).to_string()
        });
    let image_or_runtime = req
        .image_or_runtime
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            crate::experiments::adapters::default_research_runner_image().to_string()
        });
    let (backend_config, warnings, launch_payload_preview) = if backend
        == crate::experiments::ExperimentRunnerBackend::Lambda
    {
        let instance_type_name = req
            .instance_type_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or((
                StatusCode::BAD_REQUEST,
                "Lambda template requires an instance type name.".to_string(),
            ))?;
        let (backend_config, warnings) = crate::experiments::adapters::build_lambda_backend_config(
            &crate::experiments::adapters::LambdaLaunchTemplateInput {
                region_name: req.region_name.clone(),
                instance_type_name: instance_type_name.to_string(),
                quantity: req.quantity,
                ssh_key_names: req.ssh_key_names.clone(),
                file_system_names: req.file_system_names.clone(),
            },
        );
        let preview = backend_config.get("launch_payload").cloned();
        (backend_config, warnings, preview)
    } else {
        (
            crate::experiments::adapters::gpu_cloud_default_backend_config(backend),
            Vec::new(),
            None,
        )
    };
    Ok((
        experiments_api::CreateExperimentRunnerProfileRequest {
            name: runner_name,
            backend,
            backend_config,
            image_or_runtime: Some(image_or_runtime),
            gpu_requirements: crate::experiments::adapters::gpu_cloud_default_gpu_requirements(
                backend,
            ),
            env_grants: serde_json::json!({}),
            secret_references: crate::experiments::adapters::gpu_cloud_secret_name(backend)
                .map(|value| vec![value.to_string()])
                .unwrap_or_default(),
            cache_policy: serde_json::json!({
                "persist_workspace": false,
                "provider": backend.slug(),
            }),
        },
        warnings,
        launch_payload_preview,
    ))
}

fn research_limit(value: Option<usize>, default: usize) -> usize {
    value.unwrap_or(default).clamp(1, 500)
}

async fn experiments_request_identity(
    state: &GatewayState,
    request_identity: &GatewayRequestIdentity,
    requested_user_id: Option<&str>,
) -> GatewayRequestIdentity {
    request_identity_with_overrides(state, request_identity, requested_user_id, None).await
}

pub(crate) async fn experiments_projects_list_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Query(query): Query<ExperimentsQuery>,
) -> Result<Json<experiments_api::ExperimentProjectListResponse>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let request_identity =
        experiments_request_identity(&state, &request_identity, query.user_id.as_deref()).await;
    experiments_api::list_projects(store, &request_identity.principal_id)
        .await
        .map(Json)
        .map_err(experiment_api_error)
}

pub(crate) async fn experiments_project_detail_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
    Query(query): Query<ExperimentsQuery>,
) -> Result<Json<crate::experiments::ExperimentProject>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let request_identity =
        experiments_request_identity(&state, &request_identity, query.user_id.as_deref()).await;
    let id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid project ID".to_string()))?;
    experiments_api::get_project(store, &request_identity.principal_id, id)
        .await
        .map(Json)
        .map_err(experiment_api_error)
}

pub(crate) async fn experiments_project_create_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Json(req): Json<experiments_api::CreateExperimentProjectRequest>,
) -> Result<(StatusCode, Json<crate::experiments::ExperimentProject>), (StatusCode, String)> {
    let store = experiment_store(&state)?;
    experiments_api::create_project(store, &request_identity.principal_id, req)
        .await
        .map(|project| (StatusCode::CREATED, Json(project)))
        .map_err(experiment_api_error)
}

pub(crate) async fn experiments_project_update_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
    Json(req): Json<experiments_api::UpdateExperimentProjectRequest>,
) -> Result<Json<crate::experiments::ExperimentProject>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid project ID".to_string()))?;
    experiments_api::update_project(store, &request_identity.principal_id, id, req)
        .await
        .map(Json)
        .map_err(experiment_api_error)
}

pub(crate) async fn experiments_project_delete_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid project ID".to_string()))?;
    let deleted = experiments_api::delete_project(store, &request_identity.principal_id, id)
        .await
        .map_err(experiment_api_error)?;
    Ok(Json(serde_json::json!({ "deleted": deleted })))
}

pub(crate) async fn experiments_runners_list_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Query(query): Query<ExperimentsQuery>,
) -> Result<Json<experiments_api::ExperimentRunnerListResponse>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let request_identity =
        experiments_request_identity(&state, &request_identity, query.user_id.as_deref()).await;
    experiments_api::list_runners(store, &request_identity.principal_id)
        .await
        .map(Json)
        .map_err(experiment_api_error)
}

pub(crate) async fn experiments_runner_detail_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
    Query(query): Query<ExperimentsQuery>,
) -> Result<Json<crate::experiments::ExperimentRunnerProfile>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let request_identity =
        experiments_request_identity(&state, &request_identity, query.user_id.as_deref()).await;
    let id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid runner ID".to_string()))?;
    experiments_api::get_runner(store, &request_identity.principal_id, id)
        .await
        .map(Json)
        .map_err(experiment_api_error)
}

pub(crate) async fn experiments_runner_create_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Json(req): Json<experiments_api::CreateExperimentRunnerProfileRequest>,
) -> Result<
    (
        StatusCode,
        Json<crate::experiments::ExperimentRunnerProfile>,
    ),
    (StatusCode, String),
> {
    let store = experiment_store(&state)?;
    experiments_api::create_runner(store, &request_identity.principal_id, req)
        .await
        .map(|runner| (StatusCode::CREATED, Json(runner)))
        .map_err(experiment_api_error)
}

pub(crate) async fn experiments_runner_update_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
    Json(req): Json<experiments_api::UpdateExperimentRunnerProfileRequest>,
) -> Result<Json<crate::experiments::ExperimentRunnerProfile>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid runner ID".to_string()))?;
    experiments_api::update_runner(store, &request_identity.principal_id, id, req)
        .await
        .map(Json)
        .map_err(experiment_api_error)
}

pub(crate) async fn experiments_runner_delete_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid runner ID".to_string()))?;
    let deleted = experiments_api::delete_runner(store, &request_identity.principal_id, id)
        .await
        .map_err(experiment_api_error)?;
    Ok(Json(serde_json::json!({ "deleted": deleted })))
}

pub(crate) async fn experiments_runner_validate_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<experiments_api::ExperimentRunnerValidationResponse>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid runner ID".to_string()))?;
    let response = experiments_api::validate_runner(store, &request_identity.principal_id, id)
        .await
        .map_err(experiment_api_error)?;
    state.sse.broadcast(SseEvent::ExperimentRunnerUpdated {
        runner_id: response.runner.id.to_string(),
        status: status_to_sse_string(&response.runner.status),
        message: response.message.clone(),
    });
    Ok(Json(response))
}

pub(crate) async fn experiments_campaign_start_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
    Json(req): Json<experiments_api::StartExperimentCampaignRequest>,
) -> Result<
    (
        StatusCode,
        Json<experiments_api::ExperimentCampaignActionResponse>,
    ),
    (StatusCode, String),
> {
    let store = experiment_store(&state)?;
    let id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid project ID".to_string()))?;
    let response = experiments_api::start_campaign(store, &request_identity.principal_id, id, req)
        .await
        .map_err(experiment_api_error)?;
    broadcast_campaign_update(&state, &response);
    Ok((StatusCode::CREATED, Json(response)))
}

pub(crate) async fn experiments_campaigns_list_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Query(query): Query<ExperimentsQuery>,
) -> Result<Json<experiments_api::ExperimentCampaignListResponse>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let request_identity =
        experiments_request_identity(&state, &request_identity, query.user_id.as_deref()).await;
    experiments_api::list_campaigns(store, &request_identity.principal_id)
        .await
        .map(Json)
        .map_err(experiment_api_error)
}

pub(crate) async fn experiments_campaign_detail_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
    Query(query): Query<ExperimentsQuery>,
) -> Result<Json<crate::experiments::ExperimentCampaign>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let request_identity =
        experiments_request_identity(&state, &request_identity, query.user_id.as_deref()).await;
    let id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid campaign ID".to_string()))?;
    experiments_api::get_campaign(store, &request_identity.principal_id, id)
        .await
        .map(Json)
        .map_err(experiment_api_error)
}

pub(crate) async fn experiments_campaign_pause_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<experiments_api::ExperimentCampaignActionResponse>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid campaign ID".to_string()))?;
    let response = experiments_api::pause_campaign(store, &request_identity.principal_id, id)
        .await
        .map_err(experiment_api_error)?;
    broadcast_campaign_update(&state, &response);
    Ok(Json(response))
}

pub(crate) async fn experiments_campaign_resume_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<experiments_api::ExperimentCampaignActionResponse>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid campaign ID".to_string()))?;
    let response = experiments_api::resume_campaign(store, &request_identity.principal_id, id)
        .await
        .map_err(experiment_api_error)?;
    broadcast_campaign_update(&state, &response);
    Ok(Json(response))
}

pub(crate) async fn experiments_campaign_cancel_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<experiments_api::ExperimentCampaignActionResponse>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid campaign ID".to_string()))?;
    let response = experiments_api::cancel_campaign(store, &request_identity.principal_id, id)
        .await
        .map_err(experiment_api_error)?;
    broadcast_campaign_update(&state, &response);
    Ok(Json(response))
}

pub(crate) async fn experiments_campaign_promote_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<experiments_api::ExperimentCampaignActionResponse>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid campaign ID".to_string()))?;
    let response = experiments_api::promote_campaign(store, &request_identity.principal_id, id)
        .await
        .map_err(experiment_api_error)?;
    broadcast_campaign_update(&state, &response);
    Ok(Json(response))
}

pub(crate) async fn experiments_trials_list_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
    Query(query): Query<ExperimentsQuery>,
) -> Result<Json<experiments_api::ExperimentTrialListResponse>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let request_identity =
        experiments_request_identity(&state, &request_identity, query.user_id.as_deref()).await;
    let id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid campaign ID".to_string()))?;
    experiments_api::list_trials(store, &request_identity.principal_id, id)
        .await
        .map(Json)
        .map_err(experiment_api_error)
}

pub(crate) async fn experiments_trial_detail_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
    Query(query): Query<ExperimentsQuery>,
) -> Result<Json<crate::experiments::ExperimentTrial>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let request_identity =
        experiments_request_identity(&state, &request_identity, query.user_id.as_deref()).await;
    let id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid trial ID".to_string()))?;
    experiments_api::get_trial(store, &request_identity.principal_id, id)
        .await
        .map(Json)
        .map_err(experiment_api_error)
}

pub(crate) async fn experiments_artifacts_list_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
    Query(query): Query<ExperimentsQuery>,
) -> Result<Json<experiments_api::ExperimentArtifactListResponse>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let request_identity =
        experiments_request_identity(&state, &request_identity, query.user_id.as_deref()).await;
    let id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid trial ID".to_string()))?;
    experiments_api::list_artifacts(store, &request_identity.principal_id, id)
        .await
        .map(Json)
        .map_err(experiment_api_error)
}

pub(crate) async fn experiments_targets_list_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Query(query): Query<ExperimentsQuery>,
) -> Result<Json<experiments_api::ExperimentTargetListResponse>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let request_identity =
        experiments_request_identity(&state, &request_identity, query.user_id.as_deref()).await;
    experiments_api::list_targets(store, &request_identity.principal_id)
        .await
        .map(Json)
        .map_err(experiment_api_error)
}

pub(crate) async fn experiments_target_create_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Json(req): Json<experiments_api::CreateExperimentTargetRequest>,
) -> Result<(StatusCode, Json<crate::experiments::ExperimentTarget>), (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let target = experiments_api::create_target(store, &request_identity.principal_id, req)
        .await
        .map_err(experiment_api_error)?;
    broadcast_experiment_opportunity_update(
        &state,
        target.id.to_string(),
        "updated",
        format!("Target '{}' linked.", target.name),
    );
    Ok((StatusCode::CREATED, Json(target)))
}

pub(crate) async fn experiments_target_link_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Json(req): Json<experiments_api::LinkExperimentTargetRequest>,
) -> Result<Json<crate::experiments::ExperimentTarget>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let target = experiments_api::link_target(store, &request_identity.principal_id, req)
        .await
        .map_err(experiment_api_error)?;
    broadcast_experiment_opportunity_update(
        &state,
        target.id.to_string(),
        "updated",
        format!("Target '{}' linked to a research opportunity.", target.name),
    );
    Ok(Json(target))
}

pub(crate) async fn experiments_target_update_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
    Json(req): Json<experiments_api::UpdateExperimentTargetRequest>,
) -> Result<Json<crate::experiments::ExperimentTarget>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid target ID".to_string()))?;
    let target = experiments_api::update_target(store, &request_identity.principal_id, id, req)
        .await
        .map_err(experiment_api_error)?;
    broadcast_experiment_opportunity_update(
        &state,
        target.id.to_string(),
        "updated",
        format!("Target '{}' updated.", target.name),
    );
    Ok(Json(target))
}

pub(crate) async fn experiments_target_delete_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid target ID".to_string()))?;
    if !experiments_api::delete_target(store, &request_identity.principal_id, id)
        .await
        .map_err(experiment_api_error)?
    {
        return Err((
            StatusCode::NOT_FOUND,
            format!("Experiment target {id} not found"),
        ));
    }
    broadcast_experiment_opportunity_update(
        &state,
        id.to_string(),
        "deleted",
        "Target removed from research opportunities.".to_string(),
    );
    Ok(Json(serde_json::json!({ "deleted": true })))
}

pub(crate) async fn experiments_model_usage_list_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Query(query): Query<ExperimentsLimitQuery>,
) -> Result<Json<experiments_api::ExperimentModelUsageListResponse>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let request_identity =
        experiments_request_identity(&state, &request_identity, query.user_id.as_deref()).await;
    experiments_api::list_model_usage(
        store,
        &request_identity.principal_id,
        research_limit(query.limit, 100),
    )
    .await
    .map(Json)
    .map_err(experiment_api_error)
}

pub(crate) async fn experiments_opportunities_list_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Query(query): Query<ExperimentsLimitQuery>,
) -> Result<Json<experiments_api::ExperimentOpportunityListResponse>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let request_identity =
        experiments_request_identity(&state, &request_identity, query.user_id.as_deref()).await;
    experiments_api::list_opportunities(
        store,
        &request_identity.principal_id,
        research_limit(query.limit, 100),
    )
    .await
    .map(Json)
    .map_err(experiment_api_error)
}

pub(crate) async fn experiments_gpu_clouds_list_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Query(query): Query<ExperimentsQuery>,
) -> Result<Json<experiments_api::ExperimentGpuCloudProviderListResponse>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let request_identity =
        experiments_request_identity(&state, &request_identity, query.user_id.as_deref()).await;
    let mut response =
        experiments_api::list_gpu_cloud_providers(store, &request_identity.principal_id)
            .await
            .map_err(experiment_api_error)?;
    if let Some(secrets) = state.secrets_store.as_ref() {
        for provider in &mut response.providers {
            provider.connected = secret_exists(
                Some(secrets),
                &request_identity.principal_id,
                &provider.secret_name,
            )
            .await;
        }
    }
    Ok(Json(response))
}

pub(crate) async fn experiments_gpu_cloud_connect_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(provider): Path<String>,
    Json(req): Json<ExperimentGpuCloudConnectRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let backend = research_gpu_cloud_backend(&provider).ok_or((
        StatusCode::NOT_FOUND,
        "Unknown GPU cloud provider".to_string(),
    ))?;
    let secret_name = crate::experiments::adapters::gpu_cloud_secret_name(backend).ok_or((
        StatusCode::NOT_FOUND,
        "Provider does not define a research secret".to_string(),
    ))?;
    let secrets = state.secrets_store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Secrets store not available".to_string(),
    ))?;
    let api_key = req.api_key.trim().to_string();
    if api_key.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "API key is required".to_string()));
    }
    let _ = secrets
        .delete(&request_identity.principal_id, secret_name)
        .await;
    let params =
        crate::secrets::CreateSecretParams::new(secret_name, api_key).with_provider(backend.slug());
    secrets
        .create(&request_identity.principal_id, params)
        .await
        .map_err(|err| {
            tracing::error!(
                provider = %provider,
                error = %err,
                "Failed to save research GPU cloud credential"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
        })?;
    let _ = crate::config::refresh_secrets(secrets.as_ref(), &request_identity.principal_id).await;
    let info = research_gpu_cloud_info(&provider, true).ok_or((
        StatusCode::NOT_FOUND,
        "Unknown GPU cloud provider".to_string(),
    ))?;
    broadcast_experiment_opportunity_update(
        &state,
        info.slug.clone(),
        "connected",
        format!("{} credentials connected.", info.display_name),
    );
    Ok(Json(serde_json::json!({
        "status": "ok",
        "message": format!("{} credentials connected.", info.display_name),
        "provider": info,
    })))
}

pub(crate) async fn experiments_gpu_cloud_validate_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(provider): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let backend = research_gpu_cloud_backend(&provider).ok_or((
        StatusCode::NOT_FOUND,
        "Unknown GPU cloud provider".to_string(),
    ))?;
    let secret_name = crate::experiments::adapters::gpu_cloud_secret_name(backend).ok_or((
        StatusCode::NOT_FOUND,
        "Provider does not define a research secret".to_string(),
    ))?;
    let secrets = state.secrets_store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Secrets store not available".to_string(),
    ))?;
    let connected = secret_exists(Some(secrets), &request_identity.principal_id, secret_name).await;
    let info = research_gpu_cloud_info(&provider, connected).ok_or((
        StatusCode::NOT_FOUND,
        "Unknown GPU cloud provider".to_string(),
    ))?;
    let (status, message) = if connected {
        match secrets
            .get_decrypted(&request_identity.principal_id, secret_name)
            .await
        {
            Ok(secret) => match crate::experiments::adapters::validate_gpu_cloud_credentials(
                backend,
                secret.expose(),
            )
            .await
            {
                Ok(message) => ("ok".to_string(), message),
                Err(message) => ("invalid_credentials".to_string(), message),
            },
            Err(err) => (
                "invalid_credentials".to_string(),
                format!("Failed to load {} credential: {}", info.display_name, err),
            ),
        }
    } else {
        (
            "missing_credentials".to_string(),
            format!("{} credentials are missing.", info.display_name),
        )
    };
    Ok(Json(serde_json::json!({
        "status": status,
        "message": message,
        "connected": connected,
        "provider": info,
    })))
}

pub(crate) async fn experiments_gpu_cloud_template_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(provider): Path<String>,
    Json(req): Json<ExperimentGpuCloudTemplateRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    experiments_api::list_gpu_cloud_providers(store, &request_identity.principal_id)
        .await
        .map_err(experiment_api_error)?;
    let backend = research_gpu_cloud_backend(&provider).ok_or((
        StatusCode::NOT_FOUND,
        "Unknown GPU cloud provider".to_string(),
    ))?;
    let connected =
        if let Some(secret_name) = crate::experiments::adapters::gpu_cloud_secret_name(backend) {
            secret_exists(
                state.secrets_store.as_ref(),
                &request_identity.principal_id,
                secret_name,
            )
            .await
        } else {
            false
        };
    let info = research_gpu_cloud_info(&provider, connected).ok_or((
        StatusCode::NOT_FOUND,
        "Unknown GPU cloud provider".to_string(),
    ))?;
    let (runner_payload, warnings, launch_payload_preview) =
        research_gpu_cloud_default_runner_payload(backend, &req)?;
    let message = match backend {
        crate::experiments::ExperimentRunnerBackend::Lambda => {
            "Lambda runner template is launch-ready. ThinClaw built backend_config.launch_payload from the normalized Research form."
                .to_string()
        }
        _ => format!(
            "{} runner template is ready for Research campaigns.",
            info.display_name
        ),
    };
    Ok(Json(serde_json::json!({
        "status": "ok",
        "message": message,
        "warnings": warnings,
        "provider": info,
        "runner_payload": runner_payload,
        "launch_payload_preview": launch_payload_preview,
    })))
}

pub(crate) async fn experiments_gpu_cloud_launch_test_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(provider): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let backend = research_gpu_cloud_backend(&provider).ok_or((
        StatusCode::NOT_FOUND,
        "Unknown GPU cloud provider".to_string(),
    ))?;
    let secret_name = crate::experiments::adapters::gpu_cloud_secret_name(backend).ok_or((
        StatusCode::NOT_FOUND,
        "Provider does not define a research secret".to_string(),
    ))?;
    let secrets = state.secrets_store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Secrets store not available".to_string(),
    ))?;
    let connected = secret_exists(Some(secrets), &request_identity.principal_id, secret_name).await;
    if !connected {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "{} credentials must be connected before launching a test job.",
                crate::experiments::adapters::gpu_cloud_display_name(backend)
            ),
        ));
    }
    let info = research_gpu_cloud_info(&provider, connected).ok_or((
        StatusCode::NOT_FOUND,
        "Unknown GPU cloud provider".to_string(),
    ))?;
    let secret = secrets
        .get_decrypted(&request_identity.principal_id, secret_name)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let validation_message =
        crate::experiments::adapters::validate_gpu_cloud_credentials(backend, secret.expose())
            .await
            .map_err(|err| (StatusCode::BAD_REQUEST, err))?;
    broadcast_experiment_opportunity_update(
        &state,
        info.slug.clone(),
        "ready",
        format!("{} test launch requested.", info.display_name),
    );
    Ok(Json(serde_json::json!({
        "status": "ok",
        "message": match backend {
            crate::experiments::ExperimentRunnerBackend::Lambda =>
                "Lambda credentials validated. Use the Lambda launch form to create a launch-ready Research runner with a server-built launch payload.".to_string(),
            _ => format!("{validation_message} ThinClaw can now auto-launch provider compute from validated Research runners."),
        },
        "provider": info,
        "launch_hint": {
            "backend": backend.slug(),
            "runner_template_required": false,
            "bootstrap": if backend == crate::experiments::ExperimentRunnerBackend::Lambda {
                "Lambda credentials are live. Build a runner from the Lambda GPU Clouds form to get a controller-managed launch payload automatically."
            } else {
                "Create a research runner profile, then start a campaign to let ThinClaw auto-launch provider compute with a lease token."
            }
        }
    })))
}

pub(crate) async fn experiments_campaign_reissue_lease_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<experiments_api::ExperimentCampaignActionResponse>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid campaign ID".to_string()))?;
    let response = experiments_api::reissue_lease(store, &request_identity.principal_id, id)
        .await
        .map_err(experiment_api_error)?;
    broadcast_campaign_update(&state, &response);
    Ok(Json(response))
}

pub(crate) async fn experiment_lease_job_handler(
    State(state): State<Arc<GatewayState>>,
    Path(lease_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<experiments_api::ExperimentLeaseJobResponse>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let token = experiment_lease_token(&headers)?;
    let lease_id = Uuid::parse_str(&lease_id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid lease ID".to_string()))?;
    let user_id = experiments_api::lease_owner_user_id(store, lease_id, &token)
        .await
        .map_err(experiment_api_error)?;
    experiments_api::lease_job(store, &user_id, lease_id, &token)
        .await
        .map(Json)
        .map_err(experiment_api_error)
}

pub(crate) async fn experiment_lease_credentials_handler(
    State(state): State<Arc<GatewayState>>,
    Path(lease_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<experiments_api::ExperimentLeaseCredentialsResponse>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let token = experiment_lease_token(&headers)?;
    let lease_id = Uuid::parse_str(&lease_id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid lease ID".to_string()))?;
    let user_id = experiments_api::lease_owner_user_id(store, lease_id, &token)
        .await
        .map_err(experiment_api_error)?;
    experiments_api::lease_credentials(store, &user_id, lease_id, &token)
        .await
        .map(Json)
        .map_err(experiment_api_error)
}

pub(crate) async fn experiment_lease_status_handler(
    State(state): State<Arc<GatewayState>>,
    Path(lease_id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<experiments_api::ExperimentLeaseStatusRequest>,
) -> Result<Json<experiments_api::ExperimentCampaignActionResponse>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let token = experiment_lease_token(&headers)?;
    let lease_id = Uuid::parse_str(&lease_id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid lease ID".to_string()))?;
    let user_id = experiments_api::lease_owner_user_id(store, lease_id, &token)
        .await
        .map_err(experiment_api_error)?;
    let response = experiments_api::lease_status(store, &user_id, lease_id, &token, req)
        .await
        .map_err(experiment_api_error)?;
    broadcast_campaign_update(&state, &response);
    Ok(Json(response))
}

pub(crate) async fn experiment_lease_event_handler(
    State(state): State<Arc<GatewayState>>,
    Path(lease_id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<experiments_api::ExperimentLeaseEventRequest>,
) -> Result<Json<experiments_api::ExperimentCampaignActionResponse>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let token = experiment_lease_token(&headers)?;
    let lease_id = Uuid::parse_str(&lease_id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid lease ID".to_string()))?;
    let user_id = experiments_api::lease_owner_user_id(store, lease_id, &token)
        .await
        .map_err(experiment_api_error)?;
    let response = experiments_api::lease_event(store, &user_id, lease_id, &token, req)
        .await
        .map_err(experiment_api_error)?;
    broadcast_campaign_update(&state, &response);
    Ok(Json(response))
}

pub(crate) async fn experiment_lease_artifact_handler(
    State(state): State<Arc<GatewayState>>,
    Path(lease_id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<crate::experiments::ExperimentRunnerArtifactUpload>,
) -> Result<Json<experiments_api::ExperimentCampaignActionResponse>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let token = experiment_lease_token(&headers)?;
    let lease_id = Uuid::parse_str(&lease_id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid lease ID".to_string()))?;
    let user_id = experiments_api::lease_owner_user_id(store, lease_id, &token)
        .await
        .map_err(experiment_api_error)?;
    let response = experiments_api::lease_artifact(store, &user_id, lease_id, &token, req)
        .await
        .map_err(experiment_api_error)?;
    broadcast_campaign_update(&state, &response);
    Ok(Json(response))
}

pub(crate) async fn experiment_lease_complete_handler(
    State(state): State<Arc<GatewayState>>,
    Path(lease_id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<crate::experiments::ExperimentRunnerCompletion>,
) -> Result<Json<experiments_api::ExperimentCampaignActionResponse>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let token = experiment_lease_token(&headers)?;
    let lease_id = Uuid::parse_str(&lease_id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid lease ID".to_string()))?;
    let user_id = experiments_api::lease_owner_user_id(store, lease_id, &token)
        .await
        .map_err(experiment_api_error)?;
    let response = experiments_api::lease_complete(store, &user_id, lease_id, &token, req)
        .await
        .map_err(experiment_api_error)?;
    broadcast_campaign_update(&state, &response);
    Ok(Json(response))
}
