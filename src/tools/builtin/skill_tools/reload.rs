//! Skill tool: reload.

use super::*;

/// Hot-reload a skill (or all skills) from disk without restarting.
///
/// Use after editing a SKILL.md file on disk so that changes take effect
/// in the current session. A single-skill reload is surgical and fast;
/// the `all` flag triggers a full re-discovery pass.
pub struct SkillReloadTool {
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
}

impl SkillReloadTool {
    pub fn new(registry: Arc<tokio::sync::RwLock<SkillRegistry>>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for SkillReloadTool {
    fn name(&self) -> &str {
        "skill_reload"
    }

    fn description(&self) -> &str {
        "Reload a skill (or all skills) from disk after editing SKILL.md files. \
         Use after making on-disk changes so they take effect immediately without restarting. \
         Provide a skill name to reload just that skill, or set all=true to rediscover all skills."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_policy::skill_reload_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let parsed = skill_policy::parse_skill_reload_params(&params);
        let reload_all = parsed.all;

        if reload_all {
            let mut guard = self.registry.write().await;
            let loaded = guard.reload().await;
            let output = skill_policy::skill_reload_all_output(loaded);
            return Ok(ToolOutput::success(output, start.elapsed()));
        }

        // Single-skill reload
        let name = parsed.name.as_deref().ok_or_else(|| {
            ToolError::InvalidParameters("missing required parameter: name".to_string())
        })?;
        let mut guard = self.registry.write().await;

        match guard.reload_skill(name).await {
            Ok(reloaded_name) => {
                let output = skill_policy::skill_reload_output(&reloaded_name);
                Ok(ToolOutput::success(output, start.elapsed()))
            }
            Err(e) => Err(ToolError::ExecutionFailed(format!(
                "Failed to reload skill '{}': {}",
                name, e
            ))),
        }
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        // Reloading changes agent behavior — require explicit approval
        // unless auto-approve is enabled.
        ApprovalRequirement::UnlessAutoApproved
    }
}
