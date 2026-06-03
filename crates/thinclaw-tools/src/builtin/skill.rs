//! Root-independent skill tool policy helpers.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use thinclaw_tools_core::{ApprovalRequirement, Tool, ToolError, ToolOutput};
use thinclaw_types::JobContext;

#[cfg(test)]
use crate::ports::ToolOperationScope;
use crate::ports::{
    SkillInstallToolHostPort, SkillPublishToolHostPort, SkillSearchToolHostPort,
    SkillTapToolHostPort, SkillToolHostPort, ToolHostError, ToolSkillCheckRequest,
    ToolSkillCheckSource, ToolSkillInstallActionRequest, ToolSkillPublishRequest,
    ToolSkillPublishResult, ToolSkillQuery, ToolSkillRead, ToolSkillSearchCatalogEntry,
    ToolSkillSearchLocalEntry, ToolSkillSearchRemoteEntry, ToolSkillSearchRequest,
    ToolSkillSearchResult, ToolSkillSummary, ToolSkillTap, ToolSkillTapAddRequest,
    ToolSkillTapQuery, ToolSkillTapRefreshRequest, ToolSkillTapRemoveRequest, ToolSkillTapTrust,
    ToolSkillTrust, ToolSkillTrustMutationRequest, ToolSkillUpdateActionRequest,
    tool_scope_from_job_context,
};
use crate::registry::ToolRegistry;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillSearchParams {
    pub query: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillInspectParams {
    pub name: String,
    pub include_content: bool,
    pub include_files: bool,
    pub audit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillInstallParams {
    pub name: String,
    pub force: bool,
    pub approve_risky: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillUpdateParams {
    pub name: String,
    pub approve_risky: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillPublishParams {
    pub name: String,
    pub target_repo: String,
    pub dry_run: bool,
    pub remote_write: bool,
    pub confirm_remote_write: bool,
    pub approve_risky: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillReloadParams {
    pub name: Option<String>,
    pub all: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillTrustPromoteParams {
    pub name: String,
    pub target_trust: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillTapAddParams {
    pub repo: String,
    pub path: String,
    pub branch: Option<String>,
    pub trust_level: String,
    pub replace: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillTapRemoveParams {
    pub repo: String,
    pub path: String,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillTapRefreshParams {
    pub repo: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillCheckInput {
    InlineContent(String),
    Url(String),
    Path(String),
}

impl SkillCheckInput {
    pub fn source_kind(&self) -> &'static str {
        match self {
            Self::InlineContent(_) => "content",
            Self::Url(_) => "url",
            Self::Path(_) => "path",
        }
    }

    pub fn source_ref(&self) -> String {
        match self {
            Self::InlineContent(_) => "(inline content)".to_string(),
            Self::Url(url) => url.clone(),
            Self::Path(path) => path.clone(),
        }
    }

    pub fn inline_content(&self) -> Option<&str> {
        match self {
            Self::InlineContent(content) => Some(content),
            Self::Url(_) | Self::Path(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillListParams {
    pub verbose: bool,
}

fn required_str<'a>(params: &'a serde_json::Value, key: &str) -> Result<&'a str, ToolError> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidParameters(format!("missing required parameter: {}", key)))
}

pub fn restricted_skill_names(
    metadata: &serde_json::Value,
) -> Option<std::collections::HashSet<String>> {
    ToolRegistry::metadata_string_list(metadata, "allowed_skills")
        .map(|skills| skills.into_iter().collect())
}

pub fn ensure_skill_allowed(
    metadata: &serde_json::Value,
    skill_name: &str,
) -> Result<(), ToolError> {
    if ToolRegistry::skill_name_allowed_by_metadata(metadata, skill_name) {
        Ok(())
    } else {
        Err(ToolError::ExecutionFailed(format!(
            "Skill '{}' is not allowed in this agent context.",
            skill_name
        )))
    }
}

pub fn ensure_skill_admin_available(
    metadata: &serde_json::Value,
    tool_name: &str,
) -> Result<(), ToolError> {
    if ToolRegistry::metadata_string_list(metadata, "allowed_skills").is_some() {
        Err(ToolError::ExecutionFailed(format!(
            "Tool '{}' is not available when the current agent is restricted to a specific skill allowlist.",
            tool_name
        )))
    } else {
        Ok(())
    }
}

pub fn skill_inspect_parameters_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "description": "Name of the loaded skill to inspect."
            },
            "include_content": {
                "type": "boolean",
                "description": "Include full prompt content in the response.",
                "default": false
            },
            "include_files": {
                "type": "boolean",
                "description": "Include regular publishable files in the skill directory.",
                "default": true
            },
            "audit": {
                "type": "boolean",
                "description": "Run the quarantine scanner over the skill prompt.",
                "default": true
            }
        },
        "required": ["name"]
    })
}

pub fn parse_skill_inspect_params(
    params: &serde_json::Value,
) -> Result<SkillInspectParams, ToolError> {
    Ok(SkillInspectParams {
        name: required_str(params, "name")?.to_string(),
        include_content: params
            .get("include_content")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        include_files: params
            .get("include_files")
            .and_then(|value| value.as_bool())
            .unwrap_or(true),
        audit: params
            .get("audit")
            .and_then(|value| value.as_bool())
            .unwrap_or(true),
    })
}

pub fn skill_read_parameters_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "description": "Name of the skill to read (from skill_list or the Skills section)"
            }
        },
        "required": ["name"]
    })
}

pub fn parse_skill_name_param(params: &serde_json::Value) -> Result<String, ToolError> {
    Ok(required_str(params, "name")?.to_string())
}

pub fn skill_read_output(
    name: &str,
    version: &str,
    description: &str,
    trust: &str,
    source_tier: &str,
    content: &str,
) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "version": version,
        "description": description,
        "trust": trust,
        "source_tier": source_tier,
        "content": content,
    })
}

pub fn skill_source_output(kind: &str, path: &str) -> serde_json::Value {
    serde_json::json!({
        "kind": kind,
        "path": path,
    })
}

pub fn skill_inventory_error_output(error: &str) -> serde_json::Value {
    serde_json::json!({
        "error": error,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn skill_inspect_output(
    name: &str,
    version: &str,
    description: &str,
    activation: serde_json::Value,
    metadata: serde_json::Value,
    trust: &str,
    source_tier: &str,
    source: serde_json::Value,
    content_hash: &str,
    prompt_tokens_approx: usize,
    provenance_lock: Option<serde_json::Value>,
    findings: Vec<serde_json::Value>,
    files: Vec<serde_json::Value>,
    content: Option<&str>,
) -> serde_json::Value {
    let finding_count = findings.len();
    let file_count = files.len();
    let mut output = serde_json::json!({
        "name": name,
        "version": version,
        "description": description,
        "activation": activation,
        "metadata": metadata,
        "trust": trust,
        "source_tier": source_tier,
        "source": source,
        "content_hash": content_hash,
        "prompt_tokens_approx": prompt_tokens_approx,
        "provenance_lock": provenance_lock,
        "finding_count": finding_count,
        "findings": findings,
        "inventory": {
            "file_count": file_count,
            "files": files.clone(),
        },
        "files": files,
    });
    if let Some(content) = content {
        output["content"] = serde_json::Value::String(content.to_string());
    }
    output
}

pub fn skill_list_parameters_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "verbose": {
                "type": "boolean",
                "description": "Include extra detail (tags, content_hash, version)",
                "default": false
            }
        }
    })
}

pub fn parse_skill_list_params(params: &serde_json::Value) -> SkillListParams {
    SkillListParams {
        verbose: params
            .get("verbose")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
    }
}

pub fn skill_list_output(skills: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({
        "skills": skills,
        "count": skills.len(),
    })
}

pub fn skill_list_entry(
    name: &str,
    description: &str,
    trust: &str,
    source_tier: &str,
    source: &str,
    keywords: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "description": description,
        "trust": trust,
        "source_tier": source_tier,
        "source": source,
        "keywords": keywords,
    })
}

#[derive(Debug, Clone)]
pub struct SkillListVerboseFields {
    pub version: String,
    pub tags: serde_json::Value,
    pub content_hash: String,
    pub max_context_tokens: serde_json::Value,
    pub provenance: Option<serde_json::Value>,
    pub lifecycle_status: Option<serde_json::Value>,
    pub outcome_score: Option<serde_json::Value>,
    pub reuse_count: Option<serde_json::Value>,
    pub activation_reason: Option<serde_json::Value>,
}

pub fn add_skill_list_verbose_fields(
    entry: &mut serde_json::Value,
    fields: SkillListVerboseFields,
) {
    if let Some(obj) = entry.as_object_mut() {
        obj.insert(
            "version".to_string(),
            serde_json::Value::String(fields.version),
        );
        obj.insert("tags".to_string(), fields.tags);
        obj.insert(
            "content_hash".to_string(),
            serde_json::Value::String(fields.content_hash),
        );
        obj.insert("max_context_tokens".to_string(), fields.max_context_tokens);
        if let Some(value) = fields.provenance {
            obj.insert("provenance".to_string(), value);
        }
        if let Some(value) = fields.lifecycle_status {
            obj.insert("lifecycle_status".to_string(), value);
        }
        if let Some(value) = fields.outcome_score {
            obj.insert("outcome_score".to_string(), value);
        }
        if let Some(value) = fields.reuse_count {
            obj.insert("reuse_count".to_string(), value);
        }
        if let Some(value) = fields.activation_reason {
            obj.insert("activation_reason".to_string(), value);
        }
    }
}

pub fn skill_search_parameters_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "query": {
                "type": "string",
                "description": "Search query (name, keyword, or description fragment)"
            },
            "source": {
                "type": "string",
                "enum": ["all", "clawhub", "github", "well_known"],
                "description": "Optional source filter.",
                "default": "all"
            }
        },
        "required": ["query"]
    })
}

pub fn skill_check_parameters_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "content": {
                "type": "string",
                "description": "Raw SKILL.md content to validate."
            },
            "path": {
                "type": "string",
                "description": "Local SKILL.md file path or skill directory to validate."
            },
            "url": {
                "type": "string",
                "description": "Direct HTTPS URL to a SKILL.md file to fetch and validate."
            }
        }
    })
}

pub fn parse_skill_check_input(params: &serde_json::Value) -> Result<SkillCheckInput, ToolError> {
    let content = params.get("content").and_then(|value| value.as_str());
    let path = params.get("path").and_then(|value| value.as_str());
    let url = params.get("url").and_then(|value| value.as_str());

    let provided = [content.is_some(), path.is_some(), url.is_some()]
        .into_iter()
        .filter(|present| *present)
        .count();
    if provided != 1 {
        return Err(ToolError::InvalidParameters(
            "Provide exactly one of content, path, or url".to_string(),
        ));
    }

    if let Some(content) = content {
        Ok(SkillCheckInput::InlineContent(content.to_string()))
    } else if let Some(url) = url {
        Ok(SkillCheckInput::Url(url.to_string()))
    } else {
        Ok(SkillCheckInput::Path(
            path.expect("provided count checked path presence")
                .to_string(),
        ))
    }
}

pub fn skill_check_path_for_read(path: &str) -> PathBuf {
    let path_buf = PathBuf::from(path);
    if path_buf.file_name().and_then(|name| name.to_str()) == Some("SKILL.md") {
        path_buf
    } else {
        path_buf.join("SKILL.md")
    }
}

pub fn skill_finding_output(kind: &str, severity: &str, excerpt: &str) -> serde_json::Value {
    serde_json::json!({
        "kind": kind,
        "severity": severity,
        "excerpt": excerpt,
    })
}

pub fn skill_finding_summary(kind: &str, severity: &str, excerpt: &str) -> String {
    format!("{} ({}): {}", kind, severity, excerpt)
}

