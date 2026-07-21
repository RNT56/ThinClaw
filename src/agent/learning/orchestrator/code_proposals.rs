use super::*;
use std::path::Path;
impl LearningOrchestrator {
    pub(in crate::agent::learning) async fn create_code_proposal(
        &self,
        event: &DbLearningEvent,
        candidate: &DbLearningCandidate,
    ) -> Result<Uuid, String> {
        let fields = build_code_proposal_fields(
            &event.source,
            &event.payload,
            candidate.id,
            candidate.summary.as_deref(),
            candidate.confidence,
        )?;

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
                if let Some(message) = rejected_code_proposal_suppression_message(
                    &fields.fingerprint,
                    &prior_fp,
                    prior.updated_at,
                    Utc::now(),
                    PROPOSAL_SUPPRESSION_WINDOW_HOURS,
                ) {
                    return Err(message);
                }
            }
        }

        let proposal = DbLearningCodeProposal {
            id: Uuid::new_v4(),
            learning_event_id: Some(event.id),
            user_id: event.user_id.clone(),
            status: "proposed".to_string(),
            title: fields.title,
            rationale: fields.rationale,
            target_files: fields.target_files,
            diff: fields.diff,
            validation_results: fields.validation_results,
            rollback_note: fields.rollback_note,
            confidence: candidate.confidence,
            branch_name: None,
            pr_url: None,
            metadata: fields.metadata,
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
            let metadata = code_proposal_review_metadata(
                &existing.metadata,
                "reject",
                note,
                Utc::now(),
                PROPOSAL_SUPPRESSION_WINDOW_HOURS,
            );
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
        let mut metadata = code_proposal_review_metadata(
            &existing.metadata,
            "approve",
            note,
            Utc::now(),
            PROPOSAL_SUPPRESSION_WINDOW_HOURS,
        );

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
                    let publish_succeeded = publish_meta
                        .get("status")
                        .and_then(|value| value.as_str())
                        .is_some_and(|status| matches!(status, "published" | "promoted"));
                    if let Some(obj) = metadata.as_object_mut() {
                        obj.insert("publish".to_string(), publish_meta);
                    }
                    if publish_succeeded {
                        final_status = "applied".to_string();
                    }
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
        const MAX_BUNDLE_DIFF_BYTES: usize = 8 * 1024 * 1024;
        const MAX_BUNDLE_JSON_BYTES: usize = 16 * 1024 * 1024;
        const MAX_BUNDLE_TITLE_BYTES: usize = 512;
        const MAX_BUNDLE_RATIONALE_BYTES: usize = 256 * 1024;
        const MAX_BUNDLE_TARGET_FILES: usize = 256;
        const MAX_BUNDLE_TARGET_BYTES: usize = 1024 * 1024;
        const MAX_BUNDLE_ROLLBACK_BYTES: usize = 256 * 1024;
        if proposal.diff.len() > MAX_BUNDLE_DIFF_BYTES || proposal.diff.contains('\0') {
            return Err("proposal bundle diff is oversized or malformed".to_string());
        }
        let target_bytes = proposal
            .target_files
            .iter()
            .try_fold(0_usize, |total, target| total.checked_add(target.len()))
            .ok_or_else(|| "proposal target-file metadata overflowed".to_string())?;
        if proposal.title.is_empty()
            || proposal.title.len() > MAX_BUNDLE_TITLE_BYTES
            || proposal.title.chars().any(char::is_control)
            || proposal.rationale.len() > MAX_BUNDLE_RATIONALE_BYTES
            || proposal.rationale.contains('\0')
            || proposal.target_files.len() > MAX_BUNDLE_TARGET_FILES
            || target_bytes > MAX_BUNDLE_TARGET_BYTES
            || proposal.target_files.iter().any(|target| {
                target.is_empty()
                    || target.len() > 4096
                    || target.contains('\0')
                    || target.chars().any(char::is_control)
            })
            || proposal
                .rollback_note
                .as_deref()
                .is_some_and(|note| note.len() > MAX_BUNDLE_ROLLBACK_BYTES || note.contains('\0'))
        {
            return Err("proposal bundle metadata is oversized or malformed".to_string());
        }
        let proposal_root = crate::platform::state_paths()
            .home
            .join("learning-proposals")
            .join(proposal.id.to_string());

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

        let package_text = serde_json::to_string_pretty(&package).map_err(|e| e.to_string())?;
        if package_text.len() > MAX_BUNDLE_JSON_BYTES {
            return Err("proposal bundle metadata exceeds its size limit".to_string());
        }

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
        let diff = proposal.diff.clone();
        tokio::task::spawn_blocking(move || {
            publish_proposal_bundle_generation(&proposal_root, &package_text, &diff, &summary)
        })
        .await
        .map_err(|error| format!("proposal bundle publisher panicked: {error}"))?
    }

    pub(in crate::agent::learning) async fn publish_proposal_in_scratch(
        &self,
        proposal: &DbLearningCodeProposal,
        publish_mode: &str,
    ) -> Result<(Option<String>, Option<String>, serde_json::Value), String> {
        const MAX_PROPOSAL_DIFF_BYTES: usize = 8 * 1024 * 1024;
        const MAX_PROPOSAL_TITLE_BYTES: usize = 512;
        const MAX_PROPOSAL_RATIONALE_BYTES: usize = 256 * 1024;
        const MAX_PROPOSAL_TARGET_FILES: usize = 256;

        let mode = validate_learning_publish_mode(publish_mode)?;
        if proposal.diff.trim().is_empty() {
            return Err("proposal diff is empty".to_string());
        }
        if proposal.diff.len() > MAX_PROPOSAL_DIFF_BYTES || proposal.diff.contains('\0') {
            return Err("proposal diff is oversized or contains a NUL byte".to_string());
        }
        if proposal.title.is_empty()
            || proposal.title.len() > MAX_PROPOSAL_TITLE_BYTES
            || proposal.title.chars().any(char::is_control)
        {
            return Err(
                "proposal title is empty, oversized, or contains control characters".to_string(),
            );
        }
        if proposal.rationale.len() > MAX_PROPOSAL_RATIONALE_BYTES
            || proposal.rationale.contains('\0')
            || proposal.target_files.len() > MAX_PROPOSAL_TARGET_FILES
        {
            return Err("proposal metadata exceeds publication limits".to_string());
        }

        let (source_origin, source_revision) =
            crate::desktop_autonomy::resolve_thinclaw_source_for_learning().await?;
        let scratch = tempfile::Builder::new()
            .prefix("thinclaw-learning-")
            .tempdir()
            .map_err(|error| format!("failed to create learning scratch directory: {error}"))?;
        let scratch_dir = scratch.path().join("repository");

        run_cmd(
            Command::new("git")
                .arg("clone")
                .arg("--no-hardlinks")
                .arg("--")
                .arg(&source_origin)
                .arg(scratch_dir.as_os_str()),
        )
        .await?;

        run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(scratch_dir.as_os_str())
                .arg("fetch")
                .arg("--force")
                .arg("origin")
                .arg(&source_revision),
        )
        .await?;
        let source_commit = run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(scratch_dir.as_os_str())
                .arg("rev-parse")
                .arg("--verify")
                .arg("FETCH_HEAD^{commit}"),
        )
        .await?
        .trim()
        .to_string();
        validate_learning_git_ref(&source_commit)?;
        run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(scratch_dir.as_os_str())
                .arg("reset")
                .arg("--hard")
                .arg(&source_commit),
        )
        .await?;

        let base_branch = run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(scratch_dir.as_os_str())
                .arg("symbolic-ref")
                .arg("--short")
                .arg("refs/remotes/origin/HEAD"),
        )
        .await
        .unwrap_or_else(|_| "origin/main".to_string())
        .trim()
        .strip_prefix("origin/")
        .unwrap_or("main")
        .to_string();
        validate_learning_git_ref(&base_branch)?;

        let patch_path = scratch.path().join("learning_proposal.patch");
        tokio::fs::write(&patch_path, &proposal.diff)
            .await
            .map_err(|e| e.to_string())?;

        run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(scratch_dir.as_os_str())
                .arg("apply")
                .arg("--check")
                .arg("--")
                .arg(patch_path.as_os_str()),
        )
        .await?;
        run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(scratch_dir.as_os_str())
                .arg("apply")
                .arg("--")
                .arg(patch_path.as_os_str()),
        )
        .await?;

        let branch_name = format!("codex/learning-proposal-{}", &proposal.id.to_string()[..8]);
        validate_learning_git_ref(&branch_name)?;
        run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(scratch_dir.as_os_str())
                .arg("checkout")
                .arg("-B")
                .arg(&branch_name)
                .arg("--"),
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
                    "mode": mode,
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
                    .arg("--")
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
            let pr_output = match pr_output {
                Ok(output) => output,
                Err(error) => {
                    return Ok((
                        Some(branch_name),
                        None,
                        serde_json::json!({
                            "status": "branch_published_pr_failed",
                            "mode": mode,
                            "base_branch": base_branch,
                            "error": error,
                        }),
                    ));
                }
            };
            let trimmed = pr_output.trim();
            if trimmed.is_empty() {
                return Ok((
                    Some(branch_name),
                    None,
                    serde_json::json!({
                        "status": "branch_published_pr_failed",
                        "mode": mode,
                        "base_branch": base_branch,
                        "error": "GitHub CLI created no pull-request URL",
                    }),
                ));
            }
            pr_url = Some(trimmed.to_string());
        }

        let retained_scratch = if mode == "bundle_only" {
            Some(scratch.keep().join("repository"))
        } else {
            None
        };
        Ok((
            Some(branch_name),
            pr_url,
            serde_json::json!({
                "status": "published",
                "mode": mode,
                "scratch_dir": retained_scratch,
                "base_branch": base_branch,
            }),
        ))
    }
}

