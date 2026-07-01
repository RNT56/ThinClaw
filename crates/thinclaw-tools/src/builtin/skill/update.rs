//! Skill tool policy: update.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use thinclaw_tools_core::{ApprovalRequirement, Tool, ToolError, ToolOutput};
use thinclaw_types::JobContext;

use crate::ports::{
    SkillInstallToolHostPort, ToolSkillUpdateActionRequest, tool_scope_from_job_context,
};

use super::*;

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
