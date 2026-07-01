//! Experiment lifecycle/status decision logic, metric comparison, and lease policy.

use crate::types::*;
use chrono::{DateTime, Utc};
use regex::Regex;

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

#[derive(Debug, Clone, Copy)]
pub struct CampaignStatusDecisionInput<'a> {
    pub campaign: &'a ExperimentCampaign,
    pub project: &'a ExperimentProject,
    pub trial: &'a ExperimentTrial,
    pub non_improving: u32,
    pub max_trials: Option<u32>,
    pub plateau_limit: u32,
    pub runtime_limit_reached: bool,
    pub cost_limit_reached: bool,
}

pub fn next_campaign_status(input: CampaignStatusDecisionInput<'_>) -> ExperimentCampaignStatus {
    let CampaignStatusDecisionInput {
        campaign,
        project,
        trial,
        non_improving,
        max_trials,
        plateau_limit,
        runtime_limit_reached,
        cost_limit_reached,
    } = input;

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

pub fn campaign_status_message(input: CampaignStatusDecisionInput<'_>) -> String {
    let CampaignStatusDecisionInput {
        campaign,
        project,
        trial,
        non_improving,
        max_trials,
        plateau_limit,
        runtime_limit_reached,
        cost_limit_reached,
    } = input;

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
