//! Skill tool policy: install.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use thinclaw_tools_core::{ApprovalRequirement, Tool, ToolError, ToolOutput};
use thinclaw_types::JobContext;

use crate::ports::{
    SkillInstallToolHostPort, ToolSkillInstallActionRequest, tool_scope_from_job_context,
};

use super::*;

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
