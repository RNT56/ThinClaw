//! Skill tool: install.

use super::*;

pub struct SkillInstallTool {
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
    catalog: Arc<SkillCatalog>,
    remote_hub: Option<SharedRemoteSkillHub>,
    quarantine: Arc<QuarantineManager>,
}

impl SkillInstallTool {
    pub fn new(
        registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
        catalog: Arc<SkillCatalog>,
        remote_hub: Option<SharedRemoteSkillHub>,
        quarantine: Arc<QuarantineManager>,
    ) -> Self {
        Self {
            registry,
            catalog,
            remote_hub,
            quarantine,
        }
    }

    async fn resolve_external_content(
        &self,
        name: &str,
        params: &serde_json::Value,
    ) -> Result<Option<SkillContent>, ToolError> {
        if params.get("content").and_then(|v| v.as_str()).is_some() {
            return Ok(None);
        }

        if let Some(url) = params.get("url").and_then(|v| v.as_str()) {
            let content = fetch_skill_content(url).await?;
            return Ok(Some(SkillContent {
                raw_content: content,
                source_kind: "url".to_string(),
                source_adapter: "url".to_string(),
                source_ref: url.to_string(),
                source_repo: None,
                source_url: Some(url.to_string()),
                manifest_url: Some(url.to_string()),
                manifest_digest: None,
                path: None,
                branch: None,
                commit_sha: None,
                trust_level: SkillTapTrustLevel::Community,
            }));
        }

        if let Some(ref hub) = self.remote_hub
            && let Some(remote) = hub.resolve_skill(name).await
        {
            return hub
                .download_skill(&remote)
                .await
                .map(Some)
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()));
        }

        let download_url =
            crate::skills::catalog::skill_download_url(self.catalog.registry_url(), name)
                .map_err(ToolError::ExecutionFailed)?;
        let content = fetch_skill_content(&download_url).await?;
        Ok(Some(SkillContent {
            raw_content: content,
            source_kind: "clawhub_catalog".to_string(),
            source_adapter: "clawhub_catalog".to_string(),
            source_ref: name.to_string(),
            source_repo: None,
            source_url: Some(download_url.clone()),
            manifest_url: Some(download_url),
            manifest_digest: None,
            path: None,
            branch: None,
            commit_sha: None,
            trust_level: SkillTapTrustLevel::Community,
        }))
    }
}

#[async_trait]
impl Tool for SkillInstallTool {
    fn name(&self) -> &str {
        "skill_install"
    }

