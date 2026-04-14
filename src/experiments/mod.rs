pub mod adapters;
pub mod runner;

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

fn default_true() -> bool {
    true
}

fn default_experiment_preset() -> ExperimentPreset {
    ExperimentPreset::AutoresearchSingleFile
}

fn default_primary_metric_comparator() -> ExperimentMetricComparator {
    ExperimentMetricComparator::LowerIsBetter
}

fn default_require_success_exit() -> bool {
    true
}

fn default_infra_failure_pause_threshold() -> u32 {
    3
}

fn default_non_improving_pause_threshold() -> u32 {
    8
}

fn default_mutation_retry_limit() -> u32 {
    2
}

fn default_promotion_mode() -> String {
    "branch_pr_draft".to_string()
}

fn default_project_status() -> ExperimentProjectStatus {
    ExperimentProjectStatus::Draft
}

fn default_project_autonomy_mode() -> ExperimentAutonomyMode {
    ExperimentAutonomyMode::Autonomous
}

fn default_runner_status() -> ExperimentRunnerStatus {
    ExperimentRunnerStatus::Draft
}

fn default_campaign_status() -> ExperimentCampaignStatus {
    ExperimentCampaignStatus::PendingBaseline
}

fn default_trial_status() -> ExperimentTrialStatus {
    ExperimentTrialStatus::Preparing
}

fn default_lease_status() -> ExperimentLeaseStatus {
    ExperimentLeaseStatus::Pending
}

