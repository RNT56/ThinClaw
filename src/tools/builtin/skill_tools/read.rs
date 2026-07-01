//! Skill tool: read.

use super::*;

/// Read a loaded skill's full prompt content on demand.
///
/// This enables lazy skill loading: the system prompt announces which skills
/// are active (name + description only), and the agent calls `skill_read`
/// to get the full instructions when it needs them.
pub struct SkillReadTool {
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
}

impl SkillReadTool {
    pub fn new(registry: Arc<tokio::sync::RwLock<SkillRegistry>>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for SkillReadTool {
    fn name(&self) -> &str {
        "skill_read"
    }

    fn description(&self) -> &str {
        "Read a skill's full instructions by name. Use when you need detailed guidance for a specific skill."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_policy::skill_read_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let name = skill_policy::parse_skill_name_param(&params)?;
        ensure_skill_allowed(ctx, &name)?;

        let guard = self.registry.read().await;
        let skill = guard
            .skills()
            .iter()
            .find(|s| s.manifest.name.eq_ignore_ascii_case(&name));

        match skill {
            Some(s) => {
                let output = skill_policy::skill_read_output(
                    &s.manifest.name,
                    &s.manifest.version,
                    &s.manifest.description,
                    &s.trust.to_string(),
                    &s.source_tier.to_string(),
                    &s.prompt_content,
                );
                Ok(ToolOutput::success(output, start.elapsed()))
            }
            None => {
                let available: Vec<String> = guard
                    .skills()
                    .iter()
                    .map(|s| s.manifest.name.clone())
                    .collect();
                Err(ToolError::ExecutionFailed(format!(
                    "Skill '{}' not found. Available skills: {}",
                    name,
                    if available.is_empty() {
                        "none".to_string()
                    } else {
                        available.join(", ")
                    }
                )))
            }
        }
    }
}

// ── skill_list ──────────────────────────────────────────────────────────