fn publish_proposal_bundle_generation(
    proposal_root: &Path,
    package: &str,
    diff: &str,
    summary: &str,
) -> Result<PathBuf, String> {
    use fs4::FileExt as _;

    const MAX_BUNDLE_GENERATIONS: usize = 5;
    let bundle_root = proposal_root
        .parent()
        .ok_or_else(|| "proposal bundle path has no parent".to_string())?;
    std::fs::create_dir_all(bundle_root)
        .map_err(|error| format!("failed to create proposal bundle root: {error}"))?;
    ensure_real_directory(bundle_root, "proposal bundle root")?;
    match std::fs::symlink_metadata(proposal_root) {
        Ok(_) => ensure_real_directory(proposal_root, "proposal bundle directory")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(proposal_root)
                .map_err(|error| format!("failed to create proposal bundle directory: {error}"))?;
        }
        Err(error) => {
            return Err(format!(
                "failed to inspect proposal bundle directory: {error}"
            ));
        }
    }

    let lock_path = proposal_root.join(".bundle.lock");
    let mut lock_options = std::fs::OpenOptions::new();
    lock_options
        .read(true)
        .write(true)
        .create(true)
        .truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        lock_options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let lock_file = lock_options
        .open(&lock_path)
        .map_err(|error| format!("failed to open proposal bundle lock: {error}"))?;
    if !lock_file
        .metadata()
        .map_err(|error| format!("failed to inspect proposal bundle lock: {error}"))?
        .is_file()
    {
        return Err("proposal bundle lock is not a regular file".to_string());
    }
    lock_file
        .lock_exclusive()
        .map_err(|error| format!("failed to lock proposal bundle: {error}"))?;

    let generation_name = format!("generation-{}", Uuid::new_v4().simple());
    let stage = proposal_root.join(format!(".{generation_name}.tmp"));
    let generation = proposal_root.join(&generation_name);
    std::fs::create_dir(&stage)
        .map_err(|error| format!("failed to create proposal bundle staging directory: {error}"))?;
    let publish_result = (|| -> Result<(), String> {
        write_new_bundle_file(&stage.join("proposal.json"), package.as_bytes())?;
        write_new_bundle_file(&stage.join("proposal.diff"), diff.as_bytes())?;
        write_new_bundle_file(&stage.join("README.md"), summary.as_bytes())?;
        sync_directory(&stage)?;
        thinclaw_platform::rename_no_replace(&stage, &generation)
            .map_err(|error| format!("failed to publish proposal bundle generation: {error}"))?;
        sync_directory(proposal_root)?;

        let current_path = proposal_root.join("current.json");
        let current_sidecar = proposal_root.join(".current.state-sidecar");
        let current = serde_json::to_vec_pretty(&serde_json::json!({
            "version": 1,
            "generation": generation_name,
        }))
        .map_err(|error| format!("failed to serialize proposal bundle pointer: {error}"))?;
        thinclaw_platform::publish_file_pair_sync(
            &current_path,
            &current_sidecar,
            &current,
            None,
            thinclaw_platform::ExistingPairPolicy::Replace,
        )
        .map_err(|error| format!("failed to publish proposal bundle pointer: {error}"))?;
        trim_proposal_bundle_generations(proposal_root, &generation_name, MAX_BUNDLE_GENERATIONS)
    })();
    if let Err(error) = publish_result {
        if stage.starts_with(proposal_root) {
            let _ = std::fs::remove_dir_all(&stage);
        }
        return Err(error);
    }
    Ok(generation)
}

