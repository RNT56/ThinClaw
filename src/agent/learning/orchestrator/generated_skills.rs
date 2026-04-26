use super::*;
impl LearningOrchestrator {
    pub async fn review_completed_turn_for_generated_skill(
        &self,
        session: &crate::agent::session::Session,
        thread_id: Uuid,
        _incoming: &crate::channels::IncomingMessage,
        turn: &crate::agent::session::Turn,
    ) -> Result<Option<String>, String> {
        if turn.state != crate::agent::session::TurnState::Completed {
            return Ok(None);
        }

        let owner_user_id = &session.user_id;
        let settings = self.load_settings_for_user(owner_user_id).await;
        if !settings.skill_synthesis.enabled {
            return Ok(None);
        }
        let workflow_digest = generated_workflow_digest(&turn.user_input, &turn.tool_calls);
        let skill_name = format!("workflow-{}", &workflow_digest[7..19]);
        let existing_candidates = self
            .store
            .list_learning_candidates(owner_user_id, Some("skill"), None, 100)
            .await
            .map_err(|err| err.to_string())?;
        let reuse_count = existing_candidates
            .iter()
            .filter(|candidate| {
                candidate
                    .proposal
                    .get("workflow_digest")
                    .and_then(|value| value.as_str())
                    == Some(workflow_digest.as_str())
                    && candidate.created_at >= Utc::now() - chrono::Duration::days(30)
            })
            .count() as u32
            + 1;

        let triggers = generated_skill_triggers(
            turn,
            &turn.user_input,
            reuse_count,
            settings.skill_synthesis.min_tool_calls,
        );
        if triggers.is_empty() {
            return Ok(None);
        }

        let (lifecycle, activation_reason, lifecycle_should_activate) =
            generated_skill_lifecycle_for_reuse(reuse_count);
        let should_activate = lifecycle_should_activate || settings.skill_synthesis.auto_apply;
        let created_at = Utc::now();
        let skill_content = synthesize_generated_skill_markdown(
            &skill_name,
            &turn.user_input,
            &turn.tool_calls,
            lifecycle,
            reuse_count,
            activation_reason.clone(),
        )?;
        let outcome_score = match reuse_count {
            0 | 1 => 0.78,
            2 | 3 => 0.92,
            _ => 0.96,
        };
        let proposal = serde_json::json!({
            "workflow_digest": workflow_digest,
            "provenance": "generated",
            "lifecycle_status": lifecycle.as_str(),
            "reuse_count": reuse_count,
            "outcome_score": outcome_score,
            "activation_reason": activation_reason,
            "skill_content": skill_content,
            "thread_id": thread_id,
            "turn_number": turn.turn_number,
            "tool_count": turn.tool_calls.len(),
            "trigger_kinds": triggers
                .iter()
                .map(|trigger| serde_json::Value::String(trigger.as_str().to_string()))
                .collect::<Vec<_>>(),
            "last_transition_at": created_at,
            "state_history": [generated_skill_transition_entry(
                lifecycle,
                activation_reason.as_deref(),
                None,
                None,
                None,
                created_at,
            )],
        });
        let candidate = DbLearningCandidate {
            id: Uuid::new_v4(),
            learning_event_id: None,
            user_id: owner_user_id.clone(),
            candidate_type: "skill".to_string(),
            risk_tier: RiskTier::Medium.as_str().to_string(),
            confidence: Some(outcome_score),
            target_type: Some("skill".to_string()),
            target_name: Some(skill_name.clone()),
            summary: Some(format!(
                "Generated procedural skill for workflow digest {} ({})",
                &workflow_digest[7..19],
                triggers
                    .iter()
                    .map(|trigger| trigger.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            proposal: proposal.clone(),
            created_at,
        };
        self.store
            .insert_learning_candidate(&candidate)
            .await
            .map_err(|err| err.to_string())?;
        self.store
            .insert_learning_artifact_version(&DbLearningArtifactVersion {
                id: Uuid::new_v4(),
                candidate_id: Some(candidate.id),
                user_id: owner_user_id.clone(),
                artifact_type: "skill".to_string(),
                artifact_name: skill_name.clone(),
                version_label: Some(Utc::now().to_rfc3339()),
                status: lifecycle.as_str().to_string(),
                diff_summary: Some(match lifecycle {
                    GeneratedSkillLifecycle::Draft => {
                        "Generated procedural skill draft".to_string()
                    }
                    GeneratedSkillLifecycle::Shadow => {
                        "Generated procedural skill shadow candidate".to_string()
                    }
                    GeneratedSkillLifecycle::Proposed => {
                        "Generated procedural skill proposal candidate".to_string()
                    }
                    _ => "Generated procedural skill lifecycle update".to_string(),
                }),
                before_content: None,
                after_content: proposal
                    .get("skill_content")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                provenance: serde_json::json!({
                    "provenance": "generated",
                    "workflow_digest": proposal.get("workflow_digest").cloned().unwrap_or(serde_json::Value::Null),
                    "reuse_count": reuse_count,
                    "lifecycle_status": lifecycle.as_str(),
                    "activation_reason": proposal.get("activation_reason").cloned().unwrap_or(serde_json::Value::Null),
                    "duplicate_handling": "new_candidate_per_trace",
                }),
                created_at,
            })
            .await
            .map_err(|err| err.to_string())?;

        if should_activate {
            self.activate_generated_skill(
                Some(&candidate),
                owner_user_id,
                &skill_name,
                proposal
                    .get("skill_content")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default(),
                reuse_count,
                proposal
                    .get("activation_reason")
                    .and_then(|value| value.as_str())
                    .unwrap_or("generated_activation"),
                None,
                None,
            )
            .await?;
            return Ok(Some(skill_name));
        }

        Ok(None)
    }

    pub(in crate::agent::learning) async fn activate_generated_skill(
        &self,
        candidate: Option<&DbLearningCandidate>,
        user_id: &str,
        skill_name: &str,
        skill_content: &str,
        reuse_count: u32,
        activation_reason: &str,
        feedback_verdict: Option<&str>,
        feedback_note: Option<&str>,
    ) -> Result<(), String> {
        let Some(registry) = self.skill_registry.as_ref() else {
            return Ok(());
        };
        let normalized = crate::skills::normalize_line_endings(skill_content);
        let _parsed =
            crate::skills::parser::parse_skill_md(&normalized).map_err(|err| err.to_string())?;
        let quarantine = QuarantineManager::new(crate::platform::resolve_data_dir(
            "generated_skill_quarantine",
        ));
        let quarantined = quarantine
            .quarantine_skill(
                skill_name,
                &SkillContent {
                    raw_content: normalized.clone(),
                    source_kind: "generated".to_string(),
                    source_adapter: "procedural_reviewer".to_string(),
                    source_ref: skill_name.to_string(),
                    source_repo: None,
                    source_url: None,
                    manifest_url: None,
                    manifest_digest: None,
                    path: None,
                    branch: None,
                    commit_sha: None,
                    trust_level: SkillTapTrustLevel::Trusted,
                },
            )
            .await
            .map_err(|err| err.to_string())?;
        let findings = quarantine.scan_quarantined(&quarantined);
        if findings
            .iter()
            .any(|finding| finding.severity == FindingSeverity::Critical)
        {
            quarantine.cleanup(&quarantined).await;
            return Err(format!(
                "generated skill blocked by static scan: {}",
                findings
                    .iter()
                    .map(|finding| format!("{}:{}", finding.kind, finding.excerpt))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let (install_root, before_content) = {
            let guard = registry.read().await;
            (
                guard.install_root().to_path_buf(),
                guard
                    .find_by_name(skill_name)
                    .map(|skill| skill.prompt_content.clone()),
            )
        };
        let (prepared_name, loaded_skill) =
            SkillRegistry::prepare_install_to_disk(&install_root, skill_name, &normalized)
                .await
                .map_err(|err| err.to_string())?;
        quarantine.cleanup(&quarantined).await;
        let after_content = loaded_skill.prompt_content.clone();

        let existing_remove_path = {
            let guard = registry.read().await;
            if guard.has(skill_name) {
                Some(
                    guard
                        .validate_remove(skill_name)
                        .map_err(|err| err.to_string())?,
                )
            } else {
                None
            }
        };
        if let Some(path) = existing_remove_path.as_ref() {
            SkillRegistry::delete_skill_files(path)
                .await
                .map_err(|err| err.to_string())?;
        }

        let mut guard = registry.write().await;
        if guard.has(skill_name) {
            guard
                .commit_remove(skill_name)
                .map_err(|err| err.to_string())?;
        }
        guard
            .commit_install(&prepared_name, loaded_skill)
            .map_err(|err| err.to_string())?;
        drop(guard);

        let version = DbLearningArtifactVersion {
            id: Uuid::new_v4(),
            candidate_id: candidate.map(|entry| entry.id),
            user_id: user_id.to_string(),
            artifact_type: "skill".to_string(),
            artifact_name: skill_name.to_string(),
            version_label: Some(Utc::now().to_rfc3339()),
            status: GeneratedSkillLifecycle::Active.as_str().to_string(),
            diff_summary: Some("Generated procedural skill activated".to_string()),
            before_content,
            after_content: Some(after_content),
            provenance: serde_json::json!({
                "provenance": "generated",
                "lifecycle_status": GeneratedSkillLifecycle::Active.as_str(),
                "reuse_count": reuse_count,
                "activation_reason": activation_reason,
                "install_pipeline": "prepare_install_to_disk+commit_install",
                "scan_findings": findings,
            }),
            created_at: Utc::now(),
        };
        self.store
            .insert_learning_artifact_version(&version)
            .await
            .map_err(|err| err.to_string())?;
        if let Err(err) = outcomes::maybe_create_artifact_contract(&self.store, &version).await {
            tracing::debug!(error = %err, "Generated skill outcome hook skipped");
        }
        if let Some(candidate) = candidate {
            self.update_generated_skill_candidate_proposal(
                candidate,
                GeneratedSkillLifecycle::Active,
                Some(activation_reason),
                feedback_verdict,
                feedback_note,
                Some(version.id),
            )
            .await?;
        }

        Ok(())
    }
    pub(in crate::agent::learning) async fn update_generated_skill_candidate_proposal(
        &self,
        candidate: &DbLearningCandidate,
        lifecycle: GeneratedSkillLifecycle,
        activation_reason: Option<&str>,
        feedback_verdict: Option<&str>,
        feedback_note: Option<&str>,
        artifact_version_id: Option<Uuid>,
    ) -> Result<(), String> {
        let proposal = updated_generated_skill_proposal(
            candidate,
            lifecycle,
            activation_reason,
            feedback_verdict,
            feedback_note,
            artifact_version_id,
            Utc::now(),
        );
        self.store
            .update_learning_candidate_proposal(candidate.id, &proposal)
            .await
            .map_err(|err| err.to_string())
    }

    pub(in crate::agent::learning) async fn apply_generated_skill_feedback(
        &self,
        user_id: &str,
        target_type: &str,
        target_id: &str,
        verdict: &str,
        note: Option<&str>,
    ) -> Result<(), String> {
        if !target_type.eq_ignore_ascii_case("skill") {
            return Ok(());
        }
        let polarity = generated_skill_feedback_polarity(verdict);
        if polarity == 0 {
            return Ok(());
        }

        let Some(candidate) = self
            .store
            .list_learning_candidates(user_id, Some("skill"), None, 200)
            .await
            .map_err(|err| err.to_string())?
            .into_iter()
            .filter(|candidate| {
                candidate.target_name.as_deref() == Some(target_id)
                    && candidate
                        .proposal
                        .get("provenance")
                        .and_then(|value| value.as_str())
                        == Some("generated")
            })
            .max_by_key(|candidate| candidate.created_at)
        else {
            return Ok(());
        };

        let skill_content = candidate
            .proposal
            .get("skill_content")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let reuse_count = candidate
            .proposal
            .get("reuse_count")
            .and_then(|value| value.as_u64())
            .unwrap_or(1) as u32;

        if polarity > 0 {
            self.activate_generated_skill(
                Some(&candidate),
                user_id,
                target_id,
                skill_content,
                reuse_count,
                "explicit_positive_feedback",
                Some(verdict),
                note,
            )
            .await?;
            return Ok(());
        }

        let Some(registry) = self.skill_registry.as_ref() else {
            return Ok(());
        };
        let mut guard = registry.write().await;
        let before_content = guard
            .find_by_name(target_id)
            .map(|skill| skill.prompt_content.clone());
        let removed = if guard.has(target_id) {
            guard.remove_skill(target_id).await.is_ok()
        } else {
            false
        };
        drop(guard);

        let lifecycle = if removed {
            GeneratedSkillLifecycle::RolledBack
        } else {
            GeneratedSkillLifecycle::Frozen
        };
        let version = DbLearningArtifactVersion {
            id: Uuid::new_v4(),
            candidate_id: Some(candidate.id),
            user_id: user_id.to_string(),
            artifact_type: "skill".to_string(),
            artifact_name: target_id.to_string(),
            version_label: Some(Utc::now().to_rfc3339()),
            status: lifecycle.as_str().to_string(),
            diff_summary: Some(format!(
                "Generated procedural skill {} after feedback verdict '{}'",
                if removed { "rolled back" } else { "frozen" },
                verdict
            )),
            before_content,
            after_content: None,
            provenance: serde_json::json!({
                "provenance": "generated",
                "lifecycle_status": lifecycle.as_str(),
                "activation_reason": "explicit_negative_feedback",
                "feedback_verdict": verdict,
                "feedback_note": note,
            }),
            created_at: Utc::now(),
        };
        self.store
            .insert_learning_artifact_version(&version)
            .await
            .map_err(|err| err.to_string())?;
        self.update_generated_skill_candidate_proposal(
            &candidate,
            lifecycle,
            Some("explicit_negative_feedback"),
            Some(verdict),
            note,
            Some(version.id),
        )
        .await?;

        Ok(())
    }
}
