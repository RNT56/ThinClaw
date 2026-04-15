//! Experiments API — optional research automation with local and remote runners.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::time::{Duration as TokioDuration, interval};
use uuid::Uuid;

use crate::agent::subagent_executor::{SubagentExecutor, SubagentSpawnRequest};
use crate::api::{ApiError, ApiResult};
use crate::db::Database;
use crate::experiments::adapters::{self, RemoteLaunchAction, RunnerLaunchOutcome};
use crate::experiments::{
    ExperimentArtifactRef, ExperimentAutonomyMode, ExperimentCampaign,
    ExperimentCampaignQueueState, ExperimentCampaignStatus, ExperimentComparisonPolicy,
    ExperimentGpuRequirement, ExperimentLease, ExperimentLeaseAuthentication,
    ExperimentLeaseStatus, ExperimentMetricDefinition, ExperimentModelUsageRecord,
    ExperimentOpportunity, ExperimentPreset, ExperimentProject, ExperimentProjectStatus,
    ExperimentRunnerArtifactUpload, ExperimentRunnerBackend, ExperimentRunnerCompletion,
    ExperimentRunnerJob, ExperimentRunnerProfile, ExperimentRunnerStatus, ExperimentStopPolicy,
    ExperimentTarget, ExperimentTargetKind, ExperimentTargetLink, ExperimentTrial,
    ExperimentTrialStatus, compare_metrics, extract_metrics, hash_lease_token,
};
use crate::history::{OutcomeContract, OutcomeContractQuery};
use crate::llm::usage_tracking::{
    USAGE_TRACKING_EXPERIMENT_CAMPAIGN_ID_KEY, USAGE_TRACKING_EXPERIMENT_ROLE_KEY,
    USAGE_TRACKING_EXPERIMENT_TARGET_IDS_KEY, USAGE_TRACKING_EXPERIMENT_TRIAL_ID_KEY,
};
use crate::secrets::SecretsStore;
use crate::settings::Settings;

const DEFAULT_REMOTE_LEASE_MINUTES: i64 = 60;
const DEFAULT_EXPERIMENT_CONTROLLER_TICK_SECS: u64 = 30;
const STALE_LEASE_GRACE_MINUTES: i64 = 10;
const DEFAULT_EXPERIMENT_USER_ID: &str = "default";
const RESEARCH_SUBAGENT_CHANNEL: &str = "tauri";
const RESEARCH_SUBAGENT_THREAD_ID: &str = "agent:research";
const RESEARCH_SHARED_TOOL_DENYLIST: &[&str] = &[
    "send_message",
    "tool_search",
    "tool_install",
    "tool_auth",
    "tool_activate",
    "tool_list",
    "tool_remove",
    "skill_install",
    "skill_remove",
    "skill_reload",
    "skill_manage",
    "prompt_manage",
    "memory_write",
    "memory_delete",
    "tts",
    "apple_mail",
    "create_agent",
    "list_agents",
    "update_agent",
    "remove_agent",
    "message_agent",
    "create_job",
    "list_jobs",
    "job_status",
    "cancel_job",
    "job_events",
    "job_prompt",
    "routine_create",
    "routine_list",
    "routine_update",
    "routine_delete",
    "routine_history",
];
const RESEARCH_READ_ONLY_TOOL_DENYLIST: &[&str] = &[
    "write_file",
    "apply_patch",
    "shell",
    "execute_code",
    "process",
    "build_software",
    "canvas",
    "homeassistant",
];
const RESEARCH_MUTATOR_TOOL_DENYLIST: &[&str] = &["canvas", "homeassistant"];

static RESEARCH_SUBAGENT_EXECUTOR: OnceLock<Arc<SubagentExecutor>> = OnceLock::new();
static RESEARCH_SECRETS_STORE: OnceLock<Arc<dyn SecretsStore + Send + Sync>> = OnceLock::new();

#[derive(Clone)]
struct OpportunityAggregate {
    provider: String,
    model: String,
    route_key: Option<String>,
    logical_role: Option<String>,
    kind: ExperimentTargetKind,
    class: UsageClass,
    call_count: u32,
    error_count: u32,
    latency_sum_ms: u64,
    cost_sum_usd: f64,
    first_seen: DateTime<Utc>,
    last_seen: DateTime<Utc>,
    linked_target_id: Option<Uuid>,
}

#[derive(Debug, Clone, Deserialize)]
struct PlannerProposal {
    hypothesis: String,
    #[serde(default)]
    target_ids: Vec<String>,
    #[serde(default)]
    allowed_paths: Vec<String>,
    #[serde(default)]
    expected_metric_direction: Option<String>,
    mutation_brief: String,
}

#[derive(Debug, Clone, Deserialize)]
struct MutatorResult {
    #[serde(default)]
    changed_paths: Vec<String>,
    mutation_summary: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ReviewerDecision {
    approved: bool,
    scope_ok: bool,
    benchmark_ready: bool,
    reason: String,
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

fn default_strategy_prompt() -> String {
    "Operate within the configured mutable paths only. Preserve the fixed harness, compare candidates against the best-known result, and stop when the campaign no longer improves.".to_string()
}

pub fn register_experiment_subagent_executor(executor: Arc<SubagentExecutor>) {
    let _ = RESEARCH_SUBAGENT_EXECUTOR.set(executor);
}

pub fn register_experiment_secrets_store(store: Arc<dyn SecretsStore + Send + Sync>) {
    let _ = RESEARCH_SECRETS_STORE.set(store);
}

fn research_subagent_executor() -> Option<Arc<SubagentExecutor>> {
    RESEARCH_SUBAGENT_EXECUTOR.get().cloned()
}

fn research_secrets_store() -> Option<Arc<dyn SecretsStore + Send + Sync>> {
    RESEARCH_SECRETS_STORE.get().cloned()
}

async fn research_provider_api_key(
    user_id: &str,
    runner: &ExperimentRunnerProfile,
) -> Option<String> {
    if !runner.backend.is_gpu_cloud() {
        return None;
    }
    let secrets = research_secrets_store()?;
    let mut names = Vec::new();
    if let Some(default_name) = adapters::gpu_cloud_secret_name(runner.backend) {
        names.push(default_name.to_string());
    }
    for name in &runner.secret_references {
        if !names.iter().any(|entry| entry == name) {
            names.push(name.clone());
        }
    }
    for name in names {
        match secrets.get_decrypted(user_id, &name).await {
            Ok(secret) => return Some(secret.expose().to_string()),
            Err(err) => {
                tracing::debug!(
                    provider = runner.backend.slug(),
                    secret_name = %name,
                    error = %err,
                    "Research provider secret lookup failed"
                );
            }
        }
    }
    None
}

async fn ensure_experiments_enabled(
    store: &Arc<dyn Database>,
    user_id: &str,
) -> ApiResult<Settings> {
    let map = store
        .get_all_settings(user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let settings = Settings::from_db_map(&map);
    if !settings.experiments.enabled {
        return Err(ApiError::FeatureDisabled(
            "Enable experiments in Settings → Features to use this API.".to_string(),
        ));
    }
    Ok(settings)
}

fn ready_project_status(project: &ExperimentProject) -> ExperimentProjectStatus {
    let workspace_exists = Path::new(&project.workspace_path).exists();
    if workspace_exists
        && !project.mutable_paths.is_empty()
        && !project.run_command.trim().is_empty()
    {
        ExperimentProjectStatus::Ready
    } else {
        ExperimentProjectStatus::Draft
    }
}

pub async fn list_projects(
    store: &Arc<dyn Database>,
    user_id: &str,
) -> ApiResult<ExperimentProjectListResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let projects = store
        .list_experiment_projects()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(ExperimentProjectListResponse { projects })
}

pub async fn get_project(
    store: &Arc<dyn Database>,
    user_id: &str,
    id: Uuid,
) -> ApiResult<ExperimentProject> {
    ensure_experiments_enabled(store, user_id).await?;
    store
        .get_experiment_project(id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(format!("Experiment project {id} not found")))
}

pub async fn create_project(
    store: &Arc<dyn Database>,
    user_id: &str,
    req: CreateExperimentProjectRequest,
) -> ApiResult<ExperimentProject> {
    let settings = ensure_experiments_enabled(store, user_id).await?;
    let now = Utc::now();
    let mut project = ExperimentProject {
        id: Uuid::new_v4(),
        name: req.name,
        workspace_path: req.workspace_path,
        git_remote_name: req.git_remote_name,
        base_branch: req.base_branch,
        preset: req
            .preset
            .unwrap_or(ExperimentPreset::AutoresearchSingleFile),
        strategy_prompt: req.strategy_prompt.unwrap_or_else(default_strategy_prompt),
        workdir: req.workdir,
        prepare_command: req.prepare_command,
        run_command: req.run_command,
        mutable_paths: req.mutable_paths,
        fixed_paths: req.fixed_paths,
        primary_metric: req.primary_metric,
        secondary_metrics: req.secondary_metrics,
        comparison_policy: req.comparison_policy.unwrap_or_default(),
        stop_policy: req.stop_policy.unwrap_or_default(),
        default_runner_profile_id: req.default_runner_profile_id,
        promotion_mode: req
            .promotion_mode
            .unwrap_or(settings.experiments.default_promotion_mode),
        autonomy_mode: req
            .autonomy_mode
            .unwrap_or(ExperimentAutonomyMode::Autonomous),
        status: ExperimentProjectStatus::Draft,
        created_at: now,
        updated_at: now,
    };
    project.status = ready_project_status(&project);
    store
        .create_experiment_project(&project)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(project)
}

pub async fn update_project(
    store: &Arc<dyn Database>,
    user_id: &str,
    id: Uuid,
    req: UpdateExperimentProjectRequest,
) -> ApiResult<ExperimentProject> {
    ensure_experiments_enabled(store, user_id).await?;
    let mut project = get_project(store, user_id, id).await?;
    if let Some(value) = req.name {
        project.name = value;
    }
    if let Some(value) = req.workspace_path {
        project.workspace_path = value;
    }
    if let Some(value) = req.git_remote_name {
        project.git_remote_name = value;
    }
    if let Some(value) = req.base_branch {
        project.base_branch = value;
    }
    if let Some(value) = req.preset {
        project.preset = value;
    }
    if let Some(value) = req.strategy_prompt {
        project.strategy_prompt = value;
    }
    if let Some(value) = req.workdir {
        project.workdir = value;
    }
    if req.prepare_command.is_some() {
        project.prepare_command = req.prepare_command;
    }
    if let Some(value) = req.run_command {
        project.run_command = value;
    }
    if let Some(value) = req.mutable_paths {
        project.mutable_paths = value;
    }
    if let Some(value) = req.fixed_paths {
        project.fixed_paths = value;
    }
    if let Some(value) = req.primary_metric {
        project.primary_metric = value;
    }
    if let Some(value) = req.secondary_metrics {
        project.secondary_metrics = value;
    }
    if let Some(value) = req.comparison_policy {
        project.comparison_policy = value;
    }
    if let Some(value) = req.stop_policy {
        project.stop_policy = value;
    }
    if req.default_runner_profile_id.is_some() {
        project.default_runner_profile_id = req.default_runner_profile_id;
    }
    if let Some(value) = req.promotion_mode {
        project.promotion_mode = value;
    }
    if let Some(value) = req.autonomy_mode {
        project.autonomy_mode = value;
    }
    project.status = req.status.unwrap_or_else(|| ready_project_status(&project));
    project.updated_at = Utc::now();
    store
        .update_experiment_project(&project)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(project)
}

pub async fn delete_project(store: &Arc<dyn Database>, user_id: &str, id: Uuid) -> ApiResult<bool> {
    ensure_experiments_enabled(store, user_id).await?;
    store
        .delete_experiment_project(id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))
}

pub async fn list_runners(
    store: &Arc<dyn Database>,
    user_id: &str,
) -> ApiResult<ExperimentRunnerListResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let runners = store
        .list_experiment_runner_profiles()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(ExperimentRunnerListResponse { runners })
}

pub async fn get_runner(
    store: &Arc<dyn Database>,
    user_id: &str,
    id: Uuid,
) -> ApiResult<ExperimentRunnerProfile> {
    ensure_experiments_enabled(store, user_id).await?;
    store
        .get_experiment_runner_profile(id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(format!("Experiment runner {id} not found")))
}

pub async fn create_runner(
    store: &Arc<dyn Database>,
    user_id: &str,
    req: CreateExperimentRunnerProfileRequest,
) -> ApiResult<ExperimentRunnerProfile> {
    ensure_experiments_enabled(store, user_id).await?;
    let now = Utc::now();
    let runner = ExperimentRunnerProfile {
        id: Uuid::new_v4(),
        name: req.name,
        backend: req.backend,
        backend_config: req.backend_config,
        image_or_runtime: req.image_or_runtime,
        gpu_requirements: req.gpu_requirements,
        env_grants: req.env_grants,
        secret_references: req.secret_references,
        cache_policy: req.cache_policy,
        status: ExperimentRunnerStatus::Draft,
        created_at: now,
        updated_at: now,
    };
    store
        .create_experiment_runner_profile(&runner)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(runner)
}

pub async fn update_runner(
    store: &Arc<dyn Database>,
    user_id: &str,
    id: Uuid,
    req: UpdateExperimentRunnerProfileRequest,
) -> ApiResult<ExperimentRunnerProfile> {
    ensure_experiments_enabled(store, user_id).await?;
    let mut runner = get_runner(store, user_id, id).await?;
    if let Some(value) = req.name {
        runner.name = value;
    }
    if let Some(value) = req.backend {
        runner.backend = value;
    }
    if let Some(value) = req.backend_config {
        runner.backend_config = value;
    }
    if req.image_or_runtime.is_some() {
        runner.image_or_runtime = req.image_or_runtime;
    }
    if let Some(value) = req.gpu_requirements {
        runner.gpu_requirements = value;
    }
    if let Some(value) = req.env_grants {
        runner.env_grants = value;
    }
    if let Some(value) = req.secret_references {
        runner.secret_references = value;
    }
    if let Some(value) = req.cache_policy {
        runner.cache_policy = value;
    }
    if let Some(value) = req.status {
        runner.status = value;
    }
    runner.updated_at = Utc::now();
    store
        .update_experiment_runner_profile(&runner)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(runner)
}

pub async fn delete_runner(store: &Arc<dyn Database>, user_id: &str, id: Uuid) -> ApiResult<bool> {
    ensure_experiments_enabled(store, user_id).await?;
    store
        .delete_experiment_runner_profile(id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))
}

pub async fn validate_runner(
    store: &Arc<dyn Database>,
    user_id: &str,
    id: Uuid,
) -> ApiResult<ExperimentRunnerValidationResponse> {
    let settings = ensure_experiments_enabled(store, user_id).await?;
    let mut runner = get_runner(store, user_id, id).await?;
    let (valid, message) = validate_runner_profile_impl(user_id, &runner, &settings).await;
    runner.status = if valid {
        ExperimentRunnerStatus::Validated
    } else {
        ExperimentRunnerStatus::Unavailable
    };
    runner.updated_at = Utc::now();
    store
        .update_experiment_runner_profile(&runner)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(ExperimentRunnerValidationResponse {
        runner,
        valid,
        message,
    })
}

pub async fn list_campaigns(
    store: &Arc<dyn Database>,
    user_id: &str,
) -> ApiResult<ExperimentCampaignListResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let campaigns = store
        .list_experiment_campaigns()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(ExperimentCampaignListResponse { campaigns })
}

pub async fn get_campaign(
    store: &Arc<dyn Database>,
    user_id: &str,
    id: Uuid,
) -> ApiResult<ExperimentCampaign> {
    ensure_experiments_enabled(store, user_id).await?;
    store
        .get_experiment_campaign(id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(format!("Experiment campaign {id} not found")))
}

pub async fn list_trials(
    store: &Arc<dyn Database>,
    user_id: &str,
    campaign_id: Uuid,
) -> ApiResult<ExperimentTrialListResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let trials = store
        .list_experiment_trials(campaign_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(ExperimentTrialListResponse { trials })
}

pub async fn get_trial(
    store: &Arc<dyn Database>,
    user_id: &str,
    id: Uuid,
) -> ApiResult<ExperimentTrial> {
    ensure_experiments_enabled(store, user_id).await?;
    store
        .get_experiment_trial(id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(format!("Experiment trial {id} not found")))
}

pub async fn list_artifacts(
    store: &Arc<dyn Database>,
    user_id: &str,
    trial_id: Uuid,
) -> ApiResult<ExperimentArtifactListResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let artifacts = store
        .list_experiment_artifacts(trial_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(ExperimentArtifactListResponse { artifacts })
}

pub async fn list_targets(
    store: &Arc<dyn Database>,
    user_id: &str,
) -> ApiResult<ExperimentTargetListResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let targets = store
        .list_experiment_targets()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(ExperimentTargetListResponse { targets })
}

pub async fn start_experiment_controller_loop(store: Arc<dyn Database>) {
    let mut interval = interval(TokioDuration::from_secs(
        DEFAULT_EXPERIMENT_CONTROLLER_TICK_SECS,
    ));
    interval.tick().await;
    loop {
        match reconcile_experiments_once(&store, DEFAULT_EXPERIMENT_USER_ID).await {
            Ok(()) => {}
            Err(error) => match error {
                ApiError::FeatureDisabled(_) => {
                    tracing::debug!("Experiment controller loop skipped: experiments are disabled");
                }
                _ => tracing::warn!("Experiment controller reconcile failed: {error}"),
            },
        }
        interval.tick().await;
    }
}