fn trim_proposal_bundle_generations(
    proposal_root: &Path,
    current_generation: &str,
    max_generations: usize,
) -> Result<(), String> {
    const MAX_DIRECTORY_ENTRIES: usize = 4096;
    let mut generations = Vec::new();
    for (index, entry) in std::fs::read_dir(proposal_root)
        .map_err(|error| format!("failed to list proposal generations: {error}"))?
        .enumerate()
    {
        if index >= MAX_DIRECTORY_ENTRIES {
            return Err("proposal bundle directory exceeds its entry limit".to_string());
        }
        let entry = entry.map_err(|error| format!("failed to inspect proposal entry: {error}"))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(id) = name.strip_prefix("generation-") else {
            continue;
        };
        if id.len() != 32 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            continue;
        }
        let metadata = std::fs::symlink_metadata(entry.path())
            .map_err(|error| format!("failed to inspect proposal generation: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "proposal generation {name} is not a real directory"
            ));
        }
        generations.push((
            metadata
                .modified()
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
            name.to_string(),
            entry.path(),
        ));
    }
    generations.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    let mut kept = 0_usize;
    let canonical_root = proposal_root
        .canonicalize()
        .map_err(|error| format!("failed to resolve proposal bundle root: {error}"))?;
    for (_, name, path) in generations {
        if name == current_generation || kept < max_generations.saturating_sub(1) {
            kept += usize::from(name != current_generation);
            continue;
        }
        let canonical_path = path
            .canonicalize()
            .map_err(|error| format!("failed to resolve old proposal generation: {error}"))?;
        if canonical_path.parent() != Some(canonical_root.as_path()) {
            return Err("proposal generation escaped its bundle root".to_string());
        }
        std::fs::remove_dir_all(&canonical_path).map_err(|error| {
            format!(
                "failed to remove old proposal generation {}: {error}",
                canonical_path.display()
            )
        })?;
    }
    sync_directory(proposal_root)
}

