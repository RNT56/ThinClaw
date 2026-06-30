//! Skill tool policy: tap.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use thinclaw_tools_core::{ApprovalRequirement, Tool, ToolError, ToolOutput};
use thinclaw_types::JobContext;

use crate::ports::{
    SkillTapToolHostPort, ToolSkillTap, ToolSkillTapAddRequest, ToolSkillTapQuery,
    ToolSkillTapRefreshRequest, ToolSkillTapRemoveRequest, ToolSkillTapTrust,
    tool_scope_from_job_context,
};

use super::*;

pub fn skill_tap_list_parameters_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "include_health": {"type": "boolean", "default": false}
        }
    })
}

pub fn skill_tap_add_parameters_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "repo": {"type": "string", "description": "GitHub repo in owner/name form."},
            "path": {"type": "string", "default": ""},
            "branch": {"type": ["string", "null"], "default": null},
            "trust_level": {"type": "string", "enum": ["builtin", "trusted", "community"], "default": "community"},
            "replace": {"type": "boolean", "default": false}
        },
        "required": ["repo"]
    })
}

fn normalize_tap_branch(branch: Option<&str>) -> Option<String> {
    branch
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub fn parse_skill_tap_trust_level(value: &str) -> Result<String, ToolError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "builtin" => Ok("builtin".to_string()),
        "trusted" => Ok("trusted".to_string()),
        "community" | "" => Ok("community".to_string()),
        other => Err(ToolError::InvalidParameters(format!(
            "Unsupported trust_level '{}'",
            other
        ))),
    }
}

pub fn parse_skill_tap_add_params(
    params: &serde_json::Value,
) -> Result<SkillTapAddParams, ToolError> {
    let repo = required_str(params, "repo")?.trim().to_string();
    validate_github_repo(&repo)?;
    let path = normalize_tap_path(
        params
            .get("path")
            .and_then(|value| value.as_str())
            .unwrap_or(""),
    );
    validate_repo_relative_path(&path, "path")?;
    let branch = normalize_tap_branch(params.get("branch").and_then(|value| value.as_str()));
    let trust_level = parse_skill_tap_trust_level(
        params
            .get("trust_level")
            .and_then(|value| value.as_str())
            .unwrap_or("community"),
    )?;
    let replace = params
        .get("replace")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    Ok(SkillTapAddParams {
        repo,
        path,
        branch,
        trust_level,
        replace,
    })
}

pub fn skill_tap_remove_parameters_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "repo": {"type": "string"},
            "path": {"type": "string", "default": ""},
            "branch": {"type": ["string", "null"], "default": null}
        },
        "required": ["repo"]
    })
}

pub fn parse_skill_tap_remove_params(
    params: &serde_json::Value,
) -> Result<SkillTapRemoveParams, ToolError> {
    let repo = required_str(params, "repo")?.trim().to_string();
    validate_github_repo(&repo)?;
    let path = normalize_tap_path(
        params
            .get("path")
            .and_then(|value| value.as_str())
            .unwrap_or(""),
    );
    validate_repo_relative_path(&path, "path")?;
    let branch = normalize_tap_branch(params.get("branch").and_then(|value| value.as_str()));

    Ok(SkillTapRemoveParams { repo, path, branch })
}

pub fn skill_tap_refresh_parameters_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "repo": {"type": ["string", "null"], "default": null},
            "path": {"type": ["string", "null"], "default": null}
        }
    })
}

pub fn parse_skill_tap_refresh_params(
    params: &serde_json::Value,
) -> Result<SkillTapRefreshParams, ToolError> {
    let repo = params
        .get("repo")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if let Some(repo) = repo.as_deref() {
        validate_github_repo(repo)?;
    }
    let path = params
        .get("path")
        .and_then(|value| value.as_str())
        .map(normalize_tap_path)
        .filter(|value| !value.is_empty());
    if let Some(path) = path.as_deref() {
        validate_repo_relative_path(path, "path")?;
    }

    Ok(SkillTapRefreshParams { repo, path })
}

pub fn skill_tap_json(
    repo: &str,
    path: &str,
    branch: Option<&str>,
    trust_level: &str,
) -> serde_json::Value {
    serde_json::json!({
        "repo": repo,
        "path": path,
        "branch": branch,
        "trust_level": trust_level,
    })
}

pub fn skill_tap_list_output(
    taps: Vec<serde_json::Value>,
    hub_enabled: Option<bool>,
) -> serde_json::Value {
    serde_json::json!({
        "taps": taps,
        "count": taps.len(),
        "hub_enabled": hub_enabled,
    })
}

pub fn skill_tap_add_output(
    replaced: bool,
    tap: serde_json::Value,
    tap_count: usize,
) -> serde_json::Value {
    serde_json::json!({
        "status": if replaced { "replaced" } else { "added" },
        "tap": tap,
        "tap_count": tap_count,
    })
}

pub fn skill_tap_remove_output(
    repo: &str,
    path: &str,
    branch: Option<&str>,
    tap_count: usize,
) -> serde_json::Value {
    serde_json::json!({
        "status": "removed",
        "repo": repo,
        "path": path,
        "branch": branch,
        "tap_count": tap_count,
    })
}

pub fn skill_tap_refresh_output(
    tap_count: usize,
    repo: Option<&str>,
    path: Option<&str>,
    hub_enabled: bool,
) -> serde_json::Value {
    serde_json::json!({
        "status": "refreshed",
        "tap_count": tap_count,
        "filter": {
            "repo": repo,
            "path": path,
        },
        "hub_enabled": hub_enabled,
    })
}

fn tool_tap_trust_level(value: &str) -> ToolSkillTapTrust {
    match value {
        "builtin" => ToolSkillTapTrust::Builtin,
        "trusted" => ToolSkillTapTrust::Trusted,
        _ => ToolSkillTapTrust::Community,
    }
}