async fn reconcile_experiments_once(store: &Arc<dyn Database>, user_id: &str) -> ApiResult<()> {
    let map = store
        .get_all_settings(user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let settings = Settings::from_db_map(&map);
    if !settings.experiments.enabled {
        return Ok(());
    }

    let campaigns = store
        .list_experiment_campaigns()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    for mut campaign in campaigns {
        if matches!(
            campaign.status,
            ExperimentCampaignStatus::Completed
                | ExperimentCampaignStatus::Cancelled
                | ExperimentCampaignStatus::Failed
        ) {
            campaign.queue_state = ExperimentCampaignQueueState::NotQueued;
            continue;
        }

        if campaign.status == ExperimentCampaignStatus::PendingBaseline
            && campaign.queue_state == ExperimentCampaignQueueState::Queued
        {
            continue;
        }

        if campaign.status == ExperimentCampaignStatus::Running
            || (campaign.status == ExperimentCampaignStatus::PendingBaseline
                && campaign.queue_state != ExperimentCampaignQueueState::Queued)
        {
            reconcile_active_campaign(store, user_id, &mut campaign).await?;
        }
    }

    maybe_launch_next_queued_after_slot_release(store, user_id).await
}

async fn reconcile_active_campaign(
    store: &Arc<dyn Database>,
    user_id: &str,
    campaign: &mut ExperimentCampaign,
) -> ApiResult<()> {
    let project = get_project(store, user_id, campaign.project_id).await?;
    let runner = get_runner(store, user_id, campaign.runner_profile_id).await?;
    let settings = ensure_experiments_enabled(store, user_id).await?;
    let latest = latest_trial(store, campaign.id).await?;

    let max_trials = campaign
        .max_trials_override
        .or(project.stop_policy.max_trials);
    let max_trials_reached = latest
        .as_ref()
        .map(|trial| max_trials.is_some_and(|limit| trial.sequence >= limit))
        .unwrap_or(false);
    let runtime_budget_reached = project
        .stop_policy
        .max_total_runtime_secs
        .is_some_and(|limit| campaign.total_runtime_ms / 1000 >= limit);
    let cost_budget_reached = project
        .stop_policy
        .max_total_cost_usd
        .is_some_and(|limit| campaign.total_cost_usd >= limit);
    let infra_failure_threshold_reached =
        campaign.failure_count >= project.stop_policy.infra_failure_pause_threshold;
    let plateau_window = project
        .stop_policy
        .plateau_window
        .unwrap_or(project.stop_policy.non_improving_pause_threshold);
    let non_improving_threshold_reached =
        campaign.consecutive_non_improving_trials >= plateau_window;

    if let Some(mut trial) = latest {
        if matches!(
            trial.status,
            ExperimentTrialStatus::Preparing
                | ExperimentTrialStatus::Running
                | ExperimentTrialStatus::Evaluating
        ) {
            if let Some(lease) = latest_active_lease(store, trial.id).await? {
                if is_stale_lease(&lease, Utc::now()) {
                    trial.status = ExperimentTrialStatus::TimedOut;
                    trial.decision_reason = Some(
                        "Tracked lease was stale while trial was in-flight. Campaign paused for operator review.".to_string(),
                    );
                    trial.updated_at = Utc::now();
                    campaign.active_trial_id = None;
                    campaign.failure_count = campaign.failure_count.saturating_add(1);
                    campaign.status = ExperimentCampaignStatus::Paused;
                    campaign.queue_state = ExperimentCampaignQueueState::NotQueued;
                    campaign.pause_reason = Some(
                        "Tracked lease was stale and could not be confirmed. Reissue lease or resume manually."
                            .to_string(),
                    );
                    campaign.updated_at = Utc::now();
                    store
                        .update_experiment_trial(&trial)
                        .await
                        .map_err(|e| ApiError::Internal(e.to_string()))?;
                    store
                        .update_experiment_campaign(campaign)
                        .await
                        .map_err(|e| ApiError::Internal(e.to_string()))?;
                    maybe_launch_next_queued_after_slot_release(store, user_id).await?;
                    return Ok(());
                }

                return Ok(());
            }

            if runner.backend.is_remote() {
                campaign.active_trial_id = None;
                campaign.failure_count = campaign.failure_count.saturating_add(1);
                campaign.status = ExperimentCampaignStatus::Paused;
                campaign.queue_state = ExperimentCampaignQueueState::NotQueued;
                campaign.pause_reason = Some(
                    "Running remote trial is missing a claimed lease after restart. Reissue the lease or retry manually."
                        .to_string(),
                );
                campaign.updated_at = Utc::now();
                campaign.trial_count = campaign.trial_count.max(trial.sequence);
                store
                    .update_experiment_campaign(campaign)
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?;
                maybe_launch_next_queued_after_slot_release(store, user_id).await?;
            }
            return Ok(());
        }

        if campaign.status == ExperimentCampaignStatus::Running {
            if max_trials_reached {
                campaign.status = ExperimentCampaignStatus::AwaitingPromotion;
                campaign.pause_reason = Some(format!(
                    "Reached max_trials={limit}. Promote the best commit when ready.",
                    limit = max_trials.unwrap()
                ));
                campaign.updated_at = Utc::now();
                store
                    .update_experiment_campaign(campaign)
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?;
                maybe_launch_next_queued_after_slot_release(store, user_id).await?;
                return Ok(());
            }

            if runtime_budget_reached {
                campaign.status = if campaign.best_commit.is_some() {
                    ExperimentCampaignStatus::AwaitingPromotion
                } else {
                    ExperimentCampaignStatus::Failed
                };
                campaign.pause_reason = Some(
                    "Reached the campaign runtime budget. Promote the best commit when ready."
                        .to_string(),
                );
                campaign.updated_at = Utc::now();
                store
                    .update_experiment_campaign(campaign)
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?;
                maybe_launch_next_queued_after_slot_release(store, user_id).await?;
                return Ok(());
            }

            if cost_budget_reached {
                campaign.status = if campaign.best_commit.is_some() {
                    ExperimentCampaignStatus::AwaitingPromotion
                } else {
                    ExperimentCampaignStatus::Failed
                };
                campaign.pause_reason = Some(
                    "Reached the campaign cost budget. Promote the best commit when ready."
                        .to_string(),
                );
                campaign.updated_at = Utc::now();
                store
                    .update_experiment_campaign(campaign)
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?;
                maybe_launch_next_queued_after_slot_release(store, user_id).await?;
                return Ok(());
            }

            if infra_failure_threshold_reached || non_improving_threshold_reached {
                campaign.status = ExperimentCampaignStatus::Paused;
                campaign.pause_reason = Some(format!(
                    "Campaign paused after hitting configured thresholds (infra failures: {}, non-improving trials: {}).",
                    campaign.failure_count, campaign.consecutive_non_improving_trials
                ));
                campaign.updated_at = Utc::now();
                store
                    .update_experiment_campaign(campaign)
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?;
                maybe_launch_next_queued_after_slot_release(store, user_id).await?;
                return Ok(());
            }

            return launch_next_trial_if_ready(
                store, user_id, &settings, &project, &runner, campaign,
            )
            .await
            .map(|_| ());
        }

        if campaign.status == ExperimentCampaignStatus::Running {
            return launch_next_trial_if_ready(
                store, user_id, &settings, &project, &runner, campaign,
            )
            .await
            .map(|_| ());
        }
    }

    if campaign.status == ExperimentCampaignStatus::Running {
        campaign.status = ExperimentCampaignStatus::Paused;
        campaign.queue_state = ExperimentCampaignQueueState::NotQueued;
        campaign.pause_reason = Some(
            "Campaign state recovery could not find a valid trial record. Resume manually."
                .to_string(),
        );
        campaign.updated_at = Utc::now();
        store
            .update_experiment_campaign(campaign)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        maybe_launch_next_queued_after_slot_release(store, user_id).await?;
    }

    Ok(())
}

fn is_stale_lease(lease: &ExperimentLease, now: DateTime<Utc>) -> bool {
    match lease.status {
        ExperimentLeaseStatus::Pending => {
            lease.expires_at + chrono::Duration::minutes(STALE_LEASE_GRACE_MINUTES) < now
        }
        ExperimentLeaseStatus::Claimed => {
            lease.updated_at + chrono::Duration::minutes(STALE_LEASE_GRACE_MINUTES) < now
        }
        _ => false,
    }
}

async fn launch_next_trial_if_ready(
    store: &Arc<dyn Database>,
    user_id: &str,
    settings: &Settings,
    project: &ExperimentProject,
    runner: &ExperimentRunnerProfile,
    campaign: &mut ExperimentCampaign,
) -> ApiResult<()> {
    if campaign.queue_state == ExperimentCampaignQueueState::Active {
        return Ok(());
    }

    match project.autonomy_mode {
        ExperimentAutonomyMode::ManualCandidate => {
            campaign.status = ExperimentCampaignStatus::Paused;
            campaign.queue_state = ExperimentCampaignQueueState::NotQueued;
            campaign.pause_reason =
                Some("Awaiting manual candidate changes in the campaign worktree.".to_string());
            campaign.updated_at = Utc::now();
            store
                .update_experiment_campaign(campaign)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            maybe_launch_next_queued_after_slot_release(store, user_id).await?;
            return Ok(());
        }
        ExperimentAutonomyMode::SuggestOnly => {
            let planner = run_planner_subagent(store, campaign, project, None).await?;
            campaign.status = ExperimentCampaignStatus::Paused;
            campaign.queue_state = ExperimentCampaignQueueState::NotQueued;
            campaign.pause_reason = Some(format!(
                "Suggestion ready: {}",
                truncate_for_prompt(&planner.mutation_brief, 500)
            ));
            campaign.updated_at = Utc::now();
            store
                .update_experiment_campaign(campaign)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            maybe_launch_next_queued_after_slot_release(store, user_id).await?;
            return Ok(());
        }
        ExperimentAutonomyMode::Autonomous => {}
    }

    let trial = match create_experiment_trial_commit(store, campaign, project, runner).await {
        Ok(trial) => trial,
        Err(error) => {
            campaign.status = ExperimentCampaignStatus::Paused;
            campaign.queue_state = ExperimentCampaignQueueState::NotQueued;
            campaign.pause_reason =
                Some(format!("Autonomous candidate generation paused: {error}"));
            campaign.updated_at = Utc::now();
            store
                .update_experiment_campaign(campaign)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            maybe_launch_next_queued_after_slot_release(store, user_id).await?;
            return Ok(());
        }
    };
    let response = launch_trial(
        store,
        user_id,
        settings,
        project,
        runner,
        campaign.clone(),
        trial,
    )
    .await?;
    *campaign = response.campaign;
    Ok(())
}

pub async fn create_target(
    store: &Arc<dyn Database>,
    user_id: &str,
    req: CreateExperimentTargetRequest,
) -> ApiResult<ExperimentTarget> {
    ensure_experiments_enabled(store, user_id).await?;
    let kind = req.kind;
    let metadata = if req.metadata.is_object() {
        req.metadata
    } else {
        serde_json::json!({})
    };
    let targets = store
        .list_experiment_targets()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    ensure_unique_target_signature(kind, &metadata, None, &targets)?;
    let now = Utc::now();
    let target = ExperimentTarget {
        id: Uuid::new_v4(),
        name: req.name,
        kind,
        location: req.location,
        metadata,
        created_at: now,
        updated_at: now,
    };
    store
        .create_experiment_target(&target)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(target)
}

pub async fn link_target(
    store: &Arc<dyn Database>,
    user_id: &str,
    req: LinkExperimentTargetRequest,
) -> ApiResult<ExperimentTarget> {
    ensure_experiments_enabled(store, user_id).await?;
    if req.target_id.trim().is_empty() {
        return Err(ApiError::InvalidInput("target_id is required".to_string()));
    }

    let usage = store
        .list_experiment_model_usage(250)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let targets = store
        .list_experiment_targets()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let target_links = store
        .list_experiment_target_links()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let opportunity = derive_opportunities(&usage, &targets, &target_links)
        .into_iter()
        .find(|entry| entry.id == req.opportunity_id)
        .ok_or_else(|| {
            ApiError::SessionNotFound(format!(
                "Experiment opportunity {} not found",
                req.opportunity_id
            ))
        })?;

    let mut metadata = if req.metadata.is_object() {
        req.metadata
    } else {
        serde_json::json!({})
    };
    if let Some(obj) = metadata.as_object_mut() {
        obj.insert(
            "asset_id".to_string(),
            serde_json::json!(req.target_id.trim()),
        );
        obj.insert(
            "opportunity_id".to_string(),
            serde_json::json!(opportunity.id),
        );
        obj.insert(
            "provider".to_string(),
            serde_json::json!(opportunity.provider),
        );
        obj.insert("model".to_string(), serde_json::json!(opportunity.model));
        if let Some(route_key) = opportunity.route_key.clone() {
            obj.insert("route_key".to_string(), serde_json::json!(route_key));
        }
        if let Some(logical_role) = opportunity.logical_role.clone() {
            obj.insert("logical_role".to_string(), serde_json::json!(logical_role));
        }
        obj.insert(
            "suggested_preset".to_string(),
            serde_json::json!(opportunity.suggested_preset),
        );
        obj.insert(
            "gpu_requirement".to_string(),
            serde_json::json!(opportunity.gpu_requirement),
        );
    }

    let now = Utc::now();
    ensure_unique_target_signature(req.target_type, &metadata, None, &targets)?;

    if let Some(mut target) = targets.into_iter().find(|target| {
        target.kind == req.target_type
            && target
                .metadata
                .get("asset_id")
                .and_then(|value| value.as_str())
                .map(|value| value == req.target_id.trim())
                .unwrap_or(false)
    }) {
        target.name = req
            .target_name
            .clone()
            .unwrap_or_else(|| target.name.clone());
        if req.location.is_some() {
            target.location = req.location.clone();
        }
        target.metadata = merge_json(&target.metadata, &metadata);
        target.updated_at = now;
        store
            .update_experiment_target(&target)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let link = ExperimentTargetLink {
            id: Uuid::new_v4(),
            target_id: target.id,
            kind: req.target_type,
            provider: opportunity.provider.clone(),
            model: opportunity.model.clone(),
            route_key: opportunity.route_key.clone(),
            logical_role: opportunity.logical_role.clone(),
            metadata: serde_json::json!({
                "opportunity_id": opportunity.id,
                "suggested_preset": opportunity.suggested_preset,
                "gpu_requirement": opportunity.gpu_requirement,
            }),
            created_at: now,
            updated_at: now,
        };
        store
            .upsert_experiment_target_link(&link)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        return Ok(target);
    }

    let target = ExperimentTarget {
        id: Uuid::new_v4(),
        name: req
            .target_name
            .unwrap_or_else(|| req.target_id.trim().to_string()),
        kind: req.target_type,
        location: req.location,
        metadata,
        created_at: now,
        updated_at: now,
    };
    store
        .create_experiment_target(&target)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let link = ExperimentTargetLink {
        id: Uuid::new_v4(),
        target_id: target.id,
        kind: req.target_type,
        provider: opportunity.provider,
        model: opportunity.model,
        route_key: opportunity.route_key,
        logical_role: opportunity.logical_role,
        metadata: serde_json::json!({
            "opportunity_id": req.opportunity_id,
            "suggested_preset": opportunity.suggested_preset,
            "gpu_requirement": opportunity.gpu_requirement,
        }),
        created_at: now,
        updated_at: now,
    };
    store
        .upsert_experiment_target_link(&link)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(target)
}

pub async fn update_target(
    store: &Arc<dyn Database>,
    user_id: &str,
    id: Uuid,
    req: UpdateExperimentTargetRequest,
) -> ApiResult<ExperimentTarget> {
    ensure_experiments_enabled(store, user_id).await?;
    let mut target = store
        .get_experiment_target(id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(format!("Experiment target {id} not found")))?;
    let new_kind = req.kind.unwrap_or(target.kind);
    let new_metadata = req
        .metadata
        .as_ref()
        .map(|metadata| {
            if metadata.is_object() {
                metadata.clone()
            } else {
                serde_json::json!({})
            }
        })
        .or_else(|| Some(target.metadata.clone()));
    let existing_targets = store
        .list_experiment_targets()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if let Some(ref metadata) = new_metadata {
        ensure_unique_target_signature(new_kind, metadata, Some(id), &existing_targets)?;
        target.metadata = metadata.clone();
    } else {
        ensure_unique_target_signature(new_kind, &target.metadata, Some(id), &existing_targets)?;
    }

    if let Some(name) = req.name {
        target.name = name;
    }
    target.kind = new_kind;
    if req.location.is_some() {
        target.location = req.location;
    }

    target.updated_at = Utc::now();
    store
        .update_experiment_target(&target)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(target)
}

pub async fn delete_target(store: &Arc<dyn Database>, user_id: &str, id: Uuid) -> ApiResult<bool> {
    ensure_experiments_enabled(store, user_id).await?;
    store
        .delete_experiment_target_links_for_target(id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    store
        .delete_experiment_target(id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))
}

pub async fn list_model_usage(
    store: &Arc<dyn Database>,
    user_id: &str,
    limit: usize,
) -> ApiResult<ExperimentModelUsageListResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let usage = store
        .list_experiment_model_usage(limit)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(ExperimentModelUsageListResponse { usage })
}

pub async fn list_opportunities(
    store: &Arc<dyn Database>,
    user_id: &str,
    limit: usize,
) -> ApiResult<ExperimentOpportunityListResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let usage = store
        .list_experiment_model_usage(limit)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let targets = store
        .list_experiment_targets()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let target_links = store
        .list_experiment_target_links()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let outcome_contracts = store
        .list_outcome_contracts(&OutcomeContractQuery {
            user_id: user_id.to_string(),
            actor_id: None,
            status: Some("evaluated".to_string()),
            contract_type: None,
            source_kind: None,
            source_id: None,
            thread_id: None,
            limit: ((limit.max(25)) * 8) as i64,
        })
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let mut opportunities = derive_opportunities(&usage, &targets, &target_links);
    opportunities.extend(derive_outcome_opportunities(
        &outcome_contracts,
        &targets,
        limit,
    ));
    sort_experiment_opportunities(&mut opportunities);
    opportunities.truncate(limit.max(1));
    Ok(ExperimentOpportunityListResponse { opportunities })
}

pub async fn list_gpu_cloud_providers(
    store: &Arc<dyn Database>,
    user_id: &str,
) -> ApiResult<ExperimentGpuCloudProviderListResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let providers = [
        ExperimentRunnerBackend::Runpod,
        ExperimentRunnerBackend::Vast,
        ExperimentRunnerBackend::Lambda,
    ]
    .into_iter()
    .map(|backend| ExperimentGpuCloudProviderInfo {
        slug: backend.slug().to_string(),
        display_name: adapters::gpu_cloud_display_name(backend).to_string(),
        backend,
        description: format!(
            "{} setup for outbound ThinClaw experiment runners.",
            adapters::gpu_cloud_display_name(backend)
        ),
        signup_url: adapters::gpu_cloud_signup_url(backend)
            .unwrap_or_default()
            .to_string(),
        docs_url: adapters::gpu_cloud_docs_url(backend)
            .unwrap_or_default()
            .to_string(),
        secret_name: adapters::gpu_cloud_secret_name(backend)
            .unwrap_or_default()
            .to_string(),
        connected: false,
        template_hint: Some(adapters::gpu_cloud_template_hint(backend)),
    })
    .collect();
    Ok(ExperimentGpuCloudProviderListResponse { providers })
}

