//! Skill tool policy: trust.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use thinclaw_tools_core::{ApprovalRequirement, Tool, ToolError, ToolOutput};
use thinclaw_types::JobContext;

use crate::ports::{
    SkillToolHostPort, ToolSkillTrust, ToolSkillTrustMutationRequest, tool_scope_from_job_context,
};

use super::*;

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
