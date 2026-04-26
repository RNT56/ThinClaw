use super::*;
impl LearningOrchestrator {
    pub(in crate::agent::learning) async fn auto_apply_candidate(
        &self,
        settings: &LearningSettings,
        class: ImprovementClass,
        candidate: &DbLearningCandidate,
    ) -> Result<bool, String> {
        match class {
            ImprovementClass::Memory => self.auto_apply_memory(candidate).await,
            ImprovementClass::Prompt => {
                if !settings.prompt_mutation.enabled {
                    return Ok(false);
                }
                self.auto_apply_prompt(candidate).await
            }
            ImprovementClass::Routine => self.auto_apply_routine(candidate).await,
            ImprovementClass::Skill => self.auto_apply_skill(candidate).await,
            _ => Ok(false),
        }
    }

    pub(in crate::agent::learning) async fn auto_apply_memory(
        &self,
        candidate: &DbLearningCandidate,
    ) -> Result<bool, String> {
        let Some(workspace) = self.workspace.as_ref() else {
            return Ok(false);
        };

        let entry = candidate
            .proposal
            .get("memory_entry")
            .and_then(|v| v.as_str())
            .or(candidate.summary.as_deref())
            .unwrap_or("New learning captured from recent interaction.");

        let before = workspace
            .read(paths::MEMORY)
            .await
            .ok()
            .map(|doc| doc.content)
            .unwrap_or_default();
        workspace
            .append_memory(entry)
            .await
            .map_err(|e| e.to_string())?;
        let after = workspace
            .read(paths::MEMORY)
            .await
            .ok()
            .map(|doc| doc.content)
            .unwrap_or_default();

        let version = DbLearningArtifactVersion {
            id: Uuid::new_v4(),
            candidate_id: Some(candidate.id),
            user_id: candidate.user_id.clone(),
            artifact_type: "memory".to_string(),
            artifact_name: paths::MEMORY.to_string(),
            version_label: Some(Utc::now().to_rfc3339()),
            status: "applied".to_string(),
            diff_summary: Some("Auto-appended memory entry".to_string()),
            before_content: Some(before),
            after_content: Some(after),
            provenance: serde_json::json!({"auto_apply": true, "class": "memory"}),
            created_at: Utc::now(),
        };
        if self
            .store
            .insert_learning_artifact_version(&version)
            .await
            .is_ok()
            && let Err(err) = outcomes::maybe_create_artifact_contract(&self.store, &version).await
        {
            tracing::debug!(error = %err, "Outcome memory artifact hook skipped");
        }

        Ok(true)
    }