fn ensure_real_directory(path: &Path, label: &str) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("failed to inspect {label}: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("{label} is not a real directory"));
    }
    Ok(())
}

fn write_new_bundle_file(path: &Path, contents: &[u8]) -> Result<(), String> {
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("failed to create {}: {error}", path.display()))?;
    std::io::Write::write_all(&mut file, contents)
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

fn sync_directory(path: &Path) -> Result<(), String> {
    if let Ok(directory) = std::fs::File::open(path) {
        directory
            .sync_all()
            .map_err(|error| format!("failed to sync {}: {error}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod proposal_bundle_tests {
    use super::*;

    #[test]
    fn proposal_bundle_publishes_complete_generation_and_bounds_history() {
        let temp = tempfile::tempdir().expect("temporary proposal root");
        let proposal_root = temp.path().join("proposal");
        let mut current = PathBuf::new();
        for index in 0..7 {
            current = publish_proposal_bundle_generation(
                &proposal_root,
                &format!(r#"{{"index":{index}}}"#),
                "diff --git a/a b/a\n",
                "# Summary\n",
            )
            .expect("publish proposal generation");
        }

        assert!(current.join("proposal.json").is_file());
        assert!(current.join("proposal.diff").is_file());
        assert!(current.join("README.md").is_file());
        let pointer: serde_json::Value = serde_json::from_slice(
            &thinclaw_platform::read_regular_file_bounded(
                &proposal_root.join("current.json"),
                4096,
            )
            .expect("read current pointer"),
        )
        .expect("parse current pointer");
        assert_eq!(
            pointer
                .get("generation")
                .and_then(serde_json::Value::as_str),
            current.file_name().and_then(|name| name.to_str())
        );
        let generations = std::fs::read_dir(&proposal_root)
            .expect("list proposal root")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with("generation-"))
            })
            .count();
        assert_eq!(generations, 5);
    }

    #[cfg(unix)]
    #[test]
    fn proposal_bundle_rejects_symlink_destination() {
        let temp = tempfile::tempdir().expect("temporary proposal root");
        let outside = tempfile::tempdir().expect("outside root");
        let proposal_root = temp.path().join("proposal");
        std::os::unix::fs::symlink(outside.path(), &proposal_root)
            .expect("create hostile proposal symlink");
        let error = publish_proposal_bundle_generation(&proposal_root, "{}", "diff", "summary")
            .expect_err("symlink destination must fail");
        assert!(error.contains("not a real directory"));
        assert!(
            std::fs::read_dir(outside.path())
                .expect("list outside root")
                .next()
                .is_none()
        );
    }
}
