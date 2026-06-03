use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Display;
use std::path::{Component, PathBuf};

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thinclaw_history::OutcomeContract;
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

fn default_runner_readiness_class() -> ExperimentRunnerReadinessClass {
    ExperimentRunnerReadinessClass::ManualOnly
}

fn default_campaign_status() -> ExperimentCampaignStatus {
    ExperimentCampaignStatus::PendingBaseline
}

fn default_experiment_owner_user_id() -> String {
    "default".to_string()
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
    AgentEnv,
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
        !matches!(self, Self::LocalDocker | Self::AgentEnv)
    }

    pub fn is_gpu_cloud(self) -> bool {
        matches!(self, Self::Runpod | Self::Vast | Self::Lambda)
    }

    pub fn slug(self) -> &'static str {
        match self {
            Self::LocalDocker => "local_docker",
            Self::AgentEnv => "agent_env",
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
pub enum ExperimentRunnerReadinessClass {
    #[default]
    ManualOnly,
    BootstrapReady,
    LaunchReady,
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
    #[serde(default = "default_runner_readiness_class")]
    pub readiness_class: ExperimentRunnerReadinessClass,
    #[serde(default)]
    pub launch_eligible: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentCampaign {
    pub id: Uuid,
    pub project_id: Uuid,
    pub runner_profile_id: Uuid,
    #[serde(default = "default_experiment_owner_user_id")]
    pub owner_user_id: String,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub signals: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_hint: Option<serde_json::Value>,
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

pub fn merge_json(base: &serde_json::Value, overlay: &serde_json::Value) -> serde_json::Value {
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

pub fn target_signature(
    kind: ExperimentTargetKind,
    metadata: &serde_json::Value,
) -> Option<String> {
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

pub fn ensure_unique_target_signature(
    kind: ExperimentTargetKind,
    metadata: &serde_json::Value,
    skip_target_id: Option<Uuid>,
    targets: &[ExperimentTarget],
) -> Result<(), String> {
    let Some(signature) = target_signature(kind, metadata) else {
        return Ok(());
    };
    if targets.iter().any(|existing| {
        existing.kind == kind
            && skip_target_id != Some(existing.id)
            && target_signature(existing.kind, &existing.metadata).as_deref()
                == Some(signature.as_str())
    }) {
        return Err(format!(
            "Duplicate target for linked identity '{signature}'"
        ));
    }
    Ok(())
}

pub fn derive_opportunities(
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

pub fn sort_experiment_opportunities(opportunities: &mut [ExperimentOpportunity]) {
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

pub fn experiment_target_kind_sort_key(kind: ExperimentTargetKind) -> &'static str {
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

    let mut seen = HashSet::new();
    kinds.retain(|kind| seen.insert(*kind as u8));
    kinds.sort_by_key(|kind| *kind as u8);
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

fn opportunity_signals_for_usage(
    aggregate: &OpportunityAggregate,
    error_rate: f64,
    avg_latency_ms: Option<f64>,
    avg_cost_usd: Option<f64>,
) -> Vec<String> {
    let mut signals = vec![format!(
        "{} model call{}",
        aggregate.call_count,
        if aggregate.call_count == 1 { "" } else { "s" }
    )];
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

pub fn derive_outcome_opportunities(
    contracts: &[OutcomeContract],
    targets: &[ExperimentTarget],
    limit: usize,
    default_prompt_asset: &str,
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
            .or_else(|| outcome_default_artifact_name(contract, default_prompt_asset));
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
        let aggregate =
            aggregates
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
                    .cmp(experiment_target_kind_sort_key(right.kind))
            })
            .then_with(|| left.pattern_key.cmp(&right.pattern_key))
    });

    aggregates
        .into_iter()
        .take(limit.max(1))
        .map(|aggregate| outcome_aggregate_to_opportunity(aggregate, default_prompt_asset))
        .collect()
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

fn outcome_default_artifact_name(
    contract: &OutcomeContract,
    default_prompt_asset: &str,
) -> Option<String> {
    match contract.contract_type.as_str() {
        "turn_usefulness" => Some(default_prompt_asset.to_string()),
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
            asset_id
                .zip(artifact_name)
                .is_some_and(|(left, right)| left == right)
                || asset_id
                    .zip(routine_id)
                    .is_some_and(|(left, right)| left == right)
                || target_pattern.is_some_and(|value| value == pattern_key)
        })
        .map(|target| target.id)
}

fn outcome_aggregate_to_opportunity(
    aggregate: OutcomeOpportunityAggregate,
    default_prompt_asset: &str,
) -> ExperimentOpportunity {
    let (summary, project_hint) =
        outcome_summary_and_project_hint(&aggregate, default_prompt_asset);
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
    default_prompt_asset: &str,
) -> (String, serde_json::Value) {
    match aggregate.kind {
        ExperimentTargetKind::PromptAsset => {
            let target = aggregate
                .artifact_name
                .clone()
                .unwrap_or_else(|| default_prompt_asset.to_string());
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
        format!(
            "{} negative outcome{}",
            aggregate.count,
            if aggregate.count == 1 { "" } else { "s" }
        ),
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
        ExperimentTargetKind::TrainingCode | ExperimentTargetKind::TrainingConfig => {
            ExperimentGpuRequirement::Required
        }
        ExperimentTargetKind::InferenceConfig | ExperimentTargetKind::ServingConfig => {
            ExperimentGpuRequirement::Recommended
        }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannerProposal {
    pub hypothesis: String,
    #[serde(default)]
    pub target_ids: Vec<String>,
    #[serde(default)]
    pub allowed_paths: Vec<String>,
    #[serde(default)]
    pub expected_metric_direction: Option<String>,
    pub mutation_brief: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutatorResult {
    #[serde(default)]
    pub changed_paths: Vec<String>,
    pub mutation_summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewerDecision {
    pub approved: bool,
    pub scope_ok: bool,
    pub benchmark_ready: bool,
    pub reason: String,
}

pub fn default_strategy_prompt() -> String {
    "Operate within the configured mutable paths only. Preserve the fixed harness, compare candidates against the best-known result, and stop when the campaign no longer improves.".to_string()
}

pub fn ready_project_status(
    project: &ExperimentProject,
    workspace_exists: bool,
) -> ExperimentProjectStatus {
    if workspace_exists
        && !project.mutable_paths.is_empty()
        && !project.run_command.trim().is_empty()
    {
        ExperimentProjectStatus::Ready
    } else {
        ExperimentProjectStatus::Draft
    }
}

pub fn parse_secret_reference(reference: &str) -> Option<(String, Vec<String>)> {
    let trimmed = reference.trim();
    if trimmed.is_empty() {
        return None;
    }
    for separator in [':', '='] {
        if let Some((secret_name, env_var)) = trimmed.split_once(separator) {
            let secret_name = secret_name.trim();
            let env_var = env_var.trim();
            if !secret_name.is_empty() && !env_var.is_empty() {
                return Some((secret_name.to_string(), vec![env_var.to_string()]));
            }
        }
    }

    let upper = trimmed.to_ascii_uppercase();
    let env_names = if upper == trimmed {
        vec![trimmed.to_string()]
    } else {
        vec![trimmed.to_string(), upper]
    };
    Some((trimmed.to_string(), env_names))
}

pub fn truncate_for_prompt(value: &str, max_len: usize) -> String {
    if value.chars().count() <= max_len {
        return value.to_string();
    }
    let truncated: String = value.chars().take(max_len.saturating_sub(3)).collect();
    format!("{truncated}...")
}

pub fn recent_trial_context(trials: &[ExperimentTrial]) -> String {
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

pub fn parse_research_json_response<T: DeserializeOwned>(raw: &str) -> Result<T, String> {
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
    Err("Research subagent returned invalid JSON output.".to_string())
}

pub fn campaign_gateway_url(campaign: &ExperimentCampaign) -> Option<String> {
    campaign
        .gateway_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub fn short_id(id: Uuid) -> String {
    id.simple().to_string()[..12].to_string()
}

pub fn experiments_worktree_path(workspace_root: &str, campaign_id: Uuid) -> PathBuf {
    PathBuf::from(workspace_root)
        .join(".thinclaw-experiments")
        .join(short_id(campaign_id))
}

pub fn validate_project_workdir_fragment(workdir: &str) -> Result<PathBuf, String> {
    let trimmed = workdir.trim();
    let candidate = if trimmed.is_empty() {
        PathBuf::from(".")
    } else {
        PathBuf::from(trimmed)
    };

    if candidate.is_absolute() {
        return Err("Project workdir must be relative to the workspace root.".to_string());
    }

    if candidate.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err("Project workdir must stay inside the workspace root.".to_string());
    }

    Ok(candidate)
}

pub fn is_stale_lease(
    lease: &ExperimentLease,
    now: DateTime<Utc>,
    stale_lease_grace_minutes: i64,
) -> bool {
    match lease.status {
        ExperimentLeaseStatus::Pending => {
            lease.expires_at + chrono::Duration::minutes(stale_lease_grace_minutes) < now
        }
        ExperimentLeaseStatus::Claimed => {
            lease.updated_at + chrono::Duration::minutes(stale_lease_grace_minutes) < now
        }
        _ => false,
    }
}

pub fn lease_runner_trial_status(
    runner_status: &str,
    current_status: ExperimentTrialStatus,
) -> ExperimentTrialStatus {
    match runner_status {
        "runner_started" | "running_prepare" | "running_benchmark" => {
            ExperimentTrialStatus::Running
        }
        "evaluating" | "uploading_artifacts" | "completing" => ExperimentTrialStatus::Evaluating,
        _ => current_status,
    }
}

pub fn validate_lease_completion_status(status: ExperimentLeaseStatus) -> Result<(), &'static str> {
    if status == ExperimentLeaseStatus::Claimed {
        Ok(())
    } else {
        Err(lease_completion_rejection_message(status))
    }
}

pub fn lease_completion_rejection_message(status: ExperimentLeaseStatus) -> &'static str {
    match status {
        ExperimentLeaseStatus::Claimed => "lease is already claimed",
        ExperimentLeaseStatus::Completed => {
            "lease completion was already recorded; repeated terminal completions are ignored"
        }
        ExperimentLeaseStatus::Revoked => {
            "lease was revoked before completion and can no longer transition to terminal"
        }
        ExperimentLeaseStatus::Pending => "lease must be claimed before completion can be recorded",
    }
}

pub fn experiment_project_not_found_message(id: impl Display) -> String {
    format!("Experiment project {id} not found")
}

pub fn experiments_feature_disabled_message() -> &'static str {
    "Enable experiments in Settings → Features to use this API."
}

pub fn experiment_workspace_path_missing_message(workspace_path: impl Display) -> String {
    format!("Workspace path does not exist: {workspace_path}")
}

pub fn experiment_workspace_path_missing_with_error_message(
    workspace_path: impl Display,
    error: impl Display,
) -> String {
    format!("Workspace path does not exist: {workspace_path} ({error})")
}

pub fn experiment_project_workdir_missing_message(
    workdir: impl Display,
    error: impl Display,
) -> String {
    format!("Project workdir does not exist: {workdir} ({error})")
}

pub fn experiment_project_workdir_outside_workspace_message() -> &'static str {
    "Project workdir resolves outside the workspace root."
}

pub fn experiment_project_missing_mutable_paths_message() -> &'static str {
    "Project must define at least one mutable path before launch."
}

pub fn experiment_project_run_command_empty_message() -> &'static str {
    "Project run_command must not be empty."
}

pub fn experiment_workspace_not_git_repository_message(error: impl Display) -> String {
    format!("Workspace path is not a git repository ThinClaw can use: {error}")
}

pub fn experiment_project_workdir_escapes_campaign_worktree_message() -> &'static str {
    "Project workdir escapes the campaign worktree."
}

pub fn experiment_runner_not_found_message(id: impl Display) -> String {
    format!("Experiment runner {id} not found")
}

pub fn experiment_campaign_not_found_message(id: impl Display) -> String {
    format!("Experiment campaign {id} not found")
}

pub fn experiment_trial_not_found_message(id: impl Display) -> String {
    format!("Experiment trial {id} not found")
}

pub fn experiment_target_not_found_message(id: impl Display) -> String {
    format!("Experiment target {id} not found")
}

pub fn experiment_opportunity_not_found_message(id: impl Display) -> String {
    format!("Experiment opportunity {id} not found")
}

pub fn experiment_lease_not_found_message(id: impl Display) -> String {
    format!("Experiment lease {id} not found")
}

pub fn experiment_base_branch_unavailable_message(
    branch: impl Display,
    error: impl Display,
) -> String {
    format!("Base branch '{branch}' is not available locally: {error}")
}

pub fn experiment_git_remote_unavailable_message(
    remote: impl Display,
    error: impl Display,
) -> String {
    format!("Configured git remote '{remote}' is not available: {error}")
}

pub fn experiment_campaign_has_no_worktree_message() -> &'static str {
    "Campaign has no worktree"
}

pub fn experiment_campaign_has_no_trial_to_reissue_message() -> &'static str {
    "Campaign has no trial to reissue a lease for."
}

pub fn experiment_campaign_has_no_accepted_commit_message() -> &'static str {
    "Campaign has no accepted commit to promote"
}

