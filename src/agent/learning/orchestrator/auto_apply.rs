use super::*;
impl LearningOrchestrator {
    pub(in crate::agent::learning) async fn authorized_candidate_identity(
        &self,
        candidate: &DbLearningCandidate,
    ) -> Result<CandidateIdentityContext, String> {
        let context = candidate_identity_context(candidate)?;
        if let Some(conversation_id) = context.conversation_id {
            let belongs = self
                .store
                .conversation_belongs_to_identity(
                    conversation_id,
                    &context.principal_id,
                    &context.actor_id,
                    context.conversation_scope_id,
                    crate::identity::to_history_conversation_kind(context.conversation_kind),
                )
                .await
                .map_err(|error| {
                    format!("failed to authorize learning candidate conversation: {error}")
                })?;
            if !belongs {
                return Err(
                    "learning candidate does not belong to its recorded identity scope".to_string(),
                );
            }
        } else if context.conversation_kind == crate::identity::ConversationKind::Group {
            return Err(
                "group learning candidate has no persisted conversation to authorize".to_string(),
            );
        }
        Ok(context)
    }

    pub(in crate::agent::learning) async fn auto_apply_candidate(
        &self,
        _settings: &LearningSettings,
        class: ImprovementClass,
        candidate: &DbLearningCandidate,
    ) -> Result<bool, String> {
        if matches!(class, ImprovementClass::Routine | ImprovementClass::Skill)
            && !self
                .authorized_candidate_identity(candidate)
                .await?
                .is_principal_owner()
        {
            // Routines and executable skills are principal-wide control-plane
            // artifacts. Actor/group evidence may propose them, but only the
            // principal owner may trigger an automatic mutation.
            return Ok(false);
        }
        match class {
            ImprovementClass::Memory => self.auto_apply_memory(candidate).await,
            ImprovementClass::Prompt => self.auto_apply_prompt(candidate).await,
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
        let identity_context = self.authorized_candidate_identity(candidate).await?;
        let identity = identity_context.resolved_identity();

        let Some(entry) = candidate
            .proposal
            .get("memory_entry")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
        else {
            // A generic event summary is not durable knowledge. Only a
            // deliberately distilled memory entry may reach this path.
            return Ok(false);
        };

        let workspace = crate::workspace::AuthorizedWorkspace::conversation(
            workspace,
            &identity,
            &identity_context.channel,
        );
        let artifact_path = workspace.access().memory_path();

        let before = workspace
            .read("MEMORY.md")
            .await
            .ok()
            .map(|doc| doc.content)
            .unwrap_or_default();
        workspace
            .append_memory(entry)
            .await
            .map_err(|e| e.to_string())?;
        let after = workspace
            .read("MEMORY.md")
            .await
            .ok()
            .map(|doc| doc.content)
            .unwrap_or_default();

        let version = DbLearningArtifactVersion {
            id: Uuid::new_v4(),
            candidate_id: Some(candidate.id),
            user_id: candidate.user_id.clone(),
            artifact_type: "memory".to_string(),
            artifact_name: artifact_path,
            version_label: Some(Utc::now().to_rfc3339()),
            status: "applied".to_string(),
            diff_summary: Some("Auto-appended memory entry".to_string()),
            // Do not duplicate complete private memory snapshots into the
            // principal-wide learning artifact feed. The scoped document is
            // authoritative and the artifact records only a non-secret digest.
            before_content: None,
            after_content: None,
            provenance: serde_json::json!({
                "auto_apply": true,
                "class": "memory",
                "before_bytes": before.len(),
                "after_bytes": after.len(),
                "actor_id": identity_context.actor_id,
                "conversation_scope_id": identity_context.conversation_scope_id,
            }),
            created_at: Utc::now(),
        };
        match self.store.insert_learning_artifact_version(&version).await {
            Ok(_) => {
                if let Err(err) =
                    outcomes::maybe_create_artifact_contract(&self.store, &version).await
                {
                    tracing::debug!(error = %err, "Outcome memory artifact hook skipped");
                }
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    candidate_id = %candidate.id,
                    "Memory mutation succeeded but its learning artifact could not be persisted"
                );
            }
        }

        Ok(true)
    }