pub async fn start_campaign(
    store: &Arc<dyn Database>,
    user_id: &str,
    project_id: Uuid,
    req: StartExperimentCampaignRequest,
) -> ApiResult<ExperimentCampaignActionResponse> {
    let settings = ensure_experiments_enabled(store, user_id).await?;
    let project = get_project(store, user_id, project_id).await?;
    let active_before = active_campaign_count(store).await?;
    let queue_state = if active_before >= settings.experiments.max_concurrent_campaigns as usize {
        ExperimentCampaignQueueState::Queued
    } else {
        ExperimentCampaignQueueState::NotQueued
    };
    let queue_position = if queue_state == ExperimentCampaignQueueState::Queued {
        next_queue_position(store).await?
    } else {
        0
    };
    let runner_id = req
        .runner_profile_id
        .or(project.default_runner_profile_id)
        .ok_or_else(|| ApiError::InvalidInput("runner_profile_id is required".to_string()))?;
    let runner = get_runner(store, user_id, runner_id).await?;
    if runner.status != ExperimentRunnerStatus::Validated {
        return Err(ApiError::InvalidInput(
            "Runner profile must be validated before starting a campaign".to_string(),
        ));
    }

    let now = Utc::now();
    let campaign_id = Uuid::new_v4();
    let worktree_path = experiments_worktree_path(&project.workspace_path, campaign_id);
    let experiment_branch = format!("codex/experiments/{}", short_id(campaign_id));
    let campaign = ExperimentCampaign {
        id: campaign_id,
        project_id: project.id,
        runner_profile_id: runner.id,
        status: ExperimentCampaignStatus::PendingBaseline,
        baseline_commit: None,
        best_commit: None,
        best_metrics: serde_json::json!({}),
        experiment_branch: Some(experiment_branch.clone()),
        remote_ref: Some(format!("refs/heads/{experiment_branch}")),
        worktree_path: Some(worktree_path.to_string_lossy().to_string()),
        started_at: Some(now),
        ended_at: None,
        trial_count: 0,
        failure_count: 0,
        pause_reason: Some("Pending baseline launch.".to_string()),
        queue_state,
        queue_position,
        active_trial_id: None,
        total_runtime_ms: 0,
        total_cost_usd: 0.0,
        total_llm_cost_usd: 0.0,
        total_runner_cost_usd: 0.0,
        consecutive_non_improving_trials: 0,
        max_trials_override: req.max_trials_override,
        gateway_url: req.gateway_url,
        metadata: serde_json::json!({}),
        created_at: now,
        updated_at: now,
    };
    store
        .create_experiment_campaign(&campaign)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    if queue_state == ExperimentCampaignQueueState::Queued {
        let mut queued_campaign = campaign.clone();
        queued_campaign.pause_reason = Some("Queued until a research slot frees up.".to_string());
        queued_campaign.updated_at = Utc::now();
        store
            .update_experiment_campaign(&queued_campaign)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        return Ok(ExperimentCampaignActionResponse {
            campaign: queued_campaign,
            trial: None,
            lease: None,
            launch: None,
            message: format!(
                "Campaign queued. Waiting for one of the {} active campaign slots to free up.",
                settings.experiments.max_concurrent_campaigns
            ),
        });
    }

    match launch_campaign_baseline(
        store,
        user_id,
        &settings,
        &project,
        &runner,
        campaign.clone(),
    )
    .await
    {
        Ok(response) => {
            maybe_launch_next_queued_after_slot_release(store, user_id).await?;
            Ok(response)
        }
        Err(error) => {
            persist_campaign_launch_failure(store, campaign, &error.to_string()).await?;
            Err(error)
        }
    }
}

fn campaign_gateway_url(campaign: &ExperimentCampaign) -> Option<String> {
    campaign
        .gateway_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn launch_details_from_outcome(outcome: RunnerLaunchOutcome) -> ExperimentLaunchDetails {
    ExperimentLaunchDetails {
        message: outcome.message,
        bootstrap_command: outcome.bootstrap_command,
        provider_template: outcome.provider_template,
        provider_job_id: outcome.provider_job_id,
        provider_job_metadata: outcome.provider_job_metadata,
        auto_launched: outcome.auto_launched,
        requires_operator_action: outcome.requires_operator_action,
    }
}

async fn active_campaign_count(store: &Arc<dyn Database>) -> ApiResult<usize> {
    let campaigns = store
        .list_experiment_campaigns()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(campaigns
        .into_iter()
        .filter(|campaign| {
            campaign.status == ExperimentCampaignStatus::Running
                || (campaign.status == ExperimentCampaignStatus::PendingBaseline
                    && campaign.queue_state != ExperimentCampaignQueueState::Queued)
        })
        .count())
}

async fn next_queue_position(store: &Arc<dyn Database>) -> ApiResult<u32> {
    let campaigns = store
        .list_experiment_campaigns()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(campaigns
        .into_iter()
        .filter(|campaign| {
            campaign.status == ExperimentCampaignStatus::PendingBaseline
                && campaign.queue_state == ExperimentCampaignQueueState::Queued
        })
        .map(|campaign| campaign.queue_position)
        .max()
        .unwrap_or(0)
        .saturating_add(1))
}

async fn next_queued_campaign(store: &Arc<dyn Database>) -> ApiResult<Option<ExperimentCampaign>> {
    let mut queued: Vec<_> = store
        .list_experiment_campaigns()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .into_iter()
        .filter(|campaign| {
            campaign.status == ExperimentCampaignStatus::PendingBaseline
                && campaign.queue_state == ExperimentCampaignQueueState::Queued
        })
        .collect();
    queued.sort_by_key(|campaign| (campaign.queue_position, campaign.created_at));
    Ok(queued.into_iter().next())
}

async fn maybe_launch_next_queued_campaign(
    store: &Arc<dyn Database>,
    user_id: &str,
) -> ApiResult<Option<ExperimentCampaignActionResponse>> {
    let settings = ensure_experiments_enabled(store, user_id).await?;
    let active_count = active_campaign_count(store).await?;
    if active_count >= settings.experiments.max_concurrent_campaigns as usize {
        return Ok(None);
    }

    let Some(mut campaign) = next_queued_campaign(store).await? else {
        return Ok(None);
    };
    let project = get_project(store, user_id, campaign.project_id).await?;
    let runner = get_runner(store, user_id, campaign.runner_profile_id).await?;

    if runner.status != ExperimentRunnerStatus::Validated {
        campaign.status = ExperimentCampaignStatus::Failed;
        campaign.pause_reason = Some(format!(
            "Queued launch failed because runner '{}' is not validated.",
            runner.name
        ));
        campaign.ended_at = Some(Utc::now());
        campaign.updated_at = Utc::now();
        store
            .update_experiment_campaign(&campaign)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        return Ok(Some(ExperimentCampaignActionResponse {
            campaign,
            trial: None,
            lease: None,
            launch: None,
            message: "Queued campaign failed validation before launch.".to_string(),
        }));
    }

    match launch_campaign_baseline(
        store,
        user_id,
        &settings,
        &project,
        &runner,
        campaign.clone(),
    )
    .await
    {
        Ok(response) => Ok(Some(response)),
        Err(error) => {
            persist_campaign_launch_failure(store, campaign, &error.to_string()).await?;
            Err(error)
        }
    }
}

async fn maybe_launch_next_queued_after_slot_release(
    store: &Arc<dyn Database>,
    user_id: &str,
) -> ApiResult<()> {
    loop {
        match maybe_launch_next_queued_campaign(store, user_id).await {
            Ok(Some(_)) => continue,
            Ok(None) => break,
            Err(error) => {
                tracing::warn!("failed to launch queued experiment campaign: {error}");
                break;
            }
        }
    }
    Ok(())
}

async fn persist_campaign_launch_failure(
    store: &Arc<dyn Database>,
    mut campaign: ExperimentCampaign,
    reason: &str,
) -> ApiResult<()> {
    campaign.status = ExperimentCampaignStatus::Failed;
    campaign.pause_reason = Some(format!("Baseline launch failed: {reason}"));
    campaign.queue_state = ExperimentCampaignQueueState::NotQueued;
    campaign.ended_at = Some(Utc::now());
    campaign.updated_at = Utc::now();
    store
        .update_experiment_campaign(&campaign)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(())
}

async fn launch_campaign_baseline(
    store: &Arc<dyn Database>,
    user_id: &str,
    _settings: &Settings,
    project: &ExperimentProject,
    runner: &ExperimentRunnerProfile,
    mut campaign: ExperimentCampaign,
) -> ApiResult<ExperimentCampaignActionResponse> {
    let worktree_path = campaign
        .worktree_path
        .clone()
        .ok_or_else(|| ApiError::Internal("Campaign missing worktree_path".to_string()))?;
    let branch = campaign
        .experiment_branch
        .clone()
        .ok_or_else(|| ApiError::Internal("Campaign missing experiment_branch".to_string()))?;

    prepare_campaign_worktree(project, Path::new(&worktree_path)).await?;
    let _ = git_output(
        &project.workspace_path,
        &[
            "worktree",
            "add",
            "--detach",
            &worktree_path,
            &project.base_branch,
        ],
    )
    .await?;
    let _ = git_output(
        &worktree_path,
        &["checkout", "-B", &branch, &project.base_branch],
    )
    .await?;
    let baseline_commit = git_output(&worktree_path, &["rev-parse", "HEAD"]).await?;
    if runner.backend.is_remote() {
        push_experiment_branch(project, Path::new(&worktree_path), &branch).await?;
    }

    campaign.queue_state = ExperimentCampaignQueueState::Active;
    if campaign.started_at.is_none() {
        campaign.started_at = Some(Utc::now());
    }
    let trial_id = Uuid::new_v4();
    campaign.active_trial_id = Some(trial_id);
    campaign.status = ExperimentCampaignStatus::Running;
    campaign.pause_reason = Some("Baseline trial prepared.".to_string());
    campaign.updated_at = Utc::now();
    store
        .update_experiment_campaign(&campaign)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let trial = ExperimentTrial {
        id: trial_id,
        campaign_id: campaign.id,
        sequence: 1,
        candidate_commit: Some(baseline_commit),
        parent_best_commit: None,
        status: ExperimentTrialStatus::Preparing,
        runner_backend: runner.backend,
        exit_code: None,
        metrics_json: serde_json::json!({}),
        summary: Some("Baseline trial prepared".to_string()),
        decision_reason: None,
        log_preview_path: None,
        artifact_manifest_json: serde_json::json!({}),
        runtime_ms: None,
        attributed_cost_usd: None,
        llm_cost_usd: None,
        runner_cost_usd: None,
        hypothesis: Some("Baseline measurement for the configured benchmark.".to_string()),
        mutation_summary: None,
        reviewer_decision: Some("baseline".to_string()),
        provider_job_id: None,
        provider_job_metadata: serde_json::json!({}),
        started_at: None,
        completed_at: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    store
        .create_experiment_trial(&trial)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    launch_trial(store, user_id, _settings, project, runner, campaign, trial).await
}

async fn prepare_candidate_trial_from_worktree(
    _store: &Arc<dyn Database>,
    campaign: &ExperimentCampaign,
    project: &ExperimentProject,
    runner: &ExperimentRunnerProfile,
    trial_id: Uuid,
    sequence: u32,
    hypothesis: String,
    mutation_summary: String,
    reviewer_decision: String,
    artifact_manifest_json: serde_json::Value,
) -> ApiResult<ExperimentTrial> {
    let worktree_path = campaign
        .worktree_path
        .as_deref()
        .ok_or_else(|| ApiError::InvalidInput("Campaign has no worktree".to_string()))?;
    let changed_files = filtered_changed_files(git_changed_files(worktree_path).await?);
    if changed_files.is_empty() {
        return Err(ApiError::InvalidInput(
            "No candidate changes detected in the campaign worktree.".to_string(),
        ));
    }
    enforce_mutable_paths(&project.mutable_paths, &changed_files)?;
    git_run(
        worktree_path,
        &["add", "--"],
        changed_files
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .as_slice(),
    )
    .await?;
    let message = format!("Experiment trial {sequence}");
    let _ = git_output(worktree_path, &["commit", "-m", &message]).await?;
    let candidate_commit = git_output(worktree_path, &["rev-parse", "HEAD"]).await?;
    if runner.backend.is_remote()
        && let Some(branch) = campaign.experiment_branch.as_deref()
    {
        push_experiment_branch(project, Path::new(worktree_path), branch).await?;
    }

    Ok(ExperimentTrial {
        id: trial_id,
        campaign_id: campaign.id,
        sequence,
        candidate_commit: Some(candidate_commit),
        parent_best_commit: campaign.best_commit.clone(),
        status: ExperimentTrialStatus::Preparing,
        runner_backend: runner.backend,
        exit_code: None,
        metrics_json: serde_json::json!({}),
        summary: Some("Candidate trial prepared".to_string()),
        decision_reason: None,
        log_preview_path: None,
        artifact_manifest_json,
        runtime_ms: None,
        attributed_cost_usd: None,
        llm_cost_usd: None,
        runner_cost_usd: None,
        hypothesis: Some(hypothesis),
        mutation_summary: Some(mutation_summary),
        reviewer_decision: Some(reviewer_decision),
        provider_job_id: None,
        provider_job_metadata: serde_json::json!({}),
        started_at: None,
        completed_at: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    })
}

async fn create_experiment_trial_commit(
    store: &Arc<dyn Database>,
    campaign: &ExperimentCampaign,
    project: &ExperimentProject,
    runner: &ExperimentRunnerProfile,
) -> ApiResult<ExperimentTrial> {
    let sequence = latest_trial(store, campaign.id)
        .await?
        .map(|trial| trial.sequence + 1)
        .unwrap_or(1);
    let trial_id = Uuid::new_v4();
    let planner = run_planner_subagent(store, campaign, project, Some(trial_id)).await?;
    let mutator = run_mutator_subagent(campaign, project, &planner, Some(trial_id)).await?;
    let worktree_path = campaign
        .worktree_path
        .as_deref()
        .ok_or_else(|| ApiError::InvalidInput("Campaign has no worktree".to_string()))?;
    let changed_files = filtered_changed_files(git_changed_files(worktree_path).await?);
    if changed_files.is_empty() {
        return Err(ApiError::InvalidInput(
            "Autonomous mutator did not produce any candidate changes.".to_string(),
        ));
    }
    enforce_mutable_paths(&project.mutable_paths, &changed_files)?;
    let diff_stat = git_output(worktree_path, &["diff", "--stat", "--", "."]).await?;
    let diff_preview = git_output(worktree_path, &["diff", "--", "."]).await?;
    let reviewer = run_reviewer_subagent(
        campaign,
        project,
        &planner,
        &diff_stat,
        &diff_preview,
        Some(trial_id),
    )
    .await?;
    if !(reviewer.approved && reviewer.scope_ok && reviewer.benchmark_ready) {
        return Err(ApiError::InvalidInput(format!(
            "Reviewer rejected the autonomous candidate: {}",
            reviewer.reason
        )));
    }
    prepare_candidate_trial_from_worktree(
        store,
        campaign,
        project,
        runner,
        trial_id,
        sequence,
        planner.hypothesis,
        mutator.mutation_summary,
        reviewer.reason,
        serde_json::json!({
            "candidate_source": "autonomous_subagent",
            "changed_paths": changed_files,
            "planner_target_ids": planner.target_ids,
            "expected_metric_direction": planner.expected_metric_direction,
            "mutator_changed_paths": mutator.changed_paths,
            "workspace": worktree_path,
        }),
    )
    .await
}

async fn latest_active_lease(
    store: &Arc<dyn Database>,
    trial_id: Uuid,
) -> ApiResult<Option<ExperimentLease>> {
    let lease = store
        .get_experiment_lease_for_trial(trial_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(lease.filter(|lease| {
        matches!(
            lease.status,
            ExperimentLeaseStatus::Pending | ExperimentLeaseStatus::Claimed
        )
    }))
}

async fn revoke_lease_with_runner(
    store: &Arc<dyn Database>,
    user_id: &str,
    campaign: &ExperimentCampaign,
    lease: &ExperimentLease,
    action: RemoteLaunchAction,
) -> ApiResult<String> {
    let mut lease = lease.clone();
    lease.status = ExperimentLeaseStatus::Revoked;
    lease.completed_at = Some(Utc::now());
    lease.updated_at = Utc::now();
    store
        .update_experiment_lease(&lease)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let auth = ExperimentLeaseAuthentication {
        lease_id: lease.id,
        token: String::new(),
    };
    let message = if let Some(runner) = store
        .get_experiment_runner_profile(campaign.runner_profile_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        let trial = store
            .get_experiment_trial(lease.trial_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let provider_job_metadata = trial
            .as_ref()
            .map(|entry| entry.provider_job_metadata.clone())
            .unwrap_or_else(|| serde_json::json!({}));
        let provider_api_key = research_provider_api_key(user_id, &runner).await;
        adapters::revoke_remote_launch(
            &runner,
            &auth,
            trial
                .as_ref()
                .and_then(|entry| entry.provider_job_id.as_deref()),
            &provider_job_metadata,
            action,
            provider_api_key.as_deref(),
        )
        .await
        .map_err(ApiError::Internal)?
    } else {
        None
    };

    Ok(message.unwrap_or_else(|| "Lease revoked.".to_string()))
}

pub async fn pause_campaign(
    store: &Arc<dyn Database>,
    user_id: &str,
    campaign_id: Uuid,
) -> ApiResult<ExperimentCampaignActionResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let mut campaign = get_campaign(store, user_id, campaign_id).await?;
    campaign.status = ExperimentCampaignStatus::Paused;
    campaign.queue_state = ExperimentCampaignQueueState::NotQueued;
    campaign.pause_reason = Some("Paused by operator.".to_string());
    campaign.updated_at = Utc::now();
    store
        .update_experiment_campaign(&campaign)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let mut launch_message = None;
    if let Some(trial) = latest_trial(store, campaign.id).await?
        && let Some(lease) = latest_active_lease(store, trial.id).await?
    {
        launch_message = Some(
            revoke_lease_with_runner(store, user_id, &campaign, &lease, RemoteLaunchAction::Pause)
                .await?,
        );
    }
    maybe_launch_next_queued_after_slot_release(store, user_id).await?;
    Ok(ExperimentCampaignActionResponse {
        campaign,
        trial: None,
        lease: None,
        launch: None,
        message: launch_message.unwrap_or_else(|| "Campaign paused.".to_string()),
    })
}

pub async fn cancel_campaign(
    store: &Arc<dyn Database>,
    user_id: &str,
    campaign_id: Uuid,
) -> ApiResult<ExperimentCampaignActionResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let mut campaign = get_campaign(store, user_id, campaign_id).await?;
    campaign.status = ExperimentCampaignStatus::Cancelled;
    campaign.queue_state = ExperimentCampaignQueueState::NotQueued;
    campaign.pause_reason = Some("Cancelled by operator.".to_string());
    campaign.ended_at = Some(Utc::now());
    campaign.updated_at = Utc::now();
    store
        .update_experiment_campaign(&campaign)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let mut launch_message = None;
    if let Some(trial) = latest_trial(store, campaign.id).await?
        && let Some(lease) = latest_active_lease(store, trial.id).await?
    {
        launch_message = Some(
            revoke_lease_with_runner(
                store,
                user_id,
                &campaign,
                &lease,
                RemoteLaunchAction::Cancel,
            )
            .await?,
        );
    }
    maybe_launch_next_queued_after_slot_release(store, user_id).await?;
    Ok(ExperimentCampaignActionResponse {
        campaign,
        trial: None,
        lease: None,
        launch: None,
        message: launch_message.unwrap_or_else(|| "Campaign cancelled.".to_string()),
    })
}

pub async fn resume_campaign(
    store: &Arc<dyn Database>,
    user_id: &str,
    campaign_id: Uuid,
) -> ApiResult<ExperimentCampaignActionResponse> {
    let settings = ensure_experiments_enabled(store, user_id).await?;
    let mut campaign = get_campaign(store, user_id, campaign_id).await?;
    let project = get_project(store, user_id, campaign.project_id).await?;
    let runner = get_runner(store, user_id, campaign.runner_profile_id).await?;

    if let Some(active) = active_trial(store, campaign.id).await? {
        return Err(ApiError::InvalidInput(format!(
            "Campaign already has an active trial ({})",
            active.id
        )));
    }

    if project.autonomy_mode != ExperimentAutonomyMode::ManualCandidate {
        campaign.status = ExperimentCampaignStatus::Running;
        campaign.queue_state = ExperimentCampaignQueueState::NotQueued;
        campaign.queue_position = 0;
        campaign.pause_reason = None;
        campaign.updated_at = Utc::now();
        store
            .update_experiment_campaign(&campaign)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        launch_next_trial_if_ready(store, user_id, &settings, &project, &runner, &mut campaign)
            .await?;
        let refreshed = get_campaign(store, user_id, campaign.id).await?;
        let trial = latest_trial(store, campaign.id).await?;
        return Ok(ExperimentCampaignActionResponse {
            campaign: refreshed,
            trial,
            lease: None,
            launch: None,
            message: "Campaign resumed.".to_string(),
        });
    }

    let worktree_path = campaign
        .worktree_path
        .clone()
        .ok_or_else(|| ApiError::InvalidInput("Campaign has no worktree".to_string()))?;
    let filtered_changed_files = filtered_changed_files(git_changed_files(&worktree_path).await?);
    let sequence = latest_trial(store, campaign.id)
        .await?
        .map(|trial| trial.sequence + 1)
        .unwrap_or(1);
    let trial_id = Uuid::new_v4();
    let trial = prepare_candidate_trial_from_worktree(
        store,
        &campaign,
        &project,
        &runner,
        trial_id,
        sequence,
        "Manual candidate submitted for evaluation.".to_string(),
        format!(
            "Candidate diff staged from campaign worktree ({} changed paths).",
            filtered_changed_files.len()
        ),
        "manual_candidate".to_string(),
        serde_json::json!({
            "candidate_source": "manual_candidate",
            "changed_paths": filtered_changed_files,
            "workspace": worktree_path,
        }),
    )
    .await?;
    campaign.status = ExperimentCampaignStatus::Running;
    campaign.queue_state = ExperimentCampaignQueueState::Active;
    campaign.queue_position = 0;
    campaign.pause_reason = None;
    campaign.updated_at = Utc::now();
    store
        .update_experiment_campaign(&campaign)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    store
        .create_experiment_trial(&trial)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let response = launch_trial(
        store, user_id, &settings, &project, &runner, campaign, trial,
    )
    .await?;
    maybe_launch_next_queued_after_slot_release(store, user_id).await?;
    Ok(response)
}

pub async fn reissue_lease(
    store: &Arc<dyn Database>,
    user_id: &str,
    campaign_id: Uuid,
) -> ApiResult<ExperimentCampaignActionResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let mut campaign = get_campaign(store, user_id, campaign_id).await?;
    let project = get_project(store, user_id, campaign.project_id).await?;
    let runner = get_runner(store, user_id, campaign.runner_profile_id).await?;
    if !runner.backend.is_remote() {
        return Err(ApiError::InvalidInput(
            "Lease reissue is only supported for remote runners.".to_string(),
        ));
    }
    let mut trial = latest_trial(store, campaign.id).await?.ok_or_else(|| {
        ApiError::InvalidInput("Campaign has no trial to reissue a lease for.".to_string())
    })?;
    if matches!(
        trial.status,
        ExperimentTrialStatus::Accepted
            | ExperimentTrialStatus::Rejected
            | ExperimentTrialStatus::Crashed
            | ExperimentTrialStatus::TimedOut
            | ExperimentTrialStatus::InfraFailed
    ) {
        return Err(ApiError::InvalidInput(
            "Only in-flight remote trials can receive a new lease.".to_string(),
        ));
    }

    if let Some(lease) = latest_active_lease(store, trial.id).await? {
        let _ = revoke_lease_with_runner(
            store,
            user_id,
            &campaign,
            &lease,
            RemoteLaunchAction::Reissue,
        )
        .await?;
    }
    let lease = create_lease(store, &project, &runner, &campaign, &trial).await?;
    let provider_api_key = research_provider_api_key(user_id, &runner).await;
    let launch_outcome = adapters::try_auto_launch(
        &runner,
        campaign_gateway_url(&campaign).as_deref(),
        &lease,
        provider_api_key.as_deref(),
    )
    .await
    .unwrap_or_else(|err| RunnerLaunchOutcome {
        message: err,
        bootstrap_command: campaign_gateway_url(&campaign)
            .as_deref()
            .map(|gateway| adapters::build_bootstrap_command(gateway, &lease)),
        provider_template: None,
        provider_job_id: None,
        provider_job_metadata: serde_json::json!({}),
        auto_launched: false,
        requires_operator_action: true,
    });

    trial.status = if launch_outcome.auto_launched {
        ExperimentTrialStatus::Running
    } else {
        ExperimentTrialStatus::Preparing
    };
    if launch_outcome.auto_launched {
        trial.started_at = Some(Utc::now());
    }
    trial.summary = Some(launch_outcome.message.clone());
    trial.provider_job_id = launch_outcome.provider_job_id.clone();
    trial.provider_job_metadata = launch_outcome.provider_job_metadata.clone();
    trial.updated_at = Utc::now();
    campaign.status = ExperimentCampaignStatus::Running;
    campaign.queue_state = ExperimentCampaignQueueState::Active;
    campaign.pause_reason = Some(launch_outcome.message.clone());
    campaign.updated_at = Utc::now();
    store
        .update_experiment_trial(&trial)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    store
        .update_experiment_campaign(&campaign)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(ExperimentCampaignActionResponse {
        campaign,
        trial: Some(trial),
        lease: Some(lease),
        launch: Some(launch_details_from_outcome(launch_outcome)),
        message: "Lease reissued.".to_string(),
    })
}

pub async fn promote_campaign(
    store: &Arc<dyn Database>,
    user_id: &str,
    campaign_id: Uuid,
) -> ApiResult<ExperimentCampaignActionResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let mut campaign = get_campaign(store, user_id, campaign_id).await?;
    let project = get_project(store, user_id, campaign.project_id).await?;
    let best_commit = campaign
        .best_commit
        .clone()
        .or(campaign.baseline_commit.clone())
        .ok_or_else(|| {
            ApiError::InvalidInput("Campaign has no accepted commit to promote".to_string())
        })?;
    let promotion_branch = format!("codex/experiment-review/{}", short_id(campaign.id));
    let _ = git_output(
        &project.workspace_path,
        &["branch", "-f", &promotion_branch, &best_commit],
    )
    .await?;

    let mut message = format!("Created review branch {promotion_branch} at {best_commit}.");
    if project.promotion_mode == "branch_pr_draft" {
        let push_result = git_output(
            &project.workspace_path,
            &["push", "-u", &project.git_remote_name, &promotion_branch],
        )
        .await;
        if push_result.is_ok() {
            let title = format!("Experiment promotion: {}", project.name);
            let body = format!(
                "Promoting best commit from experiment campaign {}\n\nBest commit: {}\nPrimary metric: {}",
                campaign.id, best_commit, project.primary_metric.name
            );
            let pr_result = run_command_capture(
                Some(Path::new(&project.workspace_path)),
                "gh",
                &[
                    "pr",
                    "create",
                    "--draft",
                    "--base",
                    &project.base_branch,
                    "--head",
                    &promotion_branch,
                    "--title",
                    &title,
                    "--body",
                    &body,
                ],
                &[],
            )
            .await;
            if let Ok(output) = pr_result
                && !output.trim().is_empty()
            {
                message.push(' ');
                message.push_str(output.trim());
            }
        }
    }
    campaign.status = ExperimentCampaignStatus::AwaitingPromotion;
    campaign.pause_reason = Some(message.clone());
    campaign.updated_at = Utc::now();
    store
        .update_experiment_campaign(&campaign)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(ExperimentCampaignActionResponse {
        campaign,
        trial: None,
        lease: None,
        launch: None,
        message,
    })
}