pub fn experiment_promotion_pr_body(
    campaign_id: impl Display,
    best_commit: impl Display,
    primary_metric: impl Display,
) -> String {
    format!(
        "Promoting best commit from experiment campaign {campaign_id}\n\nBest commit: {best_commit}\nPrimary metric: {primary_metric}"
    )
}

pub fn experiment_primary_metric_not_found_message(primary_metric: impl Display) -> String {
    format!("Primary metric '{primary_metric}' was not found in the runner result.")
}

pub fn research_subagent_executor_unavailable_message() -> &'static str {
    "Research subagent executor is not available."
}

pub fn experiment_campaign_missing_worktree_path_message() -> &'static str {
    "Campaign missing worktree path"
}

pub fn experiment_campaign_missing_worktree_path_field_message() -> &'static str {
    "Campaign missing worktree_path"
}

pub fn experiment_campaign_missing_experiment_branch_field_message() -> &'static str {
    "Campaign missing experiment_branch"
}

pub fn experiment_campaign_missing_experiment_branch_message() -> &'static str {
    "Campaign missing experiment branch"
}

pub fn experiment_no_candidate_changes_message() -> &'static str {
    "No candidate changes detected in the campaign worktree."
}

pub fn experiment_lease_revoked_action_message() -> &'static str {
    "Lease revoked."
}

