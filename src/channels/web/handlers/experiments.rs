use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};

use crate::api::experiments as experiments_api;
use crate::channels::web::handlers::providers::secret_exists;
use crate::channels::web::identity_helpers::{
    GatewayRequestIdentity, request_identity_with_overrides,
};
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;
use crate::db::Database;
use thinclaw_gateway::web::api::{FeatureDisabledStatus, gateway_api_error_response};
use thinclaw_gateway::web::experiments::{
    ExperimentDeleteResponse, ResearchGpuCloudConnectResponse, ResearchGpuCloudLaunchTestResponse,
    ResearchGpuCloudTemplateResponse, ResearchGpuCloudValidateResponse,
    experiment_bad_request_error, experiment_campaign_action_update_message,
    experiment_database_unavailable_error, experiment_delete_response,
    experiment_internal_server_error, experiment_lease_token_from_headers,
    experiment_runner_backend, experiment_runner_payload,
    experiment_secrets_store_unavailable_error, experiment_target_created_update_message,
    experiment_target_deleted_update_message, experiment_target_linked_update_message,
    experiment_target_not_found_error, experiment_target_updated_update_message,
    parse_experiment_campaign_id, parse_experiment_lease_id, parse_experiment_project_id,
    parse_experiment_runner_id, parse_experiment_target_id, parse_experiment_trial_id,
    research_gpu_cloud_api_key, research_gpu_cloud_backend_or_error,
    research_gpu_cloud_connect_response, research_gpu_cloud_connected_update_message,
    research_gpu_cloud_credential_load_validation, research_gpu_cloud_default_runner_template,
    research_gpu_cloud_info_or_error, research_gpu_cloud_invalid_credentials_validation,
    research_gpu_cloud_launch_missing_credentials_error,
    research_gpu_cloud_launch_requested_update_message, research_gpu_cloud_launch_test_response,
    research_gpu_cloud_missing_credentials_validation, research_gpu_cloud_template_response,
    research_gpu_cloud_validate_response, research_limit, status_to_sse_string,
};

fn experiment_api_error(error: crate::api::ApiError) -> (StatusCode, String) {
    gateway_api_error_response(error, FeatureDisabledStatus::Forbidden)
}

fn experiment_store(state: &GatewayState) -> Result<&Arc<dyn Database>, (StatusCode, String)> {
    state
        .store
        .as_ref()
        .ok_or_else(experiment_database_unavailable_error)
}