pub async fn lease_job(
    store: &Arc<dyn Database>,
    user_id: &str,
    lease_id: Uuid,
    token: &str,
) -> ApiResult<ExperimentLeaseJobResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let mut lease = verified_lease(store, lease_id, token).await?;
    if lease.status == ExperimentLeaseStatus::Revoked {
        return Err(ApiError::Unavailable("Lease has been revoked".to_string()));
    }
    if lease.status == ExperimentLeaseStatus::Pending {
        lease.status = ExperimentLeaseStatus::Claimed;
        lease.claimed_at = Some(Utc::now());
        lease.updated_at = Utc::now();
        store
            .update_experiment_lease(&lease)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    }
    let job: ExperimentRunnerJob = serde_json::from_value(lease.job_payload.clone())
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(ExperimentLeaseJobResponse { job })
}

pub async fn lease_credentials(
    store: &Arc<dyn Database>,
    user_id: &str,
    lease_id: Uuid,
    token: &str,
) -> ApiResult<ExperimentLeaseCredentialsResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let lease = verified_lease(store, lease_id, token).await?;
    Ok(ExperimentLeaseCredentialsResponse {
        credentials: lease.credentials_payload,
    })
}

pub async fn lease_status(
    store: &Arc<dyn Database>,
    user_id: &str,
    lease_id: Uuid,
    token: &str,
    req: ExperimentLeaseStatusRequest,
) -> ApiResult<ExperimentCampaignActionResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let lease = verified_lease(store, lease_id, token).await?;
    let mut trial = store
        .get_experiment_trial(lease.trial_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| {
            ApiError::SessionNotFound(format!("Experiment trial {} not found", lease.trial_id))
        })?;
    trial.summary = Some(req.status.clone());
    trial.status = match req.status.as_str() {
        "runner_started" | "running_prepare" | "running_benchmark" => {
            ExperimentTrialStatus::Running
        }
        "evaluating" | "uploading_artifacts" | "completing" => ExperimentTrialStatus::Evaluating,
        _ => trial.status,
    };
    if matches!(
        trial.status,
        ExperimentTrialStatus::Running | ExperimentTrialStatus::Evaluating
    ) && trial.started_at.is_none()
    {
        trial.started_at = Some(Utc::now());
    }
    if let Some(metadata) = req.metadata {
        trial.artifact_manifest_json = merge_json(&trial.artifact_manifest_json, &metadata);
    }
    trial.updated_at = Utc::now();
    store
        .update_experiment_trial(&trial)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let campaign = get_campaign(store, user_id, lease.campaign_id).await?;
    Ok(ExperimentCampaignActionResponse {
        campaign,
        trial: Some(trial),
        lease: None,
        launch: None,
        message: "Lease status recorded.".to_string(),
    })
}

pub async fn lease_event(
    store: &Arc<dyn Database>,
    user_id: &str,
    lease_id: Uuid,
    token: &str,
    req: ExperimentLeaseEventRequest,
) -> ApiResult<ExperimentCampaignActionResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let lease = verified_lease(store, lease_id, token).await?;
    let mut trial = store
        .get_experiment_trial(lease.trial_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| {
            ApiError::SessionNotFound(format!("Experiment trial {} not found", lease.trial_id))
        })?;
    let mut manifest = if trial.artifact_manifest_json.is_object() {
        trial.artifact_manifest_json.clone()
    } else {
        serde_json::json!({})
    };
    let event_entry = serde_json::json!({
        "message": req.message,
        "metadata": req.metadata,
        "at": Utc::now().to_rfc3339(),
    });
    let events = manifest
        .as_object_mut()
        .expect("manifest initialized as object")
        .entry("events".to_string())
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    if let Some(array) = events.as_array_mut() {
        array.push(event_entry);
    }
    trial.artifact_manifest_json = manifest;
    trial.updated_at = Utc::now();
    store
        .update_experiment_trial(&trial)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let campaign = get_campaign(store, user_id, lease.campaign_id).await?;
    Ok(ExperimentCampaignActionResponse {
        campaign,
        trial: Some(trial),
        lease: None,
        launch: None,
        message: "Lease event recorded.".to_string(),
    })
}

pub async fn lease_artifact(
    store: &Arc<dyn Database>,
    user_id: &str,
    lease_id: Uuid,
    token: &str,
    artifact: ExperimentRunnerArtifactUpload,
) -> ApiResult<ExperimentCampaignActionResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let lease = verified_lease(store, lease_id, token).await?;
    let mut artifacts = store
        .list_experiment_artifacts(lease.trial_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    artifacts.push(ExperimentArtifactRef {
        id: Uuid::new_v4(),
        trial_id: lease.trial_id,
        kind: artifact.kind,
        uri_or_local_path: artifact.uri_or_local_path,
        size_bytes: artifact.size_bytes,
        fetchable: artifact.fetchable,
        metadata: artifact.metadata,
        created_at: Utc::now(),
    });
    store
        .replace_experiment_artifacts(lease.trial_id, &artifacts)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let campaign = get_campaign(store, user_id, lease.campaign_id).await?;
    Ok(ExperimentCampaignActionResponse {
        campaign,
        trial: None,
        lease: None,
        launch: None,
        message: "Artifact recorded.".to_string(),
    })
}

pub async fn lease_complete(
    store: &Arc<dyn Database>,
    user_id: &str,
    lease_id: Uuid,
    token: &str,
    completion: ExperimentRunnerCompletion,
) -> ApiResult<ExperimentCampaignActionResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let mut lease = verified_lease(store, lease_id, token).await?;
    let mut campaign = get_campaign(store, user_id, lease.campaign_id).await?;
    let project = get_project(store, user_id, campaign.project_id).await?;
    let mut trial = get_trial(store, user_id, lease.trial_id).await?;
    finalize_trial(store, &project, &mut campaign, &mut trial, completion).await?;
    lease.status = ExperimentLeaseStatus::Completed;
    lease.completed_at = Some(Utc::now());
    lease.updated_at = Utc::now();
    store
        .update_experiment_lease(&lease)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    maybe_launch_next_queued_after_slot_release(store, user_id).await?;
    Ok(ExperimentCampaignActionResponse {
        campaign,
        trial: Some(trial),
        lease: None,
        launch: None,
        message: "Lease completed.".to_string(),
    })
}

async fn launch_trial(
    store: &Arc<dyn Database>,
    user_id: &str,
    settings: &Settings,
    project: &ExperimentProject,
    runner: &ExperimentRunnerProfile,
    mut campaign: ExperimentCampaign,
    mut trial: ExperimentTrial,
) -> ApiResult<ExperimentCampaignActionResponse> {
    if runner.backend.is_remote() {
        let lease = create_lease(store, project, runner, &campaign, &trial).await?;
        let provider_api_key = research_provider_api_key(user_id, runner).await;
        let launch_outcome = adapters::try_auto_launch(
            runner,
            campaign_gateway_url(&campaign).as_deref(),
            &lease,
            provider_api_key.as_deref(),
        )
        .await
        .unwrap_or_else(|err| RunnerLaunchOutcome {
            message: err,
            bootstrap_command: campaign_gateway_url(&campaign)
                .as_deref()
                .map(|gateway| adapters::build_bootstrap_command(gateway, &lease)),
            provider_template: None,
            provider_job_id: None,
            provider_job_metadata: serde_json::json!({}),
            auto_launched: false,
            requires_operator_action: true,
        });
        trial.status = if launch_outcome.auto_launched {
            ExperimentTrialStatus::Running
        } else {
            ExperimentTrialStatus::Preparing
        };
        if launch_outcome.auto_launched {
            trial.started_at = Some(Utc::now());
        }
        trial.summary = Some(launch_outcome.message.clone());
        trial.provider_job_id = launch_outcome.provider_job_id.clone();
        trial.provider_job_metadata = launch_outcome.provider_job_metadata.clone();
        trial.updated_at = Utc::now();
        campaign.queue_state = ExperimentCampaignQueueState::Active;
        campaign.status = ExperimentCampaignStatus::Running;
        campaign.active_trial_id = Some(trial.id);
        campaign.started_at.get_or_insert_with(Utc::now);
        campaign.pause_reason = Some(launch_outcome.message.clone());
        campaign.updated_at = Utc::now();
        store
            .update_experiment_trial(&trial)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        store
            .update_experiment_campaign(&campaign)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        return Ok(ExperimentCampaignActionResponse {
            campaign,
            trial: Some(trial),
            lease: Some(lease),
            launch: Some(launch_details_from_outcome(launch_outcome)),
            message: "Remote trial prepared.".to_string(),
        });
    }

    if settings.experiments.max_concurrent_campaigns == 0 {
        return Err(ApiError::Unavailable(
            "experiments.max_concurrent_campaigns is set to 0".to_string(),
        ));
    }
    let completion = execute_local_trial(project, runner, &campaign, &mut trial).await?;
    finalize_trial(store, project, &mut campaign, &mut trial, completion).await?;
    Ok(ExperimentCampaignActionResponse {
        campaign,
        trial: Some(trial),
        lease: None,
        launch: None,
        message: "Local trial finished.".to_string(),
    })
}