pub fn experiment_campaign_paused_by_operator_message() -> &'static str {
    "Paused by operator."
}

pub fn experiment_campaign_paused_message() -> &'static str {
    "Campaign paused."
}

pub fn experiment_campaign_cancelled_by_operator_message() -> &'static str {
    "Cancelled by operator."
}

pub fn experiment_campaign_cancelled_message() -> &'static str {
    "Campaign cancelled."
}

pub fn experiment_lease_reissue_remote_only_message() -> &'static str {
    "Lease reissue is only supported for remote runners."
}

pub fn experiment_remote_trial_reissue_in_flight_only_message() -> &'static str {
    "Only in-flight remote trials can receive a new lease."
}

pub fn experiment_target_id_required_message() -> &'static str {
    "target_id is required"
}

pub fn experiment_runner_profile_id_required_message() -> &'static str {
    "runner_profile_id is required"
}

pub fn experiment_lease_revoked_message() -> &'static str {
    "Lease has been revoked"
}

pub fn experiment_lease_expired_message() -> &'static str {
    "Lease has expired"
}

pub fn invalid_experiment_lease_token_message() -> &'static str {
    "Invalid lease token"
}

pub fn normalize_trial_completion(
    mut completion: ExperimentRunnerCompletion,
) -> ExperimentRunnerCompletion {
    if !completion.artifact_manifest_json.is_object() {
        completion.artifact_manifest_json = serde_json::json!({});
    }
    let has_stage = completion
        .artifact_manifest_json
        .get("stage")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    if !has_stage {
        let stage = if completion.exit_code == Some(0) {
            "complete"
        } else {
            "run"
        };
        completion.artifact_manifest_json = merge_json(
            &completion.artifact_manifest_json,
            &serde_json::json!({ "stage": stage }),
        );
    }
    completion
}

