//! Skill tool: publish.

use super::*;

pub struct SkillPublishTool {
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
    remote_hub: Option<SharedRemoteSkillHub>,
    quarantine: Arc<QuarantineManager>,
    store: Option<Arc<dyn Database>>,
}

impl SkillPublishTool {
    pub fn new(
        registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
        remote_hub: Option<SharedRemoteSkillHub>,
        quarantine: Arc<QuarantineManager>,
        store: Option<Arc<dyn Database>>,
    ) -> Self {
        Self {
            registry,
            remote_hub,
            quarantine,
            store,
        }
    }
}

pub struct RootSkillPublishToolHost {
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
    remote_hub: Option<SharedRemoteSkillHub>,
    quarantine: Arc<QuarantineManager>,
    store: Option<Arc<dyn Database>>,
}

impl RootSkillPublishToolHost {
    pub fn new(
        registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
        remote_hub: Option<SharedRemoteSkillHub>,
        quarantine: Arc<QuarantineManager>,
        store: Option<Arc<dyn Database>>,
    ) -> Self {
        Self {
            registry,
            remote_hub,
            quarantine,
            store,
        }
    }
}

pub fn root_skill_publish_tool_host(
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
    remote_hub: Option<SharedRemoteSkillHub>,
    quarantine: Arc<QuarantineManager>,
    store: Option<Arc<dyn Database>>,
) -> Arc<dyn SkillPublishToolHostPort> {
    Arc::new(RootSkillPublishToolHost::new(
        registry, remote_hub, quarantine, store,
    ))
}

#[derive(Debug, Clone)]
struct PublishPlan {
    skill_name: String,
    target_repo: String,
    tap_path: String,
    package_path: String,
    branch: String,
    base_branch: Option<String>,
    package_hash: String,
    files: Vec<SkillPackageFile>,
    findings: Vec<SecurityFinding>,
    scan_report: SkillScanReport,
    target_trust_level: SkillTapTrustLevel,
    trust: String,
    source_tier: String,
    source: serde_json::Value,
}

fn publish_projection_from_plan(
    plan: &PublishPlan,
    status: &str,
) -> skill_policy::SkillPublishProjection {
    skill_policy::SkillPublishProjection {
        status: status.to_string(),
        name: plan.skill_name.clone(),
        target_repo: plan.target_repo.clone(),
        tap_path: plan.tap_path.clone(),
        package_path: plan.package_path.clone(),
        branch: plan.branch.clone(),
        base_branch: plan.base_branch.clone(),
        package_hash: plan.package_hash.clone(),
        files: package_file_json(&plan.files),
        findings: skill_finding_json(&plan.findings),
        trust: plan.trust.clone(),
        source_tier: plan.source_tier.clone(),
        source: plan.source.clone(),
        scan: Some(skill_policy::SkillPublishScanProjection {
            scanner_version: plan.scan_report.scanner_version.clone(),
            content_sha256: plan.scan_report.content_sha256.clone(),
            finding_summary: finding_summary_policy(&plan.scan_report.summary),
        }),
        remote_write_plan: None,
        metadata: None,
    }
}

fn publish_output_from_plan(plan: &PublishPlan, status: &str) -> serde_json::Value {
    skill_policy::skill_publish_projection_output(publish_projection_from_plan(plan, status))
}