fn broadcast_campaign_update(
    state: &GatewayState,
    response: &experiments_api::ExperimentCampaignActionResponse,
) {
    let update = experiment_campaign_action_update_message(response);
    state.sse.broadcast(SseEvent::ExperimentCampaignUpdated {
        campaign_id: update.campaign_id,
        status: update.status,
        message: update.message,
    });
    if let Some(trial) = update.trial {
        state.sse.broadcast(SseEvent::ExperimentTrialUpdated {
            campaign_id: trial.campaign_id,
            trial_id: trial.trial_id,
            status: trial.status,
            message: trial.message,
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
    let id = parse_experiment_project_id(&id)?;
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
    let id = parse_experiment_project_id(&id)?;
    experiments_api::update_project(store, &request_identity.principal_id, id, req)
        .await
        .map(Json)
        .map_err(experiment_api_error)
}

pub(crate) async fn experiments_project_delete_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<ExperimentDeleteResponse>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let id = parse_experiment_project_id(&id)?;
    let deleted = experiments_api::delete_project(store, &request_identity.principal_id, id)
        .await
        .map_err(experiment_api_error)?;
    Ok(Json(experiment_delete_response(deleted)))
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
    let id = parse_experiment_runner_id(&id)?;
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
    let id = parse_experiment_runner_id(&id)?;
    experiments_api::update_runner(store, &request_identity.principal_id, id, req)
        .await
        .map(Json)
        .map_err(experiment_api_error)
}

pub(crate) async fn experiments_runner_delete_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<ExperimentDeleteResponse>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let id = parse_experiment_runner_id(&id)?;
    let deleted = experiments_api::delete_runner(store, &request_identity.principal_id, id)
        .await
        .map_err(experiment_api_error)?;
    Ok(Json(experiment_delete_response(deleted)))
}

pub(crate) async fn experiments_runner_validate_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<experiments_api::ExperimentRunnerValidationResponse>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let id = parse_experiment_runner_id(&id)?;
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
    let id = parse_experiment_project_id(&id)?;
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
    let id = parse_experiment_campaign_id(&id)?;
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
    let id = parse_experiment_campaign_id(&id)?;
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
    let id = parse_experiment_campaign_id(&id)?;
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
    let id = parse_experiment_campaign_id(&id)?;
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
    let id = parse_experiment_campaign_id(&id)?;
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
    let id = parse_experiment_campaign_id(&id)?;
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
    let id = parse_experiment_trial_id(&id)?;
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
    let id = parse_experiment_trial_id(&id)?;
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
    let update = experiment_target_created_update_message(&target);
    broadcast_experiment_opportunity_update(
        &state,
        update.opportunity_id,
        update.status,
        update.message,
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
    let update = experiment_target_linked_update_message(&target);
    broadcast_experiment_opportunity_update(
        &state,
        update.opportunity_id,
        update.status,
        update.message,
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
    let id = parse_experiment_target_id(&id)?;
    let target = experiments_api::update_target(store, &request_identity.principal_id, id, req)
        .await
        .map_err(experiment_api_error)?;
    let update = experiment_target_updated_update_message(&target);
    broadcast_experiment_opportunity_update(
        &state,
        update.opportunity_id,
        update.status,
        update.message,
    );
    Ok(Json(target))
}

pub(crate) async fn experiments_target_delete_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<ExperimentDeleteResponse>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let id = parse_experiment_target_id(&id)?;
    if !experiments_api::delete_target(store, &request_identity.principal_id, id)
        .await
        .map_err(experiment_api_error)?
    {
        return Err(experiment_target_not_found_error(id));
    }
    let update = experiment_target_deleted_update_message(id);
    broadcast_experiment_opportunity_update(
        &state,
        update.opportunity_id,
        update.status,
        update.message,
    );
    Ok(Json(experiment_delete_response(true)))
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
) -> Result<Json<ResearchGpuCloudConnectResponse>, (StatusCode, String)> {
    let backend = research_gpu_cloud_backend_or_error(&provider)?;
    let secret_name = backend.secret_name();
    let secrets = state
        .secrets_store
        .as_ref()
        .ok_or_else(experiment_secrets_store_unavailable_error)?;
    let api_key = research_gpu_cloud_api_key(&req)
        .map_err(|error| (error.status_code(), error.to_string()))?;
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
            experiment_internal_server_error(err)
        })?;
    let _ = crate::config::refresh_secrets(secrets.as_ref(), &request_identity.principal_id).await;
    let info = research_gpu_cloud_info_or_error(&provider, true)?;
    let update = research_gpu_cloud_connected_update_message(&info);
    broadcast_experiment_opportunity_update(
        &state,
        update.opportunity_id,
        update.status,
        update.message,
    );
    Ok(Json(research_gpu_cloud_connect_response(info)))
}

pub(crate) async fn experiments_gpu_cloud_validate_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(provider): Path<String>,
) -> Result<Json<ResearchGpuCloudValidateResponse>, (StatusCode, String)> {
    let backend = research_gpu_cloud_backend_or_error(&provider)?;
    let root_backend = experiment_runner_backend(backend);
    let secret_name = backend.secret_name();
    let secrets = state
        .secrets_store
        .as_ref()
        .ok_or_else(experiment_secrets_store_unavailable_error)?;
    let connected = secret_exists(Some(secrets), &request_identity.principal_id, secret_name).await;
    let info = research_gpu_cloud_info_or_error(&provider, connected)?;
    let (status, message) = if connected {
        match secrets
            .get_for_injection(
                &request_identity.principal_id,
                secret_name,
                crate::secrets::SecretAccessContext::new(
                    "experiments.gpu_cloud_validate",
                    "provider_credential_validation",
                ),
            )
            .await
        {
            Ok(secret) => match crate::experiments::adapters::validate_gpu_cloud_credentials(
                root_backend,
                secret.expose(),
            )
            .await
            {
                Ok(message) => ("ok".to_string(), message),
                Err(message) => research_gpu_cloud_invalid_credentials_validation(message),
            },
            Err(err) => research_gpu_cloud_credential_load_validation(&info, err),
        }
    } else {
        research_gpu_cloud_missing_credentials_validation(&info)
    };
    Ok(Json(research_gpu_cloud_validate_response(
        status, message, connected, info,
    )))
}

