//! Skill tool: remove.

use super::*;

pub struct SkillRemoveTool {
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
}

pub struct SkillPromoteTrustTool {
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
}

impl SkillPromoteTrustTool {
    pub fn new(registry: Arc<tokio::sync::RwLock<SkillRegistry>>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for SkillPromoteTrustTool {
    fn name(&self) -> &str {
        "skill_trust_promote"
    }

    fn description(&self) -> &str {
        "Promote or demote a user-managed skill between installed and trusted trust ceilings."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_policy::skill_trust_promote_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let parsed = skill_policy::parse_skill_trust_promote_params(&params)?;
        let name = parsed.name.as_str();
        let target_trust = match parsed.target_trust.as_str() {
            "installed" => SkillTrust::Installed,
            "trusted" => SkillTrust::Trusted,
            _ => unreachable!("skill policy validates target_trust"),
        };
        let source_tier =
            promote_skill_trust_in_registry(&self.registry, name, target_trust).await?;

        Ok(ToolOutput::success(
            skill_policy::skill_trust_promote_output(name, &target_trust.to_string(), &source_tier),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

impl SkillRemoveTool {
    pub fn new(registry: Arc<tokio::sync::RwLock<SkillRegistry>>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for SkillRemoveTool {
    fn name(&self) -> &str {
        "skill_remove"
    }

    fn description(&self) -> &str {
        "Remove an installed skill by name. Only user-installed skills can be removed."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_policy::skill_remove_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let name = skill_policy::parse_skill_name_param(&params)?;
        let name = name.as_str();
        remove_skill_from_registry(&self.registry, name).await?;

        let output = skill_policy::skill_remove_output(name);

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

// ── skill_reload ────────────────────────────────────────────────────────
