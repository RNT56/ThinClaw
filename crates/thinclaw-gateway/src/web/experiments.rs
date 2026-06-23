//! Root-independent experiment gateway policies.

use axum::http::{HeaderMap, StatusCode, header};
use serde::{Deserialize, Serialize};
use thinclaw_experiments::{
    ExperimentArtifactRef, ExperimentAutonomyMode, ExperimentCampaign, ExperimentComparisonPolicy,
    ExperimentLeaseAuthentication, ExperimentMetricDefinition, ExperimentModelUsageRecord,
    ExperimentOpportunity, ExperimentPreset, ExperimentProject, ExperimentProjectStatus,
    ExperimentRunnerBackend, ExperimentRunnerJob, ExperimentRunnerProfile,
    ExperimentRunnerReadinessClass, ExperimentRunnerStatus, ExperimentStopPolicy, ExperimentTarget,
    ExperimentTargetKind, ExperimentTrial, experiment_target_not_found_message,
};
use uuid::Uuid;

use crate::web::api::bounded_limit;
use crate::web::types::{ExperimentGpuCloudConnectRequest, ExperimentGpuCloudTemplateRequest};

const DEFAULT_RESEARCH_RUNNER_IMAGE: &str = "ghcr.io/thinclaw/research-runner:latest";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExperimentLeaseTokenError {
    #[error("Missing experiment lease token")]
    Missing,
}

impl ExperimentLeaseTokenError {
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::Missing => StatusCode::UNAUTHORIZED,
        }
    }
}

pub fn experiment_lease_token_from_headers(
    headers: &HeaderMap,
) -> Result<String, ExperimentLeaseTokenError> {
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

    Err(ExperimentLeaseTokenError::Missing)
}

pub fn status_to_sse_string<T: Serialize>(status: &T) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|value| value.as_str().map(ToString::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

pub fn research_limit(value: Option<usize>, default: usize) -> usize {
    bounded_limit(value, default, 1, 500)
}

pub fn experiment_database_unavailable_error() -> (StatusCode, String) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    )
}

pub fn experiment_secrets_store_unavailable_error() -> (StatusCode, String) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "Secrets store not available".to_string(),
    )
}

pub fn experiment_bad_request_error(message: impl Into<String>) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, message.into())
}

pub fn experiment_internal_server_error(message: impl ToString) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, message.to_string())
}

fn parse_experiment_uuid(
    id: &str,
    invalid_message: &'static str,
) -> Result<Uuid, (StatusCode, String)> {
    Uuid::parse_str(id).map_err(|_| (StatusCode::BAD_REQUEST, invalid_message.to_string()))
}

pub fn parse_experiment_project_id(id: &str) -> Result<Uuid, (StatusCode, String)> {
    parse_experiment_uuid(id, "Invalid project ID")
}

pub fn parse_experiment_runner_id(id: &str) -> Result<Uuid, (StatusCode, String)> {
    parse_experiment_uuid(id, "Invalid runner ID")
}

pub fn parse_experiment_campaign_id(id: &str) -> Result<Uuid, (StatusCode, String)> {
    parse_experiment_uuid(id, "Invalid campaign ID")
}

pub fn parse_experiment_trial_id(id: &str) -> Result<Uuid, (StatusCode, String)> {
    parse_experiment_uuid(id, "Invalid trial ID")
}

pub fn parse_experiment_target_id(id: &str) -> Result<Uuid, (StatusCode, String)> {
    parse_experiment_uuid(id, "Invalid target ID")
}

pub fn experiment_target_not_found_error(id: Uuid) -> (StatusCode, String) {
    (
        StatusCode::NOT_FOUND,
        experiment_target_not_found_message(id),
    )
}

