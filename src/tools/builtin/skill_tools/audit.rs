//! Skill tool: audit.

use super::*;

pub struct SkillAuditTool {
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
    quarantine: Arc<QuarantineManager>,
}

impl SkillAuditTool {
    pub fn new(
        registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
        quarantine: Arc<QuarantineManager>,
    ) -> Self {
        Self {
            registry,
            quarantine,
        }
    }
}

#[async_trait]
impl Tool for SkillAuditTool {
    fn name(&self) -> &str {
        "skill_audit"
    }

    fn description(&self) -> &str {
        "Audit loaded skills for risky patterns using the quarantine scanner without modifying or removing them."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_policy::skill_audit_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let target_name = skill_policy::parse_skill_audit_target_name(&params);
        let audited =
            audit_skills_for_registry(&self.registry, &self.quarantine, target_name).await?;

        Ok(ToolOutput::success(
            skill_policy::skill_audit_output(audited),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }
}
