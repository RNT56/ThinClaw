//! Skill tool: update.

use super::*;

pub struct SkillUpdateTool {
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
    installer: SkillInstallTool,
}

impl SkillUpdateTool {
    pub fn new(
        registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
        catalog: Arc<SkillCatalog>,
        remote_hub: Option<SharedRemoteSkillHub>,
        quarantine: Arc<QuarantineManager>,
    ) -> Self {
        let installer =
            SkillInstallTool::new(Arc::clone(&registry), catalog, remote_hub, quarantine);
        Self {
            registry,
            installer,
        }
    }
}

#[async_trait]
impl Tool for SkillUpdateTool {
    fn name(&self) -> &str {
        "skill_update"
    }

    fn description(&self) -> &str {
        "Update an installed skill using its recorded provenance lock when available."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_policy::skill_update_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let parsed = skill_policy::parse_skill_update_params(&params)?;
        let name = parsed.name.as_str();
        let approve_risky = parsed.approve_risky;

        let (source_path, installed_name) = {
            let guard = self.registry.read().await;
            let skill = guard
                .skills()
                .iter()
                .find(|skill| skill.manifest.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| ToolError::ExecutionFailed(format!("Skill '{}' not found", name)))?;
            (
                source_path_for_skill(skill).ok_or_else(|| {
                    ToolError::ExecutionFailed(format!(
                        "Skill '{}' does not have a filesystem source path",
                        name
                    ))
                })?,
                skill.manifest.name.clone(),
            )
        };

        let provenance = read_skill_provenance(&source_path).await.map_err(|_| {
            ToolError::ExecutionFailed(format!(
                "Skill '{}' is missing a provenance lock and cannot be auto-updated",
                name
            ))
        })?;

        let mut install_params =
            skill_policy::skill_update_install_params(&installed_name, true, approve_risky);

        match provenance.source_adapter.as_str() {
            "clawhub_catalog" => {
                install_params["name"] = serde_json::Value::String(provenance.source_ref.clone());
            }
            "github_tap" | "well_known" => {
                install_params["name"] = serde_json::Value::String(installed_name);
            }
            "url" => {
                let url = provenance
                    .source_url
                    .clone()
                    .or(provenance.manifest_url.clone())
                    .ok_or_else(|| {
                        ToolError::ExecutionFailed(format!(
                            "Skill '{}' has URL provenance but no fetchable URL",
                            name
                        ))
                    })?;
                skill_policy::add_skill_update_url(&mut install_params, url);
            }
            _ => {
                if let Some(url) = provenance
                    .source_url
                    .clone()
                    .or(provenance.manifest_url.clone())
                {
                    skill_policy::add_skill_update_url(&mut install_params, url);
                } else {
                    return Err(ToolError::ExecutionFailed(format!(
                        "Skill '{}' does not have a supported update source",
                        name
                    )));
                }
            }
        }

        self.installer.execute(install_params, ctx).await
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

// ── skill_publish ───────────────────────────────────────────────────────
