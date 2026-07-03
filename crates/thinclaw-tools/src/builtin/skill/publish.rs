//! Skill tool policy: publish.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use thinclaw_tools_core::{ApprovalRequirement, Tool, ToolError, ToolOutput};
use thinclaw_types::JobContext;

use crate::ports::{
    SkillPublishToolHostPort, ToolSkillPublishRequest, ToolSkillPublishResult,
    tool_scope_from_job_context,
};

use super::*;

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

#[derive(Debug, Clone, PartialEq)]
pub struct SkillPublishScanProjection {
    pub scanner_version: String,
    pub content_sha256: String,
    pub finding_summary: SkillFindingSummary,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkillPublishProjection {
    pub status: String,
    pub name: String,
    pub target_repo: String,
    pub tap_path: String,
    pub package_path: String,
    pub branch: String,
    pub base_branch: Option<String>,
    pub package_hash: String,
    pub files: Vec<serde_json::Value>,
    pub findings: Vec<serde_json::Value>,
    pub trust: String,
    pub source_tier: String,
    pub source: serde_json::Value,
    pub scan: Option<SkillPublishScanProjection>,
    pub remote_write_plan: Option<serde_json::Value>,
    pub metadata: Option<serde_json::Value>,
}

pub fn skill_publish_projection_output(projection: SkillPublishProjection) -> serde_json::Value {
    let mut output = skill_publish_plan_output(
        &projection.status,
        &projection.name,
        &projection.target_repo,
        &projection.tap_path,
        &projection.package_path,
        &projection.branch,
        projection.base_branch.as_deref(),
        &projection.package_hash,
        projection.files,
        projection.findings,
        &projection.trust,
        &projection.source_tier,
        projection.source,
    );

    if let Some(scan) = projection.scan {
        add_skill_scan_report_fields(
            &mut output,
            &scan.scanner_version,
            &scan.content_sha256,
            scan.finding_summary,
        );
    }
    if let Some(remote_write_plan) = projection.remote_write_plan
        && !remote_write_plan.is_null()
    {
        output["remote_write_plan"] = remote_write_plan;
    }
    if let Some(metadata) = projection.metadata
        && let Some(metadata_object) = metadata.as_object()
        && let Some(output_object) = output.as_object_mut()
    {
        for (key, value) in metadata_object {
            output_object.insert(key.clone(), value.clone());
        }
    }
    output
}

pub fn skill_publish_result_projection(result: &ToolSkillPublishResult) -> SkillPublishProjection {
    SkillPublishProjection {
        status: result.status.clone(),
        name: result.name.clone(),
        target_repo: result.target_repo.clone(),
        tap_path: result.tap_path.clone(),
        package_path: result.package_path.clone(),
        branch: result.branch.clone(),
        base_branch: result.base_branch.clone(),
        package_hash: result.package_hash.clone(),
        files: result.files.clone(),
        findings: result.findings.clone(),
        trust: result.trust.clone(),
        source_tier: result.source_tier.clone(),
        source: result.source.clone(),
        scan: None,
        remote_write_plan: Some(result.remote_write_plan.clone()),
        metadata: Some(result.metadata.clone()),
    }
}

pub fn skill_publish_result_tool_output(result: &ToolSkillPublishResult) -> serde_json::Value {
    skill_publish_projection_output(skill_publish_result_projection(result))
}

pub fn skill_publish_metadata_output<I>(
    scanner_version: &str,
    content_sha256: &str,
    finding_summary: SkillFindingSummary,
    extras: I,
) -> serde_json::Value
where
    I: IntoIterator<Item = (&'static str, serde_json::Value)>,
{
    let mut metadata = serde_json::json!({
        "scanner_version": scanner_version,
        "content_sha256": content_sha256,
        "finding_summary": skill_finding_summary_output(finding_summary),
    });
    if let Some(object) = metadata.as_object_mut() {
        for (key, value) in extras {
            object.insert(key.to_string(), value);
        }
    }
    metadata
}

#[allow(clippy::too_many_arguments)]
pub fn skill_publish_result_output(
    status: &str,
    name: &str,
    target_repo: &str,
    tap_path: &str,
    package_path: &str,
    branch: &str,
    base_branch: Option<String>,
    package_hash: &str,
    files: Vec<serde_json::Value>,
    findings: Vec<serde_json::Value>,
    trust: &str,
    source_tier: &str,
    source: serde_json::Value,
    remote_write_plan: serde_json::Value,
    metadata: serde_json::Value,
) -> ToolSkillPublishResult {
    ToolSkillPublishResult {
        status: status.to_string(),
        name: name.to_string(),
        target_repo: target_repo.to_string(),
        tap_path: tap_path.to_string(),
        package_path: package_path.to_string(),
        branch: branch.to_string(),
        base_branch,
        package_hash: package_hash.to_string(),
        files,
        findings,
        trust: trust.to_string(),
        source_tier: source_tier.to_string(),
        source,
        remote_write_plan,
        metadata,
    }
}

fn tool_skill_publish_output(result: &ToolSkillPublishResult) -> serde_json::Value {
    skill_publish_result_tool_output(result)
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
