use super::*;
impl LearningOrchestrator {
    pub(in crate::agent::learning) async fn create_code_proposal(
        &self,
        event: &DbLearningEvent,
        candidate: &DbLearningCandidate,
    ) -> Result<Uuid, String> {
        let title = event
            .payload
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Learning-driven code proposal")
            .to_string();
        let rationale = event
            .payload
            .get("rationale")
            .and_then(|v| v.as_str())
            .or(candidate.summary.as_deref())
            .unwrap_or("Distilled from repeated failures/corrections")
            .to_string();
        let target_files = event
            .payload
            .get("target_files")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|entry| entry.as_str().map(str::to_string))
            .collect::<Vec<_>>();
        let diff = event
            .payload
            .get("diff")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if diff.trim().is_empty() {
            return Err("code proposal missing diff".to_string());
        }
        let fingerprint = proposal_fingerprint(&title, &rationale, &target_files, &diff);

        if let Ok(rejected) = self
            .store
            .list_learning_code_proposals(&event.user_id, Some("rejected"), 64)
            .await
        {
            for prior in rejected {
                let prior_fp = prior
                    .metadata
                    .get("fingerprint")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| {
                        proposal_fingerprint(
                            &prior.title,
                            &prior.rationale,
                            &prior.target_files,
                            &prior.diff,
                        )
                    });
                if prior_fp != fingerprint {
                    continue;
                }
                let age_hours = (Utc::now() - prior.updated_at).num_hours().abs();
                if age_hours <= PROPOSAL_SUPPRESSION_WINDOW_HOURS {
                    return Err(format!(
                        "similar proposal was rejected {}h ago (fingerprint={}); cooldown active",
                        age_hours, fingerprint
                    ));
                }
            }
        }

        let evidence = event
            .payload
            .get("evidence")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({ "event_payload": event.payload }));

        let proposal = DbLearningCodeProposal {
            id: Uuid::new_v4(),
            learning_event_id: Some(event.id),
            user_id: event.user_id.clone(),
            status: "proposed".to_string(),
            title: title.clone(),
            rationale: rationale.clone(),
            target_files: target_files.clone(),
            diff: diff.clone(),
            validation_results: event
                .payload
                .get("validation_results")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({"status": "not_run"})),
            rollback_note: event
                .payload
                .get("rollback_note")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            confidence: candidate.confidence,
            branch_name: None,
            pr_url: None,
            metadata: serde_json::json!({
                "candidate_id": candidate.id,
                "source": event.source,
                "fingerprint": fingerprint,
                "package": {
                    "problem_statement": title,
                    "evidence": evidence,
                    "candidate_rationale": rationale,
                    "target_files": target_files,
                    "unified_diff": diff,
                    "validation_results": event.payload.get("validation_results").cloned().unwrap_or_else(|| serde_json::json!({"status": "not_run"})),
                    "rollback_note": event.payload.get("rollback_note").cloned().unwrap_or(serde_json::Value::Null),
                    "confidence": candidate.confidence,
                },
            }),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        self.store
            .insert_learning_code_proposal(&proposal)
            .await
            .map_err(|e| e.to_string())
    }

    pub(in crate::agent::learning) async fn create_code_proposal_from_candidate(
        &self,
        candidate: &DbLearningCandidate,
    ) -> Result<Uuid, String> {
        let event = DbLearningEvent {
            id: candidate.learning_event_id.unwrap_or(candidate.id),
            user_id: candidate.user_id.clone(),
            actor_id: None,
            channel: None,
            thread_id: None,
            conversation_id: None,
            message_id: None,
            job_id: None,
            event_type: candidate.candidate_type.clone(),
            source: "outcome_backed_learning".to_string(),
            payload: candidate.proposal.clone(),
            metadata: Some(serde_json::json!({
                "source_candidate_id": candidate.id,
                "source": "outcome_backed_learning",
            })),
            created_at: candidate.created_at,
        };
        self.create_code_proposal(&event, candidate).await
    }

    pub async fn review_code_proposal(
        &self,
        user_id: &str,
        proposal_id: Uuid,
        decision: &str,
        note: Option<&str>,
    ) -> Result<Option<DbLearningCodeProposal>, String> {
        let Some(existing) = self
            .store
            .get_learning_code_proposal(user_id, proposal_id)
            .await
            .map_err(|e| e.to_string())?
        else {
            return Ok(None);
        };

        let decision_lower = decision.to_ascii_lowercase();
        if decision_lower == "reject" {
            let mut metadata = existing.metadata.clone();
            if !metadata.is_object() {
                metadata = serde_json::json!({});
            }
            if let Some(obj) = metadata.as_object_mut() {
                obj.insert(
                    "review".to_string(),
                    serde_json::json!({
                        "decision": "reject",
                        "at": Utc::now().to_rfc3339(),
                        "note": note,
                    }),
                );
                if let Some(fingerprint) = obj.get("fingerprint").cloned() {
                    obj.insert(
                        "anti_learning".to_string(),
                        serde_json::json!({
                            "fingerprint": fingerprint,
                            "suppressed_until": (Utc::now() + chrono::Duration::hours(PROPOSAL_SUPPRESSION_WINDOW_HOURS)).to_rfc3339(),
                        }),
                    );
                }
            }
            self.store
                .update_learning_code_proposal(proposal_id, "rejected", None, None, Some(&metadata))
                .await
                .map_err(|e| e.to_string())?;
            let _ = self
                .submit_feedback(
                    user_id,
                    "code_proposal",
                    &proposal_id.to_string(),
                    "dont_learn",
                    note,
                    Some(&serde_json::json!({"source": "proposal_review"})),
                )
                .await;
            if let Err(err) =
                outcomes::observe_proposal_rejection(&self.store, &existing, note).await
            {
                tracing::debug!(proposal_id = %proposal_id, error = %err, "Outcome proposal rejection hook skipped");
            }
            self.store
                .get_learning_code_proposal(user_id, proposal_id)
                .await
                .map_err(|e| e.to_string())
        } else {
            self.approve_code_proposal(user_id, proposal_id, note).await
        }
    }

    pub(in crate::agent::learning) async fn approve_code_proposal(
        &self,
        user_id: &str,
        proposal_id: Uuid,
        note: Option<&str>,
    ) -> Result<Option<DbLearningCodeProposal>, String> {
        let Some(existing) = self
            .store
            .get_learning_code_proposal(user_id, proposal_id)
            .await
            .map_err(|e| e.to_string())?
        else {
            return Ok(None);
        };

        let settings = self.load_settings_for_user(user_id).await;
        let mut metadata = existing.metadata.clone();
        if !metadata.is_object() {
            metadata = serde_json::json!({});
        }
        if let Some(obj) = metadata.as_object_mut() {
            obj.insert(
                "review".to_string(),
                serde_json::json!({
                    "decision": "approve",
                    "at": Utc::now().to_rfc3339(),
                    "note": note,
                }),
            );
        }

        match self.write_proposal_bundle(&existing).await {
            Ok(bundle_dir) => {
                if let Some(obj) = metadata.as_object_mut() {
                    obj.insert(
                        "bundle".to_string(),
                        serde_json::json!({
                            "status": "written",
                            "path": bundle_dir.to_string_lossy().to_string(),
                        }),
                    );
                }
            }
            Err(err) => {
                if let Some(obj) = metadata.as_object_mut() {
                    obj.insert(
                        "bundle".to_string(),
                        serde_json::json!({
                            "status": "failed",
                            "error": err,
                        }),
                    );
                }
            }
        }

        let mut final_status = "approved".to_string();
        let mut branch_name: Option<String> = None;
        let mut pr_url: Option<String> = None;

        if settings.code_proposals.enabled {
            match self
                .publish_proposal_in_scratch(&existing, &settings.code_proposals.publish_mode)
                .await
            {
                Ok((branch, pr, publish_meta)) => {
                    branch_name = branch;
                    pr_url = pr;
                    if let Some(obj) = metadata.as_object_mut() {
                        obj.insert("publish".to_string(), publish_meta);
                    }
                    final_status = "applied".to_string();
                }
                Err(err) => {
                    if let Some(obj) = metadata.as_object_mut() {
                        obj.insert(
                            "publish".to_string(),
                            serde_json::json!({"status": "failed", "error": err}),
                        );
                    }
                }
            }
        }

        self.store
            .update_learning_code_proposal(
                proposal_id,
                &final_status,
                branch_name.as_deref(),
                pr_url.as_deref(),
                Some(&metadata),
            )
            .await
            .map_err(|e| e.to_string())?;
        if matches!(final_status.as_str(), "approved" | "applied")
            && let Some(updated) = self
                .store
                .get_learning_code_proposal(user_id, proposal_id)
                .await
                .map_err(|e| e.to_string())?
            && let Err(err) = outcomes::maybe_create_proposal_contract(&self.store, &updated).await
        {
            tracing::debug!(proposal_id = %proposal_id, error = %err, "Outcome proposal durability hook skipped");
        }

        self.store
            .get_learning_code_proposal(user_id, proposal_id)
            .await
            .map_err(|e| e.to_string())
    }

    pub(in crate::agent::learning) async fn write_proposal_bundle(
        &self,
        proposal: &DbLearningCodeProposal,
    ) -> Result<PathBuf, String> {
        let repo_root = std::env::current_dir().map_err(|e| e.to_string())?;
        let bundle_dir = repo_root
            .join(".thinclaw")
            .join("learning-proposals")
            .join(proposal.id.to_string());
        tokio::fs::create_dir_all(&bundle_dir)
            .await
            .map_err(|e| e.to_string())?;

        let package = serde_json::json!({
            "proposal_id": proposal.id,
            "problem_statement": proposal.title,
            "evidence": proposal.metadata.get("package").and_then(|v| v.get("evidence")).cloned().unwrap_or(serde_json::json!({})),
            "candidate_rationale": proposal.rationale,
            "target_files": proposal.target_files,
            "unified_diff": proposal.diff,
            "validation_results": proposal.validation_results,
            "rollback_note": proposal.rollback_note,
            "confidence": proposal.confidence,
            "status": proposal.status,
            "created_at": proposal.created_at,
            "updated_at": proposal.updated_at,
        });

        let package_path = bundle_dir.join("proposal.json");
        let diff_path = bundle_dir.join("proposal.diff");
        let summary_path = bundle_dir.join("README.md");

        let package_text = serde_json::to_string_pretty(&package).map_err(|e| e.to_string())?;
        tokio::fs::write(&package_path, package_text)
            .await
            .map_err(|e| e.to_string())?;
        tokio::fs::write(&diff_path, &proposal.diff)
            .await
            .map_err(|e| e.to_string())?;

        let summary = format!(
            "# Learning Proposal {}\n\n- Status: {}\n- Title: {}\n- Confidence: {}\n- Files: {}\n",
            proposal.id,
            proposal.status,
            proposal.title,
            proposal
                .confidence
                .map(|v| format!("{v:.2}"))
                .unwrap_or_else(|| "-".to_string()),
            if proposal.target_files.is_empty() {
                "-".to_string()
            } else {
                proposal.target_files.join(", ")
            }
        );
        tokio::fs::write(summary_path, summary)
            .await
            .map_err(|e| e.to_string())?;

        Ok(bundle_dir)
    }

    pub(in crate::agent::learning) async fn publish_proposal_in_scratch(
        &self,
        proposal: &DbLearningCodeProposal,
        publish_mode: &str,
    ) -> Result<(Option<String>, Option<String>, serde_json::Value), String> {
        if proposal.diff.trim().is_empty() {
            return Err("proposal diff is empty".to_string());
        }

        let repo_root = std::env::current_dir().map_err(|e| e.to_string())?;
        let scratch_dir = std::env::temp_dir().join(format!(
            "thinclaw-learning-{}",
            proposal.id.to_string().replace('-', "")
        ));
        if scratch_dir.exists() {
            let _ = tokio::fs::remove_dir_all(&scratch_dir).await;
        }

        run_cmd(
            Command::new("git")
                .arg("clone")
                .arg("--no-hardlinks")
                .arg(repo_root.as_os_str())
                .arg(scratch_dir.as_os_str()),
        )
        .await?;

        let base_branch = run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(scratch_dir.as_os_str())
                .arg("rev-parse")
                .arg("--abbrev-ref")
                .arg("HEAD"),
        )
        .await?
        .trim()
        .to_string();

        let patch_path = scratch_dir.join("learning_proposal.patch");
        tokio::fs::write(&patch_path, &proposal.diff)
            .await
            .map_err(|e| e.to_string())?;

        run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(scratch_dir.as_os_str())
                .arg("apply")
                .arg("--check")
                .arg(patch_path.as_os_str()),
        )
        .await?;
        run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(scratch_dir.as_os_str())
                .arg("apply")
                .arg(patch_path.as_os_str()),
        )
        .await?;

        let branch_name = format!("codex/learning-proposal-{}", &proposal.id.to_string()[..8]);
        run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(scratch_dir.as_os_str())
                .arg("checkout")
                .arg("-B")
                .arg(&branch_name),
        )
        .await?;
        run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(scratch_dir.as_os_str())
                .arg("add")
                .arg("-A"),
        )
        .await?;

        let commit_message = format!(
            "feat(learning): apply proposal {}",
            &proposal.id.to_string()[..8]
        );
        run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(scratch_dir.as_os_str())
                .arg("commit")
                .arg("-m")
                .arg(commit_message),
        )
        .await?;

        let mode = publish_mode.to_ascii_lowercase();
        if mode == "local_autorollout" {
            let manager = crate::desktop_autonomy::desktop_autonomy_manager().ok_or_else(|| {
                "local_autorollout requires an active desktop autonomy manager".to_string()
            })?;
            let outcome = manager
                .local_autorollout(
                    &proposal.user_id,
                    proposal.id,
                    &proposal.diff,
                    &proposal.title,
                )
                .await?;

            let candidate_id = proposal
                .metadata
                .get("candidate_id")
                .and_then(|value| value.as_str())
                .and_then(|value| Uuid::parse_str(value).ok());
            let version = DbLearningArtifactVersion {
                id: Uuid::new_v4(),
                candidate_id,
                user_id: proposal.user_id.clone(),
                artifact_type: "code".to_string(),
                artifact_name: outcome.build_id.clone(),
                version_label: Some(outcome.build_id.clone()),
                status: if outcome.promoted {
                    "promoted".to_string()
                } else {
                    "failed".to_string()
                },
                diff_summary: Some(proposal.title.clone()),
                before_content: None,
                after_content: Some(proposal.diff.clone()),
                provenance: serde_json::json!({
                    "publish_mode": "local_autorollout",
                    "proposal_id": proposal.id,
                    "checks": outcome.checks,
                    "metadata": outcome.publish_metadata,
                    "build_dir": outcome.build_dir,
                    "build_id": outcome.build_id,
                    "canary_report_path": outcome.publish_metadata.get("canary_report_path").cloned(),
                    "platform": outcome.publish_metadata.get("platform").cloned(),
                    "bridge_backend": outcome.publish_metadata.get("bridge_backend").cloned(),
                    "providers": outcome.publish_metadata.get("providers").cloned(),
                    "launcher_kind": outcome.publish_metadata.get("launcher_kind").cloned(),
                    "promoted_at": if outcome.promoted { Some(Utc::now()) } else { None },
                    "actor_id": proposal.metadata.get("actor_id").cloned(),
                    "thread_id": proposal.metadata.get("thread_id").cloned(),
                }),
                created_at: Utc::now(),
            };
            let inserted = self.store.insert_learning_artifact_version(&version).await;
            if inserted.is_ok()
                && outcome.promoted
                && let Err(err) =
                    outcomes::maybe_create_artifact_contract(&self.store, &version).await
            {
                tracing::debug!(error = %err, "Outcome promoted code artifact hook skipped");
            }

            return Ok((
                Some(format!("local_autorollout/{}", outcome.build_id)),
                None,
                serde_json::json!({
                    "status": if outcome.promoted { "promoted" } else { "failed" },
                    "mode": publish_mode,
                    "build_id": outcome.build_id,
                    "build_dir": outcome.build_dir,
                    "checks": outcome.checks,
                    "metadata": outcome.publish_metadata,
                }),
            ));
        }

        let mut pr_url: Option<String> = None;

        if mode != "bundle_only" {
            run_cmd(
                Command::new("git")
                    .arg("-C")
                    .arg(scratch_dir.as_os_str())
                    .arg("push")
                    .arg("-u")
                    .arg("origin")
                    .arg(&branch_name),
            )
            .await?;
        }

        if mode == "branch_pr_draft" {
            let pr_body = format!(
                "Problem:\n{}\n\nRationale:\n{}\n\nGenerated by ThinClaw learning proposal {}.",
                proposal.title, proposal.rationale, proposal.id
            );
            let pr_title = format!("[learning] {}", proposal.title);
            let pr_output = run_cmd(
                Command::new("gh")
                    .arg("pr")
                    .arg("create")
                    .arg("--draft")
                    .arg("--base")
                    .arg(&base_branch)
                    .arg("--head")
                    .arg(&branch_name)
                    .arg("--title")
                    .arg(pr_title)
                    .arg("--body")
                    .arg(pr_body)
                    .current_dir(&scratch_dir),
            )
            .await;
            if let Ok(url) = pr_output {
                let trimmed = url.trim();
                if !trimmed.is_empty() {
                    pr_url = Some(trimmed.to_string());
                }
            }
        }

        Ok((
            Some(branch_name),
            pr_url,
            serde_json::json!({
                "status": "published",
                "mode": publish_mode,
                "scratch_dir": scratch_dir,
                "base_branch": base_branch,
            }),
        ))
    }
}