pub fn skill_findings_summary<I>(findings: I) -> String
where
    I: IntoIterator<Item = String>,
{
    findings.into_iter().collect::<Vec<_>>().join("; ")
}

pub fn skill_findings_require_approval(trust_level: &str, finding_count: usize) -> bool {
    trust_level.eq_ignore_ascii_case("community") && finding_count > 0
}

#[allow(clippy::too_many_arguments)]
pub fn skill_check_success_output(
    source_kind: &str,
    source_ref: &str,
    name: &str,
    version: &str,
    description: &str,
    activation: serde_json::Value,
    trust: &str,
    source_tier: &str,
    prompt_tokens_approx: usize,
    declared_max_context_tokens: usize,
    content_hash: &str,
    normalized_content_hash: &str,
    findings: Vec<serde_json::Value>,
) -> serde_json::Value {
    let finding_count = findings.len();
    serde_json::json!({
        "ok": true,
        "source_kind": source_kind,
        "source_ref": source_ref,
        "name": name,
        "version": version,
        "description": description,
        "activation": activation,
        "trust": trust,
        "source_tier": source_tier,
        "prompt_tokens_approx": prompt_tokens_approx,
        "declared_max_context_tokens": declared_max_context_tokens,
        "content_hash": content_hash,
        "normalized_content_hash": normalized_content_hash,
        "finding_count": finding_count,
        "findings": findings,
    })
}

pub fn skill_check_error_output(
    source_kind: &str,
    source_ref: &str,
    error: &str,
    normalized_content_hash: &str,
    findings: Vec<serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "ok": false,
        "source_kind": source_kind,
        "source_ref": source_ref,
        "error": error,
        "normalized_content_hash": normalized_content_hash,
        "finding_count": findings.len(),
        "findings": findings,
    })
}

pub fn skill_install_parameters_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "description": "Skill name or slug (from search results). Used as the catalog lookup key if neither url nor content is provided."
            },
            "url": {
                "type": "string",
                "description": "Optional: direct URL to a SKILL.md file (skips catalog lookup)"
            },
            "content": {
                "type": "string",
                "description": "Optional: raw SKILL.md content to install directly (skips fetch)"
            },
            "force": {
                "type": "boolean",
                "description": "If true, removes the existing skill before installing the new version (update/upgrade)",
                "default": false
            },
            "approve_risky": {
                "type": "boolean",
                "description": "Approve installation even when the quarantine scan finds risky patterns in a community skill.",
                "default": false
            }
        },
        "required": ["name"]
    })
}

pub fn parse_skill_install_params(
    params: &serde_json::Value,
) -> Result<SkillInstallParams, ToolError> {
    Ok(SkillInstallParams {
        name: required_str(params, "name")?.to_string(),
        force: params
            .get("force")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        approve_risky: params
            .get("approve_risky")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
    })
}

pub fn skill_install_output(
    installed_name: &str,
    force: bool,
    findings: Vec<serde_json::Value>,
) -> serde_json::Value {
    let action = if force { "updated" } else { "installed" };
    serde_json::json!({
        "name": installed_name,
        "status": action,
        "trust": "installed",
        "findings": findings,
        "message": format!(
            "Skill '{}' {} successfully. It will activate when matching keywords are detected.",
            installed_name, action
        ),
    })
}

pub fn skill_audit_parameters_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "description": "Optional single skill name to audit. Omit to audit all loaded skills."
            }
        }
    })
}

pub fn parse_skill_audit_target_name(params: &serde_json::Value) -> Option<&str> {
    params.get("name").and_then(|value| value.as_str())
}

pub fn skill_audit_output(audited: Vec<serde_json::Value>) -> serde_json::Value {
    let total_findings = audited
        .iter()
        .map(|entry| {
            entry
                .get("finding_count")
                .and_then(|value| value.as_u64())
                .unwrap_or(0)
        })
        .sum::<u64>();

    serde_json::json!({
        "audited": audited,
        "audited_count": audited.len(),
        "total_findings": total_findings,
    })
}

pub fn skill_audit_entry_output(
    name: &str,
    trust: &str,
    source_tier: &str,
    source_path: &str,
    findings: Vec<serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "trust": trust,
        "source_tier": source_tier,
        "source_path": source_path,
        "finding_count": findings.len(),
        "findings": findings,
    })
}

pub fn skill_update_parameters_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "description": "Skill name to update."
            },
            "approve_risky": {
                "type": "boolean",
                "description": "Approve update even when the quarantine scan reports risky patterns.",
                "default": false
            }
        },
        "required": ["name"]
    })
}

pub fn parse_skill_update_params(
    params: &serde_json::Value,
) -> Result<SkillUpdateParams, ToolError> {
    Ok(SkillUpdateParams {
        name: required_str(params, "name")?.to_string(),
        approve_risky: params
            .get("approve_risky")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
    })
}

pub fn skill_update_install_params(
    name: &str,
    force: bool,
    approve_risky: bool,
) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "force": force,
        "approve_risky": approve_risky,
    })
}

pub fn add_skill_update_url(params: &mut serde_json::Value, url: String) {
    params["url"] = serde_json::Value::String(url);
}

pub fn skill_publish_parameters_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "name": {"type": "string", "description": "Loaded skill name to publish."},
            "target_repo": {"type": "string", "description": "Configured GitHub tap repo in owner/name form."},
            "dry_run": {"type": "boolean", "default": true},
            "remote_write": {"type": "boolean", "default": false},
            "confirm_remote_write": {"type": "boolean", "default": false},
            "approve_risky": {"type": "boolean", "default": false}
        },
        "required": ["name", "target_repo"]
    })
}

pub fn parse_skill_publish_params(
    params: &serde_json::Value,
) -> Result<SkillPublishParams, ToolError> {
    Ok(SkillPublishParams {
        name: required_str(params, "name")?.to_string(),
        target_repo: required_str(params, "target_repo")?.trim().to_string(),
        dry_run: params
            .get("dry_run")
            .and_then(|value| value.as_bool())
            .unwrap_or(true),
        remote_write: params
            .get("remote_write")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        confirm_remote_write: params
            .get("confirm_remote_write")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        approve_risky: params
            .get("approve_risky")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
    })
}

#[allow(clippy::too_many_arguments)]
pub fn skill_publish_plan_output(
    status: &str,
    name: &str,
    target_repo: &str,
    tap_path: &str,
    package_path: &str,
    branch: &str,
    base_branch: Option<&str>,
    package_hash: &str,
    files: Vec<serde_json::Value>,
    findings: Vec<serde_json::Value>,
    trust: &str,
    source_tier: &str,
    source: serde_json::Value,
) -> serde_json::Value {
    let commit_message = format!("feat(skills): publish {}", name);
    let pr_title = format!("[skills] publish {}", name);
    let file_count = files.len();
    let finding_count = findings.len();
    serde_json::json!({
        "status": status,
        "name": name,
        "target_repo": target_repo,
        "tap_path": tap_path,
        "package_path": package_path,
        "branch": branch,
        "base_branch": base_branch,
        "package_hash": package_hash,
        "files": files,
        "file_count": file_count,
        "finding_count": finding_count,
        "findings": findings,
        "trust": trust,
        "source_tier": source_tier,
        "source": source,
        "remote_write_plan": {
            "repo_url": format!("https://github.com/{}.git", target_repo),
            "base_branch": base_branch,
            "branch": branch,
            "package_path": package_path,
            "commit_message": commit_message,
            "push": {
                "remote": "origin",
                "branch": branch,
            },
            "pull_request": {
                "draft": true,
                "title": pr_title,
                "repo": target_repo,
            },
        },
    })
}

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

fn tool_host_error(error: ToolHostError) -> ToolError {
    ToolError::ExecutionFailed(error.to_string())
}

fn tool_skill_trust_label(trust: ToolSkillTrust) -> &'static str {
    match trust {
        ToolSkillTrust::Installed => "installed",
        ToolSkillTrust::Trusted => "trusted",
        ToolSkillTrust::Community => "community",
    }
}

fn metadata_string<'a>(metadata: &'a serde_json::Value, key: &str, fallback: &'a str) -> &'a str {
    metadata
        .get(key)
        .and_then(|value| value.as_str())
        .unwrap_or(fallback)
}

fn metadata_value_or(
    metadata: &serde_json::Value,
    key: &str,
    fallback: serde_json::Value,
) -> serde_json::Value {
    metadata.get(key).cloned().unwrap_or(fallback)
}

fn tool_skill_list_entry(summary: &ToolSkillSummary, verbose: bool) -> serde_json::Value {
    let metadata = &summary.metadata;
    let mut entry = skill_list_entry(
        &summary.name,
        summary.description.as_deref().unwrap_or_default(),
        tool_skill_trust_label(summary.trust),
        metadata_string(metadata, "source_tier", "community"),
        metadata_string(metadata, "source", ""),
        metadata_value_or(metadata, "keywords", serde_json::json!([])),
    );

    if verbose {
        add_skill_list_verbose_fields(
            &mut entry,
            SkillListVerboseFields {
                version: metadata_string(metadata, "version", "").to_string(),
                tags: metadata_value_or(metadata, "tags", serde_json::json!([])),
                content_hash: metadata_string(metadata, "content_hash", "").to_string(),
                max_context_tokens: metadata_value_or(
                    metadata,
                    "max_context_tokens",
                    serde_json::Value::Null,
                ),
                provenance: metadata.get("provenance").cloned(),
                lifecycle_status: metadata.get("lifecycle_status").cloned(),
                outcome_score: metadata.get("outcome_score").cloned(),
                reuse_count: metadata.get("reuse_count").cloned(),
                activation_reason: metadata.get("activation_reason").cloned(),
            },
        );
    }

    entry
}

pub struct SkillListHostTool {
    host: Arc<dyn SkillToolHostPort>,
}

