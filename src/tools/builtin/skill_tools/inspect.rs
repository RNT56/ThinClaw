//! Skill tool: inspect.

use super::*;

pub struct SkillInspectTool {
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
    quarantine: Arc<QuarantineManager>,
}

impl SkillInspectTool {
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
impl Tool for SkillInspectTool {
    fn name(&self) -> &str {
        "skill_inspect"
    }

    fn description(&self) -> &str {
        "Inspect one loaded skill with metadata, provenance, files, and optional audit findings."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_policy::skill_inspect_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let parsed = skill_policy::parse_skill_inspect_params(&params)?;
        ensure_skill_allowed(ctx, &parsed.name)?;

        let output = inspect_skill_report(
            &self.registry,
            &self.quarantine,
            &parsed.name,
            parsed.include_content,
            parsed.include_files,
            parsed.audit,
        )
        .await?;
        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }
}
