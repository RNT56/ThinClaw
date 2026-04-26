use super::*;
impl DesktopAutonomyManager {
    pub async fn rollback(&self) -> Result<serde_json::Value, String> {
        self.ensure_dirs().await?;
        let current = self.current_build_id();
        let manifests = self.list_build_manifests().await?;
        let Some(target) = manifests
            .iter()
            .filter(|manifest| manifest.promoted)
            .find(|manifest| current.as_deref() != Some(manifest.build_id.as_str()))
        else {
            return Err("no previous promoted build available for rollback".to_string());
        };

        let build_dir = self.builds_dir().join(&target.build_id);
        self.promote_build(&build_dir).await?;

        let mut rollout = self.load_rollout_state().await.unwrap_or_default();
        rollout.last_promoted_build_id = Some(target.build_id.clone());
        self.save_rollout_state(&rollout).await?;

        if let Some(current_build_id) = current.as_deref() {
            let _ = self.record_rollback_observation(current_build_id).await;
        }

        Ok(serde_json::json!({
            "rolled_back": true,
            "build_id": target.build_id,
            "build_dir": build_dir,
        }))
    }

    pub async fn rollout_summary(&self) -> Result<AutonomyRolloutSummary, String> {
        self.ensure_dirs().await?;
        let rollout = self.load_rollout_state().await.unwrap_or_default();
        let manifests = self.list_build_manifests().await?;
        let current_build_id = self.current_build_id();
        let last_successful_build_id = manifests
            .iter()
            .find(|manifest| manifest.promoted)
            .map(|manifest| manifest.build_id.clone());
        let rollback_target_build_id = manifests
            .iter()
            .filter(|manifest| manifest.promoted)
            .find(|manifest| current_build_id.as_deref() != Some(manifest.build_id.as_str()))
            .map(|manifest| manifest.build_id.clone());

        Ok(AutonomyRolloutSummary {
            current_build_id,
            last_successful_build_id,
            rollback_target_build_id,
            code_auto_apply_paused: rollout.code_auto_apply_paused,
            pause_reason: rollout.pause_reason,
            consecutive_failed_promotions: rollout.consecutive_failed_promotions,
            failed_canary_count: rollout.failed_canaries.len(),
            recent_builds: manifests
                .into_iter()
                .take(8)
                .map(|manifest| AutonomyRolloutEntry {
                    build_id: manifest.build_id,
                    proposal_id: manifest.proposal_id,
                    title: manifest.title,
                    created_at: manifest.created_at,
                    promoted: manifest.promoted,
                    checks: manifest.checks,
                    metadata: manifest.metadata,
                })
                .collect(),
        })
    }

    pub async fn checks_summary(&self) -> Result<AutonomyChecksSummary, String> {
        let bootstrap_report = self.load_bootstrap_report().await?;
        let latest_canary_report = self.latest_canary_report().await?;
        Ok(AutonomyChecksSummary {
            bootstrap_checks: bootstrap_report
                .as_ref()
                .map(bootstrap_report_checks)
                .unwrap_or_default(),
            latest_canary_checks: latest_canary_report
                .map(|report| report.checks)
                .unwrap_or_default(),
            permission_report: bootstrap_report
                .map(|report| report.permissions)
                .unwrap_or(serde_json::Value::Null),
        })
    }

    pub async fn evidence_summary(&self) -> Result<AutonomyEvidenceSummary, String> {
        let bootstrap_report = self.load_bootstrap_report().await?;
        let latest_canary_report = self.latest_canary_report().await?;
        let mut recent_events = Vec::new();
        let last_bootstrap_at = self.runtime_state.read().await.last_bootstrap_at;

        if let Some(report) = bootstrap_report.as_ref() {
            recent_events.push(AutonomyEventItem {
                kind: if report.passed {
                    "bootstrap_passed".to_string()
                } else {
                    "bootstrap_failed".to_string()
                },
                message: report
                    .blocking_reason
                    .clone()
                    .unwrap_or_else(|| "desktop autonomy bootstrap completed".to_string()),
                timestamp: last_bootstrap_at,
            });
            recent_events.extend(report.notes.iter().map(|note| AutonomyEventItem {
                kind: "bootstrap_note".to_string(),
                message: note.clone(),
                timestamp: last_bootstrap_at,
            }));
        }

        let manifests = self.list_build_manifests().await.unwrap_or_default();
        recent_events.extend(manifests.iter().take(5).map(|manifest| AutonomyEventItem {
            kind: if manifest.promoted {
                "rollout_promoted".to_string()
            } else {
                "rollout_candidate".to_string()
            },
            message: format!("{} ({})", manifest.title, manifest.build_id),
            timestamp: Some(manifest.created_at),
        }));

        recent_events.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        recent_events.truncate(12);

        Ok(AutonomyEvidenceSummary {
            seeded_routines: bootstrap_report
                .as_ref()
                .map(|report| report.seeded_routines.clone())
                .unwrap_or_default(),
            seeded_skills: bootstrap_report
                .as_ref()
                .map(|report| report.seeded_skills.clone())
                .unwrap_or_default(),
            latest_bootstrap_report: bootstrap_report,
            latest_canary_report,
            recent_events,
        })
    }

