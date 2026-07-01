//! Operator-facing message strings for the experiments subsystem.

use std::fmt::Display;

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