    pub(in crate::agent::learning) async fn auto_apply_prompt(
        &self,
        candidate: &DbLearningCandidate,
    ) -> Result<bool, String> {
        let identity_context = self.authorized_candidate_identity(candidate).await?;
        let identity = identity_context.resolved_identity();
        let requested_target = candidate
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

        if !is_prompt_target_supported(&requested_target) {
            return Ok(false);
        }
        let is_user_target = requested_target.eq_ignore_ascii_case(paths::USER)
            || requested_target
                .to_ascii_lowercase()
                .ends_with(&format!("/{}", paths::USER.to_ascii_lowercase()));

        let (artifact_name, before, after, private_actor_prompt) = if is_user_target {
            if identity.conversation_kind != crate::identity::ConversationKind::Direct {
                return Ok(false);
            }
            let canonical_target = paths::actor_user(&identity.actor_id);
            if !requested_target.eq_ignore_ascii_case(paths::USER)
                && requested_target != canonical_target
            {
                return Err(
                    "prompt candidate attempted to modify a different actor's USER.md".to_string(),
                );
            }
            let Some(base_workspace) = self.workspace.as_deref() else {
                return Err("workspace unavailable for actor prompt target".to_string());
            };
            let workspace = crate::workspace::AuthorizedWorkspace::conversation(
                base_workspace,
                &identity,
                &identity_context.channel,
            );
            let before = workspace
                .read(paths::USER)
                .await
                .ok()
                .map(|document| document.content)
                .unwrap_or_default();
            let content = if let Some(content) = candidate
                .proposal
                .get("content")
                .and_then(|value| value.as_str())
            {
                content.to_string()
            } else {
                materialize_prompt_candidate_content(&before, &candidate.proposal, paths::USER)?
            };
            validate_prompt_content(&content)?;
            validate_prompt_target_content(paths::USER, &content)?;
            let timezone_update = crate::timezone::actor_timezone_update_for_document(
                &identity,
                paths::USER,
                &content,
            )?;
            workspace
                .write(paths::USER, &content)
                .await
                .map_err(|error| error.to_string())?;
            if let Some(update) = timezone_update {
                crate::timezone::apply_actor_timezone_change(
                    &self.store,
                    &update.principal_id,
                    &update.actor_id,
                    update.timezone.as_deref(),
                )
                .await?;
            }
            let after = workspace
                .read(paths::USER)
                .await
                .map_err(|error| error.to_string())?
                .content;
            (canonical_target, before, after, true)
        } else {
            if !identity_context.is_principal_owner() {
                return Ok(false);
            }
            let before =
                read_prompt_target_content(self.workspace.as_deref(), &requested_target).await?;
            let content = if let Some(content) = candidate
                .proposal
                .get("content")
                .and_then(|value| value.as_str())
            {
                content.to_string()
            } else {
                materialize_prompt_candidate_content(
                    &before,
                    &candidate.proposal,
                    &requested_target,
                )?
            };
            validate_prompt_content(&content)?;
            validate_prompt_target_content(&requested_target, &content)?;
            write_prompt_target_content(self.workspace.as_deref(), &requested_target, &content)
                .await?;
            let after =
                read_prompt_target_content(self.workspace.as_deref(), &requested_target).await?;
            (requested_target, before, after, false)
        };

        let version = DbLearningArtifactVersion {
            id: Uuid::new_v4(),
            candidate_id: Some(candidate.id),
            user_id: candidate.user_id.clone(),
            artifact_type: "prompt".to_string(),
            artifact_name,
            version_label: Some(Utc::now().to_rfc3339()),
            status: "applied".to_string(),
            diff_summary: Some("Auto-applied prompt file update".to_string()),
            before_content: (!private_actor_prompt).then_some(before.clone()),
            after_content: (!private_actor_prompt).then_some(after.clone()),
            provenance: serde_json::json!({
                "auto_apply": true,
                "class": "prompt",
                "actor_id": identity_context.actor_id,
                "conversation_scope_id": identity_context.conversation_scope_id,
                "before_bytes": before.len(),
                "after_bytes": after.len(),
                "private_actor_prompt": private_actor_prompt,
            }),
            created_at: Utc::now(),
        };
        match self.store.insert_learning_artifact_version(&version).await {
            Ok(_) => {
                if let Err(err) =
                    outcomes::maybe_create_artifact_contract(&self.store, &version).await
                {
                    tracing::debug!(error = %err, "Outcome prompt artifact hook skipped");
                }
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    candidate_id = %candidate.id,
                    "Prompt mutation succeeded but its learning artifact could not be persisted"
                );
            }
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
        match self.store.insert_learning_artifact_version(&version).await {
            Ok(_) => {
                if let Err(err) =
                    outcomes::maybe_create_artifact_contract(&self.store, &version).await
                {
                    tracing::debug!(error = %err, "Outcome skill artifact hook skipped");
                }
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    candidate_id = %candidate.id,
                    "Skill mutation succeeded but its learning artifact could not be persisted"
                );
            }
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
        match self.store.insert_learning_artifact_version(&version).await {
            Ok(_) => {
                if let Err(err) =
                    outcomes::maybe_create_artifact_contract(&self.store, &version).await
                {
                    tracing::debug!(error = %err, "Outcome routine artifact hook skipped");
                }
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    candidate_id = %candidate.id,
                    "Routine mutation succeeded but its learning artifact could not be persisted"
                );
            }
        }
        Ok(true)
    }
}
