//! Experiments API — optional research automation with local and remote runners.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use tokio::time::{Duration as TokioDuration, interval};
use uuid::Uuid;

use crate::agent::env::{
    AgentAction, AgentEnvBenchmarkConfig, EnvRunner, SkillBenchEnv, TerminalBenchEnv,
    agent_env_benchmark_config, average_trajectory_score, render_trajectory_log,
    trajectory_summary,
};
use crate::agent::run_artifact::{digest_json, digest_text};
use crate::agent::subagent_executor::{SubagentExecutor, SubagentSpawnRequest};
use crate::agent::{AgentRunArtifact, AgentRunStatus};
use crate::api::{ApiError, ApiResult};
use crate::db::Database;
use crate::experiments::adapters::{self, RemoteLaunchAction, RunnerLaunchOutcome};
use crate::experiments::{
    ExperimentArtifactRef, ExperimentAutonomyMode, ExperimentCampaign,
    ExperimentCampaignQueueState, ExperimentCampaignStatus, ExperimentLease,
    ExperimentLeaseAuthentication, ExperimentLeaseStatus, ExperimentPreset, ExperimentProject,
    ExperimentProjectStatus, ExperimentRunnerArtifactUpload, ExperimentRunnerBackend,
    ExperimentRunnerCompletion, ExperimentRunnerJob, ExperimentRunnerProfile,
    ExperimentRunnerStatus, ExperimentTarget, ExperimentTargetKind, ExperimentTargetLink,
    ExperimentTrial, ExperimentTrialStatus, LlmCostAttribution, MutatorResult, PlannerProposal,
    ReviewerDecision, campaign_gateway_url, campaign_status_message, compare_metrics,
    default_strategy_prompt, derive_opportunities, derive_outcome_opportunities,
    enforce_mutable_paths as enforce_mutable_paths_policy,
    ensure_unique_target_signature as ensure_unique_target_signature_policy, env_pairs_from_json,
    experiment_base_branch_unavailable_message, experiment_campaign_cancelled_by_operator_message,
    experiment_campaign_cancelled_message, experiment_campaign_has_no_accepted_commit_message,
    experiment_campaign_has_no_trial_to_reissue_message,
    experiment_campaign_has_no_worktree_message,
    experiment_campaign_missing_experiment_branch_field_message,
    experiment_campaign_missing_experiment_branch_message,
    experiment_campaign_missing_worktree_path_field_message,
    experiment_campaign_missing_worktree_path_message, experiment_campaign_not_found_message,
    experiment_campaign_paused_by_operator_message, experiment_campaign_paused_message,
    experiment_git_remote_unavailable_message, experiment_lease_expired_message,
    experiment_lease_not_found_message, experiment_lease_reissue_remote_only_message,
    experiment_lease_revoked_action_message, experiment_lease_revoked_message,
    experiment_no_candidate_changes_message, experiment_opportunity_not_found_message,
    experiment_primary_metric_not_found_message, experiment_project_missing_mutable_paths_message,
    experiment_project_not_found_message, experiment_project_run_command_empty_message,
    experiment_project_workdir_escapes_campaign_worktree_message,
    experiment_project_workdir_missing_message,
    experiment_project_workdir_outside_workspace_message, experiment_promotion_pr_body,
    experiment_remote_trial_reissue_in_flight_only_message, experiment_runner_not_found_message,
    experiment_runner_profile_id_required_message, experiment_target_id_required_message,
    experiment_target_not_found_message, experiment_trial_not_found_message,
    experiment_workspace_not_git_repository_message, experiment_workspace_path_missing_message,
    experiment_workspace_path_missing_with_error_message, experiments_feature_disabled_message,
    experiments_worktree_path, extract_metrics, filtered_changed_files, hash_lease_token,
    invalid_experiment_lease_token_message, is_stale_lease as is_stale_lease_policy,
    lease_runner_trial_status, merge_json, metadata_string_field, next_campaign_status,
    normalize_trial_completion, parse_research_json_response, parse_secret_reference,
    ready_project_status as ready_project_status_policy, recent_trial_context,
    research_subagent_executor_unavailable_message, runner_cost_breakdown, short_id,
    sort_experiment_opportunities, summarize_llm_usage, truncate_for_prompt,
    validate_lease_completion_status, validate_project_workdir_fragment,
};
use crate::history::OutcomeContractQuery;
use crate::llm::usage_tracking::{
    USAGE_TRACKING_EXPERIMENT_CAMPAIGN_ID_KEY, USAGE_TRACKING_EXPERIMENT_ROLE_KEY,
    USAGE_TRACKING_EXPERIMENT_TARGET_IDS_KEY, USAGE_TRACKING_EXPERIMENT_TRIAL_ID_KEY,
};
use crate::secrets::SecretsStore;
use crate::settings::Settings;
use crate::tools::execution_backend::{
    CommandExecutionRequest, DockerSandboxExecutionBackend, ExecutionBackend, ExecutionResult,
    LocalHostExecutionBackend, ScriptExecutionRequest, experiment_runner_runtime_descriptor,
    subagent_executor_runtime_descriptor,
};

pub use thinclaw_gateway::web::experiments::*;

const DEFAULT_REMOTE_LEASE_MINUTES: i64 = 60;
const DEFAULT_EXPERIMENT_CONTROLLER_TICK_SECS: u64 = 30;
const STALE_LEASE_GRACE_MINUTES: i64 = 10;
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
    "memory_read",
    "memory_search",
    "session_search",
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
    "shell",
    "execute_code",
    "process",
    "build_software",
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

#[derive(Debug, Clone)]
struct ResearchSubagentOutput<T> {
    value: T,
    run_artifact: AgentRunArtifact,
}

#[derive(Debug, Clone)]
struct ResearchSubagentError {
    message: String,
    run_artifact: AgentRunArtifact,
}

impl std::fmt::Display for ResearchSubagentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(f)
    }
}

#[derive(Debug)]
enum ResearchSubagentInvocationError {
    Api(ApiError),
    Run(Box<ResearchSubagentError>),
}

impl From<ApiError> for ResearchSubagentInvocationError {
    fn from(value: ApiError) -> Self {
        Self::Api(value)
    }
}

impl From<ResearchSubagentError> for ResearchSubagentInvocationError {
    fn from(value: ResearchSubagentError) -> Self {
        Self::Run(Box::new(value))
    }
}

#[derive(Debug)]
struct CandidateGenerationError {
    message: String,
    run_artifacts: Vec<AgentRunArtifact>,
}

impl CandidateGenerationError {
    fn new(message: impl Into<String>, run_artifacts: Vec<AgentRunArtifact>) -> Self {
        Self {
            message: message.into(),
            run_artifacts,
        }
    }
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
        match secrets
            .get_for_injection(
                user_id,
                &name,
                crate::secrets::SecretAccessContext::new("experiments.api", "gpu_cloud_credential"),
            )
            .await
        {
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
            experiments_feature_disabled_message().to_string(),
        ));
    }
    Ok(settings)
}

async fn resolve_project_workdir(project: &ExperimentProject) -> ApiResult<PathBuf> {
    let workspace_root = tokio::fs::canonicalize(&project.workspace_path)
        .await
        .map_err(|e| {
            ApiError::InvalidInput(experiment_workspace_path_missing_with_error_message(
                &project.workspace_path,
                e,
            ))
        })?;
    let workdir_fragment =
        validate_project_workdir_fragment(&project.workdir).map_err(ApiError::InvalidInput)?;
    let workdir = workspace_root.join(workdir_fragment);
    let resolved = tokio::fs::canonicalize(&workdir).await.map_err(|e| {
        ApiError::InvalidInput(experiment_project_workdir_missing_message(
            workdir.display(),
            e,
        ))
    })?;
    if !resolved.starts_with(&workspace_root) {
        return Err(ApiError::InvalidInput(
            experiment_project_workdir_outside_workspace_message().to_string(),
        ));
    }
    Ok(resolved)
}

async fn validate_project_launch_readiness(project: &ExperimentProject) -> ApiResult<()> {
    if !Path::new(&project.workspace_path).is_dir() {
        return Err(ApiError::InvalidInput(
            experiment_workspace_path_missing_message(&project.workspace_path),
        ));
    }
    if project.mutable_paths.is_empty() {
        return Err(ApiError::InvalidInput(
            experiment_project_missing_mutable_paths_message().to_string(),
        ));
    }
    if project.run_command.trim().is_empty() {
        return Err(ApiError::InvalidInput(
            experiment_project_run_command_empty_message().to_string(),
        ));
    }

    let _ = resolve_project_workdir(project).await?;

    git_output(&project.workspace_path, &["rev-parse", "--show-toplevel"])
        .await
        .map_err(|error| {
            ApiError::InvalidInput(experiment_workspace_not_git_repository_message(error))
        })?;
    git_output(
        &project.workspace_path,
        &["rev-parse", "--verify", &project.base_branch],
    )
    .await
    .map_err(|error| {
        ApiError::InvalidInput(experiment_base_branch_unavailable_message(
            &project.base_branch,
            error,
        ))
    })?;
    git_output(
        &project.workspace_path,
        &["remote", "get-url", &project.git_remote_name],
    )
    .await
    .map_err(|error| {
        ApiError::InvalidInput(experiment_git_remote_unavailable_message(
            &project.git_remote_name,
            error,
        ))
    })?;

    Ok(())
}

async fn resolved_secret_env_pairs(
    user_id: &str,
    runner: &ExperimentRunnerProfile,
) -> Vec<(String, String)> {
    let Some(secrets) = research_secrets_store() else {
        return Vec::new();
    };

    let mut pairs = Vec::new();
    for reference in &runner.secret_references {
        let Some((secret_name, env_names)) = parse_secret_reference(reference) else {
            continue;
        };
        match secrets
            .get_for_injection(
                user_id,
                &secret_name,
                crate::secrets::SecretAccessContext::new(
                    "experiments.api",
                    "runner_env_credential",
                ),
            )
            .await
        {
            Ok(secret) => {
                let value = secret.expose().to_string();
                for env_name in env_names {
                    pairs.push((env_name, value.clone()));
                }
            }
            Err(error) => tracing::debug!(
                secret_name = %secret_name,
                error = %error,
                "Research benchmark secret lookup failed"
            ),
        }
    }
    pairs
}

async fn resolved_runner_env_grants(
    user_id: &str,
    runner: &ExperimentRunnerProfile,
) -> serde_json::Value {
    let mut merged = runner.env_grants.as_object().cloned().unwrap_or_default();
    for (env_name, value) in resolved_secret_env_pairs(user_id, runner).await {
        merged
            .entry(env_name)
            .or_insert_with(|| serde_json::json!(value));
    }
    serde_json::Value::Object(merged)
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
        .ok_or_else(|| ApiError::SessionNotFound(experiment_project_not_found_message(id)))
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
    project.status =
        ready_project_status_policy(&project, Path::new(&project.workspace_path).exists());
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
    project.status = req.status.unwrap_or_else(|| {
        ready_project_status_policy(&project, Path::new(&project.workspace_path).exists())
    });
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
        .ok_or_else(|| ApiError::SessionNotFound(experiment_runner_not_found_message(id)))
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
        readiness_class: crate::experiments::ExperimentRunnerReadinessClass::ManualOnly,
        launch_eligible: false,
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
    let validation = validate_runner_profile_impl(user_id, &runner, &settings).await;
    runner.status = if validation.valid {
        ExperimentRunnerStatus::Validated
    } else {
        ExperimentRunnerStatus::Unavailable
    };
    runner.readiness_class = validation.readiness_class;
    runner.launch_eligible = validation.launch_eligible;
    runner.updated_at = Utc::now();
    store
        .update_experiment_runner_profile(&runner)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(ExperimentRunnerValidationResponse {
        runner,
        valid: validation.valid,
        readiness_class: validation.readiness_class,
        launch_eligible: validation.launch_eligible,
        message: validation.message,
    })
}

pub async fn list_campaigns(
    store: &Arc<dyn Database>,
    user_id: &str,
) -> ApiResult<ExperimentCampaignListResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let campaigns = store
        .list_experiment_campaigns_for_owner(user_id)
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
    let campaign = store
        .get_experiment_campaign_for_owner(id, user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(experiment_campaign_not_found_message(id)))?;
    Ok(campaign)
}