    fn description(&self) -> &str {
        "Install a skill from SKILL.md content, a URL, a configured GitHub skill tap, or by name from the ClawHub catalog. Externally sourced skills are quarantined and scanned before install."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_policy::skill_install_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let parsed = skill_policy::parse_skill_install_params(&params)?;
        let name = parsed.name.as_str();
        let force = parsed.force;
        let approve_risky = parsed.approve_risky;

        let external_content = self.resolve_external_content(name, &params).await?;
        let content = if let Some(raw) = params.get("content").and_then(|v| v.as_str()) {
            raw.to_string()
        } else if let Some(ref remote) = external_content {
            remote.raw_content.clone()
        } else {
            return Err(ToolError::ExecutionFailed(
                "No skill content available for installation".to_string(),
            ));
        };

        // Parse to extract the name (cheap, in-memory).
        let normalized = crate::skills::normalize_line_endings(&content);
        let parsed = crate::skills::parser::parse_skill_md(&normalized)
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
        let skill_name_from_parse = parsed.manifest.name.clone();

        // Check for duplicates and get install_dir under a brief read lock.
        let user_dir = {
            let guard = self.registry.read().await;

            if guard.has(&skill_name_from_parse) && !force {
                return Err(ToolError::ExecutionFailed(format!(
                    "Skill '{}' already exists (use force=true to update)",
                    skill_name_from_parse
                )));
            }

            guard.install_target_dir().to_path_buf()
        };

        // ── Force-update: remove old version first ─────────────────────
        if force {
            let mut guard = self.registry.write().await;
            if guard.has(&skill_name_from_parse)
                && let Ok(path) = guard.validate_remove(&skill_name_from_parse)
            {
                let _ = crate::skills::registry::SkillRegistry::delete_skill_files(
                    &path,
                    &skill_name_from_parse,
                )
                .await;
                let _ = guard.commit_remove(&skill_name_from_parse);
                tracing::info!(
                    skill = %skill_name_from_parse,
                    "Force-update: removed previous version"
                );
            }
        }

        let (skill_name, loaded_skill, scan_report) = if let Some(remote) = external_content {
            let quarantined = self
                .quarantine
                .quarantine_skill(&skill_name_from_parse, &remote)
                .await
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
            let scan_report = self.quarantine.scan_report(&quarantined);

            if findings_require_rejection(&scan_report.findings) {
                self.quarantine.cleanup(&quarantined).await;
                return Err(ToolError::ExecutionFailed(format!(
                    "Skill '{}' was rejected by the quarantine scanner: {}.",
                    skill_name_from_parse,
                    summarize_findings(&scan_report.findings)
                )));
            }

            if findings_require_approval(remote.trust_level, &scan_report.findings)
                && !approve_risky
            {
                self.quarantine.cleanup(&quarantined).await;
                return Err(ToolError::ExecutionFailed(format!(
                    "Skill '{}' was quarantined with findings: {}. Re-run with approve_risky=true to install anyway.",
                    skill_name_from_parse,
                    summarize_findings(&scan_report.findings)
                )));
            }

            let installed_dir = self
                .quarantine
                .approve_and_install(&quarantined, &user_dir, &scan_report.findings)
                .await
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
            self.quarantine.cleanup(&quarantined).await;

            let source = SkillSource::User(installed_dir.clone());
            let loaded = crate::skills::registry::SkillRegistry::load_skill_from_path(
                &installed_dir,
                SkillTrust::Installed,
                source,
            )
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
            (loaded.0, loaded.1, scan_report)
        } else {
            let scan_report = scan_report_for_content(
                &self.quarantine,
                &skill_name_from_parse,
                user_dir.join(&skill_name_from_parse),
                skill_content_for_scan(content.clone(), "content", "(inline content)"),
                Vec::new(),
            );
            if findings_require_rejection(&scan_report.findings) {
                return Err(ToolError::ExecutionFailed(format!(
                    "Skill '{}' was rejected by the quarantine scanner: {}.",
                    skill_name_from_parse,
                    summarize_findings(&scan_report.findings)
                )));
            }
            if findings_require_approval(SkillTapTrustLevel::Community, &scan_report.findings)
                && !approve_risky
            {
                return Err(ToolError::ExecutionFailed(format!(
                    "Skill '{}' has findings: {}. Re-run with approve_risky=true to install anyway.",
                    skill_name_from_parse,
                    summarize_findings(&scan_report.findings)
                )));
            }
            let loaded = crate::skills::registry::SkillRegistry::prepare_install_to_disk(
                &user_dir,
                &skill_name_from_parse,
                &normalized,
            )
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
            (loaded.0, loaded.1, scan_report)
        };

        // Commit the in-memory addition under a brief write lock.
        // On failure, clean up the orphaned disk files from prepare_install_to_disk.
        let installed_name = {
            let mut guard = self.registry.write().await;
            match guard.commit_install(&skill_name, loaded_skill) {
                Ok(()) => skill_name,
                Err(e) => {
                    // ── TOCTOU cleanup ──────────────────────────────────
                    // Another concurrent call installed the same skill between
                    // prepare_install and commit_install. Clean up orphaned files.
                    let orphan_dir = user_dir.join(&skill_name);
                    if orphan_dir.exists() {
                        tracing::warn!(
                            skill = %skill_name,
                            "Cleaning up orphaned skill files after failed commit"
                        );
                        let _ = crate::skills::registry::SkillRegistry::delete_skill_files(
                            &orphan_dir,
                            &skill_name,
                        )
                        .await;
                    }
                    return Err(ToolError::ExecutionFailed(e.to_string()));
                }
            }
        };

        let output = skill_policy::skill_install_output(
            &installed_name,
            force,
            skill_finding_json(&scan_report.findings),
        );
        let mut output = output;
        add_scan_report_fields(&mut output, &scan_report);

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}
