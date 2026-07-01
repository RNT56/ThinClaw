//! Skill tool policy: check.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use thinclaw_tools_core::{ApprovalRequirement, Tool, ToolError, ToolOutput};
use thinclaw_types::JobContext;

use crate::ports::{SkillToolHostPort, ToolSkillCheckRequest, tool_scope_from_job_context};

use super::*;

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

#[derive(Debug, Clone, Copy)]
pub struct SkillFindingPolicy<'a> {
    pub kind: &'a str,
    pub severity: &'a str,
    pub excerpt: &'a str,
    pub rule_id: Option<&'a str>,
    pub file: Option<&'a str>,
    pub line: Option<usize>,
    pub recommendation: Option<&'a str>,
    pub scanner_version: Option<&'a str>,
}

pub fn skill_finding_detail_output(finding: SkillFindingPolicy<'_>) -> serde_json::Value {
    let mut value = skill_finding_output(finding.kind, finding.severity, finding.excerpt);
    if let Some(obj) = value.as_object_mut() {
        if let Some(rule_id) = finding.rule_id {
            obj.insert(
                "rule_id".to_string(),
                serde_json::Value::String(rule_id.to_string()),
            );
        }
        if let Some(file) = finding.file {
            obj.insert(
                "file".to_string(),
                serde_json::Value::String(file.to_string()),
            );
        }
        if let Some(line) = finding.line {
            obj.insert("line".to_string(), serde_json::json!(line));
        }
        if let Some(recommendation) = finding.recommendation {
            obj.insert(
                "recommendation".to_string(),
                serde_json::Value::String(recommendation.to_string()),
            );
        }
        if let Some(scanner_version) = finding.scanner_version {
            obj.insert(
                "scanner_version".to_string(),
                serde_json::Value::String(scanner_version.to_string()),
            );
        }
    }
    value
}

pub fn skill_finding_detail_outputs<'a, I>(findings: I) -> Vec<serde_json::Value>
where
    I: IntoIterator<Item = SkillFindingPolicy<'a>>,
{
    findings
        .into_iter()
        .map(skill_finding_detail_output)
        .collect()
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

pub fn skill_findings_detail_summary<'a, I>(findings: I) -> String
where
    I: IntoIterator<Item = SkillFindingPolicy<'a>>,
{
    skill_findings_summary(
        findings
            .into_iter()
            .map(|finding| skill_finding_summary(finding.kind, finding.severity, finding.excerpt)),
    )
}

pub fn skill_findings_require_approval(trust_level: &str, finding_count: usize) -> bool {
    trust_level.eq_ignore_ascii_case("community") && finding_count > 0
}

pub fn skill_findings_require_approval_by_counts(
    trust_level: &str,
    critical: usize,
    warnings: usize,
) -> bool {
    match trust_level.to_ascii_lowercase().as_str() {
        "community" => critical > 0 || warnings > 1,
        "trusted" | "builtin" => critical > 0,
        _ => critical > 0,
    }
}

pub fn skill_findings_require_approval_for_details<'a, I>(trust_level: &str, findings: I) -> bool
where
    I: IntoIterator<Item = SkillFindingPolicy<'a>>,
{
    let mut critical = 0;
    let mut warnings = 0;
    for finding in findings {
        match finding.severity {
            severity if severity.eq_ignore_ascii_case("critical") => critical += 1,
            severity if severity.eq_ignore_ascii_case("warning") => warnings += 1,
            _ => {}
        }
    }
    skill_findings_require_approval_by_counts(trust_level, critical, warnings)
}

pub fn skill_finding_requires_rejection(kind: &str, severity: &str) -> bool {
    severity.eq_ignore_ascii_case("critical") && kind == "path_traversal"
}

pub fn skill_findings_require_rejection_for_details<'a, I>(findings: I) -> bool
where
    I: IntoIterator<Item = SkillFindingPolicy<'a>>,
{
    findings
        .into_iter()
        .any(|finding| skill_finding_requires_rejection(finding.kind, finding.severity))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillFindingSummary {
    pub total: usize,
    pub warnings: usize,
    pub critical: usize,
    pub categories: Vec<String>,
}

pub fn skill_finding_summary_output(summary: SkillFindingSummary) -> serde_json::Value {
    serde_json::json!({
        "total": summary.total,
        "warnings": summary.warnings,
        "critical": summary.critical,
        "categories": summary.categories,
    })
}

pub fn add_skill_scan_report_fields(
    output: &mut serde_json::Value,
    scanner_version: &str,
    content_sha256: &str,
    summary: SkillFindingSummary,
) {
    if let Some(obj) = output.as_object_mut() {
        obj.insert(
            "scanner_version".to_string(),
            serde_json::Value::String(scanner_version.to_string()),
        );
        obj.insert(
            "content_sha256".to_string(),
            serde_json::Value::String(content_sha256.to_string()),
        );
        obj.insert(
            "finding_summary".to_string(),
            skill_finding_summary_output(summary),
        );
    }
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
                source: input.into(),
            })
            .await
            .map_err(tool_host_error)?;
        Ok(ToolOutput::success(result.output, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }
}
