//! Skill tool policy: audit.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use thinclaw_tools_core::{ApprovalRequirement, Tool, ToolError, ToolOutput};
use thinclaw_types::JobContext;

use crate::ports::{SkillToolHostPort, tool_scope_from_job_context};

use super::*;

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
