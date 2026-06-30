//! Small runtime support helpers: lease/path validation, formatting, JSON parsing.

use std::path::{Component, PathBuf};

use crate::types::*;
use serde::de::DeserializeOwned;
use uuid::Uuid;

pub fn hash_lease_token(token: &str) -> String {
    blake3::hash(token.as_bytes()).to_hex().to_string()
}

pub fn default_strategy_prompt() -> String {
    "Operate within the configured mutable paths only. Preserve the fixed harness, compare candidates against the best-known result, and stop when the campaign no longer improves.".to_string()
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