async fn finalize_trial(
    store: &Arc<dyn Database>,
    project: &ExperimentProject,
    campaign: &mut ExperimentCampaign,
    trial: &mut ExperimentTrial,
    completion: ExperimentRunnerCompletion,
) -> ApiResult<()> {
    trial.exit_code = completion.exit_code;
    trial.metrics_json = completion.metrics_json;
    trial.summary = completion.summary;
    trial.log_preview_path = completion.log_preview_path;
    trial.artifact_manifest_json = completion.artifact_manifest_json;
    trial.completed_at = Some(Utc::now());
    trial.updated_at = Utc::now();
    campaign.active_trial_id = None;
    campaign.queue_state = ExperimentCampaignQueueState::NotQueued;
    trial.runtime_ms = completion.runtime_ms;
    if let Some(runtime_ms) = trial.runtime_ms {
        campaign.total_runtime_ms = campaign.total_runtime_ms.saturating_add(runtime_ms);
    } else if let Some(started_at) = trial.started_at {
        let runtime_ms = (trial.completed_at.unwrap_or_else(Utc::now) - started_at)
            .num_milliseconds()
            .max(0) as u64;
        campaign.total_runtime_ms = campaign.total_runtime_ms.saturating_add(runtime_ms);
        trial.runtime_ms = Some(runtime_ms);
    }
    let llm_cost = attributed_llm_cost_for_trial(store, campaign, trial).await?;
    let runner_cost = runner_cost_breakdown(trial, completion.attributed_cost_usd);
    trial.llm_cost_usd = Some(llm_cost.total_usd);
    trial.runner_cost_usd = Some(runner_cost.total_usd);
    trial.attributed_cost_usd = Some(llm_cost.total_usd + runner_cost.total_usd);
    campaign.total_llm_cost_usd += llm_cost.total_usd;
    campaign.total_runner_cost_usd += runner_cost.total_usd;
    campaign.total_cost_usd += trial.attributed_cost_usd.unwrap_or(0.0);
    trial.artifact_manifest_json = merge_json(
        &trial.artifact_manifest_json,
        &serde_json::json!({
            "cost_breakdown": {
                "total_usd": trial.attributed_cost_usd,
                "llm": llm_cost.details,
                "runner": runner_cost.details,
            }
        }),
    );
    if let Some(provider_overlay) = runner_cost.provider_metadata_overlay {
        trial.provider_job_metadata = merge_json(&trial.provider_job_metadata, &provider_overlay);
    }
    campaign.metadata = merge_json(
        &campaign.metadata,
        &serde_json::json!({
            "cost_summary": {
                "total_usd": campaign.total_cost_usd,
                "llm_usd": campaign.total_llm_cost_usd,
                "runner_usd": campaign.total_runner_cost_usd,
                "updated_at": Utc::now().to_rfc3339(),
            }
        }),
    );

    let success_exit = completion.exit_code.unwrap_or(1) == 0;
    let has_primary_metric = trial
        .metrics_json
        .get(&project.primary_metric.name)
        .and_then(|value| value.as_f64())
        .is_some();

    let mut non_improving = campaign.consecutive_non_improving_trials;

    if !success_exit {
        trial.status = ExperimentTrialStatus::Crashed;
        trial.decision_reason = Some("Benchmark command exited non-zero.".to_string());
        campaign.failure_count += 1;
    } else if !has_primary_metric {
        trial.status = ExperimentTrialStatus::InfraFailed;
        trial.decision_reason = Some(format!(
            "Primary metric '{}' was not found in the runner result.",
            project.primary_metric.name
        ));
        campaign.failure_count += 1;
    } else if campaign
        .best_metrics
        .as_object()
        .is_none_or(|map| map.is_empty())
    {
        trial.status = ExperimentTrialStatus::Accepted;
        trial.decision_reason = Some("Baseline recorded as the first best result.".to_string());
        campaign.best_commit = trial.candidate_commit.clone();
        campaign.best_metrics = trial.metrics_json.clone();
        campaign.baseline_commit = trial.candidate_commit.clone();
        non_improving = 0;
    } else if compare_metrics(
        &project.primary_metric,
        &project.comparison_policy,
        &trial.metrics_json,
        &campaign.best_metrics,
    ) == Some(true)
    {
        trial.status = ExperimentTrialStatus::Accepted;
        trial.decision_reason = Some(format!(
            "Candidate improved {}.",
            project.primary_metric.name
        ));
        campaign.best_commit = trial.candidate_commit.clone();
        campaign.best_metrics = trial.metrics_json.clone();
        non_improving = 0;
    } else {
        trial.status = ExperimentTrialStatus::Rejected;
        trial.decision_reason = Some(format!(
            "Candidate did not improve {}.",
            project.primary_metric.name
        ));
        non_improving += 1;
        if let (Some(worktree_path), Some(best_commit)) = (
            campaign.worktree_path.as_deref(),
            campaign.best_commit.as_deref(),
        ) {
            let _ = git_output(worktree_path, &["reset", "--hard", best_commit]).await;
            let _ = git_output(worktree_path, &["clean", "-fd"]).await;
        }
    }

    campaign.consecutive_non_improving_trials = non_improving;
    campaign.trial_count = campaign.trial_count.max(trial.sequence);
    campaign.updated_at = Utc::now();

    let max_trials = campaign
        .max_trials_override
        .or(project.stop_policy.max_trials);
    let plateau_limit = project
        .stop_policy
        .plateau_window
        .unwrap_or(project.stop_policy.non_improving_pause_threshold);
    let runtime_limit_reached = project
        .stop_policy
        .max_total_runtime_secs
        .is_some_and(|limit| (campaign.total_runtime_ms / 1000) >= limit);
    let cost_limit_reached = project
        .stop_policy
        .max_total_cost_usd
        .is_some_and(|limit| campaign.total_cost_usd >= limit);

    campaign.pause_reason = Some(campaign_status_message(
        campaign,
        project,
        trial,
        non_improving,
        max_trials,
        plateau_limit,
        runtime_limit_reached,
        cost_limit_reached,
    ));
    campaign.status = next_campaign_status(
        campaign,
        project,
        trial,
        non_improving,
        max_trials,
        plateau_limit,
        runtime_limit_reached,
        cost_limit_reached,
    );
    if matches!(
        campaign.status,
        ExperimentCampaignStatus::Completed
            | ExperimentCampaignStatus::Cancelled
            | ExperimentCampaignStatus::Failed
            | ExperimentCampaignStatus::AwaitingPromotion
    ) {
        campaign.ended_at = Some(Utc::now());
    }

    store
        .update_experiment_trial(trial)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    store
        .update_experiment_campaign(campaign)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(())
}

fn next_campaign_status(
    campaign: &ExperimentCampaign,
    project: &ExperimentProject,
    trial: &ExperimentTrial,
    non_improving: u32,
    max_trials: Option<u32>,
    plateau_limit: u32,
    runtime_limit_reached: bool,
    cost_limit_reached: bool,
) -> ExperimentCampaignStatus {
    if campaign.failure_count >= project.stop_policy.infra_failure_pause_threshold {
        return ExperimentCampaignStatus::Paused;
    }
    if runtime_limit_reached {
        return if campaign.best_commit.is_some() {
            ExperimentCampaignStatus::AwaitingPromotion
        } else {
            ExperimentCampaignStatus::Failed
        };
    }
    if cost_limit_reached {
        return if campaign.best_commit.is_some() {
            ExperimentCampaignStatus::AwaitingPromotion
        } else {
            ExperimentCampaignStatus::Failed
        };
    }
    if non_improving >= plateau_limit {
        return if campaign.best_commit.is_some() {
            ExperimentCampaignStatus::AwaitingPromotion
        } else {
            ExperimentCampaignStatus::Paused
        };
    }
    if let Some(max_trials) = max_trials
        && trial.sequence >= max_trials
    {
        return ExperimentCampaignStatus::AwaitingPromotion;
    }
    match trial.status {
        ExperimentTrialStatus::Accepted
        | ExperimentTrialStatus::Rejected
        | ExperimentTrialStatus::InfraFailed => ExperimentCampaignStatus::Running,
        ExperimentTrialStatus::Crashed | ExperimentTrialStatus::TimedOut => {
            ExperimentCampaignStatus::Paused
        }
        _ => ExperimentCampaignStatus::Paused,
    }
}

fn campaign_status_message(
    campaign: &ExperimentCampaign,
    project: &ExperimentProject,
    trial: &ExperimentTrial,
    non_improving: u32,
    max_trials: Option<u32>,
    plateau_limit: u32,
    runtime_limit_reached: bool,
    cost_limit_reached: bool,
) -> String {
    if campaign.failure_count >= project.stop_policy.infra_failure_pause_threshold {
        return format!(
            "Paused after {} infrastructure failures.",
            campaign.failure_count
        );
    }
    if runtime_limit_reached {
        return "Reached the campaign runtime budget. Promote the best commit when ready."
            .to_string();
    }
    if cost_limit_reached {
        return "Reached the campaign cost budget. Promote the best commit when ready.".to_string();
    }
    if non_improving >= plateau_limit {
        return format!(
            "Paused after {} consecutive non-improving trials (plateau window {}).",
            non_improving, plateau_limit
        );
    }
    if let Some(max_trials) = max_trials
        && trial.sequence >= max_trials
    {
        return format!(
            "Reached max_trials={}. Promote the best commit when ready.",
            max_trials
        );
    }
    match trial.status {
        ExperimentTrialStatus::Accepted => {
            "Trial accepted. Continue for another candidate."
                .to_string()
        }
        ExperimentTrialStatus::Rejected => {
            "Trial rejected. The worktree was reset to the best known commit and the controller can continue."
                .to_string()
        }
        ExperimentTrialStatus::Crashed => {
            "Trial crashed. Fix the benchmark or candidate, then resume.".to_string()
        }
        ExperimentTrialStatus::InfraFailed => {
            "Trial failed before a canonical metric could be extracted; the controller may continue until the failure threshold is reached.".to_string()
        }
        _ => "Trial complete.".to_string(),
    }
}

