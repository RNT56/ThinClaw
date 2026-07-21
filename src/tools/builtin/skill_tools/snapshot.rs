//! Skill tool: snapshot.

use super::*;

pub struct SkillSnapshotTool {
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
}

impl SkillSnapshotTool {
    pub fn new(registry: Arc<tokio::sync::RwLock<SkillRegistry>>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for SkillSnapshotTool {
    fn name(&self) -> &str {
        "skill_snapshot"
    }

    fn description(&self) -> &str {
        "Write a JSON snapshot of loaded skills, hashes, and provenance tiers to the local skills state directory."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_policy::skill_snapshot_parameters_schema()
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let (snapshot, skill_count) = {
            let guard = self.registry.read().await;
            (
                skill_policy::skill_snapshot_document(
                    Utc::now().to_rfc3339(),
                    guard
                        .skills()
                        .iter()
                        .map(|skill| {
                            skill_policy::skill_snapshot_entry(
                                &skill.manifest.name,
                                &skill.manifest.version,
                                &skill.trust.to_string(),
                                &skill.source_tier.to_string(),
                                &skill.content_hash,
                                source_path_for_skill(skill).map(|path| path.display().to_string()),
                            )
                        })
                        .collect::<Vec<_>>(),
                ),
                guard.count(),
            )
        };

        let snapshot_dir = crate::platform::state_paths().skills_dir.join(".hub");
        tokio::fs::create_dir_all(&snapshot_dir)
            .await
            .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
        let metadata = tokio::fs::symlink_metadata(&snapshot_dir)
            .await
            .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(ToolError::ExecutionFailed(
                "Skill snapshot directory is not a real directory".to_string(),
            ));
        }
        let snapshot_path = snapshot_dir.join(format!(
            "snapshot-{}-{}.json",
            Utc::now().format("%Y%m%dT%H%M%SZ"),
            uuid::Uuid::new_v4().simple()
        ));
        thinclaw_platform::write_private_file_atomic_async(
            snapshot_path.clone(),
            serde_json::to_vec_pretty(&snapshot)
                .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?,
            false,
        )
        .await
        .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;

        Ok(ToolOutput::success(
            skill_policy::skill_snapshot_output(&snapshot_path.display().to_string(), skill_count),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}