pub async fn list_trials(
    store: &Arc<dyn Database>,
    user_id: &str,
    campaign_id: Uuid,
) -> ApiResult<ExperimentTrialListResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let campaign = get_campaign(store, user_id, campaign_id).await?;
    let trials = store
        .list_experiment_trials_for_owner(campaign.id, user_id)
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
    let trial = store
        .get_experiment_trial_for_owner(id, user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(experiment_trial_not_found_message(id)))?;
    Ok(trial)
}

pub async fn list_artifacts(
    store: &Arc<dyn Database>,
    user_id: &str,
    trial_id: Uuid,
) -> ApiResult<ExperimentArtifactListResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let artifacts = store
        .list_experiment_artifacts_for_owner(trial_id, user_id)
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
        match reconcile_experiments_once(&store).await {
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

async fn reconcile_experiments_once(store: &Arc<dyn Database>) -> ApiResult<()> {
    let campaigns = store
        .list_experiment_campaigns()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let mut owners = HashSet::new();

    for mut campaign in campaigns {
        owners.insert(campaign.owner_user_id.clone());
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
            let owner_user_id = campaign.owner_user_id.clone();
            reconcile_active_campaign(store, &owner_user_id, &mut campaign).await?;
        }
    }

    for owner_user_id in owners {
        maybe_launch_next_queued_after_slot_release(store, &owner_user_id).await?;
    }
    Ok(())
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
                    limit = max_trials.unwrap_or(0)
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
    is_stale_lease_policy(lease, now, STALE_LEASE_GRACE_MINUTES)
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
            let planner = match run_planner_subagent(store, campaign, project, None).await {
                Ok(planner) => planner,
                Err(ResearchSubagentInvocationError::Api(error)) => return Err(error),
                Err(ResearchSubagentInvocationError::Run(error)) => {
                    let error = *error;
                    campaign.status = ExperimentCampaignStatus::Paused;
                    campaign.queue_state = ExperimentCampaignQueueState::NotQueued;
                    campaign.pause_reason =
                        Some(format!("Suggestion generation paused: {}", error.message));
                    record_campaign_candidate_generation(
                        campaign,
                        "suggest_only",
                        "failed",
                        &error.message,
                        &[error.run_artifact],
                    );
                    campaign.updated_at = Utc::now();
                    store
                        .update_experiment_campaign(campaign)
                        .await
                        .map_err(|e| ApiError::Internal(e.to_string()))?;
                    maybe_launch_next_queued_after_slot_release(store, user_id).await?;
                    return Ok(());
                }
            };
            campaign.status = ExperimentCampaignStatus::Paused;
            campaign.queue_state = ExperimentCampaignQueueState::NotQueued;
            campaign.pause_reason = Some(format!(
                "Suggestion ready: {}",
                truncate_for_prompt(&planner.value.mutation_brief, 500)
            ));
            record_campaign_candidate_generation(
                campaign,
                "suggest_only",
                "completed",
                &planner.value.mutation_brief,
                std::slice::from_ref(&planner.run_artifact),
            );
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
            campaign.pause_reason = Some(format!(
                "Autonomous candidate generation paused: {}",
                error.message
            ));
            record_campaign_candidate_generation(
                campaign,
                "autonomous",
                "failed",
                &error.message,
                &error.run_artifacts,
            );
            campaign.updated_at = Utc::now();
            store
                .update_experiment_campaign(campaign)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            maybe_launch_next_queued_after_slot_release(store, user_id).await?;
            return Ok(());
        }
    };
    store
        .create_experiment_trial(&trial)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
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
        return Err(ApiError::InvalidInput(
            experiment_target_id_required_message().to_string(),
        ));
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
            ApiError::SessionNotFound(experiment_opportunity_not_found_message(
                &req.opportunity_id,
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
        .ok_or_else(|| ApiError::SessionNotFound(experiment_target_not_found_message(id)))?;
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
        crate::workspace::paths::USER,
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
    validate_project_launch_readiness(&project).await?;
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
        .ok_or_else(|| {
            ApiError::InvalidInput(experiment_runner_profile_id_required_message().to_string())
        })?;
    let runner = get_runner(store, user_id, runner_id).await?;
    let validation = validate_runner_profile_impl(user_id, &runner, &settings).await;
    if !validation.valid {
        return Err(ApiError::InvalidInput(format!(
            "Runner profile is not launchable: {}",
            validation.message
        )));
    }
    if queue_state == ExperimentCampaignQueueState::Queued && !validation.launch_eligible {
        return Err(ApiError::InvalidInput(
            "This runner requires operator action and cannot be queued for automatic launch. Wait for a free slot or use a launch-ready runner.".to_string(),
        ));
    }
    let normalized_gateway_url = req
        .gateway_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if req.gateway_url.is_some() && normalized_gateway_url.is_none() {
        return Err(ApiError::InvalidInput(
            "gateway_url must not be empty when provided.".to_string(),
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
        owner_user_id: user_id.to_string(),
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
        gateway_url: normalized_gateway_url,
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

async fn next_queued_campaign_for_owner(
    store: &Arc<dyn Database>,
    owner_user_id: Option<&str>,
) -> ApiResult<Option<ExperimentCampaign>> {
    let mut queued: Vec<_> = store
        .list_experiment_campaigns()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .into_iter()
        .filter(|campaign| {
            campaign.status == ExperimentCampaignStatus::PendingBaseline
                && campaign.queue_state == ExperimentCampaignQueueState::Queued
                && owner_user_id.is_none_or(|owner| campaign.owner_user_id == owner)
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

    let Some(mut campaign) = next_queued_campaign_for_owner(store, Some(user_id)).await? else {
        return Ok(None);
    };
    let campaign_owner_user_id = campaign.owner_user_id.clone();
    let project = get_project(store, &campaign_owner_user_id, campaign.project_id).await?;
    let runner = get_runner(store, &campaign_owner_user_id, campaign.runner_profile_id).await?;
    if let Err(error) = validate_project_launch_readiness(&project).await {
        campaign.status = ExperimentCampaignStatus::Failed;
        campaign.pause_reason = Some(format!(
            "Queued launch failed project validation: {}",
            error
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
            message: "Queued campaign failed project validation before launch.".to_string(),
        }));
    }

    let validation =
        validate_runner_profile_impl(&campaign_owner_user_id, &runner, &settings).await;
    if !validation.valid {
        campaign.status = ExperimentCampaignStatus::Failed;
        campaign.pause_reason = Some(format!(
            "Queued launch failed because runner '{}' is not valid: {}",
            runner.name, validation.message
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
    if !validation.launch_eligible {
        campaign.status = ExperimentCampaignStatus::Paused;
        campaign.queue_state = ExperimentCampaignQueueState::NotQueued;
        campaign.pause_reason = Some(format!(
            "Queued launch requires operator action because runner '{}' is {}.",
            runner.name,
            match validation.readiness_class {
                crate::experiments::ExperimentRunnerReadinessClass::ManualOnly => "manual_only",
                crate::experiments::ExperimentRunnerReadinessClass::BootstrapReady =>
                    "bootstrap_ready",
                crate::experiments::ExperimentRunnerReadinessClass::LaunchReady => "launch_ready",
            }
        ));
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
            message: "Queued campaign paused until an operator starts the runner manually."
                .to_string(),
        }));
    }

    match launch_campaign_baseline(
        store,
        &campaign_owner_user_id,
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
    let worktree_path = campaign.worktree_path.clone().ok_or_else(|| {
        ApiError::Internal(experiment_campaign_missing_worktree_path_field_message().to_string())
    })?;
    let branch = campaign.experiment_branch.clone().ok_or_else(|| {
        ApiError::Internal(
            experiment_campaign_missing_experiment_branch_field_message().to_string(),
        )
    })?;

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
    let worktree_path = campaign.worktree_path.as_deref().ok_or_else(|| {
        ApiError::InvalidInput(experiment_campaign_has_no_worktree_message().to_string())
    })?;
    let changed_files = filtered_changed_files(git_changed_files(worktree_path).await?);
    if changed_files.is_empty() {
        return Err(ApiError::InvalidInput(
            experiment_no_candidate_changes_message().to_string(),
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
) -> Result<ExperimentTrial, CandidateGenerationError> {
    let sequence = latest_trial(store, campaign.id)
        .await
        .map_err(|error| CandidateGenerationError::new(error.to_string(), Vec::new()))?
        .map(|trial| trial.sequence + 1)
        .unwrap_or(1);
    let trial_id = Uuid::new_v4();
    let planner = match run_planner_subagent(store, campaign, project, Some(trial_id)).await {
        Ok(planner) => planner,
        Err(ResearchSubagentInvocationError::Api(error)) => {
            return Err(CandidateGenerationError::new(error.to_string(), Vec::new()));
        }
        Err(ResearchSubagentInvocationError::Run(error)) => {
            let error = *error;
            return Err(CandidateGenerationError::new(
                error.message,
                vec![error.run_artifact],
            ));
        }
    };
    let mutator =
        match run_mutator_subagent(campaign, project, &planner.value, Some(trial_id)).await {
            Ok(mutator) => mutator,
            Err(ResearchSubagentInvocationError::Api(error)) => {
                return Err(CandidateGenerationError::new(
                    error.to_string(),
                    vec![planner.run_artifact.clone()],
                ));
            }
            Err(ResearchSubagentInvocationError::Run(error)) => {
                let error = *error;
                return Err(CandidateGenerationError::new(
                    error.message,
                    vec![planner.run_artifact.clone(), error.run_artifact],
                ));
            }
        };
    let worktree_path = campaign.worktree_path.as_deref().ok_or_else(|| {
        CandidateGenerationError::new(
            experiment_campaign_has_no_worktree_message(),
            vec![planner.run_artifact.clone(), mutator.run_artifact.clone()],
        )
    })?;
    let changed_files =
        filtered_changed_files(git_changed_files(worktree_path).await.map_err(|e| {
            CandidateGenerationError::new(
                e.to_string(),
                vec![planner.run_artifact.clone(), mutator.run_artifact.clone()],
            )
        })?);
    if changed_files.is_empty() {
        let mut mutator_artifact = mutator.run_artifact.clone();
        mutator_artifact.mark_failed("Autonomous mutator did not produce any candidate changes.");
        return Err(CandidateGenerationError::new(
            "Autonomous mutator did not produce any candidate changes.",
            vec![planner.run_artifact.clone(), mutator_artifact],
        ));
    }
    if let Err(error) = enforce_mutable_paths(&project.mutable_paths, &changed_files) {
        let mut mutator_artifact = mutator.run_artifact.clone();
        mutator_artifact.mark_failed(error.to_string());
        return Err(CandidateGenerationError::new(
            error.to_string(),
            vec![planner.run_artifact.clone(), mutator_artifact],
        ));
    }
    let diff_stat = git_output(worktree_path, &["diff", "--stat", "--", "."])
        .await
        .map_err(|e| {
            CandidateGenerationError::new(
                e.to_string(),
                vec![planner.run_artifact.clone(), mutator.run_artifact.clone()],
            )
        })?;
    let diff_preview = git_output(worktree_path, &["diff", "--", "."])
        .await
        .map_err(|e| {
            CandidateGenerationError::new(
                e.to_string(),
                vec![planner.run_artifact.clone(), mutator.run_artifact.clone()],
            )
        })?;
    let reviewer = match run_reviewer_subagent(
        campaign,
        project,
        &planner.value,
        &diff_stat,
        &diff_preview,
        Some(trial_id),
    )
    .await
    {
        Ok(reviewer) => reviewer,
        Err(ResearchSubagentInvocationError::Api(error)) => {
            return Err(CandidateGenerationError::new(
                error.to_string(),
                vec![planner.run_artifact.clone(), mutator.run_artifact.clone()],
            ));
        }
        Err(ResearchSubagentInvocationError::Run(error)) => {
            let error = *error;
            return Err(CandidateGenerationError::new(
                error.message,
                vec![
                    planner.run_artifact.clone(),
                    mutator.run_artifact.clone(),
                    error.run_artifact,
                ],
            ));
        }
    };
    if !(reviewer.value.approved && reviewer.value.scope_ok && reviewer.value.benchmark_ready) {
        let mut reviewer_artifact = reviewer.run_artifact.clone();
        reviewer_artifact.mark_failed(reviewer.value.reason.clone());
        return Err(CandidateGenerationError::new(
            format!(
                "Reviewer rejected the autonomous candidate: {}",
                reviewer.value.reason
            ),
            vec![
                planner.run_artifact.clone(),
                mutator.run_artifact.clone(),
                reviewer_artifact,
            ],
        ));
    }
    let planner_hypothesis = planner.value.hypothesis.clone();
    let planner_target_ids = planner.value.target_ids.clone();
    let expected_metric_direction = planner.value.expected_metric_direction.clone();
    let mutator_mutation_summary = mutator.value.mutation_summary.clone();
    let mutator_changed_paths = mutator.value.changed_paths.clone();
    let reviewer_reason = reviewer.value.reason.clone();
    prepare_candidate_trial_from_worktree(
        store,
        campaign,
        project,
        runner,
        trial_id,
        sequence,
        planner_hypothesis,
        mutator_mutation_summary,
        reviewer_reason,
        serde_json::json!({
            "candidate_source": "autonomous_subagent",
            "changed_paths": changed_files,
            "planner_target_ids": planner_target_ids,
            "expected_metric_direction": expected_metric_direction,
            "mutator_changed_paths": mutator_changed_paths,
            "workspace": worktree_path,
            "run_artifacts": [
                planner.run_artifact.clone(),
                mutator.run_artifact.clone(),
                reviewer.run_artifact.clone()
            ],
        }),
    )
    .await
    .map_err(|error| {
        CandidateGenerationError::new(
            error.to_string(),
            vec![
                planner.run_artifact.clone(),
                mutator.run_artifact.clone(),
                reviewer.run_artifact.clone(),
            ],
        )
    })
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

    Ok(message.unwrap_or_else(|| experiment_lease_revoked_action_message().to_string()))
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
    campaign.pause_reason = Some(experiment_campaign_paused_by_operator_message().to_string());
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
        message: launch_message.unwrap_or_else(|| experiment_campaign_paused_message().to_string()),
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
    campaign.pause_reason = Some(experiment_campaign_cancelled_by_operator_message().to_string());
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
        message: launch_message
            .unwrap_or_else(|| experiment_campaign_cancelled_message().to_string()),
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

    let worktree_path = campaign.worktree_path.clone().ok_or_else(|| {
        ApiError::InvalidInput(experiment_campaign_has_no_worktree_message().to_string())
    })?;
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
            experiment_lease_reissue_remote_only_message().to_string(),
        ));
    }
    let mut trial = latest_trial(store, campaign.id).await?.ok_or_else(|| {
        ApiError::InvalidInput(experiment_campaign_has_no_trial_to_reissue_message().to_string())
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
            experiment_remote_trial_reissue_in_flight_only_message().to_string(),
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
    let lease = create_lease(store, user_id, &project, &runner, &campaign, &trial).await?;
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
            ApiError::InvalidInput(experiment_campaign_has_no_accepted_commit_message().to_string())
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
            let body = experiment_promotion_pr_body(
                campaign.id,
                &best_commit,
                &project.primary_metric.name,
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
        return Err(ApiError::Unavailable(
            experiment_lease_revoked_message().to_string(),
        ));
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
            ApiError::SessionNotFound(experiment_trial_not_found_message(lease.trial_id))
        })?;
    trial.summary = Some(req.status.clone());
    trial.status = lease_runner_trial_status(&req.status, trial.status);
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
            ApiError::SessionNotFound(experiment_trial_not_found_message(lease.trial_id))
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
    complete_trial_terminal(
        store,
        &project,
        &mut campaign,
        &mut trial,
        Some(&mut lease),
        completion,
    )
    .await?;
    maybe_launch_next_queued_after_slot_release(store, user_id).await?;
    Ok(ExperimentCampaignActionResponse {
        campaign,
        trial: Some(trial),
        lease: None,
        launch: None,
        message: "Lease completed.".to_string(),
    })
}

pub async fn lease_owner_user_id(
    store: &Arc<dyn Database>,
    lease_id: Uuid,
    token: &str,
) -> ApiResult<String> {
    let lease = verified_lease(store, lease_id, token).await?;
    let campaign = store
        .get_experiment_campaign(lease.campaign_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| {
            ApiError::SessionNotFound(experiment_campaign_not_found_message(lease.campaign_id))
        })?;
    Ok(campaign.owner_user_id)
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
        let lease = create_lease(store, user_id, project, runner, &campaign, &trial).await?;
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
    let completion =
        execute_local_trial(user_id, settings, project, runner, &campaign, &mut trial).await?;
    complete_trial_terminal(store, project, &mut campaign, &mut trial, None, completion).await?;
    Ok(ExperimentCampaignActionResponse {
        campaign,
        trial: Some(trial),
        lease: None,
        launch: None,
        message: "Local trial finished.".to_string(),
    })
}

async fn complete_trial_terminal(
    store: &Arc<dyn Database>,
    project: &ExperimentProject,
    campaign: &mut ExperimentCampaign,
    trial: &mut ExperimentTrial,
    lease: Option<&mut ExperimentLease>,
    completion: ExperimentRunnerCompletion,
) -> ApiResult<()> {
    if let Some(lease) = lease.as_ref()
        && let Err(message) = validate_lease_completion_status(lease.status)
    {
        return Err(ApiError::InvalidInput(message.to_string()));
    }

    let completion = normalize_trial_completion(completion);
    finalize_trial(store, project, campaign, trial, completion).await?;

    if let Some(lease) = lease {
        lease.status = ExperimentLeaseStatus::Completed;
        lease.completed_at = Some(Utc::now());
        lease.updated_at = Utc::now();
        store
            .update_experiment_lease(lease)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    }

    Ok(())
}

async fn finalize_trial(
    store: &Arc<dyn Database>,
    project: &ExperimentProject,
    campaign: &mut ExperimentCampaign,
    trial: &mut ExperimentTrial,
    completion: ExperimentRunnerCompletion,
) -> ApiResult<()> {
    trial.completed_at = Some(Utc::now());
    let runner_run_artifact = trial_runner_run_artifact(campaign, trial, &completion);
    trial.exit_code = completion.exit_code;
    trial.metrics_json = completion.metrics_json;
    trial.summary = completion.summary;
    trial.log_preview_path = completion.log_preview_path;
    trial.artifact_manifest_json = merge_json(
        &trial.artifact_manifest_json,
        &completion.artifact_manifest_json,
    );
    trial.updated_at = Utc::now();
    push_run_artifact(&mut trial.artifact_manifest_json, runner_run_artifact);
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
        let failure_stage = trial
            .artifact_manifest_json
            .get("stage")
            .and_then(|value| value.as_str());
        if matches!(
            failure_stage,
            Some("prepare" | "checkout" | "clone" | "fetch" | "run")
        ) {
            trial.status = ExperimentTrialStatus::InfraFailed;
            trial.decision_reason = Some(format!(
                "{} command exited non-zero.",
                failure_stage.unwrap_or("runner")
            ));
        } else {
            trial.status = ExperimentTrialStatus::Crashed;
            trial.decision_reason = Some("Benchmark command exited non-zero.".to_string());
        }
        campaign.failure_count += 1;
    } else if !has_primary_metric {
        trial.status = ExperimentTrialStatus::InfraFailed;
        trial.decision_reason = Some(experiment_primary_metric_not_found_message(
            &project.primary_metric.name,
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
    }

    let restore_commit = match trial.status {
        ExperimentTrialStatus::Rejected => campaign.best_commit.as_deref(),
        _ => trial
            .candidate_commit
            .as_deref()
            .or(campaign.best_commit.as_deref()),
    };
    let restore_error =
        if let Err(error) = restore_campaign_worktree_after_trial(campaign, restore_commit).await {
            trial.artifact_manifest_json = merge_json(
                &trial.artifact_manifest_json,
                &serde_json::json!({
                    "worktree_restore_error": error,
                }),
            );
            Some(error)
        } else {
            None
        };

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

    if let Some(error) = restore_error {
        campaign.status = ExperimentCampaignStatus::Paused;
        campaign.pause_reason = Some(format!(
            "Campaign paused: failed to restore campaign worktree: {error}"
        ));
    } else {
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
    }
    if matches!(
        campaign.status,
        ExperimentCampaignStatus::Completed
            | ExperimentCampaignStatus::Cancelled
            | ExperimentCampaignStatus::Failed
            | ExperimentCampaignStatus::AwaitingPromotion
    ) {
        campaign.ended_at = Some(Utc::now());
    }

    upsert_local_trial_artifact_refs(store, trial).await?;

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

async fn upsert_local_trial_artifact_refs(
    store: &Arc<dyn Database>,
    trial: &ExperimentTrial,
) -> ApiResult<()> {
    let mut desired = Vec::new();
    if let Some(path) = trial
        .artifact_manifest_json
        .get("trajectory_json_path")
        .and_then(|value| value.as_str())
    {
        desired.push(("trajectory_json".to_string(), path.to_string()));
    }
    if let Some(path) = trial
        .artifact_manifest_json
        .get("summary_json_path")
        .and_then(|value| value.as_str())
    {
        desired.push(("summary_json".to_string(), path.to_string()));
    }
    if let Some(path) = trial.log_preview_path.as_deref() {
        desired.push(("log_preview".to_string(), path.to_string()));
    }
    if desired.is_empty() {
        return Ok(());
    }

    let mut artifacts = store
        .list_experiment_artifacts(trial.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let mut changed = false;
    for (kind, path) in desired {
        if artifacts
            .iter()
            .any(|artifact| artifact.kind == kind && artifact.uri_or_local_path == path)
        {
            continue;
        }
        let size_bytes = std::fs::metadata(&path).ok().map(|metadata| metadata.len());
        artifacts.push(ExperimentArtifactRef {
            id: Uuid::new_v4(),
            trial_id: trial.id,
            kind,
            uri_or_local_path: path,
            size_bytes,
            fetchable: false,
            metadata: serde_json::json!({
                "source": "local_runner_completion",
            }),
            created_at: Utc::now(),
        });
        changed = true;
    }

    if changed {
        store
            .replace_experiment_artifacts(trial.id, &artifacts)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    }
    Ok(())
}

async fn create_lease(
    store: &Arc<dyn Database>,
    user_id: &str,
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
    let resolved_env_grants = resolved_runner_env_grants(user_id, runner).await;
    let git_ref = campaign.experiment_branch.clone().ok_or_else(|| {
        ApiError::Internal(experiment_campaign_missing_experiment_branch_message().to_string())
    })?;
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
        env_grants: resolved_env_grants.clone(),
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
            "env": resolved_env_grants,
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
        .ok_or_else(|| ApiError::SessionNotFound(experiment_lease_not_found_message(lease_id)))?;
    if lease.expires_at < Utc::now() {
        return Err(ApiError::Unavailable(
            experiment_lease_expired_message().to_string(),
        ));
    }
    if lease.token_hash != hash_lease_token(token) {
        return Err(ApiError::InvalidInput(
            invalid_experiment_lease_token_message().to_string(),
        ));
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
) -> crate::experiments::adapters::RunnerValidationOutcome {
    let provider_api_key = research_provider_api_key(user_id, runner).await;
    adapters::validate_runner_profile(runner, settings, provider_api_key.as_deref()).await
}

async fn prepare_campaign_worktree(
    project: &ExperimentProject,
    worktree_path: &Path,
) -> ApiResult<()> {
    if !Path::new(&project.workspace_path).exists() {
        return Err(ApiError::InvalidInput(
            experiment_workspace_path_missing_message(&project.workspace_path),
        ));
    }
    if worktree_path.exists() {
        let worktree = worktree_path.to_string_lossy().to_string();
        let _ = git_output(
            &project.workspace_path,
            &["worktree", "remove", "--force", &worktree],
        )
        .await;
        let _ = git_output(&project.workspace_path, &["worktree", "prune"]).await;
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

async fn git_changed_files(worktree_path: &str) -> ApiResult<Vec<String>> {
    let output = git_output_raw(worktree_path, &["status", "--porcelain", "-z"]).await?;
    let mut entries = output.split('\0').filter(|entry| !entry.is_empty());
    let mut changed_files = Vec::new();

    while let Some(entry) = entries.next() {
        if entry.len() < 4 {
            continue;
        }
        let status = &entry[..2];
        let primary_path = entry[3..].trim();
        let effective_path = if status.contains('R') || status.contains('C') {
            let _ = entries.next();
            primary_path
        } else {
            primary_path
        };
        if !effective_path.is_empty() {
            changed_files.push(effective_path.to_string());
        }
    }

    Ok(changed_files)
}

fn enforce_mutable_paths(mutable_paths: &[String], changed_files: &[String]) -> ApiResult<()> {
    enforce_mutable_paths_policy(mutable_paths, changed_files).map_err(ApiError::InvalidInput)
}

fn push_run_artifact(manifest: &mut serde_json::Value, artifact: AgentRunArtifact) {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        let artifact_for_log = artifact.clone();
        handle.spawn(async move {
            if let Err(err) = crate::agent::AgentRunHarness::new(None)
                .append_artifact(&artifact_for_log)
                .await
            {
                tracing::debug!(error = %err, "Failed to append experiment run artifact");
            }
        });
    } else {
        tracing::debug!(
            "Skipping experiment run-artifact append because no Tokio runtime is active"
        );
    }
    if !manifest.is_object() {
        *manifest = serde_json::json!({});
    }
    let Some(obj) = manifest.as_object_mut() else {
        return;
    };
    let entry = obj
        .entry("run_artifacts".to_string())
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    if !entry.is_array() {
        *entry = serde_json::Value::Array(Vec::new());
    }
    if let Some(items) = entry.as_array_mut()
        && let Ok(value) = serde_json::to_value(artifact)
    {
        items.push(value);
    }
}

fn record_campaign_candidate_generation(
    campaign: &mut ExperimentCampaign,
    mode: &str,
    status: &str,
    message: &str,
    run_artifacts: &[AgentRunArtifact],
) {
    for artifact in run_artifacts.iter().cloned() {
        push_run_artifact(&mut campaign.metadata, artifact);
    }
    let artifact_run_ids = run_artifacts
        .iter()
        .map(|artifact| artifact.run_id.clone())
        .collect::<Vec<_>>();
    campaign.metadata = merge_json(
        &campaign.metadata,
        &serde_json::json!({
            "candidate_generation": {
                "mode": mode,
                "status": status,
                "message": message,
                "updated_at": Utc::now(),
                "artifact_run_ids": artifact_run_ids,
            }
        }),
    );
}

fn trial_runner_run_artifact(
    campaign: &ExperimentCampaign,
    trial: &ExperimentTrial,
    completion: &ExperimentRunnerCompletion,
) -> AgentRunArtifact {
    let status = match completion.exit_code {
        Some(0) => AgentRunStatus::Completed,
        _ => AgentRunStatus::Failed,
    };
    let provider_context_refs = [
        Some(format!("experiment_campaign:{}", campaign.id)),
        Some(format!("experiment_trial:{}", trial.id)),
        trial
            .provider_job_id
            .as_ref()
            .map(|value| format!("runner_provider_job:{value}")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    AgentRunArtifact::new(
        "experiment_runner",
        status,
        trial.started_at.unwrap_or_else(Utc::now),
        trial.completed_at,
    )
    .with_failure_reason(match status {
        AgentRunStatus::Failed => completion.summary.clone(),
        _ => None,
    })
    .with_runtime_descriptor(Some(&crate::agent::run_artifact::run_runtime_descriptor(
        &experiment_runner_runtime_descriptor(trial.runner_backend.slug()),
    )))
    .with_prompt_hashes(None, digest_json(&completion.artifact_manifest_json))
    .with_provider_context_refs(provider_context_refs)
    .with_metadata(serde_json::json!({
        "exit_code": completion.exit_code,
        "runtime_ms": completion.runtime_ms,
        "summary": completion.summary,
        "metrics_json": completion.metrics_json,
        "log_preview_path": completion.log_preview_path,
    }))
}

fn research_channel_metadata(
    campaign: &ExperimentCampaign,
    trial_id: Option<Uuid>,
    role: &str,
    target_ids: &[String],
) -> serde_json::Value {
    let mut metadata = serde_json::json!({
        "thread_id": RESEARCH_SUBAGENT_THREAD_ID,
        "reinject_result": false,
        USAGE_TRACKING_EXPERIMENT_CAMPAIGN_ID_KEY: campaign.id.to_string(),
        USAGE_TRACKING_EXPERIMENT_TRIAL_ID_KEY: trial_id.map(|value| value.to_string()),
        USAGE_TRACKING_EXPERIMENT_ROLE_KEY: role,
        USAGE_TRACKING_EXPERIMENT_TARGET_IDS_KEY: target_ids.join(","),
    });
    if let Some(worktree_path) = campaign.worktree_path.as_deref()
        && let Some(object) = metadata.as_object_mut()
    {
        object.insert(
            "tool_base_dir".to_string(),
            serde_json::json!(worktree_path),
        );
        object.insert(
            "tool_working_dir".to_string(),
            serde_json::json!(worktree_path),
        );
    }
    metadata
}

fn research_subagent_run_artifact(
    role_name: &str,
    status: AgentRunStatus,
    started_at: DateTime<Utc>,
    system_prompt: &str,
    task: &str,
    channel_metadata: &serde_json::Value,
    allowed_tools: &[String],
    allowed_skills: &Option<Vec<String>>,
    response_preview: Option<&str>,
    failure_reason: Option<&str>,
) -> AgentRunArtifact {
    let provider_context_refs = [
        channel_metadata
            .get(USAGE_TRACKING_EXPERIMENT_CAMPAIGN_ID_KEY)
            .and_then(|value| value.as_str())
            .map(|value| format!("experiment_campaign:{value}")),
        channel_metadata
            .get(USAGE_TRACKING_EXPERIMENT_TRIAL_ID_KEY)
            .and_then(|value| value.as_str())
            .map(|value| format!("experiment_trial:{value}")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    AgentRunArtifact::new(
        format!("experiment_subagent:{role_name}"),
        status,
        started_at,
        Some(Utc::now()),
    )
    .with_failure_reason(failure_reason.map(str::to_string))
    .with_runtime_descriptor(Some(&crate::agent::run_artifact::run_runtime_descriptor(
        &subagent_executor_runtime_descriptor(),
    )))
    .with_prompt_hashes(
        digest_text(system_prompt),
        digest_json(&serde_json::json!({
            "task": task,
            "channel_metadata": channel_metadata,
            "allowed_tools": allowed_tools,
            "allowed_skills": allowed_skills,
        })),
    )
    .with_provider_context_refs(provider_context_refs)
    .with_metadata(serde_json::json!({
        "role": role_name,
        "response_preview": response_preview.map(|value| truncate_for_prompt(value, 600)),
    }))
}

async fn spawn_research_subagent<T: DeserializeOwned>(
    role_name: &str,
    owner_user_id: &str,
    task: String,
    system_prompt: String,
    channel_metadata: serde_json::Value,
) -> Result<ResearchSubagentOutput<T>, ResearchSubagentError> {
    let started_at = Utc::now();
    let executor = research_subagent_executor().ok_or_else(|| ResearchSubagentError {
        message: research_subagent_executor_unavailable_message().to_string(),
        run_artifact: research_subagent_run_artifact(
            role_name,
            AgentRunStatus::Failed,
            started_at,
            &system_prompt,
            &task,
            &channel_metadata,
            &[],
            &None,
            None,
            Some(research_subagent_executor_unavailable_message()),
        ),
    })?;
    let (allowed_tools, allowed_skills) =
        research_subagent_capabilities(role_name)
            .await
            .map_err(|error| ResearchSubagentError {
                message: error.to_string(),
                run_artifact: research_subagent_run_artifact(
                    role_name,
                    AgentRunStatus::Failed,
                    started_at,
                    &system_prompt,
                    &task,
                    &channel_metadata,
                    &[],
                    &None,
                    None,
                    Some(&error.to_string()),
                ),
            })?;
    let result = executor
        .spawn(
            SubagentSpawnRequest {
                name: format!("Research {role_name}"),
                task: task.clone(),
                system_prompt: Some(system_prompt.clone()),
                model: None,
                task_packet: None,
                memory_mode: None,
                tool_mode: None,
                skill_mode: None,
                tool_profile: None,
                allowed_tools: Some(allowed_tools.clone()),
                allowed_skills: allowed_skills.clone(),
                principal_id: Some(owner_user_id.to_string()),
                actor_id: Some(owner_user_id.to_string()),
                agent_workspace_id: None,
                timeout_secs: Some(300),
                wait: true,
            },
            RESEARCH_SUBAGENT_CHANNEL,
            &channel_metadata,
            owner_user_id,
            None,
            Some(RESEARCH_SUBAGENT_THREAD_ID),
        )
        .await
        .map_err(|e| ResearchSubagentError {
            message: e.to_string(),
            run_artifact: research_subagent_run_artifact(
                role_name,
                AgentRunStatus::Failed,
                started_at,
                &system_prompt,
                &task,
                &channel_metadata,
                &allowed_tools,
                &allowed_skills,
                None,
                Some(&e.to_string()),
            ),
        })?;
    if !result.success {
        let message = result
            .error
            .unwrap_or_else(|| format!("Research {role_name} failed."));
        return Err(ResearchSubagentError {
            message: message.clone(),
            run_artifact: research_subagent_run_artifact(
                role_name,
                AgentRunStatus::Failed,
                started_at,
                &system_prompt,
                &task,
                &channel_metadata,
                &allowed_tools,
                &allowed_skills,
                Some(&result.response),
                Some(&message),
            ),
        });
    }
    let parsed =
        parse_research_json_response(&result.response).map_err(|error| ResearchSubagentError {
            message: error.clone(),
            run_artifact: research_subagent_run_artifact(
                role_name,
                AgentRunStatus::Failed,
                started_at,
                &system_prompt,
                &task,
                &channel_metadata,
                &allowed_tools,
                &allowed_skills,
                Some(&result.response),
                Some(&error),
            ),
        })?;
    let run_artifact = research_subagent_run_artifact(
        role_name,
        AgentRunStatus::Completed,
        started_at,
        &system_prompt,
        &task,
        &channel_metadata,
        &allowed_tools,
        &allowed_skills,
        Some(&result.response),
        None,
    );
    Ok(ResearchSubagentOutput {
        value: parsed,
        run_artifact,
    })
}

async fn research_subagent_capabilities(
    role_name: &str,
) -> ApiResult<(Vec<String>, Option<Vec<String>>)> {
    let executor = research_subagent_executor().ok_or_else(|| {
        ApiError::Unavailable(research_subagent_executor_unavailable_message().to_string())
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

async fn run_planner_subagent(
    store: &Arc<dyn Database>,
    campaign: &ExperimentCampaign,
    project: &ExperimentProject,
    trial_id: Option<Uuid>,
) -> Result<ResearchSubagentOutput<PlannerProposal>, ResearchSubagentInvocationError> {
    let trials = store
        .list_experiment_trials(campaign.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let worktree_path = campaign.worktree_path.as_deref().ok_or_else(|| {
        ApiError::Internal(experiment_campaign_missing_worktree_path_message().to_string())
    })?;
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
        &campaign.owner_user_id,
        task,
        system_prompt,
        research_channel_metadata(campaign, trial_id, "planner", &[]),
    )
    .await
    .map_err(ResearchSubagentInvocationError::from)
}

async fn run_mutator_subagent(
    campaign: &ExperimentCampaign,
    project: &ExperimentProject,
    planner: &PlannerProposal,
    trial_id: Option<Uuid>,
) -> Result<ResearchSubagentOutput<MutatorResult>, ResearchSubagentInvocationError> {
    let worktree_path = campaign.worktree_path.as_deref().ok_or_else(|| {
        ApiError::Internal(experiment_campaign_missing_worktree_path_message().to_string())
    })?;
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
        &campaign.owner_user_id,
        task,
        system_prompt,
        research_channel_metadata(campaign, trial_id, "mutator", &planner.target_ids),
    )
    .await
    .map_err(ResearchSubagentInvocationError::from)
}

async fn run_reviewer_subagent(
    campaign: &ExperimentCampaign,
    project: &ExperimentProject,
    planner: &PlannerProposal,
    diff_stat: &str,
    diff_preview: &str,
    trial_id: Option<Uuid>,
) -> Result<ResearchSubagentOutput<ReviewerDecision>, ResearchSubagentInvocationError> {
    let worktree_path = campaign.worktree_path.as_deref().ok_or_else(|| {
        ApiError::Internal(experiment_campaign_missing_worktree_path_message().to_string())
    })?;
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
        &campaign.owner_user_id,
        task,
        system_prompt,
        research_channel_metadata(campaign, trial_id, "reviewer", &planner.target_ids),
    )
    .await
    .map_err(ResearchSubagentInvocationError::from)
}

async fn execute_local_trial(
    user_id: &str,
    settings: &Settings,
    project: &ExperimentProject,
    runner: &ExperimentRunnerProfile,
    campaign: &ExperimentCampaign,
    trial: &mut ExperimentTrial,
) -> ApiResult<ExperimentRunnerCompletion> {
    let worktree_root = campaign.worktree_path.as_deref().ok_or_else(|| {
        ApiError::Internal(experiment_campaign_missing_worktree_path_field_message().to_string())
    })?;
    let worktree_root = tokio::fs::canonicalize(worktree_root)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to resolve campaign worktree: {e}")))?;
    let workdir_fragment =
        validate_project_workdir_fragment(&project.workdir).map_err(ApiError::InvalidInput)?;
    let run_root = worktree_root.join(workdir_fragment);
    let run_root = tokio::fs::canonicalize(&run_root)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to resolve campaign workdir: {e}")))?;
    if !run_root.starts_with(&worktree_root) {
        return Err(ApiError::InvalidInput(
            experiment_project_workdir_escapes_campaign_worktree_message().to_string(),
        ));
    }
    let started_at = std::time::Instant::now();
    let experiments_data_dir = crate::platform::resolve_data_dir("experiments");
    let log_dir = experiments_data_dir.join("logs");
    let artifact_dir = experiments_data_dir.join("artifacts");
    tokio::fs::create_dir_all(&log_dir)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    tokio::fs::create_dir_all(&artifact_dir)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let log_path = log_dir.join(format!("{}.log", trial.id.simple()));

    trial.status = ExperimentTrialStatus::Running;
    trial.started_at = Some(Utc::now());
    trial.updated_at = Utc::now();

    if let Some(config) =
        agent_env_benchmark_config(&runner.backend_config).map_err(ApiError::InvalidInput)?
    {
        return execute_agent_env_benchmark_trial(
            config,
            &run_root,
            started_at,
            &log_path,
            &artifact_dir,
            trial,
        )
        .await;
    }

    let env_grants = resolved_runner_env_grants(user_id, runner).await;
    let backend = experiment_execution_backend(settings, runner);
    let mut log = String::new();
    if let Some(prepare_command) = project.prepare_command.as_deref() {
        let output = run_experiment_shell_command(
            Arc::clone(&backend),
            &run_root,
            prepare_command,
            &env_grants,
        )
        .await?;
        log.push_str("== prepare ==\n");
        log.push_str(&output.output);
        log.push('\n');
        if output.exit_code != 0 {
            let runtime_ms = (started_at.elapsed().as_nanos() / 1_000_000) as u64;
            tokio::fs::write(&log_path, &log)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            return Ok(ExperimentRunnerCompletion {
                exit_code: Some(output.exit_code as i32),
                metrics_json: serde_json::json!({}),
                summary: Some(format!(
                    "Local prepare command failed with exit code {}.",
                    output.exit_code
                )),
                runtime_ms: Some(runtime_ms),
                attributed_cost_usd: None,
                log_preview_path: Some(log_path.to_string_lossy().to_string()),
                artifact_manifest_json: serde_json::json!({
                    "stage": "prepare",
                    "summary_json_path": run_root.join("summary.json").to_string_lossy(),
                }),
            });
        }
    }
    let run_output = run_experiment_shell_command(
        Arc::clone(&backend),
        &run_root,
        &project.run_command,
        &env_grants,
    )
    .await?;
    let runtime_ms = (started_at.elapsed().as_nanos() / 1_000_000) as u64;
    log.push_str("== run ==\n");
    log.push_str(&run_output.output);
    tokio::fs::write(&log_path, &log)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let summary_path = run_root.join("summary.json");
    let persisted_summary_path = artifact_dir.join(format!("{}-summary.json", trial.id.simple()));
    let summary_json = if summary_path.exists() {
        let raw = tokio::fs::read_to_string(&summary_path)
            .await
            .unwrap_or_default();
        tokio::fs::write(&persisted_summary_path, &raw)
            .await
            .map_err(|e| {
                ApiError::Internal(format!("failed to persist local summary.json: {e}"))
            })?;
        serde_json::from_str::<serde_json::Value>(&raw).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };
    let summary_manifest_path = if summary_path.exists() {
        persisted_summary_path.to_string_lossy().to_string()
    } else {
        summary_path.to_string_lossy().to_string()
    };
    let metrics = extract_metrics(
        &project.primary_metric,
        &project.secondary_metrics,
        &log,
        &summary_json,
    );
    let exit_code = run_output.exit_code as i32;
    if exit_code != 0 {
        return Ok(ExperimentRunnerCompletion {
            exit_code: Some(exit_code),
            metrics_json: serde_json::json!({}),
            summary: Some(format!(
                "Local benchmark command failed with exit code {exit_code}."
            )),
            runtime_ms: Some(runtime_ms),
            attributed_cost_usd: None,
            log_preview_path: Some(log_path.to_string_lossy().to_string()),
            artifact_manifest_json: serde_json::json!({
                "stage": "run",
                "summary_json_path": summary_manifest_path,
            }),
        });
    }
    Ok(ExperimentRunnerCompletion {
        exit_code: Some(exit_code),
        metrics_json: metrics,
        summary: Some(format!("Local {} run completed.", backend.kind().as_str())),
        runtime_ms: Some(runtime_ms),
        attributed_cost_usd: None,
        log_preview_path: Some(log_path.to_string_lossy().to_string()),
        artifact_manifest_json: serde_json::json!({
            "stage": "run",
            "summary_json_path": summary_manifest_path,
        }),
    })
}

async fn execute_agent_env_benchmark_trial(
    config: AgentEnvBenchmarkConfig,
    run_root: &Path,
    started_at: std::time::Instant,
    log_path: &Path,
    artifact_dir: &Path,
    trial: &ExperimentTrial,
) -> ApiResult<ExperimentRunnerCompletion> {
    let trajectories = match config {
        AgentEnvBenchmarkConfig::TerminalBench { cases } => {
            let cases = cases
                .into_iter()
                .map(|mut case| {
                    if case.cwd.is_none() {
                        case.cwd = Some(run_root.to_path_buf());
                    }
                    case
                })
                .collect::<Vec<_>>();
            let mut runner = EnvRunner::new(TerminalBenchEnv::new(cases))
                .with_artifact_root(artifact_dir.join("agent_env_runs"));
            runner
                .evaluate(1, |_| {
                    vec![AgentAction::UserMessage {
                        content: "run terminal_bench".to_string(),
                    }]
                })
                .await
                .map_err(|err| ApiError::Internal(err.to_string()))?
        }
        AgentEnvBenchmarkConfig::SkillBench { cases } => {
            let mut runner = EnvRunner::new(SkillBenchEnv::new(cases))
                .with_artifact_root(artifact_dir.join("agent_env_runs"));
            runner
                .evaluate(1, |_| {
                    vec![AgentAction::UserMessage {
                        content: "run skill_bench".to_string(),
                    }]
                })
                .await
                .map_err(|err| ApiError::Internal(err.to_string()))?
        }
    };

    let runtime_ms = (started_at.elapsed().as_nanos() / 1_000_000) as u64;
    let score = average_trajectory_score(&trajectories);
    let trajectory_path =
        artifact_dir.join(format!("{}-agent-env-trajectory.json", trial.id.simple()));
    let trajectory_json = serde_json::to_string_pretty(&trajectories)
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    tokio::fs::write(&trajectory_path, &trajectory_json)
        .await
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    let log = render_trajectory_log(&trajectories);
    tokio::fs::write(log_path, &log)
        .await
        .map_err(|err| ApiError::Internal(err.to_string()))?;

    Ok(ExperimentRunnerCompletion {
        exit_code: Some(if score >= 1.0 { 0 } else { 1 }),
        metrics_json: serde_json::json!({
            "score": score,
            "episodes": trajectories.len(),
        }),
        summary: Some(format!(
            "AgentEnv benchmark completed with score {score:.3}."
        )),
        runtime_ms: Some(runtime_ms),
        attributed_cost_usd: None,
        log_preview_path: Some(log_path.to_string_lossy().to_string()),
        artifact_manifest_json: serde_json::json!({
            "stage": "agent_env_benchmark",
            "trajectory_json_path": trajectory_path.to_string_lossy(),
            "trajectory_summary": trajectory_summary(&trajectories),
        }),
    })
}

async fn restore_campaign_worktree_after_trial(
    campaign: &ExperimentCampaign,
    restore_commit: Option<&str>,
) -> Result<(), String> {
    let Some(worktree_path) = campaign.worktree_path.as_deref() else {
        return Ok(());
    };

    if let Some(commit) = restore_commit {
        git_output(worktree_path, &["reset", "--hard", commit])
            .await
            .map_err(|error| format!("failed to reset campaign worktree to {commit}: {error}"))?;
    }

    git_output_raw(worktree_path, &["clean", "-fd"])
        .await
        .map_err(|error| format!("failed to clean campaign worktree: {error}"))?;
    Ok(())
}

fn experiment_sandbox_config(settings: &Settings) -> crate::sandbox::SandboxConfig {
    crate::config::SandboxModeConfig::resolve(settings)
        .unwrap_or_else(|_| crate::config::SandboxModeConfig {
            enabled: settings.sandbox.enabled,
            policy: settings.sandbox.policy.clone(),
            timeout_secs: settings.sandbox.timeout_secs,
            memory_limit_mb: settings.sandbox.memory_limit_mb,
            cpu_shares: settings.sandbox.cpu_shares,
            image: settings.sandbox.image.clone(),
            interactive_idle_timeout_secs: settings.sandbox.interactive_idle_timeout_secs,
            auto_pull_image: settings.sandbox.auto_pull_image,
            extra_allowed_domains: settings.sandbox.extra_allowed_domains.clone(),
        })
        .to_sandbox_config()
}

fn experiment_execution_backend(
    settings: &Settings,
    runner: &ExperimentRunnerProfile,
) -> Arc<dyn ExecutionBackend> {
    match runner.backend {
        ExperimentRunnerBackend::LocalDocker => {
            let mut sandbox_config = experiment_sandbox_config(settings);
            sandbox_config.enabled = true;
            sandbox_config.policy = crate::sandbox::SandboxPolicy::WorkspaceWrite;
            if let Some(image) = runner
                .image_or_runtime
                .as_ref()
                .filter(|value| !value.trim().is_empty())
            {
                sandbox_config.image = image.trim().to_string();
            }
            DockerSandboxExecutionBackend::from_sandbox(
                Arc::new(crate::sandbox::SandboxManager::new(sandbox_config)),
                crate::sandbox::SandboxPolicy::WorkspaceWrite,
            )
        }
        _ => LocalHostExecutionBackend::shared(),
    }
}

async fn run_experiment_shell_command(
    backend: Arc<dyn ExecutionBackend>,
    cwd: &Path,
    command: &str,
    env_grants: &serde_json::Value,
) -> ApiResult<ExecutionResult> {
    backend
        .run_shell(CommandExecutionRequest {
            command: command.to_string(),
            workdir: cwd.to_path_buf(),
            timeout: TokioDuration::from_secs(600),
            extra_env: env_pairs_from_json(env_grants).into_iter().collect(),
            allow_network: false,
        })
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))
}

async fn run_command_capture(
    cwd: Option<&Path>,
    binary: &str,
    args: &[&str],
    env: &[(String, String)],
) -> ApiResult<String> {
    let output = LocalHostExecutionBackend::shared()
        .run_script(ScriptExecutionRequest {
            program: binary.to_string(),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
            workdir: cwd
                .map(Path::to_path_buf)
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))),
            timeout: TokioDuration::from_secs(600),
            extra_env: env.iter().cloned().collect(),
            allow_network: true,
        })
        .await
        .map_err(|e| ApiError::Internal(format!("failed to run {binary}: {e}")))?;
    let mut text = output.stdout.clone();
    if !output.stderr.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&output.stderr);
    }
    if output.exit_code != 0 {
        return Err(ApiError::Internal(format!(
            "{binary} exited with status {}{}",
            output.exit_code,
            if text.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", text.trim())
            }
        )));
    }
    Ok(text)
}

async fn git_output(cwd: &str, args: &[&str]) -> ApiResult<String> {
    let output = run_command_capture(Some(Path::new(cwd)), "git", args, &[]).await?;
    Ok(output.trim().to_string())
}

async fn git_output_raw(cwd: &str, args: &[&str]) -> ApiResult<String> {
    run_command_capture(Some(Path::new(cwd)), "git", args, &[]).await
}

async fn git_run(cwd: &str, prefix_args: &[&str], extra_args: &[&str]) -> ApiResult<()> {
    let mut args = prefix_args.to_vec();
    args.extend_from_slice(extra_args);
    let _ = git_output(cwd, &args).await?;
    Ok(())
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

fn ensure_unique_target_signature(
    kind: ExperimentTargetKind,
    metadata: &serde_json::Value,
    skip_target_id: Option<Uuid>,
    targets: &[ExperimentTarget],
) -> ApiResult<()> {
    ensure_unique_target_signature_policy(kind, metadata, skip_target_id, targets)
        .map_err(ApiError::InvalidInput)
}

#[cfg(test)]
mod tests {
    use super::{git_changed_files, record_campaign_candidate_generation};
    use crate::agent::subagent_executor::{SubagentConfig, SubagentExecutor};
    use crate::agent::{AgentRunArtifact, AgentRunStatus};
    use crate::channels::ChannelManager;
    use crate::experiments::{
        ExperimentAutonomyMode, ExperimentCampaign, ExperimentCampaignQueueState,
        ExperimentCampaignStatus, ExperimentLease, ExperimentLeaseStatus,
        ExperimentMetricComparator, ExperimentMetricDefinition, ExperimentProject,
        ExperimentProjectStatus, ExperimentRunnerBackend, ExperimentRunnerCompletion,
        ExperimentRunnerProfile, ExperimentRunnerStatus, ExperimentTrial, ExperimentTrialStatus,
    };
    use crate::llm::{
        ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, Role,
        ToolCall, ToolCompletionRequest, ToolCompletionResponse,
    };
    use crate::tools::ToolRegistry;
    use crate::tools::builtin::{
        ApplyPatchTool, ListDirTool, ReadFileTool, SearchFilesTool, WriteFileTool,
    };
    use async_trait::async_trait;
    use chrono::Utc;
    use rust_decimal::Decimal;
    use std::path::Path;
    use std::process::Command;
    use std::sync::Arc;
    use tempfile::TempDir;
    use uuid::Uuid;

    struct AutonomousResearchTestLlm;

    impl AutonomousResearchTestLlm {
        fn response_for_messages(
            &self,
            messages: &[ChatMessage],
        ) -> (Option<String>, Vec<ToolCall>, FinishReason) {
            let joined = messages
                .iter()
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>()
                .join("\n");

            if joined.contains("planning role for ThinClaw Research") {
                return (
                    Some(
                        serde_json::json!({
                            "hypothesis": "Switch app.txt to the candidate configuration to improve score.",
                            "target_ids": ["app-config"],
                            "allowed_paths": ["app.txt"],
                            "expected_metric_direction": "increase",
                            "mutation_brief": "Rewrite app.txt with the candidate configuration."
                        })
                        .to_string(),
                    ),
                    Vec::new(),
                    FinishReason::Stop,
                );
            }

            if joined.contains("mutator role for ThinClaw Research") {
                let wrote_file = messages.iter().any(|message| {
                    message.role == Role::Tool && message.name.as_deref() == Some("write_file")
                });
                if !wrote_file {
                    return (
                        None,
                        vec![ToolCall {
                            id: "mutator_write_app".to_string(),
                            name: "write_file".to_string(),
                            arguments: serde_json::json!({
                                "path": "app.txt",
                                "content": "candidate\n",
                            }),
                        }],
                        FinishReason::ToolUse,
                    );
                }
                return (
                    Some(
                        serde_json::json!({
                            "changed_paths": ["app.txt"],
                            "mutation_summary": "Updated app.txt to the candidate configuration."
                        })
                        .to_string(),
                    ),
                    Vec::new(),
                    FinishReason::Stop,
                );
            }

            if joined.contains("reviewer role for ThinClaw Research") {
                return (
                    Some(
                        serde_json::json!({
                            "approved": true,
                            "scope_ok": true,
                            "benchmark_ready": true,
                            "reason": "approved"
                        })
                        .to_string(),
                    ),
                    Vec::new(),
                    FinishReason::Stop,
                );
            }

            (Some("{}".to_string()), Vec::new(), FinishReason::Stop)
        }
    }

    #[async_trait]
    impl LlmProvider for AutonomousResearchTestLlm {
        fn model_name(&self) -> &str {
            "autonomous-research-test"
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }

        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> Result<CompletionResponse, crate::error::LlmError> {
            let (content, _, finish_reason) = self.response_for_messages(&request.messages);
            Ok(CompletionResponse {
                content: content.unwrap_or_default(),
                provider_model: Some(self.model_name().to_string()),
                cost_usd: None,
                thinking_content: None,
                input_tokens: 32,
                output_tokens: 24,
                finish_reason,
                token_capture: None,
            })
        }

        async fn complete_with_tools(
            &self,
            request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, crate::error::LlmError> {
            let (content, tool_calls, finish_reason) =
                self.response_for_messages(&request.messages);
            Ok(ToolCompletionResponse {
                content,
                provider_model: Some(self.model_name().to_string()),
                cost_usd: None,
                tool_calls,
                thinking_content: None,
                input_tokens: 32,
                output_tokens: 24,
                finish_reason,
                token_capture: None,
            })
        }
    }

    async fn ensure_test_research_subagent_executor() {
        if super::research_subagent_executor().is_some() {
            return;
        }

        let llm = Arc::new(AutonomousResearchTestLlm);
        let safety = Arc::new(crate::safety::SafetyLayer::new(
            &crate::config::SafetyConfig {
                max_output_length: 100_000,
                injection_check_enabled: false,
                redact_pii_in_prompts: true,
                smart_approval_mode: "off".to_string(),
                external_scanner_mode: "off".to_string(),
                external_scanner_path: None,
                external_scanner_require_verified: false,
            },
        ));
        let tools = Arc::new(ToolRegistry::new());
        tools.register_sync(Arc::new(ReadFileTool::new()));
        tools.register_sync(Arc::new(WriteFileTool::new()));
        tools.register_sync(Arc::new(ListDirTool::new()));
        tools.register_sync(Arc::new(ApplyPatchTool::new()));
        tools.register_sync(Arc::new(SearchFilesTool::new()));

        let channels = Arc::new(ChannelManager::new());
        let (executor, _result_rx) =
            SubagentExecutor::new(llm, safety, tools, channels, SubagentConfig::default());
        super::register_experiment_subagent_executor(Arc::new(executor));
    }

    #[test]
    fn record_campaign_candidate_generation_tracks_last_failure_and_artifacts() {
        let mut campaign = ExperimentCampaign {
            id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            runner_profile_id: Uuid::new_v4(),
            owner_user_id: "default".to_string(),
            status: ExperimentCampaignStatus::Paused,
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
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let artifact = AgentRunArtifact::new(
            "experiment_subagent:planner",
            AgentRunStatus::Failed,
            Utc::now(),
            Some(Utc::now()),
        )
        .with_failure_reason(Some("planner failed".to_string()));

        record_campaign_candidate_generation(
            &mut campaign,
            "autonomous",
            "failed",
            "planner failed",
            &[artifact.clone()],
        );

        let artifacts = campaign
            .metadata
            .get("run_artifacts")
            .and_then(|value| value.as_array())
            .expect("run artifacts should be recorded");
        assert_eq!(artifacts.len(), 1);
        assert_eq!(
            campaign.metadata["candidate_generation"]["mode"].as_str(),
            Some("autonomous")
        );
        assert_eq!(
            campaign.metadata["candidate_generation"]["status"].as_str(),
            Some("failed")
        );
        assert_eq!(
            campaign.metadata["candidate_generation"]["artifact_run_ids"][0].as_str(),
            Some(artifact.run_id.as_str())
        );
    }

    #[test]
    fn research_subagent_tool_denylist_blocks_memory_and_session_recall() {
        for tool_name in ["memory_read", "memory_search", "session_search"] {
            assert!(
                super::RESEARCH_SHARED_TOOL_DENYLIST.contains(&tool_name),
                "{tool_name} should be denied for research subagents"
            );
        }
    }

    #[tokio::test]
    async fn git_changed_files_reports_modified_path() {
        let repo = TempDir::new().expect("temp repo");
        git(repo.path(), &["init"]);
        git(repo.path(), &["config", "user.email", "tests@example.com"]);
        git(repo.path(), &["config", "user.name", "ThinClaw Tests"]);
        std::fs::write(repo.path().join("app.txt"), "baseline\n").expect("write app file");
        git(repo.path(), &["add", "app.txt"]);
        git(repo.path(), &["commit", "-m", "initial"]);
        std::fs::write(repo.path().join("app.txt"), "candidate\n").expect("rewrite app file");

        let changed = git_changed_files(&repo.path().to_string_lossy())
            .await
            .expect("changed files");
        assert_eq!(changed, vec!["app.txt".to_string()]);
    }

    #[tokio::test]
    async fn git_changed_files_reports_rename_destination_path() {
        let repo = TempDir::new().expect("temp repo");
        git(repo.path(), &["init"]);
        git(repo.path(), &["config", "user.email", "tests@example.com"]);
        git(repo.path(), &["config", "user.name", "ThinClaw Tests"]);
        std::fs::write(repo.path().join("before.txt"), "hello\n").expect("write before file");
        git(repo.path(), &["add", "before.txt"]);
        git(repo.path(), &["commit", "-m", "initial"]);
        git(repo.path(), &["mv", "before.txt", "after.txt"]);

        let changed = git_changed_files(&repo.path().to_string_lossy())
            .await
            .expect("changed files");
        assert_eq!(changed, vec!["after.txt".to_string()]);
    }

    #[tokio::test]
    async fn launch_campaign_baseline_runs_local_docker_trial_end_to_end() {
        let mut settings = crate::settings::Settings::default();
        settings.sandbox.enabled = true;
        let sandbox =
            crate::sandbox::SandboxManager::new(super::experiment_sandbox_config(&settings));
        if !sandbox.is_available().await {
            eprintln!("skipping docker-backed experiment test because sandbox is unavailable");
            return;
        }

        let (store, _guard) = crate::testing::test_db().await;
        let repo = TempDir::new().expect("temp repo");
        git(repo.path(), &["init"]);
        git(repo.path(), &["checkout", "-b", "main"]);
        git(repo.path(), &["config", "user.email", "tests@example.com"]);
        git(repo.path(), &["config", "user.name", "ThinClaw Tests"]);
        std::fs::write(repo.path().join("app.txt"), "baseline\n").expect("write repo file");
        git(repo.path(), &["add", "app.txt"]);
        git(repo.path(), &["commit", "-m", "initial"]);

        let now = Utc::now();
        let project = ExperimentProject {
            id: Uuid::new_v4(),
            name: "docker-baseline".to_string(),
            workspace_path: repo.path().to_string_lossy().to_string(),
            git_remote_name: "origin".to_string(),
            base_branch: "main".to_string(),
            preset: Default::default(),
            strategy_prompt: "Validate baseline execution".to_string(),
            workdir: ".".to_string(),
            prepare_command: None,
            run_command: "printf '{\"score\":1}\\n' > summary.json && echo benchmark-ok"
                .to_string(),
            mutable_paths: vec!["app.txt".to_string()],
            fixed_paths: Vec::new(),
            primary_metric: ExperimentMetricDefinition {
                name: "score".to_string(),
                regex: None,
                json_path: Some("score".to_string()),
                comparator: ExperimentMetricComparator::HigherIsBetter,
            },
            secondary_metrics: Vec::new(),
            comparison_policy: Default::default(),
            stop_policy: Default::default(),
            default_runner_profile_id: None,
            promotion_mode: "manual".to_string(),
            autonomy_mode: Default::default(),
            status: ExperimentProjectStatus::Ready,
            created_at: now,
            updated_at: now,
        };
        store
            .create_experiment_project(&project)
            .await
            .expect("store project");

        let runner = ExperimentRunnerProfile {
            id: Uuid::new_v4(),
            name: "local-docker".to_string(),
            backend: ExperimentRunnerBackend::LocalDocker,
            backend_config: serde_json::json!({}),
            image_or_runtime: Some("alpine:3.20".to_string()),
            gpu_requirements: serde_json::json!({}),
            env_grants: serde_json::json!({}),
            secret_references: Vec::new(),
            cache_policy: serde_json::json!({}),
            status: ExperimentRunnerStatus::Validated,
            readiness_class: crate::experiments::ExperimentRunnerReadinessClass::LaunchReady,
            launch_eligible: true,
            created_at: now,
            updated_at: now,
        };
        store
            .create_experiment_runner_profile(&runner)
            .await
            .expect("store runner");

        let campaign_id = Uuid::new_v4();
        let worktree_path = super::experiments_worktree_path(&project.workspace_path, campaign_id);
        let campaign = ExperimentCampaign {
            id: campaign_id,
            project_id: project.id,
            runner_profile_id: runner.id,
            owner_user_id: "owner-a".to_string(),
            status: ExperimentCampaignStatus::PendingBaseline,
            baseline_commit: None,
            best_commit: None,
            best_metrics: serde_json::json!({}),
            experiment_branch: Some(format!(
                "codex/experiments/{}",
                super::short_id(campaign_id)
            )),
            remote_ref: None,
            worktree_path: Some(worktree_path.to_string_lossy().to_string()),
            started_at: Some(now),
            ended_at: None,
            trial_count: 0,
            failure_count: 0,
            pause_reason: Some("Pending baseline launch.".to_string()),
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
        };
        store
            .create_experiment_campaign(&campaign)
            .await
            .expect("store campaign");

        let response = super::launch_campaign_baseline(
            &store, "owner-a", &settings, &project, &runner, campaign,
        )
        .await
        .expect("launch baseline");

        let trial = response.trial.expect("trial should be recorded");
        assert_eq!(trial.exit_code, Some(0));
        assert_eq!(trial.metrics_json["score"], 1.0);
        let worktree_path = Path::new(
            response
                .campaign
                .worktree_path
                .as_deref()
                .expect("worktree path"),
        );
        let summary_path = Path::new(
            trial.artifact_manifest_json["summary_json_path"]
                .as_str()
                .expect("summary json path"),
        );
        assert!(summary_path.exists(), "summary.json should exist");
        assert!(
            !summary_path.starts_with(worktree_path),
            "summary artifact should be persisted outside the campaign worktree"
        );
        let run_log =
            std::fs::read_to_string(trial.log_preview_path.as_deref().expect("log preview path"))
                .expect("read run log");
        assert!(
            run_log.contains("benchmark-ok"),
            "unexpected run log: {run_log}"
        );
        assert!(worktree_path.exists(), "campaign worktree should exist");
        assert!(
            super::git_changed_files(&worktree_path.to_string_lossy())
                .await
                .expect("list changed files after baseline")
                .is_empty(),
            "baseline run should restore the campaign worktree to a clean state"
        );
    }

    #[tokio::test]
    async fn agent_env_terminal_bench_completion_writes_metrics_and_artifact() {
        let dir = TempDir::new().expect("tempdir");
        let run_root = dir.path().join("run");
        let artifact_dir = dir.path().join("artifacts");
        std::fs::create_dir_all(&run_root).expect("run root");
        std::fs::create_dir_all(&artifact_dir).expect("artifact root");
        let log_path = dir.path().join("bench.log");
        let now = Utc::now();
        let trial = ExperimentTrial {
            id: Uuid::new_v4(),
            campaign_id: Uuid::new_v4(),
            sequence: 1,
            candidate_commit: None,
            parent_best_commit: None,
            status: ExperimentTrialStatus::Running,
            runner_backend: ExperimentRunnerBackend::LocalDocker,
            exit_code: None,
            metrics_json: serde_json::json!({}),
            summary: None,
            decision_reason: None,
            artifact_manifest_json: serde_json::json!({}),
            log_preview_path: None,
            reviewer_decision: None,
            runtime_ms: None,
            attributed_cost_usd: None,
            llm_cost_usd: None,
            runner_cost_usd: None,
            hypothesis: None,
            mutation_summary: None,
            provider_job_id: None,
            provider_job_metadata: serde_json::json!({}),
            started_at: Some(now),
            completed_at: None,
            created_at: now,
            updated_at: now,
        };

        let completion = super::execute_agent_env_benchmark_trial(
            super::AgentEnvBenchmarkConfig::TerminalBench {
                cases: vec![crate::agent::env::TerminalBenchCase {
                    name: "echo".to_string(),
                    command: "printf agent-env-ok".to_string(),
                    cwd: None,
                    expected_stdout_contains: vec!["agent-env-ok".to_string()],
                    expected_exit_code: Some(0),
                    timeout_secs: 5,
                }],
            },
            &run_root,
            std::time::Instant::now(),
            &log_path,
            &artifact_dir,
            &trial,
        )
        .await
        .expect("agent env benchmark completion");

        assert_eq!(completion.exit_code, Some(0));
        assert_eq!(completion.metrics_json["score"], 1.0);
        assert_eq!(
            completion.artifact_manifest_json["stage"],
            serde_json::json!("agent_env_benchmark")
        );
        assert_eq!(
            completion.artifact_manifest_json["trajectory_summary"]["env_names"][0],
            serde_json::json!("terminal_bench")
        );
        assert_eq!(
            completion.artifact_manifest_json["trajectory_summary"]["token_capture_steps"],
            serde_json::json!(1)
        );
        let trajectory_path = Path::new(
            completion.artifact_manifest_json["trajectory_json_path"]
                .as_str()
                .expect("trajectory path"),
        );
        assert!(trajectory_path.exists());
        let log = std::fs::read_to_string(log_path).expect("read log");
        assert!(log.contains("agent-env-ok"));
    }

    #[tokio::test]
    async fn agent_env_skill_bench_completion_writes_metrics_and_artifact() {
        let dir = TempDir::new().expect("tempdir");
        let run_root = dir.path().join("run");
        let artifact_dir = dir.path().join("artifacts");
        std::fs::create_dir_all(&run_root).expect("run root");
        std::fs::create_dir_all(&artifact_dir).expect("artifact root");
        let log_path = dir.path().join("skill-bench.log");
        let now = Utc::now();
        let trial = ExperimentTrial {
            id: Uuid::new_v4(),
            campaign_id: Uuid::new_v4(),
            sequence: 1,
            candidate_commit: None,
            parent_best_commit: None,
            status: ExperimentTrialStatus::Running,
            runner_backend: ExperimentRunnerBackend::LocalDocker,
            exit_code: None,
            metrics_json: serde_json::json!({}),
            summary: None,
            decision_reason: None,
            artifact_manifest_json: serde_json::json!({}),
            log_preview_path: None,
            reviewer_decision: None,
            runtime_ms: None,
            attributed_cost_usd: None,
            llm_cost_usd: None,
            runner_cost_usd: None,
            hypothesis: None,
            mutation_summary: None,
            provider_job_id: None,
            provider_job_metadata: serde_json::json!({}),
            started_at: Some(now),
            completed_at: None,
            created_at: now,
            updated_at: now,
        };

        let completion = super::execute_agent_env_benchmark_trial(
            super::AgentEnvBenchmarkConfig::SkillBench {
                cases: vec![crate::agent::env::SkillBenchCase {
                    name: "minimal-skill".to_string(),
                    skill_content: "# Skill\n\nUse this skill carefully.".to_string(),
                    required_substrings: vec!["carefully".to_string()],
                }],
            },
            &run_root,
            std::time::Instant::now(),
            &log_path,
            &artifact_dir,
            &trial,
        )
        .await
        .expect("agent env skill benchmark completion");

        assert_eq!(completion.exit_code, Some(0));
        assert_eq!(completion.metrics_json["score"], 1.0);
        assert_eq!(
            completion.artifact_manifest_json["stage"],
            serde_json::json!("agent_env_benchmark")
        );
        assert_eq!(
            completion.artifact_manifest_json["trajectory_summary"]["env_names"][0],
            serde_json::json!("skill_bench")
        );
        let trajectory_path = Path::new(
            completion.artifact_manifest_json["trajectory_json_path"]
                .as_str()
                .expect("trajectory path"),
        );
        assert!(trajectory_path.exists());
        let trajectory_json =
            std::fs::read_to_string(trajectory_path).expect("read trajectory json");
        assert!(trajectory_json.contains("skill_bench"));
        let log = std::fs::read_to_string(log_path).expect("read log");
        assert!(log.contains("minimal-skill"));
    }

    #[tokio::test]
    async fn local_trial_artifact_refs_include_agent_env_paths() {
        let (store, _guard) = crate::testing::test_db().await;
        let dir = TempDir::new().expect("tempdir");
        let trajectory_path = dir.path().join("trajectory.json");
        let log_path = dir.path().join("trial.log");
        std::fs::write(&trajectory_path, "[]").expect("write trajectory");
        std::fs::write(&log_path, "log").expect("write log");
        let now = Utc::now();
        let project = ExperimentProject {
            id: Uuid::new_v4(),
            name: "artifact-ref-project".to_string(),
            workspace_path: dir.path().to_string_lossy().to_string(),
            git_remote_name: "origin".to_string(),
            base_branch: "main".to_string(),
            preset: Default::default(),
            strategy_prompt: "Verify artifact refs".to_string(),
            workdir: ".".to_string(),
            prepare_command: None,
            run_command: "true".to_string(),
            mutable_paths: Vec::new(),
            fixed_paths: Vec::new(),
            primary_metric: ExperimentMetricDefinition {
                name: "score".to_string(),
                regex: None,
                json_path: Some("score".to_string()),
                comparator: ExperimentMetricComparator::HigherIsBetter,
            },
            secondary_metrics: Vec::new(),
            comparison_policy: Default::default(),
            stop_policy: Default::default(),
            default_runner_profile_id: None,
            promotion_mode: "manual".to_string(),
            autonomy_mode: Default::default(),
            status: ExperimentProjectStatus::Ready,
            created_at: now,
            updated_at: now,
        };
        store
            .create_experiment_project(&project)
            .await
            .expect("store project");
        let runner = ExperimentRunnerProfile {
            id: Uuid::new_v4(),
            name: "artifact-ref-runner".to_string(),
            backend: ExperimentRunnerBackend::AgentEnv,
            backend_config: serde_json::json!({}),
            image_or_runtime: None,
            gpu_requirements: serde_json::json!({}),
            env_grants: serde_json::json!({}),
            secret_references: Vec::new(),
            cache_policy: serde_json::json!({}),
            status: ExperimentRunnerStatus::Validated,
            readiness_class: crate::experiments::ExperimentRunnerReadinessClass::LaunchReady,
            launch_eligible: true,
            created_at: now,
            updated_at: now,
        };
        store
            .create_experiment_runner_profile(&runner)
            .await
            .expect("store runner");
        let campaign = ExperimentCampaign {
            id: Uuid::new_v4(),
            project_id: project.id,
            runner_profile_id: runner.id,
            owner_user_id: "owner-a".to_string(),
            status: ExperimentCampaignStatus::Running,
            baseline_commit: None,
            best_commit: None,
            best_metrics: serde_json::json!({}),
            experiment_branch: None,
            remote_ref: None,
            worktree_path: None,
            started_at: Some(now),
            ended_at: None,
            trial_count: 1,
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
        };
        store
            .create_experiment_campaign(&campaign)
            .await
            .expect("store campaign");
        let trial = ExperimentTrial {
            id: Uuid::new_v4(),
            campaign_id: campaign.id,
            sequence: 1,
            candidate_commit: None,
            parent_best_commit: None,
            status: ExperimentTrialStatus::Running,
            runner_backend: ExperimentRunnerBackend::AgentEnv,
            exit_code: Some(0),
            metrics_json: serde_json::json!({}),
            summary: None,
            decision_reason: None,
            artifact_manifest_json: serde_json::json!({
                "trajectory_json_path": trajectory_path.to_string_lossy(),
            }),
            log_preview_path: Some(log_path.to_string_lossy().to_string()),
            reviewer_decision: None,
            runtime_ms: None,
            attributed_cost_usd: None,
            llm_cost_usd: None,
            runner_cost_usd: None,
            hypothesis: None,
            mutation_summary: None,
            provider_job_id: None,
            provider_job_metadata: serde_json::json!({}),
            started_at: Some(now),
            completed_at: Some(now),
            created_at: now,
            updated_at: now,
        };
        store
            .create_experiment_trial(&trial)
            .await
            .expect("store trial");

        super::upsert_local_trial_artifact_refs(&store, &trial)
            .await
            .expect("upsert local refs");
        let artifacts = store
            .list_experiment_artifacts(trial.id)
            .await
            .expect("list refs");
        let kinds = artifacts
            .iter()
            .map(|artifact| artifact.kind.as_str())
            .collect::<std::collections::HashSet<_>>();
        assert!(kinds.contains("trajectory_json"));
        assert!(kinds.contains("log_preview"));
    }

    #[tokio::test]
    async fn autonomous_campaign_runs_planner_mutator_reviewer_and_docker_trial_end_to_end() {
        let mut settings = crate::settings::Settings::default();
        settings.sandbox.enabled = true;
        let sandbox =
            crate::sandbox::SandboxManager::new(super::experiment_sandbox_config(&settings));
        if !sandbox.is_available().await {
            eprintln!(
                "skipping autonomous docker-backed experiment test because sandbox is unavailable"
            );
            return;
        }

        ensure_test_research_subagent_executor().await;

        let (store, _guard) = crate::testing::test_db().await;
        let repo = TempDir::new().expect("temp repo");
        git(repo.path(), &["init"]);
        git(repo.path(), &["checkout", "-b", "main"]);
        git(repo.path(), &["config", "user.email", "tests@example.com"]);
        git(repo.path(), &["config", "user.name", "ThinClaw Tests"]);
        std::fs::write(repo.path().join("app.txt"), "baseline\n").expect("write repo file");
        git(repo.path(), &["add", "app.txt"]);
        git(repo.path(), &["commit", "-m", "initial"]);

        let now = Utc::now();
        let project = ExperimentProject {
            id: Uuid::new_v4(),
            name: "autonomous-docker".to_string(),
            workspace_path: repo.path().to_string_lossy().to_string(),
            git_remote_name: "origin".to_string(),
            base_branch: "main".to_string(),
            preset: Default::default(),
            strategy_prompt: "Autonomously improve app.txt".to_string(),
            workdir: ".".to_string(),
            prepare_command: None,
            run_command: "if grep -q candidate app.txt; then printf '{\"score\":2}\\n' > summary.json && echo improved; else printf '{\"score\":1}\\n' > summary.json && echo baseline; fi".to_string(),
            mutable_paths: vec!["app.txt".to_string()],
            fixed_paths: Vec::new(),
            primary_metric: ExperimentMetricDefinition {
                name: "score".to_string(),
                regex: None,
                json_path: Some("score".to_string()),
                comparator: ExperimentMetricComparator::HigherIsBetter,
            },
            secondary_metrics: Vec::new(),
            comparison_policy: Default::default(),
            stop_policy: Default::default(),
            default_runner_profile_id: None,
            promotion_mode: "manual".to_string(),
            autonomy_mode: ExperimentAutonomyMode::Autonomous,
            status: ExperimentProjectStatus::Ready,
            created_at: now,
            updated_at: now,
        };
        store
            .create_experiment_project(&project)
            .await
            .expect("store project");

        let runner = ExperimentRunnerProfile {
            id: Uuid::new_v4(),
            name: "local-docker".to_string(),
            backend: ExperimentRunnerBackend::LocalDocker,
            backend_config: serde_json::json!({}),
            image_or_runtime: Some("alpine:3.20".to_string()),
            gpu_requirements: serde_json::json!({}),
            env_grants: serde_json::json!({}),
            secret_references: Vec::new(),
            cache_policy: serde_json::json!({}),
            status: ExperimentRunnerStatus::Validated,
            readiness_class: crate::experiments::ExperimentRunnerReadinessClass::LaunchReady,
            launch_eligible: true,
            created_at: now,
            updated_at: now,
        };
        store
            .create_experiment_runner_profile(&runner)
            .await
            .expect("store runner");

        let campaign_id = Uuid::new_v4();
        let worktree_path = super::experiments_worktree_path(&project.workspace_path, campaign_id);
        let campaign = ExperimentCampaign {
            id: campaign_id,
            project_id: project.id,
            runner_profile_id: runner.id,
            owner_user_id: "owner-a".to_string(),
            status: ExperimentCampaignStatus::PendingBaseline,
            baseline_commit: None,
            best_commit: None,
            best_metrics: serde_json::json!({}),
            experiment_branch: Some(format!(
                "codex/experiments/{}",
                super::short_id(campaign_id)
            )),
            remote_ref: None,
            worktree_path: Some(worktree_path.to_string_lossy().to_string()),
            started_at: Some(now),
            ended_at: None,
            trial_count: 0,
            failure_count: 0,
            pause_reason: Some("Pending baseline launch.".to_string()),
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
        };
        store
            .create_experiment_campaign(&campaign)
            .await
            .expect("store campaign");

        let baseline_response = super::launch_campaign_baseline(
            &store, "owner-a", &settings, &project, &runner, campaign,
        )
        .await
        .expect("launch baseline");
        let baseline_trial = baseline_response.trial.expect("baseline trial");
        assert_eq!(baseline_trial.metrics_json["score"], 1.0);

        let mut active_campaign = baseline_response.campaign;
        assert!(
            super::git_changed_files(
                active_campaign
                    .worktree_path
                    .as_deref()
                    .expect("campaign worktree path"),
            )
            .await
            .expect("list changed files after baseline")
            .is_empty(),
            "baseline run should leave the campaign worktree clean before autonomous mutation"
        );
        super::launch_next_trial_if_ready(
            &store,
            "owner-a",
            &settings,
            &project,
            &runner,
            &mut active_campaign,
        )
        .await
        .expect("autonomous follow-up trial should succeed");

        let trials = store
            .list_experiment_trials(active_campaign.id)
            .await
            .expect("list trials");
        assert_eq!(
            trials.len(),
            2,
            "unexpected trial count; campaign_status={:?}; pause_reason={:?}; metadata={}",
            active_campaign.status,
            active_campaign.pause_reason,
            active_campaign.metadata
        );

        let autonomous_trial = trials.last().expect("autonomous trial");
        assert_eq!(autonomous_trial.sequence, 2);
        assert_eq!(autonomous_trial.status, ExperimentTrialStatus::Accepted);
        assert_eq!(autonomous_trial.metrics_json["score"], 2.0);
        assert_eq!(
            autonomous_trial.reviewer_decision.as_deref(),
            Some("approved")
        );
        assert_eq!(
            autonomous_trial.mutation_summary.as_deref(),
            Some("Updated app.txt to the candidate configuration.")
        );
        assert_ne!(
            autonomous_trial.candidate_commit,
            baseline_trial.candidate_commit
        );

        let run_artifacts = autonomous_trial
            .artifact_manifest_json
            .get("run_artifacts")
            .and_then(|value| value.as_array())
            .expect("run artifacts should be present");
        let sources = run_artifacts
            .iter()
            .filter_map(|artifact| artifact.get("source").and_then(|value| value.as_str()))
            .collect::<Vec<_>>();
        assert!(sources.contains(&"experiment_subagent:planner"));
        assert!(sources.contains(&"experiment_subagent:mutator"));
        assert!(sources.contains(&"experiment_subagent:reviewer"));
        assert!(sources.contains(&"experiment_runner"));

        assert_eq!(active_campaign.best_metrics["score"], 2.0);
        assert!(
            super::git_changed_files(
                active_campaign
                    .worktree_path
                    .as_deref()
                    .expect("campaign worktree path"),
            )
            .await
            .expect("list changed files after autonomous trial")
            .is_empty(),
            "autonomous trial should also leave the campaign worktree clean"
        );
    }

    #[test]
    fn normalize_trial_completion_adds_default_stage() {
        let completion = ExperimentRunnerCompletion {
            exit_code: Some(0),
            metrics_json: serde_json::json!({}),
            summary: Some("ok".to_string()),
            runtime_ms: Some(42),
            attributed_cost_usd: None,
            log_preview_path: None,
            artifact_manifest_json: serde_json::Value::Null,
        };

        let normalized = super::normalize_trial_completion(completion);
        assert_eq!(
            normalized
                .artifact_manifest_json
                .get("stage")
                .and_then(|value| value.as_str()),
            Some("complete")
        );
    }

    #[tokio::test]
    async fn complete_trial_terminal_rejects_repeated_completed_lease() {
        let (store, _guard) = crate::testing::test_db().await;
        let now = Utc::now();
        let project = ExperimentProject {
            id: Uuid::new_v4(),
            name: "demo".to_string(),
            workspace_path: ".".to_string(),
            git_remote_name: "origin".to_string(),
            base_branch: "main".to_string(),
            preset: Default::default(),
            strategy_prompt: "demo".to_string(),
            workdir: ".".to_string(),
            prepare_command: None,
            run_command: "echo ok".to_string(),
            mutable_paths: vec!["src".to_string()],
            fixed_paths: Vec::new(),
            primary_metric: ExperimentMetricDefinition::default(),
            secondary_metrics: Vec::new(),
            comparison_policy: Default::default(),
            stop_policy: Default::default(),
            default_runner_profile_id: None,
            promotion_mode: "manual".to_string(),
            autonomy_mode: Default::default(),
            status: ExperimentProjectStatus::Ready,
            created_at: now,
            updated_at: now,
        };
        let mut campaign = ExperimentCampaign {
            id: Uuid::new_v4(),
            project_id: project.id,
            runner_profile_id: Uuid::new_v4(),
            owner_user_id: "owner-a".to_string(),
            status: ExperimentCampaignStatus::Running,
            baseline_commit: None,
            best_commit: None,
            best_metrics: serde_json::json!({}),
            experiment_branch: None,
            remote_ref: None,
            worktree_path: None,
            started_at: Some(now),
            ended_at: None,
            trial_count: 1,
            failure_count: 0,
            pause_reason: None,
            queue_state: ExperimentCampaignQueueState::Active,
            queue_position: 0,
            active_trial_id: Some(Uuid::new_v4()),
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
        };
        let mut trial = ExperimentTrial {
            id: Uuid::new_v4(),
            campaign_id: campaign.id,
            sequence: 1,
            candidate_commit: None,
            parent_best_commit: None,
            status: ExperimentTrialStatus::Running,
            runner_backend: ExperimentRunnerBackend::GenericRemoteRunner,
            exit_code: None,
            metrics_json: serde_json::json!({}),
            summary: None,
            decision_reason: None,
            artifact_manifest_json: serde_json::json!({}),
            log_preview_path: None,
            reviewer_decision: None,
            runtime_ms: None,
            attributed_cost_usd: None,
            llm_cost_usd: None,
            runner_cost_usd: None,
            hypothesis: None,
            mutation_summary: None,
            provider_job_id: None,
            provider_job_metadata: serde_json::json!({}),
            started_at: Some(now),
            completed_at: None,
            created_at: now,
            updated_at: now,
        };
        let mut lease = ExperimentLease {
            id: Uuid::new_v4(),
            campaign_id: campaign.id,
            trial_id: trial.id,
            runner_profile_id: campaign.runner_profile_id,
            status: ExperimentLeaseStatus::Completed,
            token_hash: "hash".to_string(),
            job_payload: serde_json::json!({}),
            credentials_payload: serde_json::json!({}),
            expires_at: now,
            claimed_at: Some(now),
            completed_at: Some(now),
            created_at: now,
            updated_at: now,
        };
        let completion = ExperimentRunnerCompletion {
            exit_code: Some(0),
            metrics_json: serde_json::json!({}),
            summary: Some("done".to_string()),
            runtime_ms: Some(1),
            attributed_cost_usd: None,
            log_preview_path: None,
            artifact_manifest_json: serde_json::json!({}),
        };

        let error = super::complete_trial_terminal(
            &store,
            &project,
            &mut campaign,
            &mut trial,
            Some(&mut lease),
            completion,
        )
        .await
        .expect_err("completed lease should reject repeated completion");

        match error {
            crate::api::error::ApiError::InvalidInput(message) => {
                assert!(message.contains("already recorded"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    fn git(repo: &std::path::Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(repo)
            .status()
            .expect("git command should start");
        assert!(status.success(), "git {:?} failed with {:?}", args, status);
    }
}
