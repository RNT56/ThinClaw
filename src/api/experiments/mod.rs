//! Experiments API — optional research automation with local and remote runners.
//!
//! This module was decomposed from a single ~5900-line god-file into focused
//! submodules. The split is behavior-preserving: every public path
//! (`crate::api::experiments::*`) still resolves, the HTTP CRUD handlers, the
//! controller/reaper background loops, and the lease/runner callbacks are
//! unchanged. Submodules share the flat namespace through `use super::*;`, so
//! the shared imports, constants, statics, and small helper types below are the
//! single source the whole subsystem draws from.
//!
//! Layout:
//! - [`crud`] — HTTP CRUD handlers (projects/runners/campaigns/trials/artifacts/
//!   targets/usage/opportunities/gpu providers) and their shared guards.
//! - [`controller`] — the reconcile controller loop, the artifact-retention
//!   reaper loop, and the campaign queue helpers.
//! - [`campaign`] — campaign lifecycle actions (start/pause/cancel/resume/
//!   reissue/promote) plus baseline/candidate launch + worktree commit prep.
//! - [`leases`] — lease lifecycle (job/credentials/status/event/artifact/
//!   complete) and lease creation/verification.
//! - [`execution`] — trial execution (local + agent-env benchmark), trial
//!   finalization, cost/metric decisioning, and worktree restore.
//! - [`subagents`] — planner/mutator/reviewer research subagents and their
//!   run-artifact bookkeeping.
//! - [`git`] — git/worktree/shell-command helpers, execution-backend selection,
//!   and LLM cost attribution.
//! - [`types`] — small shared result/error types and the subagent/secrets
//!   registries.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use base64::Engine as _;
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
use crate::experiments::artifact_store::{ArtifactStore, LocalArtifactStore};
use crate::experiments::{
    CampaignStatusDecisionInput, ExperimentArtifactRef, ExperimentAutonomyMode, ExperimentCampaign,
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
const DEFAULT_ARTIFACT_REAPER_TICK_SECS: u64 = 86_400;
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

mod campaign;
mod controller;
mod crud;
mod execution;
mod git;
mod leases;
mod subagents;
mod types;

#[cfg(test)]
mod tests;

// Pull every submodule's internal (`pub(super)`) item into the façade namespace
// so sibling submodules resolve cross-module calls through their `use super::*;`
// the same way the original flat file resolved them. These globs do not widen
// the API beyond the crate; the externally stable surface is the explicit
// `pub use` list below.
use campaign::*;
use controller::*;
use crud::*;
use execution::*;
use git::*;
use leases::*;
use subagents::*;
use types::*;

// Re-export the stable public API surface so `crate::api::experiments::*` paths
// (gateway route registration, the `src/main.rs` controller/reaper spawns, the
// CLI, and the web handlers) keep resolving unchanged after the decomposition.
pub use campaign::{
    cancel_campaign, pause_campaign, promote_campaign, reissue_lease, resume_campaign,
    start_campaign,
};
pub use controller::{
    start_experiment_artifact_reaper_loop, start_experiment_artifact_reaper_loop_with_shutdown,
    start_experiment_controller_loop, start_experiment_controller_loop_with_shutdown,
};
pub use crud::{
    create_project, create_runner, create_target, delete_project, delete_runner, delete_target,
    get_campaign, get_project, get_runner, get_trial, link_target, list_artifacts, list_campaigns,
    list_gpu_cloud_providers, list_model_usage, list_opportunities, list_projects, list_runners,
    list_targets, list_trials, update_project, update_runner, update_target, validate_runner,
};
pub use leases::{
    lease_artifact, lease_complete, lease_credentials, lease_event, lease_job, lease_owner_user_id,
    lease_status,
};
pub use types::{register_experiment_secrets_store, register_experiment_subagent_executor};
