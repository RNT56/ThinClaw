//! Root-independent skill tool policy helpers.

use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};
use thinclaw_tools_core::ToolError;

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