async fn build_publish_plan(
    registry: &Arc<tokio::sync::RwLock<SkillRegistry>>,
    quarantine: &Arc<QuarantineManager>,
    store: Option<&Arc<dyn Database>>,
    user_id: &str,
    name: &str,
    target_repo: &str,
) -> Result<PublishPlan, ToolError> {
    skill_policy::validate_github_repo(target_repo)?;
    let (skill, source_path) = {
        let guard = registry.read().await;
        let skill = guard
            .skills()
            .iter()
            .find(|skill| skill.manifest.name.eq_ignore_ascii_case(name))
            .cloned()
            .ok_or_else(|| ToolError::ExecutionFailed(format!("Skill '{}' not found", name)))?;
        let source_path = source_path_for_skill(&skill).ok_or_else(|| {
            ToolError::ExecutionFailed(format!(
                "Skill '{}' does not have a filesystem source path",
                name
            ))
        })?;
        (skill, source_path)
    };

    let settings = if let Some(store) = store {
        load_settings_for_taps(store, user_id).await?
    } else {
        Settings::load()
    };
    let tap = settings
        .skill_taps
        .iter()
        .find(|tap| tap.repo.eq_ignore_ascii_case(target_repo))
        .cloned()
        .ok_or_else(|| {
            ToolError::ExecutionFailed(format!(
                "Target repo '{}' is not configured as a skill tap",
                target_repo
            ))
        })?;

    let files = collect_skill_package_files(&source_path)?;
    SkillRegistry::validate_skill_file(&source_path, skill.trust, skill.source.clone())
        .await
        .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
    let hash = package_hash(&files)?;
    let hash8 = hash
        .strip_prefix("sha256:")
        .unwrap_or(&hash)
        .chars()
        .take(8)
        .collect::<String>();
    skill_policy::validate_repo_path_component(&skill.manifest.name, "skill name")?;
    let tap_path = normalize_tap_path(&tap.path);
    skill_policy::validate_repo_relative_path(&tap_path, "tap.path")?;
    let package_path = if tap_path.is_empty() {
        skill.manifest.name.clone()
    } else {
        format!("{}/{}", tap_path, skill.manifest.name)
    };
    skill_policy::validate_repo_relative_path(&package_path, "package_path")?;
    let branch = format!("codex/skill-publish/{}-{}", skill.manifest.name, hash8);
    let package_files = package_scan_files(&files);
    let scan_report = scan_report_for_content(
        quarantine,
        &skill.manifest.name,
        source_path,
        SkillContent {
            raw_content: package_scan_content(&files),
            source_kind: "publish".to_string(),
            source_adapter: "publish".to_string(),
            source_ref: skill.manifest.name.clone(),
            source_repo: Some(target_repo.to_string()),
            source_url: None,
            manifest_url: None,
            manifest_digest: None,
            path: Some(package_path.clone()),
            branch: tap.branch.clone(),
            commit_sha: None,
            trust_level: tap.trust_level,
        },
        package_files,
    );
    let findings = scan_report.findings.clone();

    Ok(PublishPlan {
        skill_name: skill.manifest.name.clone(),
        target_repo: tap.repo,
        tap_path,
        package_path,
        branch,
        base_branch: tap.branch,
        package_hash: hash,
        files,
        findings,
        scan_report,
        target_trust_level: tap.trust_level,
        trust: skill.trust.to_string(),
        source_tier: skill.source_tier.to_string(),
        source: skill_source_json(&skill.source),
    })
}

async fn run_skill_publish_cmd(mut command: Command) -> Result<String, ToolError> {
    let output = command
        .output()
        .await
        .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(ToolError::ExecutionFailed(stderr.trim().to_string()))
}

async fn write_publish_package(
    scratch_dir: &Path,
    package_path: &str,
    files: &[SkillPackageFile],
) -> Result<PathBuf, ToolError> {
    let destination = scratch_dir.join(package_path);
    if tokio::fs::try_exists(&destination).await.unwrap_or(false) {
        tokio::fs::remove_dir_all(&destination)
            .await
            .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
    }
    tokio::fs::create_dir_all(&destination)
        .await
        .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;

    for file in files {
        let target = destination.join(&file.relative_path);
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
        }
        tokio::fs::copy(&file.source_path, &target)
            .await
            .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
    }

    Ok(destination)
}