    pub async fn local_autorollout(
        &self,
        user_id: &str,
        proposal_id: Uuid,
        diff: &str,
        title: &str,
    ) -> Result<LocalAutorolloutOutcome, String> {
        self.ensure_dirs().await?;
        let mut rollout_state = self.load_rollout_state().await.unwrap_or_default();
        if rollout_state.code_auto_apply_paused {
            return Err(rollout_state
                .pause_reason
                .clone()
                .unwrap_or_else(|| "code auto-apply is paused".to_string()));
        }

        let managed_source = self.sync_managed_source_clone().await?;
        let build_id = format!(
            "{}-{}",
            Utc::now().format("%Y%m%d%H%M%S"),
            &proposal_id.to_string()[..8]
        );
        let build_dir = self.builds_dir().join(&build_id);
        let patch_path = build_dir.join("proposal.patch");

        run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(&managed_source)
                .arg("worktree")
                .arg("add")
                .arg("--detach")
                .arg(&build_dir)
                .arg("HEAD"),
        )
        .await?;

        tokio::fs::write(&patch_path, diff)
            .await
            .map_err(|e| format!("failed to write rollout patch: {e}"))?;

        run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(&build_dir)
                .arg("apply")
                .arg("--check")
                .arg(&patch_path),
        )
        .await?;
        run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(&build_dir)
                .arg("apply")
                .arg(&patch_path),
        )
        .await?;

        let mut checks = Vec::new();
        checks.push(
            run_command_check(
                "cargo check",
                Command::new("cargo").arg("check").current_dir(&build_dir),
            )
            .await,
        );
        checks.push(
            run_command_check(
                "cargo test desktop_autonomy",
                Command::new("cargo")
                    .arg("test")
                    .arg("desktop_autonomy")
                    .current_dir(&build_dir),
            )
            .await,
        );
        checks.push(
            run_command_check(
                "cargo build",
                Command::new("cargo").arg("build").current_dir(&build_dir),
            )
            .await,
        );
        let canary_manifest = self
            .write_canary_manifest(user_id, proposal_id, &build_id, &build_dir)
            .await?;
        let canary_report = self.run_canaries(&build_dir, &canary_manifest).await;
        let canary_report_path = canary_manifest.report_path.clone();
        checks.extend(canary_report.checks.clone());

        let all_passed = checks.iter().all(|check| check.passed);
        let mut metadata = serde_json::json!({
            "build_id": build_id,
            "user_id": user_id,
            "proposal_id": proposal_id,
            "checks": checks,
            "managed_source": managed_source,
            "canary_report_path": canary_report_path,
            "platform": self.platform_label(),
            "bridge_backend": self.bridge_backend().as_str(),
            "providers": self.provider_matrix(),
            "launcher_kind": self.provider_matrix().get("launcher").cloned().unwrap_or(serde_json::Value::Null),
            "publish_mode": "local_autorollout",
        });

        if all_passed {
            self.promote_build(&build_dir).await?;
            rollout_state.consecutive_failed_promotions = 0;
            rollout_state.last_promoted_build_id = Some(build_id.clone());
        } else {
            rollout_state.consecutive_failed_promotions += 1;
            rollout_state.failed_canaries.push(Utc::now());
        }

        trim_failed_canaries(&mut rollout_state.failed_canaries);
        if rollout_state.consecutive_failed_promotions >= 2
            || rollout_state.failed_canaries.len() >= 3
        {
            rollout_state.code_auto_apply_paused = true;
            rollout_state.pause_reason = Some(
                "code auto-rollout paused after repeated promotion/canary failures".to_string(),
            );
        }
        self.save_rollout_state(&rollout_state).await?;

        let manifest = BuildManifest {
            build_id: build_id.clone(),
            user_id: user_id.to_string(),
            proposal_id: proposal_id.to_string(),
            title: title.to_string(),
            created_at: Utc::now(),
            promoted: all_passed,
            checks: checks.clone(),
            metadata: metadata.clone(),
        };
        self.write_build_manifest(&build_id, &manifest).await?;
        if let Some(obj) = metadata.as_object_mut() {
            obj.insert("promoted".to_string(), serde_json::json!(all_passed));
            obj.insert(
                "code_auto_apply_paused".to_string(),
                serde_json::json!(rollout_state.code_auto_apply_paused),
            );
        }

        if all_passed {
            self.trim_old_builds().await?;
        }

        Ok(LocalAutorolloutOutcome {
            build_id,
            build_dir,
            promoted: all_passed,
            checks,
            publish_metadata: metadata,
        })
    }
}