pub(crate) async fn experiments_gpu_cloud_template_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(provider): Path<String>,
    Json(req): Json<ExperimentGpuCloudTemplateRequest>,
) -> Result<
    Json<ResearchGpuCloudTemplateResponse<experiments_api::CreateExperimentRunnerProfileRequest>>,
    (StatusCode, String),
> {
    let store = experiment_store(&state)?;
    experiments_api::list_gpu_cloud_providers(store, &request_identity.principal_id)
        .await
        .map_err(experiment_api_error)?;
    let backend = research_gpu_cloud_backend_or_error(&provider)?;
    let connected = secret_exists(
        state.secrets_store.as_ref(),
        &request_identity.principal_id,
        backend.secret_name(),
    )
    .await;
    let info = research_gpu_cloud_info_or_error(&provider, connected)?;
    let template = research_gpu_cloud_default_runner_template(backend, &req)
        .map_err(|error| (error.status_code(), error.to_string()))?;
    let runner_payload = experiment_runner_payload(template.runner_payload.clone());
    Ok(Json(research_gpu_cloud_template_response(
        template,
        info,
        runner_payload,
    )))
}

pub(crate) async fn experiments_gpu_cloud_launch_test_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(provider): Path<String>,
) -> Result<Json<ResearchGpuCloudLaunchTestResponse>, (StatusCode, String)> {
    let backend = research_gpu_cloud_backend_or_error(&provider)?;
    let root_backend = experiment_runner_backend(backend);
    let secret_name = backend.secret_name();
    let secrets = state
        .secrets_store
        .as_ref()
        .ok_or_else(experiment_secrets_store_unavailable_error)?;
    let connected = secret_exists(Some(secrets), &request_identity.principal_id, secret_name).await;
    if !connected {
        return Err(research_gpu_cloud_launch_missing_credentials_error(backend));
    }
    let info = research_gpu_cloud_info_or_error(&provider, connected)?;
    let secret = secrets
        .get_for_injection(
            &request_identity.principal_id,
            secret_name,
            crate::secrets::SecretAccessContext::new(
                "experiments.gpu_cloud_launch_test",
                "provider_credential_validation",
            ),
        )
        .await
        .map_err(experiment_internal_server_error)?;
    let validation_message =
        crate::experiments::adapters::validate_gpu_cloud_credentials(root_backend, secret.expose())
            .await
            .map_err(experiment_bad_request_error)?;
    let update = research_gpu_cloud_launch_requested_update_message(&info);
    broadcast_experiment_opportunity_update(
        &state,
        update.opportunity_id,
        update.status,
        update.message,
    );
    Ok(Json(research_gpu_cloud_launch_test_response(
        backend,
        &validation_message,
        info,
    )))
}

pub(crate) async fn experiments_campaign_reissue_lease_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(id): Path<String>,
) -> Result<Json<experiments_api::ExperimentCampaignActionResponse>, (StatusCode, String)> {
    let store = experiment_store(&state)?;
    let id = parse_experiment_campaign_id(&id)?;
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
    let token = experiment_lease_token_from_headers(&headers)
        .map_err(|error| (error.status_code(), error.to_string()))?;
    let lease_id = parse_experiment_lease_id(&lease_id)?;
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
    let token = experiment_lease_token_from_headers(&headers)
        .map_err(|error| (error.status_code(), error.to_string()))?;
    let lease_id = parse_experiment_lease_id(&lease_id)?;
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
    let token = experiment_lease_token_from_headers(&headers)
        .map_err(|error| (error.status_code(), error.to_string()))?;
    let lease_id = parse_experiment_lease_id(&lease_id)?;
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
    let token = experiment_lease_token_from_headers(&headers)
        .map_err(|error| (error.status_code(), error.to_string()))?;
    let lease_id = parse_experiment_lease_id(&lease_id)?;
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
    let token = experiment_lease_token_from_headers(&headers)
        .map_err(|error| (error.status_code(), error.to_string()))?;
    let lease_id = parse_experiment_lease_id(&lease_id)?;
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
    let token = experiment_lease_token_from_headers(&headers)
        .map_err(|error| (error.status_code(), error.to_string()))?;
    let lease_id = parse_experiment_lease_id(&lease_id)?;
    let user_id = experiments_api::lease_owner_user_id(store, lease_id, &token)
        .await
        .map_err(experiment_api_error)?;
    let response = experiments_api::lease_complete(store, &user_id, lease_id, &token, req)
        .await
        .map_err(experiment_api_error)?;
    broadcast_campaign_update(&state, &response);
    Ok(Json(response))
}