async fn execute_publish_plan(plan: &PublishPlan) -> Result<serde_json::Value, ToolError> {
    let scratch_dir = std::env::temp_dir().join(format!(
        "thinclaw-skill-publish-{}-{}",
        plan.skill_name,
        plan.package_hash
            .strip_prefix("sha256:")
            .unwrap_or(&plan.package_hash)
            .chars()
            .take(8)
            .collect::<String>()
    ));
    if tokio::fs::try_exists(&scratch_dir).await.unwrap_or(false) {
        tokio::fs::remove_dir_all(&scratch_dir)
            .await
            .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
    }

    let repo_url = format!("https://github.com/{}.git", plan.target_repo);
    run_skill_publish_cmd({
        let mut command = Command::new("git");
        command
            .arg("clone")
            .arg("--no-hardlinks")
            .arg(&repo_url)
            .arg(&scratch_dir);
        command
    })
    .await?;

    let base_branch = if let Some(base_branch) = plan.base_branch.as_ref() {
        run_skill_publish_cmd({
            let mut command = Command::new("git");
            command
                .arg("-C")
                .arg(&scratch_dir)
                .arg("checkout")
                .arg(base_branch);
            command
        })
        .await?;
        base_branch.clone()
    } else {
        run_skill_publish_cmd({
            let mut command = Command::new("git");
            command
                .arg("-C")
                .arg(&scratch_dir)
                .arg("rev-parse")
                .arg("--abbrev-ref")
                .arg("HEAD");
            command
        })
        .await?
    };

    run_skill_publish_cmd({
        let mut command = Command::new("git");
        command
            .arg("-C")
            .arg(&scratch_dir)
            .arg("checkout")
            .arg("-B")
            .arg(&plan.branch);
        command
    })
    .await?;

    let package_dir = write_publish_package(&scratch_dir, &plan.package_path, &plan.files).await?;

    run_skill_publish_cmd({
        let mut command = Command::new("git");
        command
            .arg("-C")
            .arg(&scratch_dir)
            .arg("add")
            .arg(&plan.package_path);
        command
    })
    .await?;

    let diff_status = Command::new("git")
        .arg("-C")
        .arg(&scratch_dir)
        .arg("diff")
        .arg("--cached")
        .arg("--quiet")
        .output()
        .await
        .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?
        .status;
    if diff_status.success() {
        return Err(ToolError::ExecutionFailed(
            "No package changes to publish".to_string(),
        ));
    }

    run_skill_publish_cmd({
        let mut command = Command::new("git");
        command
            .arg("-C")
            .arg(&scratch_dir)
            .arg("commit")
            .arg("-m")
            .arg(format!("feat(skills): publish {}", plan.skill_name));
        command
    })
    .await?;

    run_skill_publish_cmd({
        let mut command = Command::new("git");
        command
            .arg("-C")
            .arg(&scratch_dir)
            .arg("push")
            .arg("-u")
            .arg("origin")
            .arg(&plan.branch);
        command
    })
    .await?;

    let pr_body = format!(
        "Publish ThinClaw skill `{}` to `{}`.\n\nPackage hash: `{}`\nFiles: {}",
        plan.skill_name,
        plan.package_path,
        plan.package_hash,
        plan.files.len()
    );
    let pr_url = run_skill_publish_cmd({
        let mut command = Command::new("gh");
        command
            .arg("pr")
            .arg("create")
            .arg("--draft")
            .arg("--repo")
            .arg(&plan.target_repo)
            .arg("--base")
            .arg(&base_branch)
            .arg("--head")
            .arg(&plan.branch)
            .arg("--title")
            .arg(format!("[skills] publish {}", plan.skill_name))
            .arg("--body")
            .arg(pr_body)
            .current_dir(&scratch_dir);
        command
    })
    .await?;

    let mut output = publish_output_from_plan(plan, "published");
    output["scratch_dir"] = serde_json::Value::String(scratch_dir.display().to_string());
    output["package_dir"] = serde_json::Value::String(package_dir.display().to_string());
    output["pr_url"] = serde_json::Value::String(pr_url);
    output["base_branch"] = serde_json::Value::String(base_branch);
    Ok(output)
}

