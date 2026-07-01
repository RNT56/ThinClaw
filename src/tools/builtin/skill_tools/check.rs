//! Skill tool: check.

use super::*;

pub struct SkillCheckTool {
    quarantine: Arc<QuarantineManager>,
}

impl SkillCheckTool {
    pub fn new(quarantine: Arc<QuarantineManager>) -> Self {
        Self { quarantine }
    }
}

#[async_trait]
impl Tool for SkillCheckTool {
    fn name(&self) -> &str {
        "skill_check"
    }

    fn description(&self) -> &str {
        "Validate SKILL.md content, a local SKILL.md path, or a direct HTTPS SKILL.md URL without installing it."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_policy::skill_check_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let input = skill_policy::parse_skill_check_input(&params)?;
        let output = skill_check_output_for_input(&self.quarantine, input).await?;
        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }
}

// ── skill_install ───────────────────────────────────────────────────────