pub fn parse_experiment_lease_id(id: &str) -> Result<Uuid, (StatusCode, String)> {
    parse_experiment_uuid(id, "Invalid lease ID")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchGpuCloudBackend {
    Runpod,
    Vast,
    Lambda,
}

impl ResearchGpuCloudBackend {
    pub fn slug(self) -> &'static str {
        match self {
            Self::Runpod => "runpod",
            Self::Vast => "vast",
            Self::Lambda => "lambda",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Runpod => "RunPod",
            Self::Vast => "Vast.ai",
            Self::Lambda => "Lambda",
        }
    }

    pub fn default_runner_name(self) -> &'static str {
        match self {
            Self::Runpod => "RunPod GPU Runner",
            Self::Vast => "Vast.ai GPU Runner",
            Self::Lambda => "Lambda GPU Runner",
        }
    }

    pub fn secret_name(self) -> &'static str {
        match self {
            Self::Runpod => "research_runpod_api_key",
            Self::Vast => "research_vast_api_key",
            Self::Lambda => "research_lambda_api_key",
        }
    }

    pub fn signup_url(self) -> &'static str {
        match self {
            Self::Runpod => "https://www.runpod.io",
            Self::Vast => "https://vast.ai",
            Self::Lambda => "https://cloud.lambda.ai",
        }
    }

    pub fn docs_url(self) -> &'static str {
        match self {
            Self::Runpod => "https://docs.runpod.io",
            Self::Vast => "https://docs.vast.ai",
            Self::Lambda => {
                "https://docs.lambda.ai/public-cloud/on-demand/creating-managing-instances/"
            }
        }
    }

    pub fn default_gpu_requirements(self) -> serde_json::Value {
        match self {
            Self::Runpod => serde_json::json!({ "gpu_count": 1, "gpu_type": "H100" }),
            Self::Vast => serde_json::json!({ "gpu_count": 1, "accelerator": "gpu" }),
            Self::Lambda => serde_json::json!({ "gpu_count": 1, "gpu_type": "A100" }),
        }
    }

    pub fn default_backend_config(self) -> serde_json::Value {
        match self {
            Self::Runpod => serde_json::json!({
                "provider": "runpod",
                "template_mode": "lease",
            }),
            Self::Vast => serde_json::json!({
                "provider": "vast",
                "launch_mode": "template",
            }),
            Self::Lambda => serde_json::json!({
                "provider": "lambda",
                "launch_mode": "api",
            }),
        }
    }

    pub fn template_hint(self) -> serde_json::Value {
        let mut hint = serde_json::json!({
            "backend": self.slug(),
            "recommended_secret_reference": self.secret_name(),
            "default_runner_name": self.default_runner_name(),
            "default_image_or_runtime": DEFAULT_RESEARCH_RUNNER_IMAGE,
            "default_gpu_requirements": self.default_gpu_requirements(),
        });
        if self == Self::Lambda {
            hint["launch_builder"] = serde_json::json!("normalized_lambda_form");
            hint["launch_mode"] = serde_json::json!("api");
            hint["quantity_limit"] = serde_json::json!(1);
            hint["quantity_note"] = serde_json::json!(
                "ThinClaw currently launches one Lambda instance per research trial so exactly one runner can claim the lease."
            );
            hint["field_defaults"] = serde_json::json!({
                "region_name": "",
                "instance_type_name": "",
                "quantity": 1,
                "ssh_key_names": [],
                "file_system_names": [],
            });
        }
        hint
    }

    pub fn template_message(self) -> String {
        match self {
            Self::Lambda => {
                "Lambda runner template is launch-ready. ThinClaw built backend_config.launch_payload from the normalized Research form."
                    .to_string()
            }
            _ => format!(
                "{} runner template is ready for Research campaigns.",
                self.display_name()
            ),
        }
    }

    pub fn launch_test_success_message(self, validation_message: &str) -> String {
        match self {
            Self::Lambda => {
                "Lambda credentials validated. Use the Lambda launch form to create a launch-ready Research runner with a server-built launch payload.".to_string()
            }
            _ => format!(
                "{validation_message} ThinClaw can now auto-launch provider compute from validated Research runners."
            ),
        }
    }

    pub fn launch_hint_bootstrap(self) -> &'static str {
        match self {
            Self::Lambda => {
                "Lambda credentials are live. Build a runner from the Lambda GPU Clouds form to get a controller-managed launch payload automatically."
            }
            _ => {
                "Create a research runner profile, then start a campaign to let ThinClaw auto-launch provider compute with a lease token."
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResearchGpuCloudTemplateError {
    #[error("Lambda template requires an instance type name.")]
    LambdaInstanceTypeRequired,
}

impl ResearchGpuCloudTemplateError {
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::LambdaInstanceTypeRequired => StatusCode::BAD_REQUEST,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResearchGpuCloudApiKeyError {
    #[error("API key is required")]
    Missing,
}

impl ResearchGpuCloudApiKeyError {
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::Missing => StatusCode::BAD_REQUEST,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResearchGpuCloudProviderInfo {
    pub slug: String,
    pub display_name: String,
    pub backend: ResearchGpuCloudBackend,
    pub description: String,
    pub signup_url: String,
    pub docs_url: String,
    pub secret_name: String,
    #[serde(default)]
    pub connected: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_hint: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResearchGpuCloudRunnerPayload {
    pub name: String,
    pub backend: ResearchGpuCloudBackend,
    #[serde(default)]
    pub backend_config: serde_json::Value,
    #[serde(default)]
    pub image_or_runtime: Option<String>,
    #[serde(default)]
    pub gpu_requirements: serde_json::Value,
    #[serde(default)]
    pub env_grants: serde_json::Value,
    #[serde(default)]
    pub secret_references: Vec<String>,
    #[serde(default)]
    pub cache_policy: serde_json::Value,
}

pub fn experiment_runner_backend(backend: ResearchGpuCloudBackend) -> ExperimentRunnerBackend {
    match backend {
        ResearchGpuCloudBackend::Runpod => ExperimentRunnerBackend::Runpod,
        ResearchGpuCloudBackend::Vast => ExperimentRunnerBackend::Vast,
        ResearchGpuCloudBackend::Lambda => ExperimentRunnerBackend::Lambda,
    }
}

pub fn experiment_runner_payload(
    payload: ResearchGpuCloudRunnerPayload,
) -> CreateExperimentRunnerProfileRequest {
    CreateExperimentRunnerProfileRequest {
        name: payload.name,
        backend: experiment_runner_backend(payload.backend),
        backend_config: payload.backend_config,
        image_or_runtime: payload.image_or_runtime,
        gpu_requirements: payload.gpu_requirements,
        env_grants: payload.env_grants,
        secret_references: payload.secret_references,
        cache_policy: payload.cache_policy,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResearchGpuCloudRunnerTemplate {
    pub runner_payload: ResearchGpuCloudRunnerPayload,
    pub warnings: Vec<String>,
    pub launch_payload_preview: Option<serde_json::Value>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExperimentProjectListResponse {
    pub projects: Vec<ExperimentProject>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExperimentRunnerListResponse {
    pub runners: Vec<ExperimentRunnerProfile>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExperimentCampaignListResponse {
    pub campaigns: Vec<ExperimentCampaign>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExperimentTrialListResponse {
    pub trials: Vec<ExperimentTrial>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExperimentArtifactListResponse {
    pub artifacts: Vec<ExperimentArtifactRef>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExperimentTargetListResponse {
    pub targets: Vec<ExperimentTarget>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExperimentModelUsageListResponse {
    pub usage: Vec<ExperimentModelUsageRecord>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExperimentOpportunityListResponse {
    pub opportunities: Vec<ExperimentOpportunity>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentGpuCloudProviderInfo {
    pub slug: String,
    pub display_name: String,
    pub backend: ExperimentRunnerBackend,
    pub description: String,
    pub signup_url: String,
    pub docs_url: String,
    pub secret_name: String,
    #[serde(default)]
    pub connected: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_hint: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExperimentGpuCloudProviderListResponse {
    pub providers: Vec<ExperimentGpuCloudProviderInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentLaunchDetails {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_template: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_job_id: Option<String>,
    #[serde(default)]
    pub provider_job_metadata: serde_json::Value,
    #[serde(default)]
    pub auto_launched: bool,
    #[serde(default)]
    pub requires_operator_action: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExperimentRunnerValidationResponse {
    pub runner: ExperimentRunnerProfile,
    pub valid: bool,
    pub readiness_class: ExperimentRunnerReadinessClass,
    pub launch_eligible: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExperimentCampaignActionResponse {
    pub campaign: ExperimentCampaign,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trial: Option<ExperimentTrial>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lease: Option<ExperimentLeaseAuthentication>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub launch: Option<ExperimentLaunchDetails>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExperimentTrialActionUpdateMessage {
    pub campaign_id: String,
    pub trial_id: String,
    pub status: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExperimentCampaignActionUpdateMessage {
    pub campaign_id: String,
    pub status: String,
    pub message: String,
    pub trial: Option<ExperimentTrialActionUpdateMessage>,
}

pub fn experiment_campaign_action_update_message(
    response: &ExperimentCampaignActionResponse,
) -> ExperimentCampaignActionUpdateMessage {
    ExperimentCampaignActionUpdateMessage {
        campaign_id: response.campaign.id.to_string(),
        status: status_to_sse_string(&response.campaign.status),
        message: response.message.clone(),
        trial: response
            .trial
            .as_ref()
            .map(|trial| ExperimentTrialActionUpdateMessage {
                campaign_id: response.campaign.id.to_string(),
                trial_id: trial.id.to_string(),
                status: status_to_sse_string(&trial.status),
                message: trial
                    .decision_reason
                    .clone()
                    .or(trial.summary.clone())
                    .unwrap_or_else(|| response.message.clone()),
            }),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExperimentOpportunityUpdateMessage {
    pub opportunity_id: String,
    pub status: String,
    pub message: String,
}

pub fn experiment_target_created_update_message(
    target: &ExperimentTarget,
) -> ExperimentOpportunityUpdateMessage {
    ExperimentOpportunityUpdateMessage {
        opportunity_id: target.id.to_string(),
        status: "updated".to_string(),
        message: format!("Target '{}' linked.", target.name),
    }
}

pub fn experiment_target_linked_update_message(
    target: &ExperimentTarget,
) -> ExperimentOpportunityUpdateMessage {
    ExperimentOpportunityUpdateMessage {
        opportunity_id: target.id.to_string(),
        status: "updated".to_string(),
        message: format!("Target '{}' linked to a research opportunity.", target.name),
    }
}

pub fn experiment_target_updated_update_message(
    target: &ExperimentTarget,
) -> ExperimentOpportunityUpdateMessage {
    ExperimentOpportunityUpdateMessage {
        opportunity_id: target.id.to_string(),
        status: "updated".to_string(),
        message: format!("Target '{}' updated.", target.name),
    }
}

pub fn experiment_target_deleted_update_message(
    target_id: Uuid,
) -> ExperimentOpportunityUpdateMessage {
    ExperimentOpportunityUpdateMessage {
        opportunity_id: target_id.to_string(),
        status: "deleted".to_string(),
        message: "Target removed from research opportunities.".to_string(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentLeaseJobResponse {
    pub job: ExperimentRunnerJob,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentLeaseCredentialsResponse {
    pub credentials: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateExperimentProjectRequest {
    pub name: String,
    pub workspace_path: String,
    pub git_remote_name: String,
    pub base_branch: String,
    #[serde(default)]
    pub preset: Option<ExperimentPreset>,
    #[serde(default)]
    pub strategy_prompt: Option<String>,
    pub workdir: String,
    #[serde(default)]
    pub prepare_command: Option<String>,
    pub run_command: String,
    #[serde(default)]
    pub mutable_paths: Vec<String>,
    #[serde(default)]
    pub fixed_paths: Vec<String>,
    pub primary_metric: ExperimentMetricDefinition,
    #[serde(default)]
    pub secondary_metrics: Vec<ExperimentMetricDefinition>,
    #[serde(default)]
    pub comparison_policy: Option<ExperimentComparisonPolicy>,
    #[serde(default)]
    pub stop_policy: Option<ExperimentStopPolicy>,
    #[serde(default)]
    pub default_runner_profile_id: Option<Uuid>,
    #[serde(default)]
    pub promotion_mode: Option<String>,
    #[serde(default)]
    pub autonomy_mode: Option<ExperimentAutonomyMode>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateExperimentProjectRequest {
    pub name: Option<String>,
    pub workspace_path: Option<String>,
    pub git_remote_name: Option<String>,
    pub base_branch: Option<String>,
    pub preset: Option<ExperimentPreset>,
    pub strategy_prompt: Option<String>,
    pub workdir: Option<String>,
    pub prepare_command: Option<String>,
    pub run_command: Option<String>,
    pub mutable_paths: Option<Vec<String>>,
    pub fixed_paths: Option<Vec<String>>,
    pub primary_metric: Option<ExperimentMetricDefinition>,
    pub secondary_metrics: Option<Vec<ExperimentMetricDefinition>>,
    pub comparison_policy: Option<ExperimentComparisonPolicy>,
    pub stop_policy: Option<ExperimentStopPolicy>,
    pub default_runner_profile_id: Option<Uuid>,
    pub promotion_mode: Option<String>,
    pub autonomy_mode: Option<ExperimentAutonomyMode>,
    pub status: Option<ExperimentProjectStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateExperimentRunnerProfileRequest {
    pub name: String,
    pub backend: ExperimentRunnerBackend,
    #[serde(default)]
    pub backend_config: serde_json::Value,
    #[serde(default)]
    pub image_or_runtime: Option<String>,
    #[serde(default)]
    pub gpu_requirements: serde_json::Value,
    #[serde(default)]
    pub env_grants: serde_json::Value,
    #[serde(default)]
    pub secret_references: Vec<String>,
    #[serde(default)]
    pub cache_policy: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateExperimentRunnerProfileRequest {
    pub name: Option<String>,
    pub backend: Option<ExperimentRunnerBackend>,
    pub backend_config: Option<serde_json::Value>,
    pub image_or_runtime: Option<String>,
    pub gpu_requirements: Option<serde_json::Value>,
    pub env_grants: Option<serde_json::Value>,
    pub secret_references: Option<Vec<String>>,
    pub cache_policy: Option<serde_json::Value>,
    pub status: Option<ExperimentRunnerStatus>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StartExperimentCampaignRequest {
    #[serde(default)]
    pub runner_profile_id: Option<Uuid>,
    #[serde(default)]
    pub max_trials_override: Option<u32>,
    #[serde(default)]
    pub gateway_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateExperimentTargetRequest {
    pub name: String,
    #[serde(default)]
    pub kind: ExperimentTargetKind,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LinkExperimentTargetRequest {
    pub opportunity_id: String,
    #[serde(default)]
    pub target_type: ExperimentTargetKind,
    pub target_id: String,
    #[serde(default)]
    pub target_name: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateExperimentTargetRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub kind: Option<ExperimentTargetKind>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentLeaseStatusRequest {
    pub status: String,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentLeaseEventRequest {
    pub message: String,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExperimentDeleteResponse {
    pub deleted: bool,
}

pub fn experiment_delete_response(deleted: bool) -> ExperimentDeleteResponse {
    ExperimentDeleteResponse { deleted }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResearchGpuCloudConnectResponse {
    pub status: String,
    pub message: String,
    pub provider: ResearchGpuCloudProviderInfo,
}

pub fn research_gpu_cloud_connect_response(
    provider: ResearchGpuCloudProviderInfo,
) -> ResearchGpuCloudConnectResponse {
    ResearchGpuCloudConnectResponse {
        status: "ok".to_string(),
        message: format!("{} credentials connected.", provider.display_name),
        provider,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResearchGpuCloudValidateResponse {
    pub status: String,
    pub message: String,
    pub connected: bool,
    pub provider: ResearchGpuCloudProviderInfo,
}

pub fn research_gpu_cloud_validate_response(
    status: impl Into<String>,
    message: impl Into<String>,
    connected: bool,
    provider: ResearchGpuCloudProviderInfo,
) -> ResearchGpuCloudValidateResponse {
    ResearchGpuCloudValidateResponse {
        status: status.into(),
        message: message.into(),
        connected,
        provider,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResearchGpuCloudTemplateResponse<T> {
    pub status: String,
    pub message: String,
    pub warnings: Vec<String>,
    pub provider: ResearchGpuCloudProviderInfo,
    pub runner_payload: T,
    pub launch_payload_preview: Option<serde_json::Value>,
}

pub fn research_gpu_cloud_template_response<T>(
    template: ResearchGpuCloudRunnerTemplate,
    provider: ResearchGpuCloudProviderInfo,
    runner_payload: T,
) -> ResearchGpuCloudTemplateResponse<T> {
    ResearchGpuCloudTemplateResponse {
        status: "ok".to_string(),
        message: template.message,
        warnings: template.warnings,
        provider,
        runner_payload,
        launch_payload_preview: template.launch_payload_preview,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResearchGpuCloudLaunchHint {
    pub backend: String,
    pub runner_template_required: bool,
    pub bootstrap: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResearchGpuCloudLaunchTestResponse {
    pub status: String,
    pub message: String,
    pub provider: ResearchGpuCloudProviderInfo,
    pub launch_hint: ResearchGpuCloudLaunchHint,
}

pub fn research_gpu_cloud_launch_test_response(
    backend: ResearchGpuCloudBackend,
    validation_message: &str,
    provider: ResearchGpuCloudProviderInfo,
) -> ResearchGpuCloudLaunchTestResponse {
    ResearchGpuCloudLaunchTestResponse {
        status: "ok".to_string(),
        message: backend.launch_test_success_message(validation_message),
        provider,
        launch_hint: ResearchGpuCloudLaunchHint {
            backend: backend.slug().to_string(),
            runner_template_required: false,
            bootstrap: backend.launch_hint_bootstrap().to_string(),
        },
    }
}

pub fn research_gpu_cloud_backend(provider: &str) -> Option<ResearchGpuCloudBackend> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "runpod" => Some(ResearchGpuCloudBackend::Runpod),
        "vast" | "vast.ai" => Some(ResearchGpuCloudBackend::Vast),
        "lambda" => Some(ResearchGpuCloudBackend::Lambda),
        _ => None,
    }
}

pub fn research_gpu_cloud_provider_not_found_error() -> (StatusCode, String) {
    (
        StatusCode::NOT_FOUND,
        "Unknown GPU cloud provider".to_string(),
    )
}

pub fn research_gpu_cloud_backend_or_error(
    provider: &str,
) -> Result<ResearchGpuCloudBackend, (StatusCode, String)> {
    research_gpu_cloud_backend(provider).ok_or_else(research_gpu_cloud_provider_not_found_error)
}

pub fn research_gpu_cloud_info(
    provider: &str,
    connected: bool,
) -> Option<ResearchGpuCloudProviderInfo> {
    let backend = research_gpu_cloud_backend(provider)?;
    Some(ResearchGpuCloudProviderInfo {
        slug: backend.slug().to_string(),
        display_name: backend.display_name().to_string(),
        backend,
        description: format!(
            "{} setup for outbound ThinClaw experiment runners.",
            backend.display_name()
        ),
        signup_url: backend.signup_url().to_string(),
        docs_url: backend.docs_url().to_string(),
        secret_name: backend.secret_name().to_string(),
        connected,
        template_hint: Some(backend.template_hint()),
    })
}

pub fn research_gpu_cloud_info_or_error(
    provider: &str,
    connected: bool,
) -> Result<ResearchGpuCloudProviderInfo, (StatusCode, String)> {
    research_gpu_cloud_info(provider, connected)
        .ok_or_else(research_gpu_cloud_provider_not_found_error)
}

pub fn research_gpu_cloud_api_key(
    req: &ExperimentGpuCloudConnectRequest,
) -> Result<String, ResearchGpuCloudApiKeyError> {
    let api_key = req.api_key.trim().to_string();
    if api_key.is_empty() {
        Err(ResearchGpuCloudApiKeyError::Missing)
    } else {
        Ok(api_key)
    }
}

pub fn research_gpu_cloud_missing_credentials_validation(
    provider: &ResearchGpuCloudProviderInfo,
) -> (String, String) {
    (
        "missing_credentials".to_string(),
        format!("{} credentials are missing.", provider.display_name),
    )
}

pub fn research_gpu_cloud_invalid_credentials_validation(
    message: impl Into<String>,
) -> (String, String) {
    ("invalid_credentials".to_string(), message.into())
}

pub fn research_gpu_cloud_credential_load_validation(
    provider: &ResearchGpuCloudProviderInfo,
    error: impl ToString,
) -> (String, String) {
    research_gpu_cloud_invalid_credentials_validation(format!(
        "Failed to load {} credential: {}",
        provider.display_name,
        error.to_string()
    ))
}

pub fn research_gpu_cloud_launch_missing_credentials_error(
    backend: ResearchGpuCloudBackend,
) -> (StatusCode, String) {
    experiment_bad_request_error(format!(
        "{} credentials must be connected before launching a test job.",
        backend.display_name()
    ))
}

pub fn research_gpu_cloud_connected_update_message(
    provider: &ResearchGpuCloudProviderInfo,
) -> ExperimentOpportunityUpdateMessage {
    ExperimentOpportunityUpdateMessage {
        opportunity_id: provider.slug.clone(),
        status: "connected".to_string(),
        message: format!("{} credentials connected.", provider.display_name),
    }
}

pub fn research_gpu_cloud_launch_requested_update_message(
    provider: &ResearchGpuCloudProviderInfo,
) -> ExperimentOpportunityUpdateMessage {
    ExperimentOpportunityUpdateMessage {
        opportunity_id: provider.slug.clone(),
        status: "ready".to_string(),
        message: format!("{} test launch requested.", provider.display_name),
    }
}

pub fn research_gpu_cloud_default_runner_template(
    backend: ResearchGpuCloudBackend,
    req: &ExperimentGpuCloudTemplateRequest,
) -> Result<ResearchGpuCloudRunnerTemplate, ResearchGpuCloudTemplateError> {
    let runner_name = trimmed_optional(req.runner_name.as_ref())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| backend.default_runner_name().to_string());
    let image_or_runtime = trimmed_optional(req.image_or_runtime.as_ref())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| DEFAULT_RESEARCH_RUNNER_IMAGE.to_string());
    let (backend_config, warnings, launch_payload_preview) =
        if backend == ResearchGpuCloudBackend::Lambda {
            let instance_type_name = trimmed_optional(req.instance_type_name.as_ref())
                .ok_or(ResearchGpuCloudTemplateError::LambdaInstanceTypeRequired)?;
            let (backend_config, warnings) = build_lambda_backend_config(
                trimmed_optional(req.region_name.as_ref()).map(ToOwned::to_owned),
                instance_type_name,
                req.quantity,
                &req.ssh_key_names,
                &req.file_system_names,
            );
            let preview = backend_config.get("launch_payload").cloned();
            (backend_config, warnings, preview)
        } else {
            (backend.default_backend_config(), Vec::new(), None)
        };

    Ok(ResearchGpuCloudRunnerTemplate {
        runner_payload: ResearchGpuCloudRunnerPayload {
            name: runner_name,
            backend,
            backend_config,
            image_or_runtime: Some(image_or_runtime),
            gpu_requirements: backend.default_gpu_requirements(),
            env_grants: serde_json::json!({}),
            secret_references: vec![backend.secret_name().to_string()],
            cache_policy: serde_json::json!({
                "persist_workspace": false,
                "provider": backend.slug(),
            }),
        },
        warnings,
        launch_payload_preview,
        message: backend.template_message(),
    })
}

fn trimmed_optional(value: Option<&String>) -> Option<&str> {
    value
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn build_lambda_backend_config(
    region_name: Option<String>,
    instance_type_name: &str,
    quantity: u32,
    ssh_key_names: &[String],
    file_system_names: &[String],
) -> (serde_json::Value, Vec<String>) {
    let mut warnings = Vec::new();
    let normalized_quantity = if quantity == 0 { 1 } else { quantity };
    let launch_quantity = if normalized_quantity > 1 {
        warnings.push(
            "ThinClaw currently launches one Lambda instance per research trial, so quantity was normalized to 1."
                .to_string(),
        );
        1
    } else {
        normalized_quantity
    };
    let ssh_key_names = ssh_key_names
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let file_system_names = file_system_names
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let mut launch_payload = serde_json::Map::new();
    launch_payload.insert("name".to_string(), serde_json::json!("{{THINCLAW_NAME}}"));
    launch_payload.insert(
        "instance_type_name".to_string(),
        serde_json::json!(instance_type_name.trim()),
    );
    launch_payload.insert("quantity".to_string(), serde_json::json!(launch_quantity));
    launch_payload.insert("image".to_string(), serde_json::json!("{{THINCLAW_IMAGE}}"));
    launch_payload.insert(
        "cloud_init".to_string(),
        serde_json::json!("#cloud-config\nruncmd:\n  - {{THINCLAW_BOOTSTRAP}}"),
    );
    if let Some(region_name) = &region_name {
        launch_payload.insert("region_name".to_string(), serde_json::json!(region_name));
    }
    if !ssh_key_names.is_empty() {
        launch_payload.insert(
            "ssh_key_names".to_string(),
            serde_json::json!(ssh_key_names),
        );
    }
    if !file_system_names.is_empty() {
        launch_payload.insert(
            "file_system_names".to_string(),
            serde_json::json!(file_system_names),
        );
    }
    (
        serde_json::json!({
            "provider": "lambda",
            "launch_mode": "api",
            "region_name": region_name,
            "instance_type_name": instance_type_name.trim(),
            "quantity": launch_quantity,
            "ssh_key_names": ssh_key_names,
            "file_system_names": file_system_names,
            "launch_payload": serde_json::Value::Object(launch_payload),
            "terminate_payload": {
                "instance_ids": ["{{THINCLAW_PROVIDER_JOB_ID}}"],
            },
        }),
        warnings,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use thinclaw_experiments::{
        ExperimentCampaignQueueState, ExperimentCampaignStatus, ExperimentTrialStatus,
    };

    fn test_campaign(id: Uuid, status: ExperimentCampaignStatus) -> ExperimentCampaign {
        let now = chrono::Utc::now();
        ExperimentCampaign {
            id,
            project_id: Uuid::from_u128(11),
            runner_profile_id: Uuid::from_u128(12),
            owner_user_id: "user".to_string(),
            status,
            baseline_commit: None,
            best_commit: None,
            best_metrics: serde_json::json!({}),
            experiment_branch: None,
            remote_ref: None,
            worktree_path: None,
            started_at: None,
            ended_at: None,
            trial_count: 0,
            failure_count: 0,
            pause_reason: None,
            queue_state: ExperimentCampaignQueueState::NotQueued,
            queue_position: 0,
            active_trial_id: None,
            total_runtime_ms: 0,
            total_cost_usd: 0.0,
            total_llm_cost_usd: 0.0,
            total_runner_cost_usd: 0.0,
            consecutive_non_improving_trials: 0,
            max_trials_override: None,
            gateway_url: None,
            metadata: serde_json::json!({}),
            created_at: now,
            updated_at: now,
        }
    }

    fn test_trial(
        id: Uuid,
        campaign_id: Uuid,
        status: ExperimentTrialStatus,
        summary: Option<&str>,
        decision_reason: Option<&str>,
    ) -> ExperimentTrial {
        let now = chrono::Utc::now();
        ExperimentTrial {
            id,
            campaign_id,
            sequence: 1,
            candidate_commit: None,
            parent_best_commit: None,
            status,
            runner_backend: ExperimentRunnerBackend::LocalDocker,
            exit_code: None,
            metrics_json: serde_json::json!({}),
            summary: summary.map(ToOwned::to_owned),
            decision_reason: decision_reason.map(ToOwned::to_owned),
            log_preview_path: None,
            artifact_manifest_json: serde_json::json!({}),
            runtime_ms: None,
            attributed_cost_usd: None,
            llm_cost_usd: None,
            runner_cost_usd: None,
            hypothesis: None,
            mutation_summary: None,
            reviewer_decision: None,
            provider_job_id: None,
            provider_job_metadata: serde_json::json!({}),
            started_at: None,
            completed_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn test_target(id: Uuid, name: &str) -> ExperimentTarget {
        let now = chrono::Utc::now();
        ExperimentTarget {
            id,
            name: name.to_string(),
            kind: ExperimentTargetKind::InferenceConfig,
            location: None,
            metadata: serde_json::json!({}),
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn lease_token_prefers_explicit_header() {
        let mut headers = HeaderMap::new();
        headers.insert("x-experiment-lease-token", " explicit ".parse().unwrap());
        headers.insert(header::AUTHORIZATION, "Bearer fallback".parse().unwrap());

        assert_eq!(
            experiment_lease_token_from_headers(&headers),
            Ok("explicit".to_string())
        );
    }

    #[test]
    fn lease_token_falls_back_to_bearer_token() {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer token-123 ".parse().unwrap());

        assert_eq!(
            experiment_lease_token_from_headers(&headers),
            Ok("token-123".to_string())
        );
    }

    #[test]
    fn lease_token_rejects_missing_token() {
        let err = experiment_lease_token_from_headers(&HeaderMap::new()).unwrap_err();
        assert_eq!(err, ExperimentLeaseTokenError::Missing);
        assert_eq!(err.status_code(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn status_to_sse_string_uses_string_values_only() {
        assert_eq!(status_to_sse_string(&"running"), "running");
        assert_eq!(
            status_to_sse_string(&serde_json::json!({"state": "running"})),
            "unknown"
        );
    }

    #[test]
    fn research_limit_applies_default_and_clamp() {
        assert_eq!(research_limit(None, 100), 100);
        assert_eq!(research_limit(Some(0), 100), 1);
        assert_eq!(research_limit(Some(600), 100), 500);
        assert_eq!(research_limit(Some(42), 100), 42);
    }

    #[test]
    fn experiment_boundary_errors_preserve_existing_statuses_and_messages() {
        let target_id = Uuid::from_u128(301);

        assert_eq!(
            experiment_database_unavailable_error(),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "Database not available".to_string()
            )
        );
        assert_eq!(
            experiment_secrets_store_unavailable_error(),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "Secrets store not available".to_string()
            )
        );
        assert_eq!(
            experiment_target_not_found_error(target_id),
            (
                StatusCode::NOT_FOUND,
                format!("Experiment target {target_id} not found")
            )
        );
        assert_eq!(
            experiment_bad_request_error("bad launch"),
            (StatusCode::BAD_REQUEST, "bad launch".to_string())
        );
        assert_eq!(
            experiment_internal_server_error("root failure"),
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "root failure".to_string()
            )
        );
    }

    #[test]
    fn experiment_id_parsers_preserve_existing_errors() {
        let id = "00000000-0000-0000-0000-000000000001";
        let parsed = Uuid::parse_str(id).unwrap();
        assert_eq!(parse_experiment_project_id(id), Ok(parsed));
        assert_eq!(parse_experiment_runner_id(id), Ok(parsed));
        assert_eq!(parse_experiment_campaign_id(id), Ok(parsed));
        assert_eq!(parse_experiment_trial_id(id), Ok(parsed));
        assert_eq!(parse_experiment_target_id(id), Ok(parsed));
        assert_eq!(parse_experiment_lease_id(id), Ok(parsed));

        type ParseFn = fn(&str) -> Result<Uuid, (StatusCode, String)>;
        let cases: [(&str, ParseFn); 6] = [
            ("Invalid project ID", parse_experiment_project_id),
            ("Invalid runner ID", parse_experiment_runner_id),
            ("Invalid campaign ID", parse_experiment_campaign_id),
            ("Invalid trial ID", parse_experiment_trial_id),
            ("Invalid target ID", parse_experiment_target_id),
            ("Invalid lease ID", parse_experiment_lease_id),
        ];
        for (message, parser) in cases {
            assert_eq!(
                parser("not-a-uuid"),
                Err((StatusCode::BAD_REQUEST, message.to_string()))
            );
        }
    }

    #[test]
    fn campaign_action_update_message_shapes_campaign_and_trial_payloads() {
        let campaign_id = Uuid::from_u128(101);
        let trial_id = Uuid::from_u128(102);
        let response = ExperimentCampaignActionResponse {
            campaign: test_campaign(campaign_id, ExperimentCampaignStatus::Running),
            trial: Some(test_trial(
                trial_id,
                campaign_id,
                ExperimentTrialStatus::Accepted,
                Some("summary fallback"),
                Some("decision reason"),
            )),
            lease: None,
            launch: None,
            message: "campaign message".to_string(),
        };

        let update = experiment_campaign_action_update_message(&response);

        assert_eq!(update.campaign_id, campaign_id.to_string());
        assert_eq!(update.status, "running");
        assert_eq!(update.message, "campaign message");
        let trial = update.trial.unwrap();
        assert_eq!(trial.campaign_id, campaign_id.to_string());
        assert_eq!(trial.trial_id, trial_id.to_string());
        assert_eq!(trial.status, "accepted");
        assert_eq!(trial.message, "decision reason");
    }

    #[test]
    fn campaign_action_update_message_falls_back_for_trial_message() {
        let campaign_id = Uuid::from_u128(103);
        let response = ExperimentCampaignActionResponse {
            campaign: test_campaign(campaign_id, ExperimentCampaignStatus::Paused),
            trial: Some(test_trial(
                Uuid::from_u128(104),
                campaign_id,
                ExperimentTrialStatus::Running,
                Some("trial summary"),
                None,
            )),
            lease: None,
            launch: None,
            message: "campaign fallback".to_string(),
        };

        assert_eq!(
            experiment_campaign_action_update_message(&response)
                .trial
                .unwrap()
                .message,
            "trial summary"
        );

        let response = ExperimentCampaignActionResponse {
            campaign: test_campaign(campaign_id, ExperimentCampaignStatus::Paused),
            trial: Some(test_trial(
                Uuid::from_u128(105),
                campaign_id,
                ExperimentTrialStatus::Running,
                None,
                None,
            )),
            lease: None,
            launch: None,
            message: "campaign fallback".to_string(),
        };

        assert_eq!(
            experiment_campaign_action_update_message(&response)
                .trial
                .unwrap()
                .message,
            "campaign fallback"
        );
    }

    #[test]
    fn target_update_messages_preserve_existing_text() {
        let target_id = Uuid::from_u128(201);
        let target = test_target(target_id, "GPT target");

        assert_eq!(
            experiment_target_created_update_message(&target),
            ExperimentOpportunityUpdateMessage {
                opportunity_id: target_id.to_string(),
                status: "updated".to_string(),
                message: "Target 'GPT target' linked.".to_string(),
            }
        );
        assert_eq!(
            experiment_target_linked_update_message(&target).message,
            "Target 'GPT target' linked to a research opportunity."
        );
        assert_eq!(
            experiment_target_updated_update_message(&target).message,
            "Target 'GPT target' updated."
        );
        assert_eq!(
            experiment_target_deleted_update_message(target_id),
            ExperimentOpportunityUpdateMessage {
                opportunity_id: target_id.to_string(),
                status: "deleted".to_string(),
                message: "Target removed from research opportunities.".to_string(),
            }
        );
    }

    #[test]
    fn gpu_cloud_backend_parses_supported_provider_strings() {
        assert_eq!(
            research_gpu_cloud_backend(" RunPod "),
            Some(ResearchGpuCloudBackend::Runpod)
        );
        assert_eq!(
            research_gpu_cloud_backend("vast.ai"),
            Some(ResearchGpuCloudBackend::Vast)
        );
        assert_eq!(
            research_gpu_cloud_backend("lambda"),
            Some(ResearchGpuCloudBackend::Lambda)
        );
        assert_eq!(research_gpu_cloud_backend("unknown"), None);
    }

    #[test]
    fn gpu_cloud_provider_error_helpers_preserve_existing_shapes() {
        assert_eq!(
            research_gpu_cloud_backend_or_error("lambda"),
            Ok(ResearchGpuCloudBackend::Lambda)
        );
        assert_eq!(
            research_gpu_cloud_backend_or_error("unknown"),
            Err((
                StatusCode::NOT_FOUND,
                "Unknown GPU cloud provider".to_string()
            ))
        );
        assert_eq!(
            research_gpu_cloud_info_or_error("unknown", false),
            Err((
                StatusCode::NOT_FOUND,
                "Unknown GPU cloud provider".to_string()
            ))
        );

        let backend = ResearchGpuCloudBackend::Lambda;
        assert_eq!(
            research_gpu_cloud_launch_missing_credentials_error(backend),
            (
                StatusCode::BAD_REQUEST,
                "Lambda credentials must be connected before launching a test job.".to_string()
            )
        );
    }

    #[test]
    fn gpu_cloud_info_assembles_provider_metadata() {
        let info = research_gpu_cloud_info("lambda", true).unwrap();

        assert_eq!(info.slug, "lambda");
        assert_eq!(info.display_name, "Lambda");
        assert_eq!(info.backend, ResearchGpuCloudBackend::Lambda);
        assert_eq!(info.secret_name, "research_lambda_api_key");
        assert!(info.connected);
        assert_eq!(
            info.template_hint
                .as_ref()
                .and_then(|hint| hint.get("launch_builder"))
                .and_then(|value| value.as_str()),
            Some("normalized_lambda_form")
        );
    }

    #[test]
    fn gpu_cloud_api_key_validation_trims_and_rejects_empty_values() {
        assert_eq!(
            research_gpu_cloud_api_key(&ExperimentGpuCloudConnectRequest {
                api_key: " token ".to_string(),
            }),
            Ok("token".to_string())
        );

        let error = research_gpu_cloud_api_key(&ExperimentGpuCloudConnectRequest {
            api_key: " ".to_string(),
        })
        .unwrap_err();

        assert_eq!(error, ResearchGpuCloudApiKeyError::Missing);
        assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn gpu_cloud_credential_status_and_update_helpers_preserve_existing_text() {
        let info = research_gpu_cloud_info("lambda", true).unwrap();

        assert_eq!(
            research_gpu_cloud_missing_credentials_validation(&info),
            (
                "missing_credentials".to_string(),
                "Lambda credentials are missing.".to_string()
            )
        );
        assert_eq!(
            research_gpu_cloud_invalid_credentials_validation("bad key"),
            ("invalid_credentials".to_string(), "bad key".to_string())
        );
        assert_eq!(
            research_gpu_cloud_credential_load_validation(&info, "permission denied"),
            (
                "invalid_credentials".to_string(),
                "Failed to load Lambda credential: permission denied".to_string()
            )
        );
        assert_eq!(
            research_gpu_cloud_connected_update_message(&info),
            ExperimentOpportunityUpdateMessage {
                opportunity_id: "lambda".to_string(),
                status: "connected".to_string(),
                message: "Lambda credentials connected.".to_string(),
            }
        );
        assert_eq!(
            research_gpu_cloud_launch_requested_update_message(&info),
            ExperimentOpportunityUpdateMessage {
                opportunity_id: "lambda".to_string(),
                status: "ready".to_string(),
                message: "Lambda test launch requested.".to_string(),
            }
        );
    }

    #[test]
    fn gpu_cloud_runner_template_uses_defaults_and_trimmed_values() {
        let req = ExperimentGpuCloudTemplateRequest {
            runner_name: Some("  Custom runner  ".to_string()),
            image_or_runtime: Some("  image:tag  ".to_string()),
            region_name: None,
            instance_type_name: None,
            quantity: 1,
            ssh_key_names: Vec::new(),
            file_system_names: Vec::new(),
        };

        let template =
            research_gpu_cloud_default_runner_template(ResearchGpuCloudBackend::Runpod, &req)
                .unwrap();

        assert_eq!(template.runner_payload.name, "Custom runner");
        assert_eq!(
            template.runner_payload.image_or_runtime.as_deref(),
            Some("image:tag")
        );
        assert_eq!(
            template.runner_payload.backend_config,
            serde_json::json!({
                "provider": "runpod",
                "template_mode": "lease",
            })
        );
        assert_eq!(
            template.runner_payload.secret_references,
            ["research_runpod_api_key"]
        );
        assert!(template.warnings.is_empty());
        assert!(template.launch_payload_preview.is_none());
    }

    #[test]
    fn lambda_runner_template_builds_launch_payload_and_warnings() {
        let req = ExperimentGpuCloudTemplateRequest {
            runner_name: None,
            image_or_runtime: None,
            region_name: Some(" us-east-1 ".to_string()),
            instance_type_name: Some(" gpu_1x_a100 ".to_string()),
            quantity: 3,
            ssh_key_names: vec![" key-one ".to_string(), " ".to_string()],
            file_system_names: vec![" fs-one ".to_string()],
        };

        let template =
            research_gpu_cloud_default_runner_template(ResearchGpuCloudBackend::Lambda, &req)
                .unwrap();

        assert_eq!(template.runner_payload.name, "Lambda GPU Runner");
        assert_eq!(
            template.runner_payload.image_or_runtime.as_deref(),
            Some(DEFAULT_RESEARCH_RUNNER_IMAGE)
        );
        assert_eq!(
            template.warnings,
            [
                "ThinClaw currently launches one Lambda instance per research trial, so quantity was normalized to 1."
            ]
        );
        assert_eq!(
            template.runner_payload.backend_config.get("quantity"),
            Some(&serde_json::json!(1))
        );
        assert_eq!(
            template
                .runner_payload
                .backend_config
                .get("region_name")
                .and_then(|value| value.as_str()),
            Some("us-east-1")
        );
        let preview = template.launch_payload_preview.unwrap();
        assert_eq!(
            preview
                .get("instance_type_name")
                .and_then(|value| value.as_str()),
            Some("gpu_1x_a100")
        );
        assert_eq!(
            preview.get("ssh_key_names"),
            Some(&serde_json::json!(["key-one"]))
        );
    }

    #[test]
    fn lambda_runner_template_requires_instance_type() {
        let req = ExperimentGpuCloudTemplateRequest {
            runner_name: None,
            image_or_runtime: None,
            region_name: None,
            instance_type_name: Some(" ".to_string()),
            quantity: 1,
            ssh_key_names: Vec::new(),
            file_system_names: Vec::new(),
        };

        let error =
            research_gpu_cloud_default_runner_template(ResearchGpuCloudBackend::Lambda, &req)
                .unwrap_err();

        assert_eq!(
            error,
            ResearchGpuCloudTemplateError::LambdaInstanceTypeRequired
        );
        assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn experiment_delete_response_preserves_existing_json_shape() {
        let value =
            serde_json::to_value(experiment_delete_response(true)).expect("serialize response");

        assert_eq!(value, serde_json::json!({ "deleted": true }));
    }

    #[test]
    fn gpu_cloud_action_responses_preserve_existing_json_shapes() {
        let provider = research_gpu_cloud_info("lambda", true).unwrap();
        let connect = research_gpu_cloud_connect_response(provider.clone());
        let connect_value = serde_json::to_value(connect).expect("serialize connect response");
        assert_eq!(connect_value["status"], serde_json::json!("ok"));
        assert_eq!(
            connect_value["message"],
            serde_json::json!("Lambda credentials connected.")
        );
        assert_eq!(
            connect_value["provider"]["slug"],
            serde_json::json!("lambda")
        );

        let validate = research_gpu_cloud_validate_response(
            "invalid_credentials",
            "bad key",
            false,
            provider.clone(),
        );
        let validate_value = serde_json::to_value(validate).expect("serialize validate response");
        assert_eq!(
            validate_value,
            serde_json::json!({
                "status": "invalid_credentials",
                "message": "bad key",
                "connected": false,
                "provider": provider.clone(),
            })
        );

        let launch = research_gpu_cloud_launch_test_response(
            ResearchGpuCloudBackend::Lambda,
            "validated",
            provider,
        );
        let launch_value = serde_json::to_value(launch).expect("serialize launch response");
        assert_eq!(launch_value["status"], serde_json::json!("ok"));
        assert_eq!(
            launch_value["provider"]["slug"],
            serde_json::json!("lambda")
        );
        assert_eq!(
            launch_value["launch_hint"],
            serde_json::json!({
                "backend": "lambda",
                "runner_template_required": false,
                "bootstrap": ResearchGpuCloudBackend::Lambda.launch_hint_bootstrap(),
            })
        );
    }

    #[test]
    fn gpu_cloud_template_response_keeps_runner_payload_generic() {
        let provider = research_gpu_cloud_info("runpod", false).unwrap();
        let template = ResearchGpuCloudRunnerTemplate {
            runner_payload: ResearchGpuCloudRunnerPayload {
                name: "ignored".to_string(),
                backend: ResearchGpuCloudBackend::Runpod,
                backend_config: serde_json::json!({}),
                image_or_runtime: None,
                gpu_requirements: serde_json::json!({}),
                env_grants: serde_json::json!({}),
                secret_references: Vec::new(),
                cache_policy: serde_json::json!({}),
            },
            warnings: vec!["warn".to_string()],
            launch_payload_preview: Some(serde_json::json!({ "preview": true })),
            message: "ready".to_string(),
        };
        let response = research_gpu_cloud_template_response(
            template,
            provider,
            serde_json::json!({ "root": "payload" }),
        );
        let value = serde_json::to_value(response).expect("serialize template response");

        assert_eq!(value["status"], serde_json::json!("ok"));
        assert_eq!(value["message"], serde_json::json!("ready"));
        assert_eq!(value["warnings"], serde_json::json!(["warn"]));
        assert_eq!(value["provider"]["slug"], serde_json::json!("runpod"));
        assert_eq!(
            value["runner_payload"],
            serde_json::json!({ "root": "payload" })
        );
        assert_eq!(
            value["launch_payload_preview"],
            serde_json::json!({ "preview": true })
        );
    }

    #[test]
    fn gpu_cloud_runner_payload_converts_to_experiment_runner_request() {
        let request = experiment_runner_payload(ResearchGpuCloudRunnerPayload {
            name: "RunPod runner".to_string(),
            backend: ResearchGpuCloudBackend::Runpod,
            backend_config: serde_json::json!({ "pod": true }),
            image_or_runtime: Some("image:latest".to_string()),
            gpu_requirements: serde_json::json!({ "gpu": "A100" }),
            env_grants: serde_json::json!({ "TOKEN": "secret" }),
            secret_references: vec!["runpod_api_key".to_string()],
            cache_policy: serde_json::json!({ "cache": true }),
        });

        assert_eq!(request.name, "RunPod runner");
        assert_eq!(request.backend, ExperimentRunnerBackend::Runpod);
        assert_eq!(request.backend_config["pod"], true);
        assert_eq!(request.image_or_runtime.as_deref(), Some("image:latest"));
        assert_eq!(request.secret_references, vec!["runpod_api_key"]);
    }
}
