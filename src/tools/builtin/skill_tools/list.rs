//! Skill tool: list.

use super::*;

pub struct SkillListTool {
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
}

impl SkillListTool {
    pub fn new(registry: Arc<tokio::sync::RwLock<SkillRegistry>>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for SkillListTool {
    fn name(&self) -> &str {
        "skill_list"
    }

    fn description(&self) -> &str {
        "List all loaded skills with their trust level, source, and activation keywords."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_policy::skill_list_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let parsed = skill_policy::parse_skill_list_params(&params);
        let allowed_skills = restricted_skill_names(ctx);

        let guard = self.registry.read().await;

        let skills: Vec<serde_json::Value> = guard
            .skills()
            .iter()
            .filter(|s| {
                allowed_skills
                    .as_ref()
                    .is_none_or(|allowed| allowed.contains(s.manifest.name.as_str()))
            })
            .map(|s| {
                let mut entry = skill_policy::skill_list_entry(
                    &s.manifest.name,
                    &s.manifest.description,
                    &s.trust.to_string(),
                    &s.source_tier.to_string(),
                    &format!("{:?}", s.source),
                    serde_json::json!(s.manifest.activation.keywords),
                );

                if parsed.verbose {
                    let mut provenance = None;
                    let mut lifecycle_status = None;
                    let mut outcome_score = None;
                    let mut reuse_count = None;
                    let mut activation_reason = None;
                    if let Some(openclaw) = s
                        .manifest
                        .metadata
                        .as_ref()
                        .and_then(|metadata| metadata.openclaw.as_ref())
                    {
                        provenance = Some(serde_json::json!(openclaw.provenance.clone()));
                        lifecycle_status =
                            Some(serde_json::json!(openclaw.lifecycle_status.clone()));
                        outcome_score = Some(serde_json::json!(openclaw.outcome_score));
                        reuse_count = Some(serde_json::json!(openclaw.reuse_count));
                        activation_reason =
                            Some(serde_json::json!(openclaw.activation_reason.clone()));
                    }
                    skill_policy::add_skill_list_verbose_fields(
                        &mut entry,
                        skill_policy::SkillListVerboseFields {
                            version: s.manifest.version.clone(),
                            tags: serde_json::json!(s.manifest.activation.tags),
                            content_hash: s.content_hash.clone(),
                            max_context_tokens: serde_json::json!(
                                s.manifest.activation.max_context_tokens
                            ),
                            provenance,
                            lifecycle_status,
                            outcome_score,
                            reuse_count,
                            activation_reason,
                        },
                    );
                }

                entry
            })
            .collect();

        let output = skill_policy::skill_list_output(skills);

        Ok(ToolOutput::success(output, start.elapsed()))
    }
}

// ── skill_search ────────────────────────────────────────────────────────