async fn create_lease(
    store: &Arc<dyn Database>,
    project: &ExperimentProject,
    runner: &ExperimentRunnerProfile,
    campaign: &ExperimentCampaign,
    trial: &ExperimentTrial,
) -> ApiResult<ExperimentLeaseAuthentication> {
    let token = format!("exp_{}_{}", short_id(campaign.id), Uuid::new_v4().simple());
    let repo_url = git_output(
        &project.workspace_path,
        &["remote", "get-url", &project.git_remote_name],
    )
    .await?;
    let git_ref = campaign
        .experiment_branch
        .clone()
        .ok_or_else(|| ApiError::Internal("Campaign missing experiment branch".to_string()))?;
    let job = ExperimentRunnerJob {
        lease_id: Uuid::new_v4(),
        trial_id: trial.id,
        campaign_id: campaign.id,
        project_id: project.id,
        runner_profile_id: runner.id,
        backend: runner.backend,
        repo_url,
        git_ref,
        workdir: project.workdir.clone(),
        prepare_command: project.prepare_command.clone(),
        run_command: project.run_command.clone(),
        primary_metric: project.primary_metric.clone(),
        secondary_metrics: project.secondary_metrics.clone(),
        env_grants: runner.env_grants.clone(),
        artifact_paths: vec!["run.log".to_string(), "summary.json".to_string()],
    };
    let lease = ExperimentLease {
        id: job.lease_id,
        campaign_id: campaign.id,
        trial_id: trial.id,
        runner_profile_id: runner.id,
        status: ExperimentLeaseStatus::Pending,
        token_hash: hash_lease_token(&token),
        job_payload: serde_json::to_value(&job).map_err(|e| ApiError::Internal(e.to_string()))?,
        credentials_payload: serde_json::json!({
            "env": runner.env_grants,
            "secret_references": runner.secret_references,
        }),
        expires_at: Utc::now() + chrono::Duration::minutes(DEFAULT_REMOTE_LEASE_MINUTES),
        claimed_at: None,
        completed_at: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    store
        .create_experiment_lease(&lease)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(ExperimentLeaseAuthentication {
        lease_id: lease.id,
        token,
    })
}

async fn verified_lease(
    store: &Arc<dyn Database>,
    lease_id: Uuid,
    token: &str,
) -> ApiResult<ExperimentLease> {
    let lease = store
        .get_experiment_lease(lease_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| {
            ApiError::SessionNotFound(format!("Experiment lease {lease_id} not found"))
        })?;
    if lease.expires_at < Utc::now() {
        return Err(ApiError::Unavailable("Lease has expired".to_string()));
    }
    if lease.token_hash != hash_lease_token(token) {
        return Err(ApiError::InvalidInput("Invalid lease token".to_string()));
    }
    Ok(lease)
}

async fn latest_trial(
    store: &Arc<dyn Database>,
    campaign_id: Uuid,
) -> ApiResult<Option<ExperimentTrial>> {
    let mut trials = store
        .list_experiment_trials(campaign_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(trials.pop())
}

async fn active_trial(
    store: &Arc<dyn Database>,
    campaign_id: Uuid,
) -> ApiResult<Option<ExperimentTrial>> {
    let trials = store
        .list_experiment_trials(campaign_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(trials.into_iter().find(|trial| {
        matches!(
            trial.status,
            ExperimentTrialStatus::Preparing
                | ExperimentTrialStatus::Running
                | ExperimentTrialStatus::Evaluating
        )
    }))
}

async fn validate_runner_profile_impl(
    user_id: &str,
    runner: &ExperimentRunnerProfile,
    settings: &Settings,
) -> (bool, String) {
    let provider_api_key = research_provider_api_key(user_id, runner).await;
    adapters::validate_runner_profile(runner, settings, provider_api_key.as_deref()).await
}

async fn prepare_campaign_worktree(
    project: &ExperimentProject,
    worktree_path: &Path,
) -> ApiResult<()> {
    if !Path::new(&project.workspace_path).exists() {
        return Err(ApiError::InvalidInput(format!(
            "Workspace path does not exist: {}",
            project.workspace_path
        )));
    }
    if worktree_path.exists() {
        tokio::fs::remove_dir_all(worktree_path)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    }
    if let Some(parent) = worktree_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    }
    Ok(())
}

async fn push_experiment_branch(
    project: &ExperimentProject,
    worktree_path: &Path,
    branch: &str,
) -> ApiResult<()> {
    let worktree = worktree_path.to_string_lossy().to_string();
    let _ = git_output(&worktree, &["push", "-u", &project.git_remote_name, branch]).await?;
    Ok(())
}

fn experiments_worktree_path(workspace_root: &str, campaign_id: Uuid) -> PathBuf {
    Path::new(workspace_root)
        .join(".thinclaw-experiments")
        .join(short_id(campaign_id))
}

fn short_id(id: Uuid) -> String {
    id.simple().to_string()[..12].to_string()
}

async fn git_changed_files(worktree_path: &str) -> ApiResult<Vec<String>> {
    let output = git_output(worktree_path, &["status", "--porcelain"]).await?;
    Ok(output
        .lines()
        .filter_map(|line| line.get(3..).map(str::trim))
        .filter(|path| !path.is_empty())
        .map(str::to_string)
        .collect())
}

fn filtered_changed_files(changed_files: Vec<String>) -> Vec<String> {
    changed_files
        .into_iter()
        .filter(|path| !path.starts_with(".thinclaw-experiments/"))
        .collect()
}

fn enforce_mutable_paths(mutable_paths: &[String], changed_files: &[String]) -> ApiResult<()> {
    for changed in changed_files {
        let allowed = mutable_paths
            .iter()
            .any(|allowed| changed == allowed || changed.starts_with(&(allowed.clone() + "/")));
        if !allowed {
            return Err(ApiError::InvalidInput(format!(
                "Changed file '{}' is outside the mutable_paths allowlist",
                changed
            )));
        }
    }
    Ok(())
}

fn truncate_for_prompt(value: &str, max_len: usize) -> String {
    if value.chars().count() <= max_len {
        return value.to_string();
    }
    let truncated: String = value.chars().take(max_len.saturating_sub(3)).collect();
    format!("{truncated}...")
}

fn research_channel_metadata(
    campaign: &ExperimentCampaign,
    trial_id: Option<Uuid>,
    role: &str,
    target_ids: &[String],
) -> serde_json::Value {
    serde_json::json!({
        "thread_id": RESEARCH_SUBAGENT_THREAD_ID,
        "reinject_result": false,
        USAGE_TRACKING_EXPERIMENT_CAMPAIGN_ID_KEY: campaign.id.to_string(),
        USAGE_TRACKING_EXPERIMENT_TRIAL_ID_KEY: trial_id.map(|value| value.to_string()),
        USAGE_TRACKING_EXPERIMENT_ROLE_KEY: role,
        USAGE_TRACKING_EXPERIMENT_TARGET_IDS_KEY: target_ids.join(","),
    })
}

fn parse_json_response<T: DeserializeOwned>(raw: &str) -> ApiResult<T> {
    let trimmed = raw.trim();
    if let Ok(value) = serde_json::from_str::<T>(trimmed) {
        return Ok(value);
    }
    if let Some(stripped) = trimmed
        .strip_prefix("```json")
        .and_then(|value| value.strip_suffix("```"))
        .map(str::trim)
        && let Ok(value) = serde_json::from_str::<T>(stripped)
    {
        return Ok(value);
    }
    if let Some(stripped) = trimmed
        .strip_prefix("```")
        .and_then(|value| value.strip_suffix("```"))
        .map(str::trim)
        && let Ok(value) = serde_json::from_str::<T>(stripped)
    {
        return Ok(value);
    }
    Err(ApiError::Internal(
        "Research subagent returned invalid JSON output.".to_string(),
    ))
}

async fn spawn_research_subagent<T: DeserializeOwned>(
    role_name: &str,
    task: String,
    system_prompt: String,
    channel_metadata: serde_json::Value,
) -> ApiResult<T> {
    let executor = research_subagent_executor().ok_or_else(|| {
        ApiError::Unavailable("Research subagent executor is not available.".to_string())
    })?;
    let (allowed_tools, allowed_skills) = research_subagent_capabilities(role_name).await?;
    let result = executor
        .spawn(
            SubagentSpawnRequest {
                name: format!("Research {role_name}"),
                task,
                system_prompt: Some(system_prompt),
                model: None,
                allowed_tools: Some(allowed_tools),
                allowed_skills,
                principal_id: Some(DEFAULT_EXPERIMENT_USER_ID.to_string()),
                actor_id: Some(DEFAULT_EXPERIMENT_USER_ID.to_string()),
                agent_workspace_id: None,
                timeout_secs: Some(300),
                wait: true,
            },
            RESEARCH_SUBAGENT_CHANNEL,
            &channel_metadata,
            DEFAULT_EXPERIMENT_USER_ID,
            None,
            Some(RESEARCH_SUBAGENT_THREAD_ID),
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !result.success {
        return Err(ApiError::Internal(
            result
                .error
                .unwrap_or_else(|| format!("Research {role_name} failed.")),
        ));
    }
    parse_json_response(&result.response)
}

async fn research_subagent_capabilities(
    role_name: &str,
) -> ApiResult<(Vec<String>, Option<Vec<String>>)> {
    let executor = research_subagent_executor().ok_or_else(|| {
        ApiError::Unavailable("Research subagent executor is not available.".to_string())
    })?;

    let mut denylist: HashSet<&'static str> =
        RESEARCH_SHARED_TOOL_DENYLIST.iter().copied().collect();
    match role_name {
        "planner" | "reviewer" => {
            denylist.extend(RESEARCH_READ_ONLY_TOOL_DENYLIST.iter().copied());
        }
        "mutator" => {
            denylist.extend(RESEARCH_MUTATOR_TOOL_DENYLIST.iter().copied());
        }
        _ => {}
    }

    let mut allowed_tools = executor.autonomous_tool_names().await;
    allowed_tools.retain(|tool_name| !denylist.contains(tool_name.as_str()));
    allowed_tools.sort();
    allowed_tools.dedup();

    let allowed_skills = executor.available_skill_names().await;
    let allowed_skills = if allowed_skills.is_empty() {
        None
    } else {
        Some(allowed_skills)
    };

    Ok((allowed_tools, allowed_skills))
}

fn recent_trial_context(trials: &[ExperimentTrial]) -> String {
    if trials.is_empty() {
        return "No prior trials yet.".to_string();
    }
    trials
        .iter()
        .rev()
        .take(5)
        .map(|trial| {
            format!(
                "Trial #{seq}: status={status:?}; hypothesis={hyp}; summary={summary}; metrics={metrics}",
                seq = trial.sequence,
                status = trial.status,
                hyp = trial.hypothesis.as_deref().unwrap_or("n/a"),
                summary = trial.summary.as_deref().unwrap_or("n/a"),
                metrics = truncate_for_prompt(&trial.metrics_json.to_string(), 500),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn run_planner_subagent(
    store: &Arc<dyn Database>,
    campaign: &ExperimentCampaign,
    project: &ExperimentProject,
    trial_id: Option<Uuid>,
) -> ApiResult<PlannerProposal> {
    let trials = store
        .list_experiment_trials(campaign.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let worktree_path = campaign
        .worktree_path
        .as_deref()
        .ok_or_else(|| ApiError::Internal("Campaign missing worktree path".to_string()))?;
    let task = format!(
        "You are planning the next experiment candidate.\n\
         Worktree: {worktree}\n\
         Preset: {:?}\n\
         Primary metric: {}\n\
         Comparator: {:?}\n\
         Mutable paths: {}\n\
         Recent trials:\n{}\n\n\
         Return JSON only with keys: hypothesis, target_ids, allowed_paths, expected_metric_direction, mutation_brief.\n\
         Keep allowed_paths within the mutable paths and prefer a single focused hypothesis.",
        project.preset,
        project.primary_metric.name,
        project.primary_metric.comparator,
        project.mutable_paths.join(", "),
        recent_trial_context(&trials),
        worktree = worktree_path,
    );
    let system_prompt = "You are the planning role for ThinClaw Research.\n\
         Read context and propose exactly one benchmarkable next mutation.\n\
         Do not edit files. Return raw JSON only."
        .to_string();
    spawn_research_subagent(
        "planner",
        task,
        system_prompt,
        research_channel_metadata(campaign, trial_id, "planner", &[]),
    )
    .await
}

async fn run_mutator_subagent(
    campaign: &ExperimentCampaign,
    project: &ExperimentProject,
    planner: &PlannerProposal,
    trial_id: Option<Uuid>,
) -> ApiResult<MutatorResult> {
    let worktree_path = campaign
        .worktree_path
        .as_deref()
        .ok_or_else(|| ApiError::Internal("Campaign missing worktree path".to_string()))?;
    let allowed_paths = if planner.allowed_paths.is_empty() {
        project.mutable_paths.clone()
    } else {
        planner.allowed_paths.clone()
    };
    let allowed_absolute_paths = allowed_paths
        .iter()
        .map(|path| {
            Path::new(worktree_path)
                .join(path)
                .to_string_lossy()
                .to_string()
        })
        .collect::<Vec<_>>();
    let task = format!(
        "Edit the experiment worktree to implement the planned mutation.\n\
         Worktree root: {worktree}\n\
         Allowed relative paths: {}\n\
         Allowed absolute paths: {}\n\
         Hypothesis: {}\n\
         Mutation brief: {}\n\n\
         Use file-editing tools to change only those files. Do not touch any other paths.\n\
         Return JSON only with keys: changed_paths, mutation_summary.",
        allowed_paths.join(", "),
        allowed_absolute_paths.join(", "),
        planner.hypothesis,
        planner.mutation_brief,
        worktree = worktree_path,
    );
    let system_prompt = "You are the mutator role for ThinClaw Research. Edit files only inside the provided worktree and allowed paths. Return raw JSON only.".to_string();
    spawn_research_subagent(
        "mutator",
        task,
        system_prompt,
        research_channel_metadata(campaign, trial_id, "mutator", &planner.target_ids),
    )
    .await
}

async fn run_reviewer_subagent(
    campaign: &ExperimentCampaign,
    project: &ExperimentProject,
    planner: &PlannerProposal,
    diff_stat: &str,
    diff_preview: &str,
    trial_id: Option<Uuid>,
) -> ApiResult<ReviewerDecision> {
    let worktree_path = campaign
        .worktree_path
        .as_deref()
        .ok_or_else(|| ApiError::Internal("Campaign missing worktree path".to_string()))?;
    let task = format!(
        "Review the prepared experiment candidate.\n\
         Worktree root: {worktree}\n\
         Mutable paths: {}\n\
         Hypothesis: {}\n\
         Mutation brief: {}\n\
         Diff stat:\n{}\n\n\
         Diff preview:\n{}\n\n\
         Approve only if the diff stays within scope and is benchmark-ready.\n\
         Return JSON only with keys: approved, scope_ok, benchmark_ready, reason.",
        project.mutable_paths.join(", "),
        planner.hypothesis,
        planner.mutation_brief,
        truncate_for_prompt(diff_stat, 4000),
        truncate_for_prompt(diff_preview, 12000),
        worktree = worktree_path,
    );
    let system_prompt = "You are the reviewer role for ThinClaw Research. Validate scope and benchmark readiness only. Return raw JSON only.".to_string();
    spawn_research_subagent(
        "reviewer",
        task,
        system_prompt,
        research_channel_metadata(campaign, trial_id, "reviewer", &planner.target_ids),
    )
    .await
}

async fn execute_local_trial(
    project: &ExperimentProject,
    runner: &ExperimentRunnerProfile,
    campaign: &ExperimentCampaign,
    trial: &mut ExperimentTrial,
) -> ApiResult<ExperimentRunnerCompletion> {
    let worktree_root = campaign
        .worktree_path
        .as_deref()
        .ok_or_else(|| ApiError::Internal("Campaign missing worktree_path".to_string()))?;
    let run_root = Path::new(worktree_root).join(&project.workdir);
    let run_root = run_root
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(&run_root));
    let started_at = std::time::Instant::now();
    let log_dir = crate::platform::resolve_data_dir("experiments").join("logs");
    tokio::fs::create_dir_all(&log_dir)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let log_path = log_dir.join(format!("{}.log", trial.id.simple()));

    trial.status = ExperimentTrialStatus::Running;
    trial.started_at = Some(Utc::now());
    trial.updated_at = Utc::now();

    let mut log = String::new();
    if let Some(prepare_command) = project.prepare_command.as_deref() {
        let output = run_shell_command(&run_root, prepare_command, runner).await?;
        log.push_str("== prepare ==\n");
        log.push_str(&output);
        log.push('\n');
    }
    let run_output = run_shell_command(&run_root, &project.run_command, runner).await?;
    let runtime_ms = (started_at.elapsed().as_nanos() / 1_000_000) as u64;
    log.push_str("== run ==\n");
    log.push_str(&run_output);
    tokio::fs::write(&log_path, &log)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let summary_path = run_root.join("summary.json");
    let summary_json = if summary_path.exists() {
        let raw = tokio::fs::read_to_string(&summary_path)
            .await
            .unwrap_or_default();
        serde_json::from_str::<serde_json::Value>(&raw).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };
    let metrics = extract_metrics(
        &project.primary_metric,
        &project.secondary_metrics,
        &log,
        &summary_json,
    );
    let exit_code = parse_exit_code(&run_output);
    Ok(ExperimentRunnerCompletion {
        exit_code,
        metrics_json: metrics,
        summary: Some(format!(
            "Local {} run completed.",
            serde_json::to_string(&runner.backend).unwrap_or_default()
        )),
        runtime_ms: Some(runtime_ms),
        attributed_cost_usd: None,
        log_preview_path: Some(log_path.to_string_lossy().to_string()),
        artifact_manifest_json: serde_json::json!({
            "summary_json_path": summary_path.to_string_lossy(),
        }),
    })
}

fn parse_exit_code(output: &str) -> Option<i32> {
    if output.contains("__THINCLAW_EXIT_CODE__:") {
        return output
            .lines()
            .find_map(|line| line.split("__THINCLAW_EXIT_CODE__:").nth(1))
            .and_then(|value| value.trim().parse::<i32>().ok());
    }
    Some(0)
}

#[cfg(target_os = "windows")]
fn shell_command_invocation(command: &str) -> (&'static str, Vec<String>) {
    (
        "cmd",
        vec![
            "/V:ON".to_string(),
            "/C".to_string(),
            format!("{command} & echo __THINCLAW_EXIT_CODE__:!ERRORLEVEL!"),
        ],
    )
}

#[cfg(not(target_os = "windows"))]
fn shell_command_invocation(command: &str) -> (&'static str, Vec<String>) {
    (
        "sh",
        vec![
            "-lc".to_string(),
            format!("{command}; printf '\\n__THINCLAW_EXIT_CODE__:%s\\n' \"$?\""),
        ],
    )
}

async fn run_shell_command(
    cwd: &Path,
    command: &str,
    runner: &ExperimentRunnerProfile,
) -> ApiResult<String> {
    let env = env_pairs_from_profile(runner);
    let (shell, args) = shell_command_invocation(command);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_command_capture(Some(cwd), shell, &arg_refs, &env).await
}

fn env_pairs_from_profile(runner: &ExperimentRunnerProfile) -> Vec<(String, String)> {
    runner
        .env_grants
        .as_object()
        .map(|map| {
            map.iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

async fn run_command_capture(
    cwd: Option<&Path>,
    binary: &str,
    args: &[&str],
    env: &[(String, String)],
) -> ApiResult<String> {
    let mut command = Command::new(binary);
    command.args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command
        .output()
        .await
        .map_err(|e| ApiError::Internal(format!("failed to run {binary}: {e}")))?;
    let mut text = String::new();
    text.push_str(&String::from_utf8_lossy(&output.stdout));
    if !output.stderr.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    Ok(text)
}

async fn git_output(cwd: &str, args: &[&str]) -> ApiResult<String> {
    let output = run_command_capture(Some(Path::new(cwd)), "git", args, &[]).await?;
    Ok(output.trim().to_string())
}

async fn git_run(cwd: &str, prefix_args: &[&str], extra_args: &[&str]) -> ApiResult<()> {
    let mut args = prefix_args.to_vec();
    args.extend_from_slice(extra_args);
    let _ = git_output(cwd, &args).await?;
    Ok(())
}

fn derive_opportunities(
    usage: &[ExperimentModelUsageRecord],
    targets: &[ExperimentTarget],
    target_links: &[ExperimentTargetLink],
) -> Vec<ExperimentOpportunity> {
    let mut opportunities_by_key: HashMap<String, OpportunityAggregate> = HashMap::new();

    for record in usage {
        let class = usage_classification(record);
        let route_key = record.route_key.clone();
        let logical_role = record.logical_role.clone();
        let candidate_kinds = candidate_kinds_for_usage(record, class, targets);

        for kind in candidate_kinds {
            let key = opportunity_key_string(
                &record.provider,
                &record.model,
                route_key.as_deref(),
                logical_role.as_deref(),
                kind,
            );
            let linked_target_id = find_linked_target_id(target_links, targets, record, kind)
                .or_else(|| find_linked_target(targets, record, kind).map(|target| target.id));
            let aggregate =
                opportunities_by_key
                    .entry(key)
                    .or_insert_with(|| OpportunityAggregate {
                        provider: record.provider.clone(),
                        model: record.model.clone(),
                        route_key: route_key.clone(),
                        logical_role: logical_role.clone(),
                        kind,
                        class,
                        call_count: 0,
                        error_count: 0,
                        latency_sum_ms: 0,
                        cost_sum_usd: 0.0,
                        first_seen: record.created_at,
                        last_seen: record.created_at,
                        linked_target_id,
                    });
            aggregate.call_count = aggregate.call_count.saturating_add(1);
            if !record.success {
                aggregate.error_count = aggregate.error_count.saturating_add(1);
            }
            aggregate.last_seen = aggregate.last_seen.max(record.created_at);
            aggregate.first_seen = aggregate.first_seen.min(record.created_at);
            if let Some(linked_target_id) = linked_target_id {
                aggregate.linked_target_id = Some(linked_target_id);
            }
            aggregate.latency_sum_ms = aggregate
                .latency_sum_ms
                .saturating_add(record.latency_ms.unwrap_or_default());
            aggregate.cost_sum_usd += record.cost_usd.unwrap_or(0.0);
        }
    }

    let mut aggregates: Vec<_> = opportunities_by_key.into_values().collect();
    aggregates.sort_by(|left, right| {
        aggregate_opportunity_score(right)
            .partial_cmp(&aggregate_opportunity_score(left))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.call_count.cmp(&right.call_count).reverse())
            .then_with(|| left.provider.cmp(&right.provider))
            .then_with(|| left.model.cmp(&right.model))
    });

    let mut opportunities = Vec::with_capacity(aggregates.len());
    for aggregate in aggregates {
        let self_hosted = aggregate.class != UsageClass::Hosted;
        let avg_latency_ms = if aggregate.call_count == 0 {
            None
        } else {
            Some(aggregate.latency_sum_ms as f64 / aggregate.call_count as f64)
        };
        let avg_cost_usd = if aggregate.call_count == 0 {
            None
        } else {
            Some(aggregate.cost_sum_usd / aggregate.call_count as f64)
        };
        let error_rate = if aggregate.call_count == 0 {
            0.0
        } else {
            aggregate.error_count as f64 / aggregate.call_count as f64
        };
        let key = opportunity_key_string(
            &aggregate.provider,
            &aggregate.model,
            aggregate.route_key.as_deref(),
            aggregate.logical_role.as_deref(),
            aggregate.kind,
        );
        let hash = blake3::hash(key.as_bytes()).to_hex().to_string();
        let rank_score = aggregate_opportunity_score(&aggregate);
        let route_key = aggregate.route_key.clone();
        let logical_role = aggregate.logical_role.clone();
        let signals =
            opportunity_signals_for_usage(&aggregate, error_rate, avg_latency_ms, avg_cost_usd);
        let summary = opportunity_summary(
            aggregate.kind,
            aggregate.provider.as_str(),
            aggregate.model.as_str(),
            route_key.as_deref(),
            logical_role.as_deref(),
            self_hosted,
        );
        opportunities.push(ExperimentOpportunity {
            id: format!("opp_{}", &hash[..16]),
            provider: aggregate.provider,
            model: aggregate.model,
            route_key: route_key.clone(),
            logical_role,
            opportunity_type: aggregate.kind,
            summary,
            gpu_requirement: opportunity_gpu_requirement(aggregate.kind, self_hosted),
            suggested_preset: opportunity_preset(aggregate.kind, self_hosted),
            linked_target_id: aggregate.linked_target_id,
            source: Some("telemetry".to_string()),
            confidence: Some((0.4 + (aggregate.call_count.min(8) as f64 * 0.05)).clamp(0.4, 0.9)),
            signals,
            project_hint: None,
            metadata: serde_json::json!({
                "usage_class": format!("{:?}", aggregate.class),
                "call_count": aggregate.call_count,
                "error_count": aggregate.error_count,
                "error_rate": error_rate,
                "avg_latency_ms": avg_latency_ms,
                "avg_cost_usd": avg_cost_usd,
                "rank_score": rank_score,
                "linked_target": aggregate.linked_target_id.is_some(),
                "route_key": route_key,
            }),
            created_at: aggregate.first_seen,
            updated_at: aggregate.last_seen,
        });
    }

    opportunities
}

#[derive(Debug, Clone)]
struct OutcomeOpportunityAggregate {
    kind: ExperimentTargetKind,
    contract_type: String,
    artifact_type: Option<String>,
    artifact_name: Option<String>,
    routine_id: Option<String>,
    routine_name: Option<String>,
    pattern_key: String,
    count: u32,
    first_seen: DateTime<Utc>,
    last_seen: DateTime<Utc>,
    rank_score: f64,
    linked_target_id: Option<Uuid>,
}

fn derive_outcome_opportunities(
    contracts: &[OutcomeContract],
    targets: &[ExperimentTarget],
    limit: usize,
) -> Vec<ExperimentOpportunity> {
    let cutoff = Utc::now() - chrono::Duration::days(30);
    let mut aggregates: HashMap<String, OutcomeOpportunityAggregate> = HashMap::new();

    for contract in contracts.iter().filter(|contract| {
        contract.final_verdict.as_deref() == Some("negative")
            && contract.evaluated_at.unwrap_or(contract.updated_at) >= cutoff
    }) {
        let Some(kind) = outcome_target_kind(contract) else {
            continue;
        };
        let pattern_key = contract
            .metadata
            .get("pattern_key")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| {
                format!(
                    "{}:{}:{}",
                    contract.contract_type, contract.source_kind, contract.source_id
                )
            });
        let artifact_type = contract
            .metadata
            .get("artifact_type")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let artifact_name = contract
            .metadata
            .get("artifact_name")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or_else(|| outcome_default_artifact_name(contract));
        let routine_id = contract
            .metadata
            .get("routine_id")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let routine_name = contract
            .metadata
            .get("routine_name")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let linked_target_id = find_outcome_linked_target_id(
            targets,
            kind,
            artifact_name.as_deref(),
            routine_id.as_deref(),
            &pattern_key,
        );
        let evaluated_at = contract.evaluated_at.unwrap_or(contract.updated_at);
        let aggregate = aggregates
            .entry(pattern_key.clone())
            .or_insert_with(|| OutcomeOpportunityAggregate {
                kind,
                contract_type: contract.contract_type.clone(),
                artifact_type: artifact_type.clone(),
                artifact_name: artifact_name.clone(),
                routine_id: routine_id.clone(),
                routine_name: routine_name.clone(),
                pattern_key: pattern_key.clone(),
                count: 0,
                first_seen: evaluated_at,
                last_seen: evaluated_at,
                rank_score: 0.0,
                linked_target_id,
            });
        aggregate.count = aggregate.count.saturating_add(1);
        aggregate.first_seen = aggregate.first_seen.min(evaluated_at);
        aggregate.last_seen = aggregate.last_seen.max(evaluated_at);
        if aggregate.linked_target_id.is_none() {
            aggregate.linked_target_id = linked_target_id;
        }
    }

    let mut aggregates: Vec<_> = aggregates.into_values().collect();
    for aggregate in &mut aggregates {
        let recency_bonus = ((Utc::now() - aggregate.last_seen).num_days().max(0) as f64).min(14.0);
        aggregate.rank_score = aggregate.count as f64 * 4.0 - recency_bonus;
    }
    aggregates.sort_by(|left, right| {
        right
            .rank_score
            .partial_cmp(&left.rank_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                experiment_target_kind_sort_key(left.kind)
                    .cmp(&experiment_target_kind_sort_key(right.kind))
            })
            .then_with(|| left.pattern_key.cmp(&right.pattern_key))
    });

    aggregates
        .into_iter()
        .take(limit.max(1))
        .map(|aggregate| outcome_aggregate_to_opportunity(aggregate))
        .collect()
}

fn outcome_target_kind(contract: &OutcomeContract) -> Option<ExperimentTargetKind> {
    match contract.contract_type.as_str() {
        "turn_usefulness" => Some(ExperimentTargetKind::PromptAsset),
        "routine_usefulness" => Some(ExperimentTargetKind::ToolPolicy),
        "tool_durability" => match contract
            .metadata
            .get("artifact_type")
            .and_then(|value| value.as_str())
        {
            Some("prompt") => Some(ExperimentTargetKind::PromptAsset),
            Some("skill") | Some("routine") => Some(ExperimentTargetKind::ToolPolicy),
            Some("parser") => Some(ExperimentTargetKind::Parser),
            Some("evaluator") => Some(ExperimentTargetKind::Evaluator),
            Some("inference") => Some(ExperimentTargetKind::InferenceConfig),
            Some("serving") => Some(ExperimentTargetKind::ServingConfig),
            Some("training") => Some(ExperimentTargetKind::TrainingConfig),
            Some("training_code") | Some("code") => Some(ExperimentTargetKind::TrainingCode),
            _ if contract.source_kind == "learning_code_proposal" => {
                Some(ExperimentTargetKind::TrainingCode)
            }
            _ => None,
        },
        _ => None,
    }
}

fn outcome_default_artifact_name(contract: &OutcomeContract) -> Option<String> {
    match contract.contract_type.as_str() {
        "turn_usefulness" => Some(crate::workspace::paths::USER.to_string()),
        _ => None,
    }
}

fn find_outcome_linked_target_id(
    targets: &[ExperimentTarget],
    kind: ExperimentTargetKind,
    artifact_name: Option<&str>,
    routine_id: Option<&str>,
    pattern_key: &str,
) -> Option<Uuid> {
    targets
        .iter()
        .find(|target| {
            if target.kind != kind {
                return false;
            }
            let asset_id = target
                .metadata
                .get("asset_id")
                .and_then(|value| value.as_str());
            let target_pattern = target
                .metadata
                .get("pattern_key")
                .and_then(|value| value.as_str());
            asset_id.zip(artifact_name).is_some_and(|(left, right)| left == right)
                || asset_id.zip(routine_id).is_some_and(|(left, right)| left == right)
                || target_pattern.is_some_and(|value| value == pattern_key)
        })
        .map(|target| target.id)
}

fn outcome_aggregate_to_opportunity(aggregate: OutcomeOpportunityAggregate) -> ExperimentOpportunity {
    let (summary, project_hint) = outcome_summary_and_project_hint(&aggregate);
    let id_source = format!("{}|{:?}", aggregate.pattern_key, aggregate.kind);
    let hash = blake3::hash(id_source.as_bytes()).to_hex().to_string();
    let signals = outcome_signals(&aggregate);
    ExperimentOpportunity {
        id: format!("opp_outcome_{}", &hash[..16]),
        provider: "outcome_learning".to_string(),
        model: aggregate
            .artifact_name
            .clone()
            .or_else(|| aggregate.routine_name.clone())
            .unwrap_or_else(|| "negative pattern".to_string()),
        route_key: None,
        logical_role: None,
        opportunity_type: aggregate.kind,
        summary,
        gpu_requirement: outcome_gpu_requirement(aggregate.kind),
        suggested_preset: outcome_preset(aggregate.kind),
        linked_target_id: aggregate.linked_target_id,
        source: Some("outcome_learning".to_string()),
        confidence: Some((0.45 + aggregate.count.min(5) as f64 * 0.1).clamp(0.45, 0.95)),
        signals,
        project_hint: Some(project_hint),
        metadata: serde_json::json!({
            "rank_score": aggregate.rank_score,
            "negative_outcome_count": aggregate.count,
            "pattern_key": aggregate.pattern_key,
            "contract_type": aggregate.contract_type,
            "artifact_type": aggregate.artifact_type,
            "artifact_name": aggregate.artifact_name,
            "routine_id": aggregate.routine_id,
            "routine_name": aggregate.routine_name,
        }),
        created_at: aggregate.first_seen,
        updated_at: aggregate.last_seen,
    }
}

fn outcome_summary_and_project_hint(
    aggregate: &OutcomeOpportunityAggregate,
) -> (String, serde_json::Value) {
    match aggregate.kind {
        ExperimentTargetKind::PromptAsset => {
            let target = aggregate
                .artifact_name
                .clone()
                .unwrap_or_else(|| crate::workspace::paths::USER.to_string());
            (
                format!(
                    "Use repeated negative outcome signals to benchmark and improve prompt behavior for {}.",
                    target
                ),
                serde_json::json!({
                    "name": format!("Outcome prompt benchmark for {}", target),
                    "mutable_paths": [target],
                    "fixed_paths": ["README.md"],
                    "metric_name": "outcome_success_rate",
                    "comparator": "higher_is_better",
                    "strategy": "Use the repeated negative outcome pattern as a benchmark seed, improve the prompt surface conservatively, and compare against the current baseline."
                }),
            )
        }
        ExperimentTargetKind::ToolPolicy => {
            let label = aggregate
                .routine_name
                .clone()
                .or_else(|| aggregate.artifact_name.clone())
                .unwrap_or_else(|| "tool orchestration".to_string());
            (
                format!(
                    "Investigate repeated negative outcome signals around {} and refine orchestration or notification policy.",
                    label
                ),
                serde_json::json!({
                    "name": format!("Outcome orchestration benchmark for {}", label),
                    "mutable_paths": ["src/agent/routine_engine.rs", "src/agent/outcomes.rs"],
                    "fixed_paths": ["README.md"],
                    "metric_name": "negative_outcome_rate",
                    "comparator": "lower_is_better",
                    "strategy": "Reduce repeated negative outcome patterns without broadening scope, and keep operator-facing behavior benchmarkable."
                }),
            )
        }
        ExperimentTargetKind::TrainingCode => (
            "Promote repeated negative durability signals into a benchmarked code-improvement search.".to_string(),
            serde_json::json!({
                "name": "Outcome-driven code benchmark",
                "mutable_paths": aggregate.artifact_name.clone().map(|value| vec![value]).unwrap_or_default(),
                "fixed_paths": ["README.md"],
                "metric_name": "regression_rate",
                "comparator": "lower_is_better",
                "strategy": "Use repeated negative durability outcomes as the seed benchmark and only mutate the code surface implicated by the pattern."
            }),
        ),
        kind => (
            format!(
                "Use repeated negative outcome signals to drive a focused {:?} benchmark.",
                kind
            ),
            serde_json::json!({
                "name": format!("Outcome-driven {:?} benchmark", kind),
                "mutable_paths": [],
                "fixed_paths": ["README.md"],
                "metric_name": "outcome_success_rate",
                "comparator": "higher_is_better",
                "strategy": "Turn repeated negative outcome evidence into a repeatable benchmark and search only the target surface."
            }),
        ),
    }
}

fn outcome_signals(aggregate: &OutcomeOpportunityAggregate) -> Vec<String> {
    let mut signals = vec![
        "outcome-backed evidence".to_string(),
        format!("{} negative outcome{}", aggregate.count, if aggregate.count == 1 { "" } else { "s" }),
    ];
    if let Some(artifact_name) = aggregate.artifact_name.as_deref() {
        signals.push(format!("target {}", artifact_name));
    }
    if let Some(routine_name) = aggregate.routine_name.as_deref() {
        signals.push(format!("routine {}", routine_name));
    }
    signals
}

fn outcome_gpu_requirement(kind: ExperimentTargetKind) -> ExperimentGpuRequirement {
    match kind {
        ExperimentTargetKind::TrainingCode
        | ExperimentTargetKind::TrainingConfig => ExperimentGpuRequirement::Required,
        ExperimentTargetKind::InferenceConfig
        | ExperimentTargetKind::ServingConfig => ExperimentGpuRequirement::Recommended,
        _ => ExperimentGpuRequirement::NotNeeded,
    }
}

fn outcome_preset(kind: ExperimentTargetKind) -> ExperimentPreset {
    match kind {
        ExperimentTargetKind::PromptAsset | ExperimentTargetKind::RoutingPolicy => {
            ExperimentPreset::HostedPromptRouting
        }
        ExperimentTargetKind::RagConfig => ExperimentPreset::RagPipeline,
        ExperimentTargetKind::ToolPolicy => ExperimentPreset::ToolOrchestration,
        ExperimentTargetKind::InferenceConfig | ExperimentTargetKind::ServingConfig => {
            ExperimentPreset::OpenWeightsInferenceTuning
        }
        ExperimentTargetKind::TrainingConfig => ExperimentPreset::SelfHostedFinetune,
        ExperimentTargetKind::TrainingCode => ExperimentPreset::OpenWeightsTrainingCode,
        ExperimentTargetKind::Evaluator | ExperimentTargetKind::Parser => {
            ExperimentPreset::AutoresearchSingleFile
        }
    }
}

fn experiment_target_kind_sort_key(kind: ExperimentTargetKind) -> &'static str {
    match kind {
        ExperimentTargetKind::PromptAsset => "prompt_asset",
        ExperimentTargetKind::RoutingPolicy => "routing_policy",
        ExperimentTargetKind::RagConfig => "rag_config",
        ExperimentTargetKind::ToolPolicy => "tool_policy",
        ExperimentTargetKind::Evaluator => "evaluator",
        ExperimentTargetKind::Parser => "parser",
        ExperimentTargetKind::InferenceConfig => "inference_config",
        ExperimentTargetKind::TrainingConfig => "training_config",
        ExperimentTargetKind::TrainingCode => "training_code",
        ExperimentTargetKind::ServingConfig => "serving_config",
    }
}

fn opportunity_signals_for_usage(
    aggregate: &OpportunityAggregate,
    error_rate: f64,
    avg_latency_ms: Option<f64>,
    avg_cost_usd: Option<f64>,
) -> Vec<String> {
    let mut signals = vec![format!("{} model call{}", aggregate.call_count, if aggregate.call_count == 1 { "" } else { "s" })];
    if error_rate > 0.0 {
        signals.push(format!("{:.0}% error rate", error_rate * 100.0));
    }
    if let Some(avg_latency_ms) = avg_latency_ms {
        signals.push(format!("{:.0} ms avg latency", avg_latency_ms));
    }
    if let Some(avg_cost_usd) = avg_cost_usd {
        signals.push(format!("${:.4} avg cost", avg_cost_usd));
    }
    signals
}

fn sort_experiment_opportunities(opportunities: &mut [ExperimentOpportunity]) {
    opportunities.sort_by(|left, right| {
        let right_score = right
            .metadata
            .get("rank_score")
            .and_then(|value| value.as_f64())
            .unwrap_or_default();
        let left_score = left
            .metadata
            .get("rank_score")
            .and_then(|value| value.as_f64())
            .unwrap_or_default();
        right_score
            .partial_cmp(&left_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| right.updated_at.cmp(&left.updated_at))
            .then_with(|| left.id.cmp(&right.id))
    });
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UsageClass {
    Hosted,
    SelfHosted,
    CustomHostedOrSelf,
}

impl std::fmt::Debug for UsageClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Hosted => f.write_str("hosted"),
            Self::SelfHosted => f.write_str("self_hosted"),
            Self::CustomHostedOrSelf => f.write_str("custom_hosted_or_self_hosted"),
        }
    }
}

fn usage_classification(record: &ExperimentModelUsageRecord) -> UsageClass {
    let provider = record.provider.to_ascii_lowercase();
    let endpoint_type = record
        .endpoint_type
        .clone()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let base_url = metadata_string(&record.metadata, "base_url")
        .unwrap_or_default()
        .to_ascii_lowercase();

    if is_known_hosted_provider(&provider) {
        return UsageClass::Hosted;
    }

    if is_known_self_hosted_provider(&provider)
        || endpoint_type.contains("local")
        || endpoint_type.contains("self")
        || endpoint_type.contains("cluster")
        || endpoint_type.contains("private")
        || base_url.contains("localhost")
        || base_url.contains("127.0.0.1")
        || base_url.contains("0.0.0.0")
    {
        return UsageClass::SelfHosted;
    }

    if endpoint_type.contains("openai-compatible")
        || metadata_bool(&record.metadata, "openai_compatible")
        || metadata_bool(&record.metadata, "openai_compatible_or_self_hosted")
    {
        return UsageClass::CustomHostedOrSelf;
    }

    UsageClass::CustomHostedOrSelf
}

fn is_known_hosted_provider(provider: &str) -> bool {
    const KNOWN_HOSTED: &[&str] = &[
        "openai",
        "anthropic",
        "gemini",
        "google",
        "cohere",
        "mistral",
        "azure",
        "perplexity",
        "xai",
        "deepseek",
        "groq",
    ];
    let provider = provider.to_ascii_lowercase();
    KNOWN_HOSTED
        .iter()
        .any(|name| provider == *name || provider.contains(name))
}

fn is_known_self_hosted_provider(provider: &str) -> bool {
    const SELF_HOSTED: &[&str] = &[
        "ollama",
        "lmstudio",
        "vllm",
        "llama_cpp",
        "llama-cpp",
        "llamacpp",
        "localai",
        "tgi",
    ];
    let provider = provider.to_ascii_lowercase();
    SELF_HOSTED
        .iter()
        .any(|name| provider == *name || provider.contains(name))
}

fn metadata_string(metadata: &serde_json::Value, key: &str) -> Option<String> {
    metadata
        .get(key)
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
        .filter(|value| !value.trim().is_empty())
}

fn metadata_bool(metadata: &serde_json::Value, key: &str) -> bool {
    metadata
        .get(key)
        .and_then(|value| value.as_bool())
        .unwrap_or_default()
}

fn candidate_kinds_for_usage(
    record: &ExperimentModelUsageRecord,
    class: UsageClass,
    targets: &[ExperimentTarget],
) -> Vec<ExperimentTargetKind> {
    let mut kinds = Vec::new();

    if record.route_key.is_some() || record.logical_role.is_some() {
        kinds.push(ExperimentTargetKind::RoutingPolicy);
    }
    if !record.prompt_asset_ids.is_empty() || record.route_key.is_some() {
        kinds.push(ExperimentTargetKind::PromptAsset);
    }
    if !record.retrieval_asset_ids.is_empty() {
        kinds.push(ExperimentTargetKind::RagConfig);
    }
    if !record.tool_policy_ids.is_empty() {
        kinds.push(ExperimentTargetKind::ToolPolicy);
    }
    if !record.success
        || record
            .workload_tag
            .as_deref()
            .map(|value| {
                let value = value.to_ascii_lowercase();
                value.contains("parse")
                    || value.contains("json")
                    || value.contains("structured")
                    || value.contains("extract")
            })
            .unwrap_or(false)
    {
        kinds.push(ExperimentTargetKind::Parser);
    }
    if !record.parser_ids.is_empty() {
        kinds.push(ExperimentTargetKind::Parser);
    }
    if record
        .workload_tag
        .as_deref()
        .map(|value| {
            let value = value.to_ascii_lowercase();
            value.contains("eval") || value.contains("judge") || value.contains("score")
        })
        .unwrap_or(false)
    {
        kinds.push(ExperimentTargetKind::Evaluator);
    }
    if !record.evaluator_ids.is_empty() {
        kinds.push(ExperimentTargetKind::Evaluator);
    }

    match class {
        UsageClass::Hosted => {}
        UsageClass::SelfHosted => {
            kinds.extend([
                ExperimentTargetKind::InferenceConfig,
                ExperimentTargetKind::ServingConfig,
                ExperimentTargetKind::TrainingConfig,
                ExperimentTargetKind::TrainingCode,
            ]);
        }
        UsageClass::CustomHostedOrSelf => {
            kinds.extend(
                [
                    ExperimentTargetKind::InferenceConfig,
                    ExperimentTargetKind::ServingConfig,
                    ExperimentTargetKind::TrainingConfig,
                    ExperimentTargetKind::TrainingCode,
                ]
                .into_iter()
                .filter(|kind| find_linked_target(targets, record, *kind).is_some()),
            );
        }
    }

    if kinds.is_empty() {
        kinds.push(ExperimentTargetKind::PromptAsset);
    }

    kinds.sort_by_key(|kind| *kind as u8);
    kinds.dedup();
    kinds
}

fn opportunity_key_string(
    provider: &str,
    model: &str,
    route_key: Option<&str>,
    logical_role: Option<&str>,
    kind: ExperimentTargetKind,
) -> String {
    format!(
        "{provider}|{model}|{}|{}|{:?}",
        route_key.unwrap_or(""),
        logical_role.unwrap_or(""),
        kind,
    )
}

fn aggregate_opportunity_score(aggregate: &OpportunityAggregate) -> f64 {
    if aggregate.call_count == 0 {
        return 0.0;
    }
    let error_rate = aggregate.error_count as f64 / aggregate.call_count as f64;
    let avg_latency = aggregate.latency_sum_ms as f64 / aggregate.call_count as f64;
    let avg_cost = aggregate.cost_sum_usd / aggregate.call_count as f64;
    let missing_link_penalty = if aggregate.linked_target_id.is_none()
        && matches!(
            aggregate.kind,
            ExperimentTargetKind::InferenceConfig
                | ExperimentTargetKind::ServingConfig
                | ExperimentTargetKind::TrainingConfig
                | ExperimentTargetKind::TrainingCode,
        ) {
        1.25
    } else {
        0.0
    };
    let gpu_penalty = if matches!(aggregate.kind, ExperimentTargetKind::TrainingCode) {
        -2.0
    } else if matches!(
        aggregate.kind,
        ExperimentTargetKind::InferenceConfig | ExperimentTargetKind::ServingConfig
    ) {
        -1.0
    } else {
        0.0
    };
    aggregate.call_count as f64 * 2.0
        - (error_rate * 100.0)
        - (avg_latency.min(4000.0) / 60.0)
        - avg_cost
        + gpu_penalty
        - missing_link_penalty
}

fn find_linked_target<'a>(
    targets: &'a [ExperimentTarget],
    record: &ExperimentModelUsageRecord,
    kind: ExperimentTargetKind,
) -> Option<&'a ExperimentTarget> {
    targets.iter().find(|target| {
        if target.kind != kind {
            return false;
        }
        let provider_match = target
            .metadata
            .get("provider")
            .and_then(|value| value.as_str())
            .map(|value| value.eq_ignore_ascii_case(&record.provider))
            .unwrap_or(false);
        let model_match = target
            .metadata
            .get("model")
            .and_then(|value| value.as_str())
            .map(|value| value.eq_ignore_ascii_case(&record.model))
            .unwrap_or(false);
        let route_match = target
            .metadata
            .get("route_key")
            .and_then(|value| value.as_str())
            .zip(record.route_key.as_deref())
            .map(|(left, right)| left == right)
            .unwrap_or(false);
        let asset_id_match = target
            .metadata
            .get("asset_id")
            .and_then(|value| value.as_str())
            .map(|asset_id| {
                record.prompt_asset_ids.iter().any(|id| id == asset_id)
                    || record.retrieval_asset_ids.iter().any(|id| id == asset_id)
                    || record.tool_policy_ids.iter().any(|id| id == asset_id)
            })
            .unwrap_or(false);
        provider_match || model_match || route_match || asset_id_match
    })
}

fn find_linked_target_id(
    target_links: &[ExperimentTargetLink],
    targets: &[ExperimentTarget],
    record: &ExperimentModelUsageRecord,
    kind: ExperimentTargetKind,
) -> Option<Uuid> {
    let route_key = record.route_key.as_deref().unwrap_or_default();
    let logical_role = record.logical_role.as_deref().unwrap_or_default();

    target_links
        .iter()
        .find(|link| {
            link.kind == kind
                && link.provider.eq_ignore_ascii_case(&record.provider)
                && link.model.eq_ignore_ascii_case(&record.model)
                && link.route_key.as_deref().unwrap_or_default() == route_key
                && link.logical_role.as_deref().unwrap_or_default() == logical_role
                && targets
                    .iter()
                    .any(|target| target.id == link.target_id && target.kind == kind)
        })
        .map(|link| link.target_id)
}

fn opportunity_summary(
    kind: ExperimentTargetKind,
    provider: &str,
    model: &str,
    route_key: Option<&str>,
    logical_role: Option<&str>,
    self_hosted: bool,
) -> String {
    match kind {
        ExperimentTargetKind::PromptAsset => format!(
            "Optimize prompts and system instructions for {} on {}.",
            model, provider
        ),
        ExperimentTargetKind::RoutingPolicy => format!(
            "Tune routing and fallback policy for {} on {} (route: {}, role: {}).",
            model,
            provider,
            route_key.unwrap_or("default route"),
            logical_role.unwrap_or("default role")
        ),
        ExperimentTargetKind::RagConfig => format!(
            "Improve retrieval and ranking for {} on {}.",
            model, provider
        ),
        ExperimentTargetKind::ToolPolicy => format!(
            "Refine tool selection and execution policy around {} on {}.",
            model, provider
        ),
        ExperimentTargetKind::InferenceConfig => format!(
            "Tune inference parameters for self-hosted model {} on {}.",
            model, provider
        ),
        ExperimentTargetKind::ServingConfig => format!(
            "Adjust serving/runtime settings for self-hosted model {} on {}.",
            model, provider
        ),
        ExperimentTargetKind::TrainingConfig => format!(
            "Benchmark fine-tuning or training configuration for {} on {}.",
            model, provider
        ),
        ExperimentTargetKind::TrainingCode => format!(
            "Improve training code or benchmark harness for {} on {}.",
            model, provider
        ),
        ExperimentTargetKind::Evaluator | ExperimentTargetKind::Parser => {
            if self_hosted {
                format!(
                    "Improve evaluator and parsing reliability around {} on {}.",
                    model, provider
                )
            } else {
                format!(
                    "Tighten evaluator and output parsing around {} on {}.",
                    model, provider
                )
            }
        }
    }
}

fn opportunity_gpu_requirement(
    kind: ExperimentTargetKind,
    self_hosted: bool,
) -> ExperimentGpuRequirement {
    if !self_hosted {
        return ExperimentGpuRequirement::NotNeeded;
    }
    match kind {
        ExperimentTargetKind::TrainingConfig | ExperimentTargetKind::TrainingCode => {
            ExperimentGpuRequirement::Required
        }
        ExperimentTargetKind::InferenceConfig | ExperimentTargetKind::ServingConfig => {
            ExperimentGpuRequirement::Recommended
        }
        _ => ExperimentGpuRequirement::NotNeeded,
    }
}

fn opportunity_preset(kind: ExperimentTargetKind, self_hosted: bool) -> ExperimentPreset {
    match kind {
        ExperimentTargetKind::PromptAsset | ExperimentTargetKind::RoutingPolicy => {
            ExperimentPreset::HostedPromptRouting
        }
        ExperimentTargetKind::RagConfig => ExperimentPreset::RagPipeline,
        ExperimentTargetKind::ToolPolicy
        | ExperimentTargetKind::Evaluator
        | ExperimentTargetKind::Parser => ExperimentPreset::ToolOrchestration,
        ExperimentTargetKind::InferenceConfig | ExperimentTargetKind::ServingConfig => {
            ExperimentPreset::OpenWeightsInferenceTuning
        }
        ExperimentTargetKind::TrainingConfig => ExperimentPreset::SelfHostedFinetune,
        ExperimentTargetKind::TrainingCode => {
            if self_hosted {
                ExperimentPreset::OpenWeightsTrainingCode
            } else {
                ExperimentPreset::AutoresearchSingleFile
            }
        }
    }
}

fn merge_json(base: &serde_json::Value, overlay: &serde_json::Value) -> serde_json::Value {
    match (base, overlay) {
        (serde_json::Value::Object(base), serde_json::Value::Object(overlay)) => {
            let mut merged = base.clone();
            for (key, value) in overlay {
                let next = merged
                    .get(key)
                    .map(|existing| merge_json(existing, value))
                    .unwrap_or_else(|| value.clone());
                merged.insert(key.clone(), next);
            }
            serde_json::Value::Object(merged)
        }
        (_, overlay) => overlay.clone(),
    }
}

#[derive(Debug, Clone)]
struct LlmCostAttribution {
    total_usd: f64,
    details: serde_json::Value,
}

#[derive(Debug, Clone)]
struct RunnerCostBreakdown {
    total_usd: f64,
    details: serde_json::Value,
    provider_metadata_overlay: Option<serde_json::Value>,
}

async fn attributed_llm_cost_for_trial(
    store: &Arc<dyn Database>,
    campaign: &ExperimentCampaign,
    trial: &ExperimentTrial,
) -> ApiResult<LlmCostAttribution> {
    let exact = store
        .list_experiment_model_usage_for_trial(trial.id, 2_000)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !exact.is_empty() {
        return Ok(summarize_llm_usage(&exact, "trial_id"));
    }

    let campaign_records = store
        .list_experiment_model_usage_for_campaign(campaign.id, 5_000)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let trials = store
        .list_experiment_trials(campaign.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let lower_bound = trials
        .iter()
        .filter(|candidate| candidate.sequence < trial.sequence)
        .max_by_key(|candidate| candidate.sequence)
        .map(|candidate| {
            candidate
                .completed_at
                .or(candidate.started_at)
                .unwrap_or(candidate.created_at)
        })
        .unwrap_or(campaign.created_at);
    let fallback = campaign_records
        .into_iter()
        .filter(|record| {
            metadata_string_field(&record.metadata, "experiment_trial_id").is_none()
                && record.created_at >= lower_bound
                && record.created_at <= trial.created_at
        })
        .collect::<Vec<_>>();
    Ok(summarize_llm_usage(&fallback, "campaign_window"))
}

fn summarize_llm_usage(records: &[ExperimentModelUsageRecord], source: &str) -> LlmCostAttribution {
    let mut total_usd = 0.0;
    let mut latency_sum_ms: u64 = 0;
    let mut latency_count: u64 = 0;
    let mut by_role: BTreeMap<String, f64> = BTreeMap::new();
    let mut by_provider: BTreeMap<String, f64> = BTreeMap::new();
    let mut by_model: BTreeMap<String, f64> = BTreeMap::new();
    for record in records {
        let cost = record.cost_usd.unwrap_or(0.0);
        total_usd += cost;
        if let Some(latency_ms) = record.latency_ms {
            latency_sum_ms += latency_ms;
            latency_count += 1;
        }
        let role_key = record
            .logical_role
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        *by_role.entry(role_key).or_insert(0.0) += cost;
        *by_provider.entry(record.provider.clone()).or_insert(0.0) += cost;
        *by_model
            .entry(format!("{}/{}", record.provider, record.model))
            .or_insert(0.0) += cost;
    }
    let avg_latency_ms = if latency_count == 0 {
        None
    } else {
        Some(latency_sum_ms as f64 / latency_count as f64)
    };
    LlmCostAttribution {
        total_usd,
        details: serde_json::json!({
            "source": source,
            "usage_record_count": records.len(),
            "total_usd": total_usd,
            "avg_latency_ms": avg_latency_ms,
            "by_role_usd": by_role,
            "by_provider_usd": by_provider,
            "by_model_usd": by_model,
        }),
    }
}

fn runner_cost_breakdown(
    trial: &ExperimentTrial,
    reported_runner_cost_usd: Option<f64>,
) -> RunnerCostBreakdown {
    if let Some(cost) = reported_runner_cost_usd.filter(|value| value.is_finite() && *value >= 0.0)
    {
        return RunnerCostBreakdown {
            total_usd: cost,
            details: serde_json::json!({
                "source": "runner_completion",
                "reported": true,
                "total_usd": cost,
            }),
            provider_metadata_overlay: Some(serde_json::json!({
                "cost_estimate": {
                    "estimated": false,
                    "usd": cost,
                    "source": "runner_completion",
                }
            })),
        };
    }
    if let Some(estimate) = estimated_provider_runtime_cost_usd(trial) {
        return RunnerCostBreakdown {
            total_usd: estimate.total_usd,
            details: serde_json::json!({
                "source": estimate.source,
                "estimated": true,
                "total_usd": estimate.total_usd,
                "hourly_rate_usd": estimate.hourly_rate_usd,
                "native_hourly_rate": estimate.native_hourly_rate,
                "native_currency": estimate.native_currency,
                "normalization": estimate.normalization,
            }),
            provider_metadata_overlay: Some(serde_json::json!({
                "cost_estimate": {
                    "estimated": true,
                    "usd": estimate.total_usd,
                    "hourly_rate_usd": estimate.hourly_rate_usd,
                    "native_hourly_rate": estimate.native_hourly_rate,
                    "native_currency": estimate.native_currency,
                    "normalization": estimate.normalization,
                    "source": estimate.source,
                }
            })),
        };
    }
    RunnerCostBreakdown {
        total_usd: 0.0,
        details: serde_json::json!({
            "source": "none",
            "estimated": false,
            "total_usd": 0.0,
        }),
        provider_metadata_overlay: None,
    }
}

fn metadata_string_field(metadata: &serde_json::Value, key: &str) -> Option<String> {
    metadata
        .get(key)
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
}

#[derive(Debug, Clone)]
struct ProviderCostEstimate {
    total_usd: f64,
    hourly_rate_usd: f64,
    source: String,
    native_hourly_rate: Option<f64>,
    native_currency: Option<String>,
    normalization: Option<String>,
}

type ProviderHourlyRate = (f64, String, Option<f64>, Option<String>, Option<String>);

fn estimated_provider_runtime_cost_usd(trial: &ExperimentTrial) -> Option<ProviderCostEstimate> {
    let runtime_ms = trial.runtime_ms?;
    if runtime_ms == 0 {
        return Some(ProviderCostEstimate {
            total_usd: 0.0,
            hourly_rate_usd: 0.0,
            source: "runtime_ms".to_string(),
            native_hourly_rate: None,
            native_currency: None,
            normalization: None,
        });
    }
    let (hourly_rate_usd, source, native_hourly_rate, native_currency, normalization) =
        provider_hourly_rate_usd(&trial.provider_job_metadata, trial.runner_backend)?;
    if !hourly_rate_usd.is_finite() || hourly_rate_usd < 0.0 {
        return None;
    }
    Some(ProviderCostEstimate {
        total_usd: hourly_rate_usd * (runtime_ms as f64 / 3_600_000.0),
        hourly_rate_usd,
        source,
        native_hourly_rate,
        native_currency,
        normalization,
    })
}

fn provider_hourly_rate_usd(
    metadata: &serde_json::Value,
    backend: ExperimentRunnerBackend,
) -> Option<ProviderHourlyRate> {
    match backend {
        ExperimentRunnerBackend::Runpod => numeric_pointer_candidates(
            metadata,
            &[
                "/pod/adjustedCostPerHr",
                "/pod/costPerHr",
                "/launch_request/costPerHr",
            ],
        )
        .map(|(credits_per_hour, source)| {
            (
                credits_per_hour,
                source,
                Some(credits_per_hour),
                Some("runpod_credits".to_string()),
                Some("assumed_1_credit_equals_1_usd".to_string()),
            )
        }),
        ExperimentRunnerBackend::Vast => numeric_pointer_candidates(
            metadata,
            &[
                "/selected_offer/dph_total",
                "/selected_offer/search/totalHour",
                "/selected_offer/totalHour",
                "/instance/dph_total",
                "/instance/search/totalHour",
            ],
        )
        .map(|(usd_per_hour, source)| {
            (
                usd_per_hour,
                source,
                Some(usd_per_hour),
                Some("usd".to_string()),
                None,
            )
        }),
        ExperimentRunnerBackend::Lambda => numeric_pointer_candidates(
            metadata,
            &[
                "/instance/hourly_cost_usd",
                "/instance/usd_per_hour",
                "/instance/price_usd_per_hour",
                "/launch_request/hourly_cost_usd",
                "/launch_request/usd_per_hour",
                "/launch_request/price_usd_per_hour",
            ],
        )
        .map(|(usd_per_hour, source)| {
            (
                usd_per_hour,
                source,
                Some(usd_per_hour),
                Some("usd".to_string()),
                None,
            )
        })
        .or_else(|| {
            numeric_pointer_candidates(
                metadata,
                &[
                    "/instance/price_cents_per_hour",
                    "/launch_request/price_cents_per_hour",
                ],
            )
            .map(|(cents, source)| {
                (
                    cents / 100.0,
                    format!("{source} (converted_from_cents)"),
                    Some(cents),
                    Some("cents".to_string()),
                    Some("converted_from_cents".to_string()),
                )
            })
        }),
        _ => None,
    }
}

fn numeric_pointer_candidates(
    value: &serde_json::Value,
    pointers: &[&str],
) -> Option<(f64, String)> {
    pointers.iter().find_map(|pointer| {
        value
            .pointer(pointer)
            .and_then(json_value_as_f64)
            .map(|value| (value, pointer.trim_start_matches('/').replace('/', ".")))
    })
}

fn json_value_as_f64(value: &serde_json::Value) -> Option<f64> {
    match value {
        serde_json::Value::Number(number) => number.as_f64(),
        serde_json::Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn target_signature(kind: ExperimentTargetKind, metadata: &serde_json::Value) -> Option<String> {
    let provider = metadata
        .get("provider")
        .and_then(|value| value.as_str())
        .map(|value| value.to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .unwrap_or_default();
    let model = metadata
        .get("model")
        .and_then(|value| value.as_str())
        .map(|value| value.to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .unwrap_or_default();
    let route_key = metadata
        .get("route_key")
        .and_then(|value| value.as_str())
        .map(|value| value.to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .unwrap_or_default();
    let asset_id = metadata
        .get("asset_id")
        .and_then(|value| value.as_str())
        .map(|value| value.to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .unwrap_or_default();

    let mut parts = vec![format!("{kind:?}")];
    if !provider.is_empty() {
        parts.push(provider);
    }
    if !model.is_empty() {
        parts.push(model);
    }
    if !route_key.is_empty() {
        parts.push(route_key);
    }
    if !asset_id.is_empty() {
        parts.push(asset_id);
    }
    if parts.len() == 1 {
        return None;
    }
    Some(parts.join("|"))
}

fn ensure_unique_target_signature(
    kind: ExperimentTargetKind,
    metadata: &serde_json::Value,
    skip_target_id: Option<Uuid>,
    targets: &[ExperimentTarget],
) -> ApiResult<()> {
    let Some(signature) = target_signature(kind, metadata) else {
        return Ok(());
    };
    if targets.iter().any(|existing| {
        existing.kind == kind
            && skip_target_id != Some(existing.id)
            && target_signature(existing.kind, &existing.metadata).as_deref()
                == Some(signature.as_str())
    }) {
        return Err(ApiError::InvalidInput(format!(
            "Duplicate target for linked identity '{signature}'"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{provider_hourly_rate_usd, shell_command_invocation, summarize_llm_usage};
    use crate::experiments::{ExperimentModelUsageRecord, ExperimentRunnerBackend};
    use chrono::Utc;
    use uuid::Uuid;

    #[test]
    fn runpod_cost_is_normalized_from_credits() {
        let (usd_per_hour, source, native_hourly_rate, native_currency, normalization) =
            provider_hourly_rate_usd(
                &serde_json::json!({
                    "pod": {
                        "adjustedCostPerHr": 1.75
                    }
                }),
                ExperimentRunnerBackend::Runpod,
            )
            .expect("runpod metadata should produce a cost");
        assert!((usd_per_hour - 1.75).abs() < 1e-9);
        assert_eq!(source, "pod.adjustedCostPerHr");
        assert_eq!(native_hourly_rate, Some(1.75));
        assert_eq!(native_currency.as_deref(), Some("runpod_credits"));
        assert_eq!(
            normalization.as_deref(),
            Some("assumed_1_credit_equals_1_usd")
        );
    }

    #[test]
    fn llm_usage_summary_groups_costs_by_role_and_provider() {
        let records = vec![
            ExperimentModelUsageRecord {
                id: Uuid::new_v4(),
                provider: "openai".to_string(),
                model: "gpt-5.4-mini".to_string(),
                route_key: Some("planner|openai|gpt-5.4-mini".to_string()),
                logical_role: Some("planner".to_string()),
                endpoint_type: None,
                workload_tag: None,
                latency_ms: Some(100),
                cost_usd: Some(0.12),
                success: true,
                prompt_asset_ids: Vec::new(),
                retrieval_asset_ids: Vec::new(),
                tool_policy_ids: Vec::new(),
                evaluator_ids: Vec::new(),
                parser_ids: Vec::new(),
                metadata: serde_json::json!({}),
                created_at: Utc::now(),
            },
            ExperimentModelUsageRecord {
                id: Uuid::new_v4(),
                provider: "openai".to_string(),
                model: "gpt-5.4-mini".to_string(),
                route_key: Some("mutator|openai|gpt-5.4-mini".to_string()),
                logical_role: Some("mutator".to_string()),
                endpoint_type: None,
                workload_tag: None,
                latency_ms: Some(200),
                cost_usd: Some(0.08),
                success: true,
                prompt_asset_ids: Vec::new(),
                retrieval_asset_ids: Vec::new(),
                tool_policy_ids: Vec::new(),
                evaluator_ids: Vec::new(),
                parser_ids: Vec::new(),
                metadata: serde_json::json!({}),
                created_at: Utc::now(),
            },
        ];
        let summary = summarize_llm_usage(&records, "trial_id");
        assert!((summary.total_usd - 0.20).abs() < 1e-9);
        assert_eq!(summary.details["source"], "trial_id");
        assert_eq!(summary.details["usage_record_count"], 2);
        assert_eq!(summary.details["by_role_usd"]["planner"], 0.12);
        assert_eq!(summary.details["by_role_usd"]["mutator"], 0.08);
        assert_eq!(summary.details["by_provider_usd"]["openai"], 0.20);
    }

    #[test]
    fn shell_command_invocation_appends_exit_marker_for_host_shell() {
        let (shell, args) = shell_command_invocation("echo hello");
        #[cfg(target_os = "windows")]
        {
            assert_eq!(shell, "cmd");
            assert_eq!(args[0], "/V:ON");
            assert_eq!(args[1], "/C");
            assert!(args[2].contains("__THINCLAW_EXIT_CODE__:!ERRORLEVEL!"));
        }
        #[cfg(not(target_os = "windows"))]
        {
            assert_eq!(shell, "sh");
            assert_eq!(args[0], "-lc");
            assert!(args[1].contains("__THINCLAW_EXIT_CODE__"));
            assert!(args[1].contains("printf"));
        }
    }
}