pub fn next_campaign_status(
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

pub fn campaign_status_message(
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
            "Trial accepted. Continue for another candidate.".to_string()
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

pub fn filtered_changed_files(changed_files: Vec<String>) -> Vec<String> {
    changed_files
        .into_iter()
        .filter(|path| !path.starts_with(".thinclaw-experiments/"))
        .collect()
}

pub fn enforce_mutable_paths(
    mutable_paths: &[String],
    changed_files: &[String],
) -> Result<(), String> {
    for changed in changed_files {
        let allowed = mutable_paths
            .iter()
            .any(|allowed| changed == allowed || changed.starts_with(&(allowed.clone() + "/")));
        if !allowed {
            return Err(format!(
                "Changed file '{}' is outside the mutable_paths allowlist",
                changed
            ));
        }
    }
    Ok(())
}

pub fn env_pairs_from_json(env_grants: &serde_json::Value) -> Vec<(String, String)> {
    env_grants
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

#[derive(Debug, Clone)]
pub struct LlmCostAttribution {
    pub total_usd: f64,
    pub details: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct RunnerCostBreakdown {
    pub total_usd: f64,
    pub details: serde_json::Value,
    pub provider_metadata_overlay: Option<serde_json::Value>,
}

pub fn summarize_llm_usage(
    records: &[ExperimentModelUsageRecord],
    source: &str,
) -> LlmCostAttribution {
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

pub fn runner_cost_breakdown(
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

pub fn metadata_string_field(metadata: &serde_json::Value, key: &str) -> Option<String> {
    metadata
        .get(key)
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
}

#[derive(Debug, Clone)]
pub struct ProviderCostEstimate {
    pub total_usd: f64,
    pub hourly_rate_usd: f64,
    pub source: String,
    pub native_hourly_rate: Option<f64>,
    pub native_currency: Option<String>,
    pub normalization: Option<String>,
}

pub type ProviderHourlyRate = (f64, String, Option<f64>, Option<String>, Option<String>);

pub fn estimated_provider_runtime_cost_usd(
    trial: &ExperimentTrial,
) -> Option<ProviderCostEstimate> {
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

pub fn provider_hourly_rate_usd(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_json_recursively_overlays_objects() {
        let base = serde_json::json!({
            "a": 1,
            "nested": {
                "left": true,
                "replace": "old"
            }
        });
        let overlay = serde_json::json!({
            "nested": {
                "replace": "new",
                "right": 2
            }
        });

        let merged = merge_json(&base, &overlay);
        assert_eq!(merged["a"], 1);
        assert_eq!(merged["nested"]["left"], true);
        assert_eq!(merged["nested"]["replace"], "new");
        assert_eq!(merged["nested"]["right"], 2);
    }

    #[test]
    fn target_signature_normalizes_link_identity() {
        let metadata = serde_json::json!({
            "provider": "OpenAI",
            "model": "GPT-5",
            "route_key": "Primary",
            "asset_id": "Prompt/User"
        });

        assert_eq!(
            target_signature(ExperimentTargetKind::PromptAsset, &metadata).as_deref(),
            Some("PromptAsset|openai|gpt-5|primary|prompt/user")
        );
    }

    #[test]
    fn ensure_unique_target_signature_detects_duplicates_and_honors_skip() {
        let id = Uuid::new_v4();
        let metadata = serde_json::json!({
            "provider": "openai",
            "model": "gpt-5"
        });
        let targets = vec![ExperimentTarget {
            id,
            name: "existing".to_string(),
            kind: ExperimentTargetKind::InferenceConfig,
            location: None,
            metadata: metadata.clone(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }];

        assert!(
            ensure_unique_target_signature(
                ExperimentTargetKind::InferenceConfig,
                &metadata,
                None,
                &targets
            )
            .is_err()
        );
        assert!(
            ensure_unique_target_signature(
                ExperimentTargetKind::InferenceConfig,
                &metadata,
                Some(id),
                &targets
            )
            .is_ok()
        );
    }

    #[test]
    fn derive_opportunities_groups_usage_and_links_targets() {
        let now = Utc::now();
        let target_id = Uuid::new_v4();
        let usage = vec![ExperimentModelUsageRecord {
            id: Uuid::new_v4(),
            provider: "OpenAI".to_string(),
            model: "gpt-5".to_string(),
            route_key: Some("primary".to_string()),
            logical_role: Some("planner".to_string()),
            endpoint_type: Some("hosted".to_string()),
            workload_tag: Some("json_parser".to_string()),
            latency_ms: Some(120),
            cost_usd: Some(0.01),
            success: false,
            prompt_asset_ids: vec!["system".to_string()],
            retrieval_asset_ids: Vec::new(),
            tool_policy_ids: Vec::new(),
            evaluator_ids: Vec::new(),
            parser_ids: Vec::new(),
            metadata: serde_json::json!({}),
            created_at: now,
        }];
        let targets = vec![ExperimentTarget {
            id: target_id,
            name: "prompt".to_string(),
            kind: ExperimentTargetKind::PromptAsset,
            location: None,
            metadata: serde_json::json!({
                "provider": "openai",
                "model": "gpt-5",
                "route_key": "primary",
                "asset_id": "system",
            }),
            created_at: now,
            updated_at: now,
        }];
        let links = vec![ExperimentTargetLink {
            id: Uuid::new_v4(),
            target_id,
            kind: ExperimentTargetKind::PromptAsset,
            provider: "openai".to_string(),
            model: "gpt-5".to_string(),
            route_key: Some("primary".to_string()),
            logical_role: Some("planner".to_string()),
            metadata: serde_json::json!({}),
            created_at: now,
            updated_at: now,
        }];

        let mut opportunities = derive_opportunities(&usage, &targets, &links);
        sort_experiment_opportunities(&mut opportunities);

        assert!(
            opportunities
                .iter()
                .any(|opportunity| opportunity.opportunity_type
                    == ExperimentTargetKind::PromptAsset
                    && opportunity.linked_target_id == Some(target_id)
                    && opportunity.metadata["call_count"] == 1
                    && opportunity.metadata["error_count"] == 1)
        );
        assert!(
            opportunities
                .iter()
                .any(|opportunity| opportunity.opportunity_type == ExperimentTargetKind::Parser)
        );
    }

    #[test]
    fn derive_outcome_opportunities_uses_negative_contract_patterns() {
        let now = Utc::now();
        let contract = OutcomeContract {
            id: Uuid::new_v4(),
            user_id: "user".to_string(),
            actor_id: None,
            channel: None,
            thread_id: None,
            source_kind: "turn".to_string(),
            source_id: "turn-1".to_string(),
            contract_type: "turn_usefulness".to_string(),
            status: "evaluated".to_string(),
            summary: None,
            due_at: now,
            expires_at: now,
            final_verdict: Some("negative".to_string()),
            final_score: Some(0.1),
            evaluation_details: serde_json::json!({}),
            metadata: serde_json::json!({
                "pattern_key": "prompt-drift"
            }),
            dedupe_key: "dedupe".to_string(),
            claimed_at: None,
            evaluated_at: Some(now),
            created_at: now,
            updated_at: now,
        };
        let target_id = Uuid::new_v4();
        let target = ExperimentTarget {
            id: target_id,
            name: "USER.md".to_string(),
            kind: ExperimentTargetKind::PromptAsset,
            location: None,
            metadata: serde_json::json!({
                "asset_id": "USER.md",
                "pattern_key": "prompt-drift"
            }),
            created_at: now,
            updated_at: now,
        };

        let opportunities = derive_outcome_opportunities(&[contract], &[target], 10, "USER.md");

        assert_eq!(opportunities.len(), 1);
        assert_eq!(
            opportunities[0].opportunity_type,
            ExperimentTargetKind::PromptAsset
        );
        assert_eq!(opportunities[0].linked_target_id, Some(target_id));
        assert_eq!(opportunities[0].source.as_deref(), Some("outcome_learning"));
        assert!(opportunities[0].summary.contains("USER.md"));
    }

    #[test]
    fn validate_project_workdir_fragment_rejects_parent_traversal() {
        let error = validate_project_workdir_fragment("../escape")
            .expect_err("parent traversal should be rejected");
        assert!(error.contains("Project workdir must stay inside the workspace root"));
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

        let normalized = normalize_trial_completion(completion);
        assert_eq!(
            normalized
                .artifact_manifest_json
                .get("stage")
                .and_then(|value| value.as_str()),
            Some("complete")
        );
    }

    #[test]
    fn changed_path_policy_filters_internal_paths_and_enforces_allowlist() {
        let changed = filtered_changed_files(vec![
            ".thinclaw-experiments/state.json".to_string(),
            "src/lib.rs".to_string(),
            "README.md".to_string(),
        ]);

        assert_eq!(
            changed,
            vec!["src/lib.rs".to_string(), "README.md".to_string()]
        );
        assert!(enforce_mutable_paths(&["src".to_string()], &changed).is_err());
        assert!(
            enforce_mutable_paths(&["src".to_string(), "README.md".to_string()], &changed).is_ok()
        );
    }

    #[test]
    fn env_pairs_from_json_keeps_only_string_values() {
        let mut pairs = env_pairs_from_json(&serde_json::json!({
            "TOKEN": "secret",
            "COUNT": 3,
            "EMPTY": ""
        }));
        pairs.sort();

        assert_eq!(
            pairs,
            vec![
                ("EMPTY".to_string(), "".to_string()),
                ("TOKEN".to_string(), "secret".to_string())
            ]
        );
    }

    #[test]
    fn parse_secret_reference_infers_uppercase_env_alias() {
        assert_eq!(
            parse_secret_reference("runpod_api_key"),
            Some((
                "runpod_api_key".to_string(),
                vec!["runpod_api_key".to_string(), "RUNPOD_API_KEY".to_string()]
            ))
        );
        assert_eq!(
            parse_secret_reference("runpod:RUNPOD_API_KEY"),
            Some(("runpod".to_string(), vec!["RUNPOD_API_KEY".to_string()]))
        );
    }

    #[test]
    fn ready_project_status_requires_workspace_mutable_paths_and_command() {
        let now = Utc::now();
        let mut project = ExperimentProject {
            id: Uuid::new_v4(),
            name: "demo".to_string(),
            workspace_path: ".".to_string(),
            git_remote_name: "origin".to_string(),
            base_branch: "main".to_string(),
            preset: Default::default(),
            strategy_prompt: "test".to_string(),
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
            status: ExperimentProjectStatus::Draft,
            created_at: now,
            updated_at: now,
        };

        assert_eq!(
            ready_project_status(&project, true),
            ExperimentProjectStatus::Ready
        );
        assert_eq!(
            ready_project_status(&project, false),
            ExperimentProjectStatus::Draft
        );
        project.mutable_paths.clear();
        assert_eq!(
            ready_project_status(&project, true),
            ExperimentProjectStatus::Draft
        );
        project.mutable_paths.push("src".to_string());
        project.run_command = "   ".to_string();
        assert_eq!(
            ready_project_status(&project, true),
            ExperimentProjectStatus::Draft
        );
    }

    #[test]
    fn recent_trial_context_renders_latest_trial_summary() {
        let now = Utc::now();
        let trial = ExperimentTrial {
            id: Uuid::new_v4(),
            campaign_id: Uuid::new_v4(),
            sequence: 7,
            candidate_commit: None,
            parent_best_commit: None,
            status: ExperimentTrialStatus::Accepted,
            runner_backend: ExperimentRunnerBackend::LocalDocker,
            exit_code: Some(0),
            metrics_json: serde_json::json!({ "score": 0.95 }),
            summary: Some("candidate improved score".to_string()),
            decision_reason: None,
            log_preview_path: None,
            artifact_manifest_json: serde_json::json!({}),
            runtime_ms: Some(100),
            attributed_cost_usd: None,
            llm_cost_usd: None,
            runner_cost_usd: None,
            hypothesis: Some("tune parser".to_string()),
            mutation_summary: None,
            reviewer_decision: None,
            provider_job_id: None,
            provider_job_metadata: serde_json::json!({}),
            started_at: Some(now),
            completed_at: Some(now),
            created_at: now,
            updated_at: now,
        };

        let context = recent_trial_context(&[trial]);

        assert!(context.contains("Trial #7"));
        assert!(context.contains("status=Accepted"));
        assert!(context.contains("hypothesis=tune parser"));
    }

    #[test]
    fn truncate_for_prompt_preserves_short_text_and_truncates_long_text() {
        assert_eq!(truncate_for_prompt("short", 10), "short");
        assert_eq!(truncate_for_prompt("abcdef", 5), "ab...");
    }

    #[test]
    fn parse_research_json_response_accepts_fenced_json() {
        #[derive(Debug, Deserialize, PartialEq)]
        struct Response {
            ok: bool,
        }

        let parsed: Response =
            parse_research_json_response("```json\n{\"ok\":true}\n```").expect("fenced json");
        assert_eq!(parsed, Response { ok: true });
        assert!(parse_research_json_response::<Response>("not json").is_err());
    }

    #[test]
    fn lease_completion_rejection_message_covers_terminal_statuses() {
        assert_eq!(
            lease_completion_rejection_message(ExperimentLeaseStatus::Completed),
            "lease completion was already recorded; repeated terminal completions are ignored"
        );
        assert_eq!(
            lease_completion_rejection_message(ExperimentLeaseStatus::Revoked),
            "lease was revoked before completion and can no longer transition to terminal"
        );
    }

    #[test]
    fn lease_runner_trial_status_maps_runner_progress_strings() {
        for status in ["runner_started", "running_prepare", "running_benchmark"] {
            assert_eq!(
                lease_runner_trial_status(status, ExperimentTrialStatus::Preparing),
                ExperimentTrialStatus::Running
            );
        }
        for status in ["evaluating", "uploading_artifacts", "completing"] {
            assert_eq!(
                lease_runner_trial_status(status, ExperimentTrialStatus::Running),
                ExperimentTrialStatus::Evaluating
            );
        }
    }

    #[test]
    fn lease_runner_trial_status_preserves_unknown_statuses() {
        assert_eq!(
            lease_runner_trial_status("runner_started ", ExperimentTrialStatus::Preparing),
            ExperimentTrialStatus::Preparing
        );
        assert_eq!(
            lease_runner_trial_status("custom_status", ExperimentTrialStatus::Accepted),
            ExperimentTrialStatus::Accepted
        );
    }

    #[test]
    fn validate_lease_completion_status_requires_claimed_lease() {
        assert_eq!(
            validate_lease_completion_status(ExperimentLeaseStatus::Claimed),
            Ok(())
        );
        assert_eq!(
            validate_lease_completion_status(ExperimentLeaseStatus::Pending),
            Err("lease must be claimed before completion can be recorded")
        );
        assert_eq!(
            validate_lease_completion_status(ExperimentLeaseStatus::Completed),
            Err("lease completion was already recorded; repeated terminal completions are ignored")
        );
    }

    #[test]
    fn experiment_api_messages_preserve_existing_text() {
        let id = Uuid::from_u128(7);

        assert_eq!(
            experiment_project_not_found_message(id),
            format!("Experiment project {id} not found")
        );
        assert_eq!(
            experiments_feature_disabled_message(),
            "Enable experiments in Settings → Features to use this API."
        );
        assert_eq!(
            experiment_workspace_path_missing_message("/tmp/project"),
            "Workspace path does not exist: /tmp/project"
        );
        assert_eq!(
            experiment_workspace_path_missing_with_error_message("/tmp/project", "missing"),
            "Workspace path does not exist: /tmp/project (missing)"
        );
        assert_eq!(
            experiment_project_workdir_missing_message("/tmp/project/bench", "missing"),
            "Project workdir does not exist: /tmp/project/bench (missing)"
        );
        assert_eq!(
            experiment_project_workdir_outside_workspace_message(),
            "Project workdir resolves outside the workspace root."
        );
        assert_eq!(
            experiment_project_missing_mutable_paths_message(),
            "Project must define at least one mutable path before launch."
        );
        assert_eq!(
            experiment_project_run_command_empty_message(),
            "Project run_command must not be empty."
        );
        assert_eq!(
            experiment_workspace_not_git_repository_message("fatal"),
            "Workspace path is not a git repository ThinClaw can use: fatal"
        );
        assert_eq!(
            experiment_project_workdir_escapes_campaign_worktree_message(),
            "Project workdir escapes the campaign worktree."
        );
        assert_eq!(
            experiment_runner_not_found_message(id),
            format!("Experiment runner {id} not found")
        );
        assert_eq!(
            experiment_campaign_not_found_message(id),
            format!("Experiment campaign {id} not found")
        );
        assert_eq!(
            experiment_trial_not_found_message(id),
            format!("Experiment trial {id} not found")
        );
        assert_eq!(
            experiment_target_not_found_message(id),
            format!("Experiment target {id} not found")
        );
        assert_eq!(
            experiment_opportunity_not_found_message(id),
            format!("Experiment opportunity {id} not found")
        );
        assert_eq!(
            experiment_lease_not_found_message(id),
            format!("Experiment lease {id} not found")
        );
        assert_eq!(
            experiment_base_branch_unavailable_message("main", "missing"),
            "Base branch 'main' is not available locally: missing"
        );
        assert_eq!(
            experiment_git_remote_unavailable_message("origin", "missing"),
            "Configured git remote 'origin' is not available: missing"
        );
        assert_eq!(
            experiment_campaign_has_no_worktree_message(),
            "Campaign has no worktree"
        );
        assert_eq!(
            experiment_campaign_has_no_trial_to_reissue_message(),
            "Campaign has no trial to reissue a lease for."
        );
        assert_eq!(
            experiment_campaign_has_no_accepted_commit_message(),
            "Campaign has no accepted commit to promote"
        );
        assert_eq!(
            experiment_promotion_pr_body(id, "abc123", "latency"),
            format!(
                "Promoting best commit from experiment campaign {id}\n\nBest commit: abc123\nPrimary metric: latency"
            )
        );
        assert_eq!(
            experiment_primary_metric_not_found_message("latency"),
            "Primary metric 'latency' was not found in the runner result."
        );
        assert_eq!(
            research_subagent_executor_unavailable_message(),
            "Research subagent executor is not available."
        );
        assert_eq!(
            experiment_campaign_missing_worktree_path_message(),
            "Campaign missing worktree path"
        );
        assert_eq!(
            experiment_campaign_missing_worktree_path_field_message(),
            "Campaign missing worktree_path"
        );
        assert_eq!(
            experiment_campaign_missing_experiment_branch_field_message(),
            "Campaign missing experiment_branch"
        );
        assert_eq!(
            experiment_campaign_missing_experiment_branch_message(),
            "Campaign missing experiment branch"
        );
        assert_eq!(
            experiment_no_candidate_changes_message(),
            "No candidate changes detected in the campaign worktree."
        );
        assert_eq!(experiment_lease_revoked_action_message(), "Lease revoked.");
        assert_eq!(
            experiment_campaign_paused_by_operator_message(),
            "Paused by operator."
        );
        assert_eq!(experiment_campaign_paused_message(), "Campaign paused.");
        assert_eq!(
            experiment_campaign_cancelled_by_operator_message(),
            "Cancelled by operator."
        );
        assert_eq!(
            experiment_campaign_cancelled_message(),
            "Campaign cancelled."
        );
        assert_eq!(
            experiment_lease_reissue_remote_only_message(),
            "Lease reissue is only supported for remote runners."
        );
        assert_eq!(
            experiment_remote_trial_reissue_in_flight_only_message(),
            "Only in-flight remote trials can receive a new lease."
        );
        assert_eq!(
            experiment_target_id_required_message(),
            "target_id is required"
        );
        assert_eq!(
            experiment_runner_profile_id_required_message(),
            "runner_profile_id is required"
        );
        assert_eq!(experiment_lease_revoked_message(), "Lease has been revoked");
        assert_eq!(experiment_lease_expired_message(), "Lease has expired");
        assert_eq!(
            invalid_experiment_lease_token_message(),
            "Invalid lease token"
        );
    }

    #[test]
    fn campaign_path_helpers_build_stable_short_paths_and_gateway_url() {
        let now = Utc::now();
        let campaign_id = Uuid::parse_str("12345678-90ab-cdef-1234-567890abcdef").unwrap();
        let campaign = ExperimentCampaign {
            id: campaign_id,
            project_id: Uuid::new_v4(),
            runner_profile_id: Uuid::new_v4(),
            owner_user_id: "user".to_string(),
            status: ExperimentCampaignStatus::Running,
            baseline_commit: None,
            best_commit: None,
            best_metrics: serde_json::json!({}),
            experiment_branch: None,
            remote_ref: None,
            worktree_path: None,
            started_at: Some(now),
            ended_at: None,
            trial_count: 0,
            failure_count: 0,
            pause_reason: None,
            queue_state: ExperimentCampaignQueueState::Active,
            queue_position: 0,
            active_trial_id: None,
            total_runtime_ms: 0,
            total_cost_usd: 0.0,
            total_llm_cost_usd: 0.0,
            total_runner_cost_usd: 0.0,
            consecutive_non_improving_trials: 0,
            max_trials_override: None,
            gateway_url: Some(" https://example.test/gateway ".to_string()),
            metadata: serde_json::json!({}),
            created_at: now,
            updated_at: now,
        };

        assert_eq!(short_id(campaign_id), "1234567890ab");
        assert_eq!(
            experiments_worktree_path("/workspace", campaign_id),
            PathBuf::from("/workspace/.thinclaw-experiments/1234567890ab")
        );
        assert_eq!(
            campaign_gateway_url(&campaign).as_deref(),
            Some("https://example.test/gateway")
        );
    }

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
}
