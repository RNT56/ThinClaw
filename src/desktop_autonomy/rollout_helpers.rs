use super::*;
impl DesktopAutonomyManager {
    pub(super) async fn promote_build(&self, build_dir: &Path) -> Result<(), String> {
        let current = self.current_build_link();
        let tmp_link = self.state_root.join("current.next");
        let _ = tokio::fs::remove_file(&tmp_link).await;
        create_symlink_dir(build_dir, &tmp_link)?;
        tokio::fs::rename(&tmp_link, &current)
            .await
            .map_err(|e| format!("failed to atomically promote build: {e}"))?;
        Ok(())
    }

    pub(super) async fn trim_old_builds(&self) -> Result<(), String> {
        let manifests = self.list_build_manifests().await?;
        let current = self.current_build_id();
        let mut promoted: Vec<_> = manifests
            .into_iter()
            .filter(|manifest| manifest.promoted)
            .collect();
        promoted.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        for manifest in promoted.iter().skip(3) {
            if current.as_deref() == Some(manifest.build_id.as_str()) {
                continue;
            }
            let build_dir = self.builds_dir().join(&manifest.build_id);
            let _ = tokio::fs::remove_dir_all(build_dir).await;
        }
        Ok(())
    }

    pub(super) async fn load_rollout_state(&self) -> Result<RolloutState, String> {
        match tokio::fs::read_to_string(self.rollout_state_path()).await {
            Ok(raw) => serde_json::from_str(&raw)
                .map_err(|e| format!("failed to parse rollout state: {e}")),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(RolloutState::default()),
            Err(err) => Err(format!("failed to read rollout state: {err}")),
        }
    }

    pub(super) async fn save_rollout_state(&self, state: &RolloutState) -> Result<(), String> {
        let raw = serde_json::to_string_pretty(state)
            .map_err(|e| format!("failed to serialize rollout state: {e}"))?;
        tokio::fs::write(self.rollout_state_path(), raw)
            .await
            .map_err(|e| format!("failed to write rollout state: {e}"))
    }

    pub(super) async fn write_build_manifest(
        &self,
        build_id: &str,
        manifest: &BuildManifest,
    ) -> Result<(), String> {
        let path = self
            .state_root
            .join("manifests")
            .join(format!("{build_id}.json"));
        let raw = serde_json::to_string_pretty(manifest)
            .map_err(|e| format!("failed to serialize build manifest: {e}"))?;
        tokio::fs::write(path, raw)
            .await
            .map_err(|e| format!("failed to write build manifest: {e}"))
    }

    pub(super) async fn list_build_manifests(&self) -> Result<Vec<BuildManifest>, String> {
        let mut results = Vec::new();
        let manifest_dir = self.state_root.join("manifests");
        if !manifest_dir.exists() {
            return Ok(results);
        }
        let mut entries = tokio::fs::read_dir(&manifest_dir)
            .await
            .map_err(|e| format!("failed to read manifest dir: {e}"))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| format!("failed to iterate manifest dir: {e}"))?
        {
            let raw = tokio::fs::read_to_string(entry.path())
                .await
                .map_err(|e| format!("failed to read manifest {}: {e}", entry.path().display()))?;
            let manifest: BuildManifest = serde_json::from_str(&raw)
                .map_err(|e| format!("failed to parse manifest {}: {e}", entry.path().display()))?;
            results.push(manifest);
        }
        results.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(results)
    }

    pub(super) async fn record_rollback_observation(
        &self,
        current_build_id: &str,
    ) -> Result<(), String> {
        let Some(store) = self.store.as_ref() else {
            return Ok(());
        };
        let manifests = self.list_build_manifests().await?;
        let Some(current_manifest) = manifests
            .iter()
            .find(|manifest| manifest.build_id == current_build_id)
        else {
            return Ok(());
        };
        let versions = store
            .list_learning_artifact_versions(
                &current_manifest.user_id,
                Some("code"),
                Some(current_build_id),
                5,
            )
            .await
            .map_err(|e| format!("failed to list code artifact versions for rollback: {e}"))?;
        let Some(version) = versions.first() else {
            return Ok(());
        };
        let rollback = crate::history::LearningRollbackRecord {
            id: Uuid::new_v4(),
            user_id: current_manifest.user_id.clone(),
            artifact_type: "code".to_string(),
            artifact_name: current_build_id.to_string(),
            artifact_version_id: Some(version.id),
            reason: "desktop autonomy rollback".to_string(),
            metadata: serde_json::json!({
                "build_id": current_build_id,
                "rolled_back_at": Utc::now(),
                "autonomy_control": true,
            }),
            created_at: Utc::now(),
        };
        store
            .insert_learning_rollback(&rollback)
            .await
            .map_err(|e| format!("failed to persist rollback record: {e}"))?;
        crate::agent::outcomes::observe_rollback(store, &rollback).await?;
        Ok(())
    }
}