impl SkillListHostTool {
    pub fn new(host: Arc<dyn SkillToolHostPort>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Tool for SkillListHostTool {
    fn name(&self) -> &str {
        "skill_list"
    }

    fn description(&self) -> &str {
        "List all loaded skills with their trust level, source, and activation keywords."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_list_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let parsed = parse_skill_list_params(&params);
        let summaries = self
            .host
            .list_skills(ToolSkillQuery {
                scope: tool_scope_from_job_context(ctx),
                query: None,
                source: None,
            })
            .await
            .map_err(tool_host_error)?;
        let skills = summaries
            .iter()
            .map(|summary| tool_skill_list_entry(summary, parsed.verbose))
            .collect();
        Ok(ToolOutput::success(
            skill_list_output(skills),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }
}

fn tool_skill_search_catalog_entry(entry: &ToolSkillSearchCatalogEntry) -> serde_json::Value {
    skill_search_catalog_entry(
        &entry.slug,
        &entry.name,
        &entry.description,
        &entry.version,
        entry.score,
        entry.installed,
        entry.stars,
        entry.downloads,
        entry.owner.as_deref(),
    )
}

fn tool_skill_search_remote_entry(entry: &ToolSkillSearchRemoteEntry) -> serde_json::Value {
    skill_search_remote_entry(
        &entry.slug,
        &entry.name,
        &entry.description,
        &entry.version,
        &entry.source,
        &entry.source_label,
        &entry.source_ref,
        entry.manifest_url.as_deref(),
        entry.manifest_digest.as_deref(),
        entry.repo.as_deref(),
        entry.path.as_deref(),
        entry.branch.as_deref(),
        &entry.trust_level,
    )
}

fn tool_skill_search_local_entry(entry: &ToolSkillSearchLocalEntry) -> serde_json::Value {
    skill_search_local_entry(
        &entry.name,
        &entry.description,
        &entry.trust,
        &entry.source_tier,
    )
}

fn tool_skill_search_output(
    source_filter: &str,
    result: &ToolSkillSearchResult,
) -> serde_json::Value {
    skill_search_output(
        source_filter,
        result
            .catalog
            .iter()
            .map(tool_skill_search_catalog_entry)
            .collect(),
        result
            .remote
            .iter()
            .map(tool_skill_search_remote_entry)
            .collect(),
        result
            .local
            .iter()
            .map(tool_skill_search_local_entry)
            .collect(),
        &result.registry_url,
        result.catalog_error.clone(),
    )
}

pub struct SkillSearchHostTool {
    host: Arc<dyn SkillSearchToolHostPort>,
}

impl SkillSearchHostTool {
    pub fn new(host: Arc<dyn SkillSearchToolHostPort>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Tool for SkillSearchHostTool {
    fn name(&self) -> &str {
        "skill_search"
    }

    fn description(&self) -> &str {
        "Search for skills in the ClawHub catalog and among locally loaded skills."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_search_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(&ctx.metadata, self.name())?;
        let start = Instant::now();
        let parsed = parse_skill_search_params(&params)?;
        let result = self
            .host
            .search_skills(ToolSkillSearchRequest {
                scope: tool_scope_from_job_context(ctx),
                query: parsed.query,
                source: parsed.source.clone(),
            })
            .await
            .map_err(tool_host_error)?;
        Ok(ToolOutput::success(
            tool_skill_search_output(&parsed.source, &result),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }
}

fn tool_skill_read_output(read: &ToolSkillRead) -> serde_json::Value {
    skill_read_output(
        &read.name,
        &read.version,
        &read.description,
        tool_skill_trust_label(read.trust),
        &read.source_tier,
        &read.content,
    )
}

pub struct SkillReadHostTool {
    host: Arc<dyn SkillToolHostPort>,
}

impl SkillReadHostTool {
    pub fn new(host: Arc<dyn SkillToolHostPort>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Tool for SkillReadHostTool {
    fn name(&self) -> &str {
        "skill_read"
    }

    fn description(&self) -> &str {
        "Read a skill's full instructions by name. Use when you need detailed guidance for a specific skill."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_read_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let name = parse_skill_name_param(&params)?;
        ensure_skill_allowed(&ctx.metadata, &name)?;
        let output = self
            .host
            .read_skill(tool_scope_from_job_context(ctx), name)
            .await
            .map_err(tool_host_error)?;
        Ok(ToolOutput::success(
            tool_skill_read_output(&output),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }
}

pub struct SkillReloadHostTool {
    host: Arc<dyn SkillToolHostPort>,
}

impl SkillReloadHostTool {
    pub fn new(host: Arc<dyn SkillToolHostPort>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Tool for SkillReloadHostTool {
    fn name(&self) -> &str {
        "skill_reload"
    }

    fn description(&self) -> &str {
        "Reload a skill (or all skills) from disk after editing SKILL.md files. \
         Use after making on-disk changes so they take effect immediately without restarting. \
         Provide a skill name to reload just that skill, or set all=true to rediscover all skills."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_reload_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(&ctx.metadata, self.name())?;
        let start = Instant::now();
        let parsed = parse_skill_reload_params(&params);
        let scope = tool_scope_from_job_context(ctx);

        if parsed.all {
            let loaded = self
                .host
                .reload_skills(scope, None)
                .await
                .map_err(tool_host_error)?
                .into_iter()
                .map(|summary| summary.name)
                .collect::<Vec<_>>();
            return Ok(ToolOutput::success(
                skill_reload_all_output(loaded),
                start.elapsed(),
            ));
        }

        let name = parsed.name.ok_or_else(|| {
            ToolError::InvalidParameters("missing required parameter: name".to_string())
        })?;
        let mut loaded = self
            .host
            .reload_skills(scope, Some(name.clone()))
            .await
            .map_err(tool_host_error)?;
        let reloaded_name = loaded.pop().map(|summary| summary.name).unwrap_or(name);
        Ok(ToolOutput::success(
            skill_reload_output(&reloaded_name),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

pub struct SkillSnapshotHostTool {
    host: Arc<dyn SkillToolHostPort>,
}

impl SkillSnapshotHostTool {
    pub fn new(host: Arc<dyn SkillToolHostPort>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Tool for SkillSnapshotHostTool {
    fn name(&self) -> &str {
        "skill_snapshot"
    }

    fn description(&self) -> &str {
        "Write a JSON snapshot of loaded skills, hashes, and provenance tiers to the local skills state directory."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_snapshot_parameters_schema()
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(&ctx.metadata, self.name())?;
        let start = Instant::now();
        let result = self
            .host
            .snapshot_skills(tool_scope_from_job_context(ctx))
            .await
            .map_err(tool_host_error)?;
        Ok(ToolOutput::success(
            skill_snapshot_output(&result.path, result.count),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

fn tool_skill_check_source(input: SkillCheckInput) -> ToolSkillCheckSource {
    match input {
        SkillCheckInput::InlineContent(content) => ToolSkillCheckSource::InlineContent { content },
        SkillCheckInput::Path(path) => ToolSkillCheckSource::Path { path },
        SkillCheckInput::Url(url) => ToolSkillCheckSource::Url { url },
    }
}

pub struct SkillCheckHostTool {
    host: Arc<dyn SkillToolHostPort>,
}

impl SkillCheckHostTool {
    pub fn new(host: Arc<dyn SkillToolHostPort>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Tool for SkillCheckHostTool {
    fn name(&self) -> &str {
        "skill_check"
    }

    fn description(&self) -> &str {
        "Validate SKILL.md content, a local SKILL.md path, or a direct HTTPS SKILL.md URL without installing it."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_check_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(&ctx.metadata, self.name())?;
        let start = Instant::now();
        let input = parse_skill_check_input(&params)?;
        let result = self
            .host
            .check_skill(ToolSkillCheckRequest {
                scope: tool_scope_from_job_context(ctx),
                source: tool_skill_check_source(input),
            })
            .await
            .map_err(tool_host_error)?;
        Ok(ToolOutput::success(result.output, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }
}

pub struct SkillInstallHostTool {
    host: Arc<dyn SkillInstallToolHostPort>,
}

impl SkillInstallHostTool {
    pub fn new(host: Arc<dyn SkillInstallToolHostPort>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Tool for SkillInstallHostTool {
    fn name(&self) -> &str {
        "skill_install"
    }

    fn description(&self) -> &str {
        "Install a skill by name from the catalog, a URL, or inline SKILL.md content."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_install_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(&ctx.metadata, self.name())?;
        let start = Instant::now();
        let parsed = parse_skill_install_params(&params)?;
        let result = self
            .host
            .install_skill_action(ToolSkillInstallActionRequest {
                scope: tool_scope_from_job_context(ctx),
                name: parsed.name,
                url: params
                    .get("url")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                content: params
                    .get("content")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                force: parsed.force,
                approve_risky: parsed.approve_risky,
            })
            .await
            .map_err(tool_host_error)?;
        Ok(ToolOutput::success(result.output, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

pub struct SkillUpdateHostTool {
    host: Arc<dyn SkillInstallToolHostPort>,
}

impl SkillUpdateHostTool {
    pub fn new(host: Arc<dyn SkillInstallToolHostPort>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Tool for SkillUpdateHostTool {
    fn name(&self) -> &str {
        "skill_update"
    }

    fn description(&self) -> &str {
        "Update an installed skill from its recorded provenance source."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_update_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(&ctx.metadata, self.name())?;
        let start = Instant::now();
        let parsed = parse_skill_update_params(&params)?;
        let result = self
            .host
            .update_skill_action(ToolSkillUpdateActionRequest {
                scope: tool_scope_from_job_context(ctx),
                name: parsed.name,
                approve_risky: parsed.approve_risky,
            })
            .await
            .map_err(tool_host_error)?;
        Ok(ToolOutput::success(result.output, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

pub struct SkillRemoveHostTool {
    host: Arc<dyn SkillToolHostPort>,
}

impl SkillRemoveHostTool {
    pub fn new(host: Arc<dyn SkillToolHostPort>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Tool for SkillRemoveHostTool {
    fn name(&self) -> &str {
        "skill_remove"
    }

    fn description(&self) -> &str {
        "Remove an installed skill by name. Only user-installed skills can be removed."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_remove_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(&ctx.metadata, self.name())?;
        let start = Instant::now();
        let name = parse_skill_name_param(&params)?;
        let result = self
            .host
            .remove_skill(tool_scope_from_job_context(ctx), name)
            .await
            .map_err(tool_host_error)?;
        Ok(ToolOutput::success(
            skill_remove_output(&result.name),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

pub struct SkillAuditHostTool {
    host: Arc<dyn SkillToolHostPort>,
}

impl SkillAuditHostTool {
    pub fn new(host: Arc<dyn SkillToolHostPort>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Tool for SkillAuditHostTool {
    fn name(&self) -> &str {
        "skill_audit"
    }

    fn description(&self) -> &str {
        "Audit loaded skills for risky patterns using the quarantine scanner without modifying or removing them."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_audit_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(&ctx.metadata, self.name())?;
        let start = Instant::now();
        let target = parse_skill_audit_target_name(&params).map(str::to_string);
        let audited = self
            .host
            .audit_skills(tool_scope_from_job_context(ctx), target)
            .await
            .map_err(tool_host_error)?;
        Ok(ToolOutput::success(
            skill_audit_output(audited),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }
}

fn tool_skill_trust_from_label(label: &str) -> Result<ToolSkillTrust, ToolError> {
    match label {
        "installed" => Ok(ToolSkillTrust::Installed),
        "trusted" => Ok(ToolSkillTrust::Trusted),
        _ => Err(ToolError::InvalidParameters(format!(
            "Unsupported target_trust '{}'",
            label
        ))),
    }
}

pub struct SkillPromoteTrustHostTool {
    host: Arc<dyn SkillToolHostPort>,
}

impl SkillPromoteTrustHostTool {
    pub fn new(host: Arc<dyn SkillToolHostPort>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Tool for SkillPromoteTrustHostTool {
    fn name(&self) -> &str {
        "skill_trust_promote"
    }

    fn description(&self) -> &str {
        "Promote or demote a user-managed skill between installed and trusted trust ceilings."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_trust_promote_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(&ctx.metadata, self.name())?;
        let start = Instant::now();
        let parsed = parse_skill_trust_promote_params(&params)?;
        let result = self
            .host
            .promote_skill_trust(ToolSkillTrustMutationRequest {
                scope: tool_scope_from_job_context(ctx),
                name: parsed.name,
                target_trust: tool_skill_trust_from_label(&parsed.target_trust)?,
            })
            .await
            .map_err(tool_host_error)?;
        Ok(ToolOutput::success(
            skill_trust_promote_output(
                &result.name,
                tool_skill_trust_label(result.trust),
                &result.source_tier,
            ),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

pub struct SkillInspectHostTool {
    host: Arc<dyn SkillToolHostPort>,
}

impl SkillInspectHostTool {
    pub fn new(host: Arc<dyn SkillToolHostPort>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Tool for SkillInspectHostTool {
    fn name(&self) -> &str {
        "skill_inspect"
    }

    fn description(&self) -> &str {
        "Inspect one loaded skill with metadata, provenance, files, and optional audit findings."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_inspect_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let parsed = parse_skill_inspect_params(&params)?;
        ensure_skill_allowed(&ctx.metadata, &parsed.name)?;
        let output = self
            .host
            .inspect_skill(
                tool_scope_from_job_context(ctx),
                parsed.name,
                parsed.include_content,
                parsed.include_files,
                parsed.audit,
            )
            .await
            .map_err(tool_host_error)?;
        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }
}

fn tool_skill_publish_output(result: &ToolSkillPublishResult) -> serde_json::Value {
    let mut output = skill_publish_plan_output(
        &result.status,
        &result.name,
        &result.target_repo,
        &result.tap_path,
        &result.package_path,
        &result.branch,
        result.base_branch.as_deref(),
        &result.package_hash,
        result.files.clone(),
        result.findings.clone(),
        &result.trust,
        &result.source_tier,
        result.source.clone(),
    );

    if !result.remote_write_plan.is_null() {
        output["remote_write_plan"] = result.remote_write_plan.clone();
    }
    if let Some(metadata) = result.metadata.as_object()
        && let Some(output_object) = output.as_object_mut()
    {
        for (key, value) in metadata {
            output_object.insert(key.clone(), value.clone());
        }
    }
    output
}

pub struct SkillPublishHostTool {
    host: Arc<dyn SkillPublishToolHostPort>,
}

impl SkillPublishHostTool {
    pub fn new(host: Arc<dyn SkillPublishToolHostPort>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Tool for SkillPublishHostTool {
    fn name(&self) -> &str {
        "skill_publish"
    }

    fn description(&self) -> &str {
        "Dry-run or publish a local skill to a configured GitHub skill tap as a draft pull request."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_publish_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(&ctx.metadata, self.name())?;
        let start = Instant::now();
        let parsed = parse_skill_publish_params(&params)?;
        let result = self
            .host
            .publish_skill(ToolSkillPublishRequest {
                scope: tool_scope_from_job_context(ctx),
                name: parsed.name,
                target_repo: parsed.target_repo,
                dry_run: parsed.dry_run,
                remote_write: parsed.remote_write,
                confirm_remote_write: parsed.confirm_remote_write,
                approve_risky: parsed.approve_risky,
            })
            .await
            .map_err(tool_host_error)?;

        Ok(ToolOutput::success(
            tool_skill_publish_output(&result),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, params: &serde_json::Value) -> ApprovalRequirement {
        if params
            .get("remote_write")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            ApprovalRequirement::UnlessAutoApproved
        } else {
            ApprovalRequirement::Never
        }
    }
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

pub fn skill_snapshot_parameters_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {}
    })
}

pub fn skill_snapshot_document(
    generated_at: String,
    skills: Vec<serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "generated_at": generated_at,
        "skills": skills,
    })
}

pub fn skill_snapshot_entry(
    name: &str,
    version: &str,
    trust: &str,
    source_tier: &str,
    content_hash: &str,
    source_path: Option<String>,
) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "version": version,
        "trust": trust,
        "source_tier": source_tier,
        "content_hash": content_hash,
        "source_path": source_path,
    })
}

pub fn skill_snapshot_output(path: &str, count: usize) -> serde_json::Value {
    serde_json::json!({
        "path": path,
        "count": count,
    })
}

pub fn skill_trust_promote_parameters_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "description": "Skill name to move between trust ceilings."
            },
            "target_trust": {
                "type": "string",
                "enum": ["installed", "trusted"],
                "description": "Target trust ceiling."
            }
        },
        "required": ["name", "target_trust"]
    })
}

pub fn parse_skill_trust_promote_params(
    params: &serde_json::Value,
) -> Result<SkillTrustPromoteParams, ToolError> {
    let target_trust = required_str(params, "target_trust")?
        .trim()
        .to_ascii_lowercase();
    match target_trust.as_str() {
        "installed" | "trusted" => {}
        other => {
            return Err(ToolError::InvalidParameters(format!(
                "Unsupported target_trust '{}'",
                other
            )));
        }
    }

    Ok(SkillTrustPromoteParams {
        name: required_str(params, "name")?.to_string(),
        target_trust,
    })
}

pub fn skill_trust_promote_output(name: &str, trust: &str, source_tier: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "trust": trust,
        "source_tier": source_tier,
        "status": "updated",
    })
}

pub fn skill_remove_parameters_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "description": "Name of the skill to remove"
            }
        },
        "required": ["name"]
    })
}

pub fn skill_remove_output(name: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "status": "removed",
        "message": format!("Skill '{}' has been removed.", name),
    })
}

pub fn skill_reload_parameters_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "description": "Name of the specific skill to reload from disk. Required unless all=true."
            },
            "all": {
                "type": "boolean",
                "description": "When true, reload ALL skills (full re-discovery). Use after adding new skill files on disk.",
                "default": false
            }
        }
    })
}

pub fn parse_skill_reload_params(params: &serde_json::Value) -> SkillReloadParams {
    SkillReloadParams {
        name: params
            .get("name")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        all: params
            .get("all")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
    }
}

pub fn skill_reload_all_output(loaded: Vec<String>) -> serde_json::Value {
    serde_json::json!({
        "status": "reloaded_all",
        "skills": loaded,
        "count": loaded.len(),
        "message": format!("Reloaded all skills: {}", loaded.join(", ")),
    })
}

pub fn skill_reload_output(name: &str) -> serde_json::Value {
    serde_json::json!({
        "status": "reloaded",
        "name": name,
        "message": format!(
            "Skill '{}' has been reloaded from disk. \
             Updated keywords, descriptions, and prompt content are now active.",
            name
        ),
    })
}

pub fn parse_skill_search_params(
    params: &serde_json::Value,
) -> Result<SkillSearchParams, ToolError> {
    Ok(SkillSearchParams {
        query: required_str(params, "query")?.to_string(),
        source: params
            .get("source")
            .and_then(|value| value.as_str())
            .unwrap_or("all")
            .to_ascii_lowercase(),
    })
}

pub fn skill_search_local_entry(
    name: &str,
    description: &str,
    trust: &str,
    source_tier: &str,
) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "description": description,
        "trust": trust,
        "source_tier": source_tier,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn skill_search_catalog_entry(
    slug: &str,
    name: &str,
    description: &str,
    version: &str,
    score: f64,
    installed: bool,
    stars: Option<u64>,
    downloads: Option<u64>,
    owner: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "slug": slug,
        "name": name,
        "description": description,
        "version": version,
        "score": score,
        "installed": installed,
        "stars": stars,
        "downloads": downloads,
        "owner": owner,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn skill_search_remote_entry(
    slug: &str,
    name: &str,
    description: &str,
    version: &str,
    source: &str,
    source_label: &str,
    source_ref: &str,
    manifest_url: Option<&str>,
    manifest_digest: Option<&str>,
    repo: Option<&str>,
    path: Option<&str>,
    branch: Option<&str>,
    trust_level: &str,
) -> serde_json::Value {
    serde_json::json!({
        "slug": slug,
        "name": name,
        "description": description,
        "version": version,
        "source": source,
        "source_label": source_label,
        "source_ref": source_ref,
        "manifest_url": manifest_url,
        "manifest_digest": manifest_digest,
        "repo": repo,
        "path": path,
        "branch": branch,
        "trust_level": trust_level,
    })
}

pub fn skill_search_output(
    source_filter: &str,
    catalog_json: Vec<serde_json::Value>,
    remote_json: Vec<serde_json::Value>,
    local_matches: Vec<serde_json::Value>,
    registry_url: &str,
    catalog_error: Option<String>,
) -> serde_json::Value {
    let github_json: Vec<serde_json::Value> = remote_json
        .iter()
        .filter(|entry| entry.get("source").and_then(|v| v.as_str()) == Some("github_tap"))
        .cloned()
        .collect();
    let well_known_json: Vec<serde_json::Value> = remote_json
        .iter()
        .filter(|entry| entry.get("source").and_then(|v| v.as_str()) == Some("well_known"))
        .cloned()
        .collect();

    let mut output = match source_filter {
        "clawhub" => serde_json::json!({
            "catalog": catalog_json,
            "catalog_count": catalog_json.len(),
            "registry_url": registry_url,
        }),
        "github" => serde_json::json!({
            "github": github_json,
            "github_count": github_json.len(),
        }),
        "well_known" => serde_json::json!({
            "well_known": well_known_json,
            "well_known_count": well_known_json.len(),
        }),
        _ => serde_json::json!({
            "catalog": catalog_json,
            "catalog_count": catalog_json.len(),
            "remote": remote_json,
            "remote_count": remote_json.len(),
            "github": github_json,
            "github_count": github_json.len(),
            "well_known": well_known_json,
            "well_known_count": well_known_json.len(),
            "installed": local_matches,
            "installed_count": local_matches.len(),
            "registry_url": registry_url,
        }),
    };
    if let Some(err) = catalog_error {
        output["catalog_error"] = serde_json::Value::String(err);
    }
    output
}

pub fn is_skipped_package_name(name: &str) -> bool {
    name == ".git"
        || name == ".DS_Store"
        || name == ".thinclaw-skill-lock.json"
        || name == ".cache"
        || name == "__pycache__"
        || name == "target"
        || name == "node_modules"
        || name == "tmp"
        || name == "temp"
        || name.starts_with('.')
}

pub fn relative_path_is_safe(path: &Path) -> bool {
    path.components()
        .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

#[derive(Debug, Clone)]
pub struct SkillPackageFile {
    pub relative_path: String,
    pub source_path: PathBuf,
    pub bytes: u64,
}

pub fn collect_skill_package_files(root: &Path) -> Result<Vec<SkillPackageFile>, ToolError> {
    fn walk(root: &Path, dir: &Path, files: &mut Vec<SkillPackageFile>) -> Result<(), ToolError> {
        let entries = std::fs::read_dir(dir).map_err(|err| {
            ToolError::ExecutionFailed(format!(
                "Failed to read skill directory '{}': {}",
                dir.display(),
                err
            ))
        })?;

        for entry in entries {
            let entry = entry.map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if is_skipped_package_name(&name) {
                continue;
            }

            let meta = std::fs::symlink_metadata(&path).map_err(|err| {
                ToolError::ExecutionFailed(format!("Failed to stat '{}': {}", path.display(), err))
            })?;
            if meta.file_type().is_symlink() {
                return Err(ToolError::ExecutionFailed(format!(
                    "Refusing to publish symlink '{}'",
                    path.display()
                )));
            }
            if meta.is_dir() {
                walk(root, &path, files)?;
                continue;
            }
            if !meta.is_file() {
                continue;
            }

            let relative = path.strip_prefix(root).map_err(|err| {
                ToolError::ExecutionFailed(format!(
                    "Failed to derive package path for '{}': {}",
                    path.display(),
                    err
                ))
            })?;
            if !relative_path_is_safe(relative) {
                return Err(ToolError::ExecutionFailed(format!(
                    "Refusing unsafe package path '{}'",
                    relative.display()
                )));
            }
            files.push(SkillPackageFile {
                relative_path: relative.to_string_lossy().replace('\\', "/"),
                source_path: path,
                bytes: meta.len(),
            });
        }
        Ok(())
    }

    if !root.join("SKILL.md").is_file() {
        return Err(ToolError::ExecutionFailed(format!(
            "Skill directory '{}' is missing SKILL.md",
            root.display()
        )));
    }

    let mut files = Vec::new();
    walk(root, root, &mut files)?;
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    if !files.iter().any(|file| file.relative_path == "SKILL.md") {
        return Err(ToolError::ExecutionFailed(
            "Skill package must include SKILL.md".to_string(),
        ));
    }
    Ok(files)
}

pub fn package_hash(files: &[SkillPackageFile]) -> Result<String, ToolError> {
    let mut hasher = Sha256::new();
    for file in files {
        hasher.update(file.relative_path.as_bytes());
        hasher.update(b"\0");
        let bytes = std::fs::read(&file.source_path).map_err(|err| {
            ToolError::ExecutionFailed(format!(
                "Failed to read package file '{}': {}",
                file.source_path.display(),
                err
            ))
        })?;
        hasher.update(&bytes);
        hasher.update(b"\0");
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

pub fn package_scan_content(files: &[SkillPackageFile]) -> String {
    let mut out = String::new();
    for file in files {
        if let Ok(bytes) = std::fs::read(&file.source_path) {
            out.push_str("\n--- ");
            out.push_str(&file.relative_path);
            out.push_str(" ---\n");
            out.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    out
}

pub fn package_file_json(files: &[SkillPackageFile]) -> Vec<serde_json::Value> {
    files
        .iter()
        .map(|file| {
            serde_json::json!({
                "path": file.relative_path,
                "bytes": file.bytes,
            })
        })
        .collect()
}

pub fn validate_fetch_url(url_str: &str) -> Result<(), ToolError> {
    let parsed = url::Url::parse(url_str)
        .map_err(|e| ToolError::ExecutionFailed(format!("Invalid URL '{}': {}", url_str, e)))?;

    if parsed.scheme() != "https" {
        return Err(ToolError::ExecutionFailed(format!(
            "Only HTTPS URLs are allowed for skill fetching, got scheme '{}'",
            parsed.scheme()
        )));
    }

    let host = parsed
        .host()
        .ok_or_else(|| ToolError::ExecutionFailed("URL has no host".to_string()))?;

    match host {
        url::Host::Domain(host) => {
            let host_lower = host.to_lowercase();
            if host_lower == "localhost"
                || host_lower == "metadata.google.internal"
                || host_lower.ends_with(".internal")
                || host_lower.ends_with(".local")
            {
                return Err(ToolError::ExecutionFailed(format!(
                    "URL points to an internal hostname: {}",
                    host
                )));
            }
        }
        url::Host::Ipv4(ip) => {
            let ip = std::net::IpAddr::V4(ip);
            if ip.is_loopback()
                || ip.is_unspecified()
                || is_private_ip(&ip)
                || is_link_local_ip(&ip)
            {
                return Err(ToolError::ExecutionFailed(format!(
                    "URL points to a private/loopback/link-local address: {}",
                    ip
                )));
            }
        }
        url::Host::Ipv6(ip) => {
            let ip = ip
                .to_ipv4_mapped()
                .map(std::net::IpAddr::V4)
                .unwrap_or(std::net::IpAddr::V6(ip));
            if ip.is_loopback()
                || ip.is_unspecified()
                || is_private_ip(&ip)
                || is_link_local_ip(&ip)
            {
                return Err(ToolError::ExecutionFailed(format!(
                    "URL points to a private/loopback/link-local address: {}",
                    ip
                )));
            }
        }
    }

    Ok(())
}

/// Extract `SKILL.md` from a ZIP archive returned by the skill download API.
///
/// Walks ZIP local file headers looking for an entry named `SKILL.md`.
/// Supports Store (method 0) and Deflate (method 8) compression.
pub fn extract_skill_from_zip(data: &[u8]) -> Result<String, ToolError> {
    use flate2::read::DeflateDecoder;
    use std::io::Read;

    const MAX_DECOMPRESSED: usize = 1_024 * 1_024;

    let mut offset = 0;
    while offset + 30 <= data.len() {
        if data[offset..offset + 4] != [0x50, 0x4B, 0x03, 0x04] {
            break;
        }

        let compression = u16::from_le_bytes([data[offset + 8], data[offset + 9]]);
        let compressed_size = u32::from_le_bytes([
            data[offset + 18],
            data[offset + 19],
            data[offset + 20],
            data[offset + 21],
        ]) as usize;
        let uncompressed_size = u32::from_le_bytes([
            data[offset + 22],
            data[offset + 23],
            data[offset + 24],
            data[offset + 25],
        ]) as usize;
        let name_len = u16::from_le_bytes([data[offset + 26], data[offset + 27]]) as usize;
        let extra_len = u16::from_le_bytes([data[offset + 28], data[offset + 29]]) as usize;

        let name_start = offset + 30;
        let name_end = name_start + name_len;
        if name_end > data.len() {
            break;
        }
        let file_name = std::str::from_utf8(&data[name_start..name_end]).unwrap_or("");

        let data_start = name_end
            .checked_add(extra_len)
            .ok_or_else(|| ToolError::ExecutionFailed("ZIP header offset overflow".to_string()))?;
        let data_end = data_start
            .checked_add(compressed_size)
            .ok_or_else(|| ToolError::ExecutionFailed("ZIP header size overflow".to_string()))?;

        if file_name == "SKILL.md" {
            if data_end > data.len() {
                return Err(ToolError::ExecutionFailed(
                    "ZIP archive truncated".to_string(),
                ));
            }

            if uncompressed_size > MAX_DECOMPRESSED {
                return Err(ToolError::ExecutionFailed(
                    "ZIP entry too large to decompress safely".to_string(),
                ));
            }

            let raw = &data[data_start..data_end];
            let decompressed = match compression {
                0 => raw.to_vec(),
                8 => {
                    let mut decoder = DeflateDecoder::new(raw).take(MAX_DECOMPRESSED as u64);
                    let mut buf = Vec::with_capacity(uncompressed_size.min(MAX_DECOMPRESSED));
                    decoder.read_to_end(&mut buf).map_err(|e| {
                        ToolError::ExecutionFailed(format!("Failed to decompress SKILL.md: {}", e))
                    })?;
                    buf
                }
                other => {
                    return Err(ToolError::ExecutionFailed(format!(
                        "Unsupported ZIP compression method: {}",
                        other
                    )));
                }
            };

            return String::from_utf8(decompressed).map_err(|e| {
                ToolError::ExecutionFailed(format!("SKILL.md in archive is not valid UTF-8: {}", e))
            });
        }

        offset = data_end;
    }

    Err(ToolError::ExecutionFailed(
        "ZIP archive does not contain SKILL.md".to_string(),
    ))
}

fn is_private_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => v4.is_private() || v4.is_link_local(),
        std::net::IpAddr::V6(v6) => {
            let segments = v6.segments();
            (segments[0] & 0xfe00) == 0xfc00
        }
    }
}

fn is_link_local_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => v4.is_link_local(),
        std::net::IpAddr::V6(v6) => {
            let segments = v6.segments();
            (segments[0] & 0xffc0) == 0xfe80
        }
    }
}

pub fn normalize_tap_path(path: &str) -> String {
    path.trim().trim_matches('/').to_string()
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

pub fn validate_github_repo(repo: &str) -> Result<(), ToolError> {
    let mut parts = repo.split('/');
    let owner = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();
    if parts.next().is_some()
        || owner.is_empty()
        || name.is_empty()
        || [owner, name].iter().any(|part| {
            part == &"."
                || part == &".."
                || part
                    .chars()
                    .any(|ch| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.'))
        })
    {
        return Err(ToolError::InvalidParameters(
            "repo must be in owner/name form".to_string(),
        ));
    }
    Ok(())
}

pub fn validate_repo_relative_path(path: &str, field: &str) -> Result<(), ToolError> {
    if path.is_empty() {
        return Ok(());
    }
    let candidate = Path::new(path);
    if candidate.is_absolute()
        || !candidate
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(ToolError::InvalidParameters(format!(
            "{} must be a relative repository path without traversal",
            field
        )));
    }
    Ok(())
}

pub fn validate_repo_path_component(value: &str, field: &str) -> Result<(), ToolError> {
    validate_repo_relative_path(value, field)?;
    if Path::new(value).components().count() != 1 {
        return Err(ToolError::InvalidParameters(format!(
            "{} must be a single repository path component",
            field
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ports::{
        SkillInstallToolHostPort, SkillSearchToolHostPort, ToolSkillCheckRequest,
        ToolSkillCheckResult, ToolSkillCheckSource, ToolSkillInstallActionRequest,
        ToolSkillInstallRequest, ToolSkillMutationActionResult, ToolSkillPublishRequest,
        ToolSkillPublishResult, ToolSkillQuery, ToolSkillRead, ToolSkillRemoveResult,
        ToolSkillSearchCatalogEntry, ToolSkillSearchLocalEntry, ToolSkillSearchRemoteEntry,
        ToolSkillSearchRequest, ToolSkillSearchResult, ToolSkillSnapshotResult, ToolSkillSummary,
        ToolSkillTapList, ToolSkillTapMutationResult, ToolSkillTapRefreshResult, ToolSkillTrust,
        ToolSkillTrustMutationRequest, ToolSkillTrustMutationResult, ToolSkillUpdateActionRequest,
    };

    struct StubSkillHost;

    struct StubSkillSearchHost;

    struct StubSkillInstallHost;

    #[async_trait]
    impl SkillInstallToolHostPort for StubSkillInstallHost {
        async fn install_skill_action(
            &self,
            request: ToolSkillInstallActionRequest,
        ) -> Result<ToolSkillMutationActionResult, ToolHostError> {
            Ok(ToolSkillMutationActionResult {
                output: skill_install_output(
                    &request.name,
                    request.force,
                    vec![skill_finding_output("network", "high", "curl")],
                ),
            })
        }

        async fn update_skill_action(
            &self,
            request: ToolSkillUpdateActionRequest,
        ) -> Result<ToolSkillMutationActionResult, ToolHostError> {
            Ok(ToolSkillMutationActionResult {
                output: skill_install_output(&request.name, true, Vec::new()),
            })
        }
    }

    #[async_trait]
    impl SkillSearchToolHostPort for StubSkillSearchHost {
        async fn search_skills(
            &self,
            request: ToolSkillSearchRequest,
        ) -> Result<ToolSkillSearchResult, ToolHostError> {
            Ok(ToolSkillSearchResult {
                catalog: vec![ToolSkillSearchCatalogEntry {
                    slug: "owner/docs".to_string(),
                    name: "docs".to_string(),
                    description: "Documentation helper".to_string(),
                    version: "1.0.0".to_string(),
                    score: 0.95,
                    installed: true,
                    stars: Some(10),
                    downloads: Some(20),
                    owner: Some("owner".to_string()),
                }],
                remote: vec![ToolSkillSearchRemoteEntry {
                    slug: "owner/review".to_string(),
                    name: "review".to_string(),
                    description: "Review helper".to_string(),
                    version: "0.1.0".to_string(),
                    source: "github_tap".to_string(),
                    source_label: "GitHub".to_string(),
                    source_ref: "owner/skills".to_string(),
                    manifest_url: Some("https://example.test/SKILL.md".to_string()),
                    manifest_digest: Some("sha256:remote".to_string()),
                    repo: Some("owner/skills".to_string()),
                    path: Some("skills/review".to_string()),
                    branch: Some("main".to_string()),
                    trust_level: "trusted".to_string(),
                }],
                local: vec![ToolSkillSearchLocalEntry {
                    name: "docs".to_string(),
                    description: "Documentation helper".to_string(),
                    trust: "trusted".to_string(),
                    source_tier: "user".to_string(),
                }],
                registry_url: "https://registry.test".to_string(),
                catalog_error: (request.source == "clawhub").then(|| "offline".to_string()),
            })
        }
    }

    #[async_trait]
    impl SkillToolHostPort for StubSkillHost {
        async fn list_skills(
            &self,
            _query: ToolSkillQuery,
        ) -> Result<Vec<ToolSkillSummary>, ToolHostError> {
            Ok(vec![ToolSkillSummary {
                name: "docs".to_string(),
                description: Some("Documentation helper".to_string()),
                trust: ToolSkillTrust::Trusted,
                enabled: true,
                metadata: serde_json::json!({
                    "source_tier": "user",
                    "source": "User",
                    "keywords": ["docs"],
                    "version": "1.0.0",
                    "tags": ["writing"],
                    "content_hash": "sha256:abc",
                    "max_context_tokens": 1200,
                    "provenance": {"kind": "test"},
                    "lifecycle_status": "active",
                    "outcome_score": 0.9,
                    "reuse_count": 3,
                    "activation_reason": "keyword"
                }),
            }])
        }

        async fn inspect_skill(
            &self,
            _scope: ToolOperationScope,
            name: String,
            include_content: bool,
            include_files: bool,
            audit: bool,
        ) -> Result<serde_json::Value, ToolHostError> {
            Ok(serde_json::json!({
                "name": name,
                "include_content": include_content,
                "include_files": include_files,
                "audit": audit
            }))
        }

        async fn read_skill(
            &self,
            _scope: ToolOperationScope,
            name: String,
        ) -> Result<ToolSkillRead, ToolHostError> {
            Ok(ToolSkillRead {
                name,
                version: "1.0.0".to_string(),
                description: "Documentation helper".to_string(),
                trust: ToolSkillTrust::Trusted,
                source_tier: "user".to_string(),
                content: "Use this skill for docs.".to_string(),
            })
        }

        async fn install_skill(
            &self,
            request: ToolSkillInstallRequest,
        ) -> Result<ToolSkillSummary, ToolHostError> {
            Ok(ToolSkillSummary {
                name: request.name,
                description: None,
                trust: ToolSkillTrust::Community,
                enabled: true,
                metadata: serde_json::Value::Null,
            })
        }

        async fn check_skill(
            &self,
            request: ToolSkillCheckRequest,
        ) -> Result<ToolSkillCheckResult, ToolHostError> {
            let (source_kind, source_ref) = match request.source {
                ToolSkillCheckSource::InlineContent { .. } => {
                    ("content".to_string(), "(inline content)".to_string())
                }
                ToolSkillCheckSource::Path { path } => ("path".to_string(), path),
                ToolSkillCheckSource::Url { url } => ("url".to_string(), url),
            };
            Ok(ToolSkillCheckResult {
                output: skill_check_success_output(
                    &source_kind,
                    &source_ref,
                    "docs",
                    "1.0.0",
                    "Documentation helper",
                    serde_json::json!({"keywords": ["docs"]}),
                    "installed",
                    "user",
                    64,
                    1200,
                    "sha256:abc",
                    "sha256:def",
                    Vec::new(),
                ),
            })
        }

        async fn remove_skill(
            &self,
            _scope: ToolOperationScope,
            name: String,
        ) -> Result<ToolSkillRemoveResult, ToolHostError> {
            Ok(ToolSkillRemoveResult { name })
        }

        async fn promote_skill_trust(
            &self,
            request: ToolSkillTrustMutationRequest,
        ) -> Result<ToolSkillTrustMutationResult, ToolHostError> {
            Ok(ToolSkillTrustMutationResult {
                name: request.name,
                trust: request.target_trust,
                source_tier: "user".to_string(),
            })
        }

        async fn audit_skills(
            &self,
            _scope: ToolOperationScope,
            name: Option<String>,
        ) -> Result<Vec<serde_json::Value>, ToolHostError> {
            Ok(vec![skill_audit_entry_output(
                name.as_deref().unwrap_or("docs"),
                "trusted",
                "user",
                "/tmp/docs",
                vec![skill_finding_output("network", "high", "curl")],
            )])
        }

        async fn reload_skills(
            &self,
            _scope: ToolOperationScope,
            name: Option<String>,
        ) -> Result<Vec<ToolSkillSummary>, ToolHostError> {
            let names = name
                .map(|name| vec![name])
                .unwrap_or_else(|| vec!["docs".to_string(), "review".to_string()]);
            Ok(names
                .into_iter()
                .map(|name| ToolSkillSummary {
                    name,
                    description: None,
                    trust: ToolSkillTrust::Trusted,
                    enabled: true,
                    metadata: serde_json::Value::Null,
                })
                .collect())
        }

        async fn snapshot_skills(
            &self,
            _scope: ToolOperationScope,
        ) -> Result<ToolSkillSnapshotResult, ToolHostError> {
            Ok(ToolSkillSnapshotResult {
                path: "/tmp/skills/snapshot.json".to_string(),
                count: 2,
            })
        }
    }

    struct StubSkillPublishHost;

    #[async_trait]
    impl SkillPublishToolHostPort for StubSkillPublishHost {
        async fn publish_skill(
            &self,
            request: ToolSkillPublishRequest,
        ) -> Result<ToolSkillPublishResult, ToolHostError> {
            Ok(ToolSkillPublishResult {
                status: if request.remote_write {
                    "published".to_string()
                } else {
                    "dry_run".to_string()
                },
                name: request.name,
                target_repo: request.target_repo,
                tap_path: "community".to_string(),
                package_path: "community/docs".to_string(),
                branch: "codex/skill-publish/docs-1234abcd".to_string(),
                base_branch: Some("main".to_string()),
                package_hash: "sha256:1234".to_string(),
                files: vec![serde_json::json!({"path": "SKILL.md", "bytes": 10})],
                findings: Vec::new(),
                trust: "trusted".to_string(),
                source_tier: "user".to_string(),
                source: skill_source_output("user", "/tmp/docs"),
                remote_write_plan: serde_json::Value::Null,
                metadata: serde_json::json!({
                    "scanner_version": "test",
                    "content_sha256": "sha256:content"
                }),
            })
        }
    }

    struct StubSkillTapHost;

    #[async_trait]
    impl SkillTapToolHostPort for StubSkillTapHost {
        async fn list_skill_taps(
            &self,
            query: ToolSkillTapQuery,
        ) -> Result<ToolSkillTapList, ToolHostError> {
            Ok(ToolSkillTapList::new(
                vec![ToolSkillTap {
                    repo: "owner/skills".to_string(),
                    path: "packs/core".to_string(),
                    branch: Some("main".to_string()),
                    trust_level: ToolSkillTapTrust::Trusted,
                }],
                Some(query.include_health),
            ))
        }

        async fn add_skill_tap(
            &self,
            request: ToolSkillTapAddRequest,
        ) -> Result<ToolSkillTapMutationResult, ToolHostError> {
            Ok(ToolSkillTapMutationResult {
                status: if request.replace {
                    "replaced".to_string()
                } else {
                    "added".to_string()
                },
                tap: Some(ToolSkillTap {
                    repo: request.repo,
                    path: request.path,
                    branch: request.branch,
                    trust_level: request.trust_level,
                }),
                tap_count: 1,
            })
        }

        async fn remove_skill_tap(
            &self,
            request: ToolSkillTapRemoveRequest,
        ) -> Result<ToolSkillTapMutationResult, ToolHostError> {
            Ok(ToolSkillTapMutationResult {
                status: "removed".to_string(),
                tap: Some(ToolSkillTap {
                    repo: request.repo,
                    path: request.path,
                    branch: request.branch,
                    trust_level: ToolSkillTapTrust::Community,
                }),
                tap_count: 0,
            })
        }

        async fn refresh_skill_taps(
            &self,
            request: ToolSkillTapRefreshRequest,
        ) -> Result<ToolSkillTapRefreshResult, ToolHostError> {
            Ok(ToolSkillTapRefreshResult {
                status: "refreshed".to_string(),
                tap_count: 1,
                repo: request.repo,
                path: request.path,
                hub_enabled: true,
            })
        }
    }

    fn stub_tap_host() -> Arc<dyn SkillTapToolHostPort> {
        Arc::new(StubSkillTapHost)
    }

    #[test]
    fn package_skip_policy_filters_generated_and_hidden_names() {
        assert!(is_skipped_package_name(".git"));
        assert!(is_skipped_package_name("node_modules"));
        assert!(is_skipped_package_name(".hidden"));
        assert!(!is_skipped_package_name("SKILL.md"));
    }

    #[test]
    fn skill_allowlist_policy_allows_only_listed_skills() {
        let metadata = serde_json::json!({ "allowed_skills": ["github"] });
        assert!(ensure_skill_allowed(&metadata, "github").is_ok());
        assert!(ensure_skill_allowed(&metadata, "calendar").is_err());
        assert_eq!(
            restricted_skill_names(&metadata).unwrap(),
            std::collections::HashSet::from(["github".to_string()])
        );
        assert!(ensure_skill_admin_available(&metadata, "skill_install").is_err());
        assert!(ensure_skill_admin_available(&serde_json::json!({}), "skill_install").is_ok());
    }

    #[test]
    fn skill_discovery_schemas_and_search_params_are_root_independent() {
        assert_eq!(skill_inspect_parameters_schema()["required"][0], "name");
        let inspect = parse_skill_inspect_params(&serde_json::json!({
            "name": "github",
            "include_content": true,
            "include_files": false,
            "audit": false
        }))
        .unwrap();
        assert_eq!(inspect.name, "github");
        assert!(inspect.include_content);
        assert!(!inspect.include_files);
        assert!(!inspect.audit);

        assert_eq!(skill_read_parameters_schema()["required"][0], "name");
        assert_eq!(
            parse_skill_name_param(&serde_json::json!({"name": "github"})).unwrap(),
            "github"
        );
        let read = skill_read_output("github", "1.0.0", "desc", "trusted", "user", "body");
        assert_eq!(read["name"], "github");
        assert_eq!(read["content"], "body");
        let source = skill_source_output("user", "/tmp/github");
        assert_eq!(source["kind"], "user");
        let inspect = skill_inspect_output(
            "github",
            "1.0.0",
            "desc",
            serde_json::json!({"keywords": ["git"]}),
            serde_json::json!({"owner": "dev"}),
            "trusted",
            "user",
            source,
            "abc",
            12,
            Some(serde_json::json!({"source": "lock"})),
            vec![skill_finding_output("network", "high", "curl")],
            vec![skill_inventory_error_output("missing file")],
            Some("prompt"),
        );
        assert_eq!(inspect["finding_count"], 1);
        assert_eq!(inspect["inventory"]["file_count"], 1);
        assert_eq!(inspect["content"], "prompt");

        assert_eq!(
            skill_list_parameters_schema()["properties"]["verbose"]["default"],
            false
        );
        assert!(parse_skill_list_params(&serde_json::json!({"verbose": true})).verbose);
        let mut entry = skill_list_entry(
            "github",
            "desc",
            "trusted",
            "user",
            "User(/tmp/skill)",
            serde_json::json!(["git"]),
        );
        add_skill_list_verbose_fields(
            &mut entry,
            SkillListVerboseFields {
                version: "1.0.0".to_string(),
                tags: serde_json::json!(["code"]),
                content_hash: "abc".to_string(),
                max_context_tokens: serde_json::json!(1024),
                provenance: Some(serde_json::json!("manual")),
                lifecycle_status: None,
                outcome_score: None,
                reuse_count: Some(serde_json::json!(3)),
                activation_reason: None,
            },
        );
        assert_eq!(entry["version"], "1.0.0");
        let listed = skill_list_output(vec![entry]);
        assert_eq!(listed["count"], 1);

        assert_eq!(
            skill_search_parameters_schema()["properties"]["source"]["default"],
            "all"
        );

        let params = parse_skill_search_params(&serde_json::json!({
            "query": "browser",
            "source": "GITHUB"
        }))
        .unwrap();
        assert_eq!(params.query, "browser");
        assert_eq!(params.source, "github");
        assert!(parse_skill_search_params(&serde_json::json!({})).is_err());

        let local = skill_search_local_entry("github", "desc", "trusted", "user");
        assert_eq!(local["name"], "github");
        let catalog = skill_search_catalog_entry(
            "owner/github",
            "github",
            "desc",
            "1.0.0",
            0.9,
            true,
            Some(10),
            Some(20),
            Some("owner"),
        );
        assert_eq!(catalog["installed"], true);
        let remote = skill_search_remote_entry(
            "owner/github",
            "github",
            "desc",
            "1.0.0",
            "github_tap",
            "GitHub",
            "owner/repo",
            Some("https://example.test/SKILL.md"),
            Some("sha256:abc"),
            Some("owner/repo"),
            Some("skills/github"),
            Some("main"),
            "trusted",
        );
        let search = skill_search_output(
            "all",
            vec![catalog],
            vec![remote],
            vec![local],
            "https://registry.test",
            Some("offline".to_string()),
        );
        assert_eq!(search["github_count"], 1);
        assert_eq!(search["installed_count"], 1);
        assert_eq!(search["catalog_error"], "offline");

        assert_eq!(
            skill_check_parameters_schema()["properties"]["url"]["type"],
            "string"
        );
        assert_eq!(
            parse_skill_check_input(&serde_json::json!({"content": "name: test"})).unwrap(),
            SkillCheckInput::InlineContent("name: test".to_string())
        );
        assert!(parse_skill_check_input(&serde_json::json!({})).is_err());
        assert!(
            parse_skill_check_input(&serde_json::json!({
                "content": "x",
                "url": "https://example.test/SKILL.md"
            }))
            .is_err()
        );
        assert_eq!(
            skill_check_path_for_read("/tmp/example")
                .file_name()
                .and_then(|name| name.to_str()),
            Some("SKILL.md")
        );
        let findings = vec![skill_finding_output("network", "high", "curl")];
        let check = skill_check_success_output(
            "content",
            "(inline content)",
            "github",
            "1.0.0",
            "desc",
            serde_json::json!({"keywords": ["git"]}),
            "installed",
            "user",
            10,
            1024,
            "abc",
            "def",
            findings.clone(),
        );
        assert_eq!(check["ok"], true);
        assert_eq!(check["finding_count"], 1);
        let check_error =
            skill_check_error_output("content", "(inline content)", "invalid", "def", findings);
        assert_eq!(check_error["ok"], false);
        assert_eq!(check_error["error"], "invalid");

        assert_eq!(skill_install_parameters_schema()["required"][0], "name");
        let install_output = skill_install_output("github", false, Vec::new());
        assert_eq!(install_output["status"], "installed");
        assert_eq!(
            skill_audit_parameters_schema()["properties"]["name"]["type"],
            "string"
        );
        assert_eq!(
            parse_skill_audit_target_name(&serde_json::json!({"name": "github"})),
            Some("github")
        );
        let audit = skill_audit_output(vec![serde_json::json!({
            "finding_count": 2
        })]);
        assert_eq!(audit["total_findings"], 2);
        let audit_entry = skill_audit_entry_output(
            "github",
            "trusted",
            "user",
            "/tmp/github",
            vec![skill_finding_output("network", "high", "curl")],
        );
        assert_eq!(audit_entry["finding_count"], 1);

        assert_eq!(skill_update_parameters_schema()["required"][0], "name");
        let mut update_params = skill_update_install_params("github", true, true);
        add_skill_update_url(
            &mut update_params,
            "https://example.test/SKILL.md".to_string(),
        );
        assert_eq!(update_params["url"], "https://example.test/SKILL.md");
        assert_eq!(
            skill_publish_parameters_schema()["required"][1],
            "target_repo"
        );
        assert_eq!(
            skill_tap_list_parameters_schema()["properties"]["include_health"]["default"],
            false
        );
        assert_eq!(skill_tap_add_parameters_schema()["required"][0], "repo");
        let tap_add = parse_skill_tap_add_params(&serde_json::json!({
            "repo": "owner/repo",
            "path": "/skills/github/",
            "branch": " main ",
            "trust_level": "Trusted",
            "replace": true
        }))
        .unwrap();
        assert_eq!(tap_add.path, "skills/github");
        assert_eq!(tap_add.branch.as_deref(), Some("main"));
        assert_eq!(tap_add.trust_level, "trusted");
        assert!(tap_add.replace);
        let tap = skill_tap_json("owner/repo", "skills/github", Some("main"), "trusted");
        assert_eq!(tap["trust_level"], "trusted");
        let tap_list = skill_tap_list_output(vec![tap.clone()], Some(true));
        assert_eq!(tap_list["count"], 1);
        let tap_added = skill_tap_add_output(true, tap, 3);
        assert_eq!(tap_added["status"], "replaced");

        assert_eq!(skill_tap_remove_parameters_schema()["required"][0], "repo");
        let tap_remove = parse_skill_tap_remove_params(&serde_json::json!({
            "repo": "owner/repo",
            "path": "skills/github"
        }))
        .unwrap();
        assert_eq!(tap_remove.repo, "owner/repo");
        assert_eq!(
            skill_tap_remove_output("owner/repo", "skills/github", None, 2)["status"],
            "removed"
        );
        assert_eq!(
            skill_tap_refresh_parameters_schema()["properties"]["repo"]["default"],
            serde_json::Value::Null
        );
        let refresh = parse_skill_tap_refresh_params(&serde_json::json!({
            "repo": "owner/repo",
            "path": "/skills"
        }))
        .unwrap();
        assert_eq!(refresh.path.as_deref(), Some("skills"));
        let refresh_output =
            skill_tap_refresh_output(2, refresh.repo.as_deref(), refresh.path.as_deref(), true);
        assert_eq!(refresh_output["status"], "refreshed");
        assert_eq!(skill_snapshot_parameters_schema()["type"], "object");
        let snapshot_entry =
            skill_snapshot_entry("github", "1.0.0", "trusted", "user", "abc", None);
        let snapshot = skill_snapshot_document("now".to_string(), vec![snapshot_entry]);
        assert_eq!(snapshot["skills"].as_array().unwrap().len(), 1);
        assert_eq!(skill_snapshot_output("/tmp/snapshot.json", 1)["count"], 1);
        assert_eq!(
            skill_trust_promote_parameters_schema()["required"][1],
            "target_trust"
        );
        let promote = parse_skill_trust_promote_params(&serde_json::json!({
            "name": "github",
            "target_trust": "Trusted"
        }))
        .unwrap();
        assert_eq!(promote.name, "github");
        assert_eq!(promote.target_trust, "trusted");
        assert!(
            parse_skill_trust_promote_params(&serde_json::json!({
                "name": "github",
                "target_trust": "system"
            }))
            .is_err()
        );
        let promote_output = skill_trust_promote_output("github", "trusted", "user");
        assert_eq!(promote_output["status"], "updated");

        assert_eq!(skill_remove_parameters_schema()["required"][0], "name");
        assert_eq!(skill_remove_output("github")["status"], "removed");
        assert_eq!(
            skill_reload_parameters_schema()["properties"]["all"]["default"],
            false
        );

        let install = parse_skill_install_params(&serde_json::json!({
            "name": "docs",
            "force": true
        }))
        .unwrap();
        assert_eq!(install.name, "docs");
        assert!(install.force);
        assert!(!install.approve_risky);

        let update = parse_skill_update_params(&serde_json::json!({
            "name": "docs",
            "approve_risky": true
        }))
        .unwrap();
        assert_eq!(update.name, "docs");
        assert!(update.approve_risky);

        let publish = parse_skill_publish_params(&serde_json::json!({
            "name": "docs",
            "target_repo": " owner/repo "
        }))
        .unwrap();
        assert_eq!(publish.target_repo, "owner/repo");
        assert!(publish.dry_run);
        assert!(!publish.remote_write);
        let publish_plan = skill_publish_plan_output(
            "planned",
            "docs",
            "owner/repo",
            "skills",
            "skills/docs",
            "codex/skill-publish/docs-1234abcd",
            Some("main"),
            "sha256:1234",
            vec![serde_json::json!({"path": "SKILL.md", "bytes": 10})],
            Vec::new(),
            "trusted",
            "user",
            skill_source_output("user", "/tmp/docs"),
        );
        assert_eq!(publish_plan["file_count"], 1);
        assert_eq!(
            publish_plan["remote_write_plan"]["pull_request"]["title"],
            "[skills] publish docs"
        );

        let reload = parse_skill_reload_params(&serde_json::json!({
            "name": "docs"
        }));
        assert_eq!(reload.name.as_deref(), Some("docs"));
        assert!(!reload.all);
        assert_eq!(
            skill_reload_all_output(vec!["docs".to_string()])["count"],
            1
        );
        assert_eq!(skill_reload_output("docs")["status"], "reloaded");
    }

    #[test]
    fn relative_path_policy_blocks_traversal() {
        assert!(relative_path_is_safe(Path::new("docs/NOTE.md")));
        assert!(relative_path_is_safe(Path::new("./docs/NOTE.md")));
        assert!(!relative_path_is_safe(Path::new("../outside")));
        assert!(!relative_path_is_safe(Path::new("/absolute")));
    }

    #[test]
    fn package_file_json_reports_relative_paths_and_sizes() {
        let files = vec![SkillPackageFile {
            relative_path: "SKILL.md".to_string(),
            source_path: PathBuf::from("SKILL.md"),
            bytes: 42,
        }];

        assert_eq!(
            package_file_json(&files),
            vec![serde_json::json!({"path": "SKILL.md", "bytes": 42})]
        );
    }

    #[test]
    fn tap_path_normalization_trims_outer_slashes() {
        assert_eq!(normalize_tap_path("/skills/community/"), "skills/community");
        assert!(skill_tap_key_matches(
            "Owner/Repo",
            "/skills/community/",
            Some("main"),
            "owner/repo",
            "skills/community",
            Some("main"),
        ));
        assert!(skill_findings_require_approval("community", 1));
        assert_eq!(
            skill_findings_summary([skill_finding_summary("network", "high", "curl")]),
            "network (high): curl"
        );
    }

    #[tokio::test]
    async fn skill_list_host_tool_preserves_basic_and_verbose_output_shapes() {
        let ctx = JobContext::with_identity("user-1", "actor-1", "list", "test");
        let tool = SkillListHostTool::new(Arc::new(StubSkillHost));

        let output = tool
            .execute(serde_json::json!({ "verbose": true }), &ctx)
            .await
            .unwrap();

        assert_eq!(output.result["count"], 1);
        assert_eq!(output.result["skills"][0]["name"], "docs");
        assert_eq!(
            output.result["skills"][0]["description"],
            "Documentation helper"
        );
        assert_eq!(output.result["skills"][0]["trust"], "trusted");
        assert_eq!(output.result["skills"][0]["source_tier"], "user");
        assert_eq!(output.result["skills"][0]["keywords"][0], "docs");
        assert_eq!(output.result["skills"][0]["version"], "1.0.0");
        assert_eq!(output.result["skills"][0]["content_hash"], "sha256:abc");
        assert_eq!(output.result["skills"][0]["reuse_count"], 3);
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::Never
        );
    }

    #[tokio::test]
    async fn skill_search_host_tool_preserves_existing_output_shape() {
        let ctx = JobContext::with_identity("user-1", "actor-1", "search", "test");
        let tool = SkillSearchHostTool::new(Arc::new(StubSkillSearchHost));

        let output = tool
            .execute(
                serde_json::json!({
                    "query": "docs",
                    "source": "all"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(output.result["installed_count"], 1);
        assert_eq!(output.result["catalog_count"], 1);
        assert_eq!(output.result["github_count"], 1);
        assert_eq!(output.result["registry_url"], "https://registry.test");
        assert_eq!(
            output.result["catalog"][0]["slug"],
            serde_json::json!("owner/docs")
        );
        assert_eq!(
            output.result["github"][0]["source"],
            serde_json::json!("github_tap")
        );
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::Never
        );
    }

    #[tokio::test]
    async fn skill_snapshot_host_tool_preserves_existing_output_shape() {
        let ctx = JobContext::with_identity("user-1", "actor-1", "snapshot", "test");
        let tool = SkillSnapshotHostTool::new(Arc::new(StubSkillHost));

        let output = tool.execute(serde_json::json!({}), &ctx).await.unwrap();

        assert_eq!(output.result["path"], "/tmp/skills/snapshot.json");
        assert_eq!(output.result["count"], 2);
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::UnlessAutoApproved
        );
    }

    #[tokio::test]
    async fn skill_check_host_tool_preserves_existing_output_shape_and_restrictions() {
        let ctx = JobContext::with_identity("user-1", "actor-1", "check", "test");
        let tool = SkillCheckHostTool::new(Arc::new(StubSkillHost));

        let output = tool
            .execute(serde_json::json!({ "content": "# docs" }), &ctx)
            .await
            .unwrap();

        assert_eq!(output.result["ok"], true);
        assert_eq!(output.result["source_kind"], "content");
        assert_eq!(output.result["source_ref"], "(inline content)");
        assert_eq!(output.result["name"], "docs");
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::Never
        );

        let mut restricted = ctx.clone();
        restricted.metadata = serde_json::json!({ "allowed_skills": ["docs"] });
        let err = tool
            .execute(serde_json::json!({ "content": "# docs" }), &restricted)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not available"));
    }

    #[tokio::test]
    async fn skill_install_and_update_host_tools_preserve_output_shapes() {
        let ctx = JobContext::with_identity("user-1", "actor-1", "install", "test");

        let install = SkillInstallHostTool::new(Arc::new(StubSkillInstallHost));
        let output = install
            .execute(
                serde_json::json!({
                    "name": "docs",
                    "force": true,
                    "approve_risky": true,
                    "content": "---\nname: docs\n---\nBody\n"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(output.result["status"], "updated");
        assert_eq!(output.result["name"], "docs");
        assert_eq!(output.result["findings"].as_array().unwrap().len(), 1);
        assert_eq!(
            install.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::UnlessAutoApproved
        );

        let update = SkillUpdateHostTool::new(Arc::new(StubSkillInstallHost));
        let output = update
            .execute(
                serde_json::json!({
                    "name": "docs",
                    "approve_risky": true
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(output.result["status"], "updated");
        assert_eq!(output.result["name"], "docs");
        assert_eq!(
            update.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::UnlessAutoApproved
        );
    }

    #[tokio::test]
    async fn skill_remove_and_promote_host_tools_preserve_output_shapes() {
        let ctx = JobContext::with_identity("user-1", "actor-1", "mutate", "test");

        let remove = SkillRemoveHostTool::new(Arc::new(StubSkillHost));
        let output = remove
            .execute(serde_json::json!({ "name": "docs" }), &ctx)
            .await
            .unwrap();
        assert_eq!(output.result["status"], "removed");
        assert_eq!(output.result["name"], "docs");
        assert_eq!(
            remove.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::UnlessAutoApproved
        );

        let promote = SkillPromoteTrustHostTool::new(Arc::new(StubSkillHost));
        let output = promote
            .execute(
                serde_json::json!({
                    "name": "docs",
                    "target_trust": "trusted"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(output.result["status"], "updated");
        assert_eq!(output.result["name"], "docs");
        assert_eq!(output.result["trust"], "trusted");
        assert_eq!(output.result["source_tier"], "user");
        assert_eq!(
            promote.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::UnlessAutoApproved
        );
    }

    #[tokio::test]
    async fn skill_audit_host_tool_preserves_existing_output_shape() {
        let ctx = JobContext::with_identity("user-1", "actor-1", "audit", "test");
        let tool = SkillAuditHostTool::new(Arc::new(StubSkillHost));

        let output = tool
            .execute(serde_json::json!({ "name": "docs" }), &ctx)
            .await
            .unwrap();

        assert_eq!(output.result["audited_count"], 1);
        assert_eq!(output.result["total_findings"], 1);
        assert_eq!(output.result["audited"][0]["name"], "docs");
        assert_eq!(output.result["audited"][0]["finding_count"], 1);
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::Never
        );
    }

    #[tokio::test]
    async fn skill_reload_host_tool_preserves_single_and_all_output_shapes() {
        let ctx = JobContext::with_identity("user-1", "actor-1", "reload", "test");
        let tool = SkillReloadHostTool::new(Arc::new(StubSkillHost));

        let single = tool
            .execute(serde_json::json!({ "name": "docs" }), &ctx)
            .await
            .unwrap();
        assert_eq!(single.result["status"], "reloaded");
        assert_eq!(single.result["name"], "docs");
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::UnlessAutoApproved
        );

        let all = tool
            .execute(serde_json::json!({ "all": true }), &ctx)
            .await
            .unwrap();
        assert_eq!(all.result["status"], "reloaded_all");
        assert_eq!(all.result["count"], 2);
        assert_eq!(all.result["skills"][0], "docs");
    }

    #[tokio::test]
    async fn skill_read_host_tool_preserves_existing_output_shape() {
        let ctx = JobContext::with_identity("user-1", "actor-1", "read", "test");
        let tool = SkillReadHostTool::new(Arc::new(StubSkillHost));

        let output = tool
            .execute(serde_json::json!({ "name": "docs" }), &ctx)
            .await
            .unwrap();

        assert_eq!(output.result["name"], "docs");
        assert_eq!(output.result["version"], "1.0.0");
        assert_eq!(output.result["description"], "Documentation helper");
        assert_eq!(output.result["trust"], "trusted");
        assert_eq!(output.result["source_tier"], "user");
        assert_eq!(output.result["content"], "Use this skill for docs.");
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::Never
        );
    }

    #[tokio::test]
    async fn skill_inspect_host_tool_preserves_request_shape_and_restrictions() {
        let ctx = JobContext::with_identity("user-1", "actor-1", "inspect", "test");
        let tool = SkillInspectHostTool::new(Arc::new(StubSkillHost));
        let output = tool
            .execute(
                serde_json::json!({
                    "name": "docs",
                    "include_content": true,
                    "include_files": false,
                    "audit": false
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(output.result["name"], "docs");
        assert_eq!(output.result["include_content"], true);
        assert_eq!(output.result["include_files"], false);
        assert_eq!(output.result["audit"], false);

        let mut restricted = ctx.clone();
        restricted.metadata = serde_json::json!({ "allowed_skills": ["other"] });
        let err = tool
            .execute(serde_json::json!({ "name": "docs" }), &restricted)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not allowed"));
    }

    #[tokio::test]
    async fn skill_publish_host_tool_preserves_existing_output_shape() {
        let ctx = JobContext::with_identity("user-1", "actor-1", "publish", "test");
        let tool = SkillPublishHostTool::new(Arc::new(StubSkillPublishHost));

        let output = tool
            .execute(
                serde_json::json!({
                    "name": "docs",
                    "target_repo": "owner/skills",
                    "dry_run": true
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(output.result["status"], "dry_run");
        assert_eq!(output.result["name"], "docs");
        assert_eq!(output.result["target_repo"], "owner/skills");
        assert_eq!(output.result["package_path"], "community/docs");
        assert_eq!(output.result["file_count"], 1);
        assert_eq!(output.result["scanner_version"], "test");
        assert_eq!(
            output.result["remote_write_plan"]["pull_request"]["title"],
            "[skills] publish docs"
        );
        assert_eq!(
            tool.requires_approval(&serde_json::json!({"remote_write": true})),
            ApprovalRequirement::UnlessAutoApproved
        );
    }

    #[tokio::test]
    async fn skill_tap_host_tools_preserve_existing_output_shapes() {
        let ctx = JobContext::with_identity("user-1", "actor-1", "tap", "test");

        let list = SkillTapListHostTool::new(stub_tap_host());
        let output = list
            .execute(serde_json::json!({ "include_health": true }), &ctx)
            .await
            .unwrap();
        assert_eq!(output.result["count"], 1);
        assert_eq!(output.result["hub_enabled"], true);
        assert_eq!(output.result["taps"][0]["trust_level"], "trusted");

        let add = SkillTapAddHostTool::new(stub_tap_host());
        let output = add
            .execute(
                serde_json::json!({
                    "repo": "owner/skills",
                    "path": "/packs/core/",
                    "branch": "main",
                    "trust_level": "trusted",
                    "replace": true
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(output.result["status"], "replaced");
        assert_eq!(output.result["tap"]["path"], "packs/core");
        assert_eq!(
            add.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::UnlessAutoApproved
        );

        let remove = SkillTapRemoveHostTool::new(stub_tap_host());
        let output = remove
            .execute(
                serde_json::json!({
                    "repo": "owner/skills",
                    "path": "packs/core",
                    "branch": "main"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(output.result["status"], "removed");
        assert_eq!(output.result["tap_count"], 0);

        let refresh = SkillTapRefreshHostTool::new(stub_tap_host());
        let output = refresh
            .execute(
                serde_json::json!({
                    "repo": "owner/skills",
                    "path": "packs/core"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(output.result["status"], "refreshed");
        assert_eq!(output.result["filter"]["repo"], "owner/skills");
        assert_eq!(output.result["hub_enabled"], true);
    }

    #[test]
    fn repo_validation_requires_owner_name() {
        assert!(validate_github_repo("owner/repo").is_ok());
        assert!(validate_github_repo("owner/repo/extra").is_err());
        assert!(validate_github_repo("../repo").is_err());
        assert!(validate_github_repo("owner/").is_err());
    }

    #[test]
    fn repo_relative_path_validation_rejects_traversal() {
        assert!(validate_repo_relative_path("skills/community", "path").is_ok());
        assert!(validate_repo_relative_path("", "path").is_ok());
        assert!(validate_repo_relative_path("../outside", "path").is_err());
        assert!(validate_repo_relative_path("skills/../outside", "path").is_err());
        assert!(validate_repo_path_component("skills", "path").is_ok());
        assert!(validate_repo_path_component("skills/community", "path").is_err());
    }

    #[test]
    fn fetch_url_validation_blocks_internal_targets() {
        assert!(validate_fetch_url("https://example.com/skill.zip").is_ok());
        assert!(validate_fetch_url("http://example.com/skill.zip").is_err());
        assert!(validate_fetch_url("https://localhost/skill.zip").is_err());
        assert!(validate_fetch_url("https://127.0.0.1/skill.zip").is_err());
        assert!(validate_fetch_url("https://[::ffff:192.168.0.1]/skill.zip").is_err());
        assert!(validate_fetch_url("https://metadata.google.internal/skill.zip").is_err());
    }

    #[test]
    fn extracts_stored_skill_from_zip() {
        let skill_md = b"---\nname: stored\n---\n# Stored\n";
        let mut zip = Vec::new();
        zip.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04]);
        zip.extend_from_slice(&[0x0A, 0x00]);
        zip.extend_from_slice(&[0x00, 0x00]);
        zip.extend_from_slice(&[0x00, 0x00]);
        zip.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        zip.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        zip.extend_from_slice(&(skill_md.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(skill_md.len() as u32).to_le_bytes());
        zip.extend_from_slice(&8u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(b"SKILL.md");
        zip.extend_from_slice(skill_md);

        assert_eq!(
            extract_skill_from_zip(&zip).unwrap(),
            "---\nname: stored\n---\n# Stored\n"
        );
    }

    #[test]
    fn zip_extraction_requires_skill_md() {
        let mut zip = Vec::new();
        zip.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04]);
        zip.extend_from_slice(&[0x0A, 0x00]);
        zip.extend_from_slice(&[0x00, 0x00]);
        zip.extend_from_slice(&[0x00, 0x00]);
        zip.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        zip.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        zip.extend_from_slice(&2u32.to_le_bytes());
        zip.extend_from_slice(&2u32.to_le_bytes());
        zip.extend_from_slice(&10u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(b"_meta.json");
        zip.extend_from_slice(b"{}");

        assert!(
            extract_skill_from_zip(&zip)
                .unwrap_err()
                .to_string()
                .contains("does not contain SKILL.md")
        );
    }
}