    pub(in crate::agent::learning) async fn auto_apply_prompt(
        &self,
        candidate: &DbLearningCandidate,
    ) -> Result<bool, String> {
        let target = candidate
            .target_name
            .clone()
            .or_else(|| {
                candidate
                    .proposal
                    .get("target")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| paths::USER.to_string());

        if !is_prompt_target_supported(&target) {
            return Ok(false);
        }

        let before = read_prompt_target_content(self.workspace.as_deref(), &target).await?;
        let content =
            if let Some(content) = candidate.proposal.get("content").and_then(|v| v.as_str()) {
                content.to_string()
            } else {
                materialize_prompt_candidate_content(&before, &candidate.proposal, &target)?
            };

        validate_prompt_content(&content)?;
        validate_prompt_target_content(&target, &content)?;
        write_prompt_target_content(self.workspace.as_deref(), &target, &content).await?;
        let after = read_prompt_target_content(self.workspace.as_deref(), &target).await?;

        let version = DbLearningArtifactVersion {
            id: Uuid::new_v4(),
            candidate_id: Some(candidate.id),
            user_id: candidate.user_id.clone(),
            artifact_type: "prompt".to_string(),
            artifact_name: target,
            version_label: Some(Utc::now().to_rfc3339()),
            status: "applied".to_string(),
            diff_summary: Some("Auto-applied prompt file update".to_string()),
            before_content: Some(before),
            after_content: Some(after),
            provenance: serde_json::json!({"auto_apply": true, "class": "prompt"}),
            created_at: Utc::now(),
        };
        if self
            .store
            .insert_learning_artifact_version(&version)
            .await
            .is_ok()
            && let Err(err) = outcomes::maybe_create_artifact_contract(&self.store, &version).await
        {
            tracing::debug!(error = %err, "Outcome prompt artifact hook skipped");
        }

        Ok(true)
    }

    pub(in crate::agent::learning) async fn auto_apply_skill(
        &self,
        candidate: &DbLearningCandidate,
    ) -> Result<bool, String> {
        let Some(registry) = self.skill_registry.as_ref() else {
            return Ok(false);
        };

        let Some(skill_content) = candidate
            .proposal
            .get("skill_content")
            .and_then(|v| v.as_str())
            .map(str::to_string)
        else {
            return Ok(false);
        };

        let parsed = crate::skills::parser::parse_skill_md(&crate::skills::normalize_line_endings(
            &skill_content,
        ))
        .map_err(|e| e.to_string())?;
        let skill_name = parsed.manifest.name.clone();

        let mut guard = registry.write().await;
        let before_content = guard
            .find_by_name(&skill_name)
            .map(|s| s.prompt_content.clone());
        if guard.has(&skill_name) {
            let _ = guard.remove_skill(&skill_name).await;
        }
        guard
            .install_skill(&skill_content)
            .await
            .map_err(|e| e.to_string())?;

        let after_content = guard
            .find_by_name(&skill_name)
            .map(|s| s.prompt_content.clone());

        let version = DbLearningArtifactVersion {
            id: Uuid::new_v4(),
            candidate_id: Some(candidate.id),
            user_id: candidate.user_id.clone(),
            artifact_type: "skill".to_string(),
            artifact_name: skill_name,
            version_label: Some(Utc::now().to_rfc3339()),
            status: "applied".to_string(),
            diff_summary: Some("Auto-applied skill revision".to_string()),
            before_content,
            after_content,
            provenance: serde_json::json!({"auto_apply": true, "class": "skill"}),
            created_at: Utc::now(),
        };
        if self
            .store
            .insert_learning_artifact_version(&version)
            .await
            .is_ok()
            && let Err(err) = outcomes::maybe_create_artifact_contract(&self.store, &version).await
        {
            tracing::debug!(error = %err, "Outcome skill artifact hook skipped");
        }

        Ok(true)
    }

    pub(in crate::agent::learning) async fn auto_apply_routine(
        &self,
        candidate: &DbLearningCandidate,
    ) -> Result<bool, String> {
        let Some(engine) = self.routine_engine.as_ref() else {
            return Ok(false);
        };
        let Some(patch) = candidate.proposal.get("routine_patch") else {
            return Ok(false);
        };
        let patch_type = patch
            .get("type")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        if patch_type != "notification_noise_reduction" {
            return Ok(false);
        }
        let routine_id = patch
            .get("routine_id")
            .and_then(|value| value.as_str())
            .ok_or_else(|| "routine patch missing routine_id".to_string())
            .and_then(|value| Uuid::parse_str(value).map_err(|err| err.to_string()))?;

        let Some(mut routine) = self
            .store
            .get_routine(routine_id)
            .await
            .map_err(|err| err.to_string())?
        else {
            return Ok(false);
        };

        if !routine.notify.on_success {
            return Ok(false);
        }

        let before = serde_json::to_string_pretty(&routine).map_err(|err| err.to_string())?;
        routine.notify.on_success = false;
        routine.updated_at = Utc::now();
        self.store
            .update_routine(&routine)
            .await
            .map_err(|err| err.to_string())?;
        let after = serde_json::to_string_pretty(&routine).map_err(|err| err.to_string())?;
        engine.refresh_event_cache().await;

        let version = DbLearningArtifactVersion {
            id: Uuid::new_v4(),
            candidate_id: Some(candidate.id),
            user_id: candidate.user_id.clone(),
            artifact_type: "routine".to_string(),
            artifact_name: routine.name.clone(),
            version_label: Some(Utc::now().to_rfc3339()),
            status: "applied".to_string(),
            diff_summary: Some("Auto-disabled routine success notifications".to_string()),
            before_content: Some(before),
            after_content: Some(after),
            provenance: serde_json::json!({
                "auto_apply": true,
                "class": "routine",
                "patch_type": patch_type,
                "routine_id": routine.id.to_string(),
                "routine_name": routine.name,
                "actor_id": routine.owner_actor_id(),
            }),
            created_at: Utc::now(),
        };
        if self
            .store
            .insert_learning_artifact_version(&version)
            .await
            .is_ok()
            && let Err(err) = outcomes::maybe_create_artifact_contract(&self.store, &version).await
        {
            tracing::debug!(error = %err, "Outcome routine artifact hook skipped");
        }
        Ok(true)
    }
}