fn default_campaign_queue_state() -> ExperimentCampaignQueueState {
    ExperimentCampaignQueueState::NotQueued
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentPreset {
    #[default]
    AutoresearchSingleFile,
    HostedPromptRouting,
    RagPipeline,
    ToolOrchestration,
    OpenWeightsInferenceTuning,
    SelfHostedFinetune,
    OpenWeightsTrainingCode,
}

impl ExperimentPreset {
    pub fn default_gpu_requirement(self) -> ExperimentGpuRequirement {
        match self {
            Self::HostedPromptRouting | Self::RagPipeline | Self::ToolOrchestration => {
                ExperimentGpuRequirement::NotNeeded
            }
            Self::OpenWeightsInferenceTuning => ExperimentGpuRequirement::Recommended,
            Self::AutoresearchSingleFile
            | Self::SelfHostedFinetune
            | Self::OpenWeightsTrainingCode => ExperimentGpuRequirement::Required,
        }
    }

    pub fn default_opportunity_type(self) -> ExperimentTargetKind {
        match self {
            Self::HostedPromptRouting => ExperimentTargetKind::PromptAsset,
            Self::RagPipeline => ExperimentTargetKind::RagConfig,
            Self::ToolOrchestration => ExperimentTargetKind::ToolPolicy,
            Self::OpenWeightsInferenceTuning => ExperimentTargetKind::InferenceConfig,
            Self::SelfHostedFinetune => ExperimentTargetKind::TrainingConfig,
            Self::AutoresearchSingleFile | Self::OpenWeightsTrainingCode => {
                ExperimentTargetKind::TrainingCode
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentMetricComparator {
    #[default]
    LowerIsBetter,
    HigherIsBetter,
    EqualIsBetter,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExperimentMetricDefinition {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json_path: Option<String>,
    #[serde(default = "default_primary_metric_comparator")]
    pub comparator: ExperimentMetricComparator,
}

impl Default for ExperimentMetricDefinition {
    fn default() -> Self {
        Self {
            name: "primary_metric".to_string(),
            regex: None,
            json_path: None,
            comparator: default_primary_metric_comparator(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExperimentComparisonPolicy {
    #[serde(default = "default_require_success_exit")]
    pub require_success_exit: bool,
    #[serde(default)]
    pub allow_equal: bool,
    #[serde(default)]
    pub minimum_delta: Option<f64>,
}

impl Default for ExperimentComparisonPolicy {
    fn default() -> Self {
        Self {
            require_success_exit: default_require_success_exit(),
            allow_equal: false,
            minimum_delta: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExperimentStopPolicy {
    #[serde(default)]
    pub max_trials: Option<u32>,
    #[serde(default)]
    pub max_total_runtime_secs: Option<u64>,
    #[serde(default)]
    pub max_total_cost_usd: Option<f64>,
    #[serde(default)]
    pub plateau_window: Option<u32>,
    #[serde(default = "default_infra_failure_pause_threshold")]
    pub infra_failure_pause_threshold: u32,
    #[serde(default = "default_non_improving_pause_threshold")]
    pub non_improving_pause_threshold: u32,
    #[serde(default = "default_mutation_retry_limit")]
    pub mutation_retry_limit: u32,
}

impl Default for ExperimentStopPolicy {
    fn default() -> Self {
        Self {
            max_trials: None,
            max_total_runtime_secs: None,
            max_total_cost_usd: None,
            plateau_window: None,
            infra_failure_pause_threshold: default_infra_failure_pause_threshold(),
            non_improving_pause_threshold: default_non_improving_pause_threshold(),
            mutation_retry_limit: default_mutation_retry_limit(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentGpuRequirement {
    #[default]
    NotNeeded,
    Recommended,
    Required,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentTargetKind {
    #[default]
    PromptAsset,
    RoutingPolicy,
    RagConfig,
    ToolPolicy,
    Evaluator,
    Parser,
    InferenceConfig,
    TrainingConfig,
    TrainingCode,
    ServingConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentProjectStatus {
    #[default]
    Draft,
    Ready,
    Archived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentAutonomyMode {
    #[default]
    Autonomous,
    ManualCandidate,
    SuggestOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentRunnerBackend {
    #[default]
    LocalDocker,
    GenericRemoteRunner,
    Ssh,
    Slurm,
    Kubernetes,
    Runpod,
    Vast,
    Lambda,
}

impl ExperimentRunnerBackend {
    pub fn is_remote(self) -> bool {
        !matches!(self, Self::LocalDocker)
    }

    pub fn is_gpu_cloud(self) -> bool {
        matches!(self, Self::Runpod | Self::Vast | Self::Lambda)
    }

    pub fn slug(self) -> &'static str {
        match self {
            Self::LocalDocker => "local_docker",
            Self::GenericRemoteRunner => "generic_remote_runner",
            Self::Ssh => "ssh",
            Self::Slurm => "slurm",
            Self::Kubernetes => "kubernetes",
            Self::Runpod => "runpod",
            Self::Vast => "vast",
            Self::Lambda => "lambda",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentRunnerStatus {
    #[default]
    Draft,
    Validated,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentCampaignStatus {
    #[default]
    PendingBaseline,
    Running,
    Paused,
    Completed,
    Failed,
    Cancelled,
    AwaitingPromotion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentCampaignQueueState {
    #[default]
    NotQueued,
    Queued,
    Active,
}

impl ExperimentCampaignQueueState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotQueued => "not_queued",
            Self::Queued => "queued",
            Self::Active => "active",
        }
    }
}

impl std::str::FromStr for ExperimentCampaignQueueState {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "not_queued" => Ok(Self::NotQueued),
            "queued" => Ok(Self::Queued),
            "active" => Ok(Self::Active),
            other => Err(format!("unknown experiment campaign queue state '{other}'")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentTrialStatus {
    #[default]
    Preparing,
    Running,
    Evaluating,
    Accepted,
    Rejected,
    Crashed,
    TimedOut,
    InfraFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentLeaseStatus {
    #[default]
    Pending,
    Claimed,
    Completed,
    Revoked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentProject {
    pub id: Uuid,
    pub name: String,
    pub workspace_path: String,
    pub git_remote_name: String,
    pub base_branch: String,
    #[serde(default = "default_experiment_preset")]
    pub preset: ExperimentPreset,
    pub strategy_prompt: String,
    pub workdir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prepare_command: Option<String>,
    pub run_command: String,
    #[serde(default)]
    pub mutable_paths: Vec<String>,
    #[serde(default)]
    pub fixed_paths: Vec<String>,
    #[serde(default)]
    pub primary_metric: ExperimentMetricDefinition,
    #[serde(default)]
    pub secondary_metrics: Vec<ExperimentMetricDefinition>,
    #[serde(default)]
    pub comparison_policy: ExperimentComparisonPolicy,
    #[serde(default)]
    pub stop_policy: ExperimentStopPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_runner_profile_id: Option<Uuid>,
    #[serde(default = "default_promotion_mode")]
    pub promotion_mode: String,
    #[serde(default = "default_project_autonomy_mode")]
    pub autonomy_mode: ExperimentAutonomyMode,
    #[serde(default = "default_project_status")]
    pub status: ExperimentProjectStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentRunnerProfile {
    pub id: Uuid,
    pub name: String,
    pub backend: ExperimentRunnerBackend,
    #[serde(default)]
    pub backend_config: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_or_runtime: Option<String>,
    #[serde(default)]
    pub gpu_requirements: serde_json::Value,
    #[serde(default)]
    pub env_grants: serde_json::Value,
    #[serde(default)]
    pub secret_references: Vec<String>,
    #[serde(default)]
    pub cache_policy: serde_json::Value,
    #[serde(default = "default_runner_status")]
    pub status: ExperimentRunnerStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentCampaign {
    pub id: Uuid,
    pub project_id: Uuid,
    pub runner_profile_id: Uuid,
    #[serde(default = "default_campaign_status")]
    pub status: ExperimentCampaignStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best_commit: Option<String>,
    #[serde(default)]
    pub best_metrics: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experiment_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub trial_count: u32,
    #[serde(default)]
    pub failure_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pause_reason: Option<String>,
    #[serde(default = "default_campaign_queue_state")]
    pub queue_state: ExperimentCampaignQueueState,
    #[serde(default)]
    pub queue_position: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_trial_id: Option<Uuid>,
    #[serde(default)]
    pub total_runtime_ms: u64,
    #[serde(default)]
    pub total_cost_usd: f64,
    #[serde(default)]
    pub total_llm_cost_usd: f64,
    #[serde(default)]
    pub total_runner_cost_usd: f64,
    #[serde(default)]
    pub consecutive_non_improving_trials: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_trials_override: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_url: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentTrial {
    pub id: Uuid,
    pub campaign_id: Uuid,
    pub sequence: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_best_commit: Option<String>,
    #[serde(default = "default_trial_status")]
    pub status: ExperimentTrialStatus,
    pub runner_backend: ExperimentRunnerBackend,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub metrics_json: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_preview_path: Option<String>,
    #[serde(default)]
    pub artifact_manifest_json: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attributed_cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hypothesis: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mutation_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewer_decision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_job_id: Option<String>,
    #[serde(default)]
    pub provider_job_metadata: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentArtifactRef {
    pub id: Uuid,
    pub trial_id: Uuid,
    pub kind: String,
    pub uri_or_local_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default)]
    pub fetchable: bool,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentTarget {
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub kind: ExperimentTargetKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentTargetLink {
    pub id: Uuid,
    pub target_id: Uuid,
    #[serde(default)]
    pub kind: ExperimentTargetKind,
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_role: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentModelUsageRecord {
    pub id: Uuid,
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workload_tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default = "default_true")]
    pub success: bool,
    #[serde(default)]
    pub prompt_asset_ids: Vec<String>,
    #[serde(default)]
    pub retrieval_asset_ids: Vec<String>,
    #[serde(default)]
    pub tool_policy_ids: Vec<String>,
    #[serde(default)]
    pub evaluator_ids: Vec<String>,
    #[serde(default)]
    pub parser_ids: Vec<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentOpportunity {
    pub id: String,
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_role: Option<String>,
    #[serde(default)]
    pub opportunity_type: ExperimentTargetKind,
    pub summary: String,
    #[serde(default)]
    pub gpu_requirement: ExperimentGpuRequirement,
    #[serde(default)]
    pub suggested_preset: ExperimentPreset,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_target_id: Option<Uuid>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentLease {
    pub id: Uuid,
    pub campaign_id: Uuid,
    pub trial_id: Uuid,
    pub runner_profile_id: Uuid,
    #[serde(default = "default_lease_status")]
    pub status: ExperimentLeaseStatus,
    pub token_hash: String,
    #[serde(default)]
    pub job_payload: serde_json::Value,
    #[serde(default)]
    pub credentials_payload: serde_json::Value,
    pub expires_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentLeaseAuthentication {
    pub lease_id: Uuid,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentRunnerJob {
    pub lease_id: Uuid,
    pub trial_id: Uuid,
    pub campaign_id: Uuid,
    pub project_id: Uuid,
    pub runner_profile_id: Uuid,
    pub backend: ExperimentRunnerBackend,
    pub repo_url: String,
    pub git_ref: String,
    pub workdir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prepare_command: Option<String>,
    pub run_command: String,
    pub primary_metric: ExperimentMetricDefinition,
    #[serde(default)]
    pub secondary_metrics: Vec<ExperimentMetricDefinition>,
    #[serde(default)]
    pub env_grants: serde_json::Value,
    #[serde(default)]
    pub artifact_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentRunnerCompletion {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub metrics_json: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attributed_cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_preview_path: Option<String>,
    #[serde(default)]
    pub artifact_manifest_json: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentRunnerArtifactUpload {
    pub kind: String,
    pub uri_or_local_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default = "default_true")]
    pub fetchable: bool,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

pub fn hash_lease_token(token: &str) -> String {
    blake3::hash(token.as_bytes()).to_hex().to_string()
}

pub fn extract_metric_value(
    definition: &ExperimentMetricDefinition,
    run_log: &str,
    summary_json: &serde_json::Value,
) -> Option<f64> {
    if let Some(json_path) = definition.json_path.as_deref()
        && let Some(value) = json_number_at_path(summary_json, json_path)
    {
        return Some(value);
    }

    let regex = definition.regex.as_deref()?;
    let compiled = Regex::new(regex).ok()?;
    let captures = compiled.captures(run_log)?;
    captures
        .get(1)
        .and_then(|m| m.as_str().trim().parse::<f64>().ok())
}

pub fn extract_metrics(
    primary_metric: &ExperimentMetricDefinition,
    secondary_metrics: &[ExperimentMetricDefinition],
    run_log: &str,
    summary_json: &serde_json::Value,
) -> serde_json::Value {
    let mut metrics = serde_json::Map::new();

    if let Some(value) = extract_metric_value(primary_metric, run_log, summary_json) {
        metrics.insert(primary_metric.name.clone(), serde_json::json!(value));
    }

    for metric in secondary_metrics {
        if let Some(value) = extract_metric_value(metric, run_log, summary_json) {
            metrics.insert(metric.name.clone(), serde_json::json!(value));
        }
    }

    serde_json::Value::Object(metrics)
}

pub fn compare_metrics(
    definition: &ExperimentMetricDefinition,
    policy: &ExperimentComparisonPolicy,
    candidate_metrics: &serde_json::Value,
    current_best_metrics: &serde_json::Value,
) -> Option<bool> {
    let candidate = json_number_at_path(candidate_metrics, &definition.name)?;
    let current = json_number_at_path(current_best_metrics, &definition.name)?;
    let delta = match definition.comparator {
        ExperimentMetricComparator::LowerIsBetter => current - candidate,
        ExperimentMetricComparator::HigherIsBetter => candidate - current,
        ExperimentMetricComparator::EqualIsBetter => {
            if (candidate - current).abs() < f64::EPSILON {
                return Some(true);
            }
            return Some(false);
        }
    };

    if let Some(minimum_delta) = policy.minimum_delta {
        if delta > minimum_delta {
            return Some(true);
        }
        if delta.abs() <= minimum_delta {
            return Some(policy.allow_equal);
        }
        return Some(false);
    }

    if delta > 0.0 {
        Some(true)
    } else if delta.abs() < f64::EPSILON {
        Some(policy.allow_equal)
    } else {
        Some(false)
    }
}

pub fn json_number_at_path(value: &serde_json::Value, path: &str) -> Option<f64> {
    let mut current = value;
    for part in path.split('.') {
        current = current.get(part)?;
    }
    current
        .as_f64()
        .or_else(|| current.as_i64().map(|v| v as f64))
}