fn tool_tap_trust_level_label(value: ToolSkillTapTrust) -> &'static str {
    match value {
        ToolSkillTapTrust::Builtin => "builtin",
        ToolSkillTapTrust::Trusted => "trusted",
        ToolSkillTapTrust::Community => "community",
    }
}

fn tool_skill_tap_json(tap: &ToolSkillTap) -> serde_json::Value {
    skill_tap_json(
        &tap.repo,
        &tap.path,
        tap.branch.as_deref(),
        tool_tap_trust_level_label(tap.trust_level),
    )
}

pub struct SkillTapListHostTool {
    host: Arc<dyn SkillTapToolHostPort>,
}

pub struct SkillTapAddHostTool {
    host: Arc<dyn SkillTapToolHostPort>,
}

pub struct SkillTapRemoveHostTool {
    host: Arc<dyn SkillTapToolHostPort>,
}

pub struct SkillTapRefreshHostTool {
    host: Arc<dyn SkillTapToolHostPort>,
}

impl SkillTapListHostTool {
    pub fn new(host: Arc<dyn SkillTapToolHostPort>) -> Self {
        Self { host }
    }
}

impl SkillTapAddHostTool {
    pub fn new(host: Arc<dyn SkillTapToolHostPort>) -> Self {
        Self { host }
    }
}

impl SkillTapRemoveHostTool {
    pub fn new(host: Arc<dyn SkillTapToolHostPort>) -> Self {
        Self { host }
    }
}

impl SkillTapRefreshHostTool {
    pub fn new(host: Arc<dyn SkillTapToolHostPort>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Tool for SkillTapListHostTool {
    fn name(&self) -> &str {
        "skill_tap_list"
    }

    fn description(&self) -> &str {
        "List configured GitHub skill taps."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_tap_list_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(&ctx.metadata, self.name())?;
        let start = Instant::now();
        let include_health = params
            .get("include_health")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let result = self
            .host
            .list_skill_taps(ToolSkillTapQuery {
                scope: tool_scope_from_job_context(ctx),
                include_health,
            })
            .await
            .map_err(tool_host_error)?;

        Ok(ToolOutput::success(
            skill_tap_list_output(
                result
                    .taps
                    .iter()
                    .map(tool_skill_tap_json)
                    .collect::<Vec<_>>(),
                result.hub_enabled,
            ),
            start.elapsed(),
        ))
    }
}

#[async_trait]
impl Tool for SkillTapAddHostTool {
    fn name(&self) -> &str {
        "skill_tap_add"
    }

    fn description(&self) -> &str {
        "Persist a GitHub skill tap and refresh remote skill discovery."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_tap_add_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(&ctx.metadata, self.name())?;
        let start = Instant::now();
        let parsed = parse_skill_tap_add_params(&params)?;
        let result = self
            .host
            .add_skill_tap(ToolSkillTapAddRequest {
                scope: tool_scope_from_job_context(ctx),
                repo: parsed.repo,
                path: parsed.path,
                branch: parsed.branch,
                trust_level: tool_tap_trust_level(&parsed.trust_level),
                replace: parsed.replace,
            })
            .await
            .map_err(tool_host_error)?;
        let tap = result
            .tap
            .as_ref()
            .map(tool_skill_tap_json)
            .unwrap_or(serde_json::Value::Null);

        Ok(ToolOutput::success(
            skill_tap_add_output(result.status == "replaced", tap, result.tap_count),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

#[async_trait]
impl Tool for SkillTapRemoveHostTool {
    fn name(&self) -> &str {
        "skill_tap_remove"
    }

    fn description(&self) -> &str {
        "Remove a persisted GitHub skill tap and refresh remote skill discovery."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_tap_remove_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(&ctx.metadata, self.name())?;
        let start = Instant::now();
        let parsed = parse_skill_tap_remove_params(&params)?;
        let result = self
            .host
            .remove_skill_tap(ToolSkillTapRemoveRequest {
                scope: tool_scope_from_job_context(ctx),
                repo: parsed.repo.clone(),
                path: parsed.path.clone(),
                branch: parsed.branch.clone(),
            })
            .await
            .map_err(tool_host_error)?;

        Ok(ToolOutput::success(
            skill_tap_remove_output(
                &parsed.repo,
                &parsed.path,
                parsed.branch.as_deref(),
                result.tap_count,
            ),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

#[async_trait]
impl Tool for SkillTapRefreshHostTool {
    fn name(&self) -> &str {
        "skill_tap_refresh"
    }

    fn description(&self) -> &str {
        "Rebuild remote skill discovery from persisted skill tap settings."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_tap_refresh_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(&ctx.metadata, self.name())?;
        let start = Instant::now();
        let parsed = parse_skill_tap_refresh_params(&params)?;
        let result = self
            .host
            .refresh_skill_taps(ToolSkillTapRefreshRequest {
                scope: tool_scope_from_job_context(ctx),
                repo: parsed.repo,
                path: parsed.path,
            })
            .await
            .map_err(tool_host_error)?;

        Ok(ToolOutput::success(
            skill_tap_refresh_output(
                result.tap_count,
                result.repo.as_deref(),
                result.path.as_deref(),
                result.hub_enabled,
            ),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

pub fn skill_tap_key_matches(
    tap_repo: &str,
    tap_path: &str,
    tap_branch: Option<&str>,
    repo: &str,
    path: &str,
    branch: Option<&str>,
) -> bool {
    tap_repo.eq_ignore_ascii_case(repo)
        && normalize_tap_path(tap_path) == normalize_tap_path(path)
        && tap_branch == branch
}