fn publish_metadata_from_plan(plan: &PublishPlan) -> serde_json::Value {
    skill_policy::skill_publish_metadata_output(
        &plan.scan_report.scanner_version,
        &plan.scan_report.content_sha256,
        finding_summary_policy(&plan.scan_report.summary),
        std::iter::empty::<(&'static str, serde_json::Value)>(),
    )
}

fn publish_metadata_from_output(
    plan: &PublishPlan,
    output: &serde_json::Value,
) -> serde_json::Value {
    skill_policy::skill_publish_metadata_output(
        &plan.scan_report.scanner_version,
        &plan.scan_report.content_sha256,
        finding_summary_policy(&plan.scan_report.summary),
        ["scratch_dir", "package_dir", "pr_url", "base_branch"]
            .into_iter()
            .filter_map(|key| output.get(key).cloned().map(|value| (key, value))),
    )
}

fn publish_result_from_plan(
    plan: &PublishPlan,
    status: &str,
    metadata: serde_json::Value,
) -> ToolSkillPublishResult {
    skill_policy::skill_publish_result_output(
        status,
        &plan.skill_name,
        &plan.target_repo,
        &plan.tap_path,
        &plan.package_path,
        &plan.branch,
        plan.base_branch.clone(),
        &plan.package_hash,
        package_file_json(&plan.files),
        skill_finding_json(&plan.findings),
        &plan.trust,
        &plan.source_tier,
        plan.source.clone(),
        serde_json::Value::Null,
        metadata,
    )
}

#[async_trait]
impl SkillPublishToolHostPort for RootSkillPublishToolHost {
    async fn publish_skill(
        &self,
        request: ToolSkillPublishRequest,
    ) -> Result<ToolSkillPublishResult, ToolHostError> {
        let plan = build_publish_plan(
            &self.registry,
            &self.quarantine,
            self.store.as_ref(),
            tool_scope_user_id(&request.scope),
            &request.name,
            &request.target_repo,
        )
        .await
        .map_err(tool_host_error_from_tool)?;

        if findings_require_rejection(&plan.findings) && request.remote_write {
            return Err(ToolHostError::OperationFailed {
                reason: format!(
                    "Skill '{}' was rejected by the quarantine scanner: {}.",
                    plan.skill_name,
                    summarize_findings(&plan.findings)
                ),
            });
        }

        if !request.approve_risky
            && findings_require_approval(plan.target_trust_level, &plan.findings)
            && request.remote_write
        {
            return Err(ToolHostError::OperationFailed {
                reason: format!(
                    "Skill '{}' has audit findings: {}. Re-run with approve_risky=true to publish anyway.",
                    plan.skill_name,
                    summarize_findings(&plan.findings)
                ),
            });
        }

        if request.dry_run || !request.remote_write {
            return Ok(publish_result_from_plan(
                &plan,
                "dry_run",
                publish_metadata_from_plan(&plan),
            ));
        }

        if !request.confirm_remote_write {
            return Err(ToolHostError::OperationFailed {
                reason: "Remote write requires confirm_remote_write=true".to_string(),
            });
        }

        let output = execute_publish_plan(&plan)
            .await
            .map_err(tool_host_error_from_tool)?;

        if let Some(remote_hub) = self.remote_hub.as_ref() {
            let _ = remote_hub.is_enabled().await;
        }

        Ok(publish_result_from_plan(
            &plan,
            "published",
            publish_metadata_from_output(&plan, &output),
        ))
    }
}

#[async_trait]
impl Tool for SkillPublishTool {
    fn name(&self) -> &str {
        "skill_publish"
    }

    fn description(&self) -> &str {
        "Dry-run or publish a local skill to a configured GitHub skill tap as a draft pull request."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_policy::skill_publish_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let parsed = skill_policy::parse_skill_publish_params(&params)?;
        let name = parsed.name.as_str();
        let target_repo = parsed.target_repo;
        let dry_run = parsed.dry_run;
        let remote_write = parsed.remote_write;
        let confirm_remote_write = parsed.confirm_remote_write;
        let approve_risky = parsed.approve_risky;

        let plan = build_publish_plan(
            &self.registry,
            &self.quarantine,
            self.store.as_ref(),
            &ctx.user_id,
            name,
            &target_repo,
        )
        .await?;

        if findings_require_rejection(&plan.findings) && remote_write {
            return Err(ToolError::ExecutionFailed(format!(
                "Skill '{}' was rejected by the quarantine scanner: {}.",
                plan.skill_name,
                summarize_findings(&plan.findings)
            )));
        }

        if !approve_risky
            && findings_require_approval(plan.target_trust_level, &plan.findings)
            && remote_write
        {
            return Err(ToolError::ExecutionFailed(format!(
                "Skill '{}' has audit findings: {}. Re-run with approve_risky=true to publish anyway.",
                plan.skill_name,
                summarize_findings(&plan.findings)
            )));
        }

        let output = if dry_run || !remote_write {
            publish_output_from_plan(&plan, "dry_run")
        } else if confirm_remote_write {
            execute_publish_plan(&plan).await?
        } else {
            return Err(ToolError::ExecutionFailed(
                "Remote write requires confirm_remote_write=true".to_string(),
            ));
        };

        if let Some(remote_hub) = self.remote_hub.as_ref()
            && remote_write
            && confirm_remote_write
        {
            let _ = remote_hub.is_enabled().await;
        }

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_approval(&self, params: &serde_json::Value) -> ApprovalRequirement {
        if params
            .get("remote_write")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            ApprovalRequirement::UnlessAutoApproved
        } else {
            ApprovalRequirement::Never
        }
    }
}

// ── skill_tap_* ────────────────────────────────────────────────────────
