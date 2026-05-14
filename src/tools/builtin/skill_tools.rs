//! Agent-callable tools for managing skills (prompt-level extensions).
//!
//! Five tools for discovering, reading, installing, listing, and removing skills
//! entirely through conversation, following the extension_tools pattern.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use tokio::process::Command;

use crate::context::JobContext;
use crate::db::Database;
use crate::settings::{Settings, SkillTapConfig, SkillTapTrustLevel};
use crate::skills::catalog::SkillCatalog;
use crate::skills::quarantine::{
    FindingSeverity, FindingSummary, QuarantineManager, QuarantinedSkill, SecurityFinding,
    SkillContent, SkillProvenance, SkillScanFile, SkillScanReport,
};
use crate::skills::registry::SkillRegistry;
use crate::skills::{SharedRemoteSkillHub, SkillSource, SkillTrust};
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput};
use thinclaw_tools::builtin::skill as skill_policy;
use thinclaw_tools::builtin::skill::{
    SkillPackageFile, collect_skill_package_files, package_file_json, package_hash,
    package_scan_content,
};

fn restricted_skill_names(ctx: &JobContext) -> Option<std::collections::HashSet<String>> {
    skill_policy::restricted_skill_names(&ctx.metadata)
}

fn ensure_skill_allowed(ctx: &JobContext, skill_name: &str) -> Result<(), ToolError> {
    skill_policy::ensure_skill_allowed(&ctx.metadata, skill_name)
}

fn ensure_skill_admin_available(ctx: &JobContext, tool_name: &str) -> Result<(), ToolError> {
    skill_policy::ensure_skill_admin_available(&ctx.metadata, tool_name)
}

fn summarize_findings(findings: &[SecurityFinding]) -> String {
    skill_policy::skill_findings_summary(findings.iter().map(|finding| {
        skill_policy::skill_finding_summary(
            &finding.kind,
            &format!("{:?}", finding.severity).to_lowercase(),
            &finding.excerpt,
        )
    }))
}

fn skill_finding_json(findings: &[SecurityFinding]) -> Vec<serde_json::Value> {
    findings
        .iter()
        .map(|finding| {
            let mut value = skill_policy::skill_finding_output(
                &finding.kind,
                &format!("{:?}", finding.severity).to_lowercase(),
                &finding.excerpt,
            );
            if let Some(obj) = value.as_object_mut() {
                if let Some(rule_id) = finding.rule_id.as_ref() {
                    obj.insert(
                        "rule_id".to_string(),
                        serde_json::Value::String(rule_id.clone()),
                    );
                }
                if let Some(file) = finding.file.as_ref() {
                    obj.insert("file".to_string(), serde_json::Value::String(file.clone()));
                }
                if let Some(line) = finding.line {
                    obj.insert("line".to_string(), serde_json::json!(line));
                }
                if let Some(recommendation) = finding.recommendation.as_ref() {
                    obj.insert(
                        "recommendation".to_string(),
                        serde_json::Value::String(recommendation.clone()),
                    );
                }
                if let Some(scanner_version) = finding.scanner_version.as_ref() {
                    obj.insert(
                        "scanner_version".to_string(),
                        serde_json::Value::String(scanner_version.clone()),
                    );
                }
            }
            value
        })
        .collect()
}

fn finding_summary_json(summary: &FindingSummary) -> serde_json::Value {
    serde_json::json!({
        "total": summary.total,
        "warnings": summary.warnings,
        "critical": summary.critical,
        "categories": summary.categories.clone(),
    })
}

fn add_scan_report_fields(output: &mut serde_json::Value, report: &SkillScanReport) {
    if let Some(obj) = output.as_object_mut() {
        obj.insert(
            "scanner_version".to_string(),
            serde_json::Value::String(report.scanner_version.clone()),
        );
        obj.insert(
            "content_sha256".to_string(),
            serde_json::Value::String(report.content_sha256.clone()),
        );
        obj.insert(
            "finding_summary".to_string(),
            finding_summary_json(&report.summary),
        );
    }
}

fn findings_require_approval(
    trust_level: SkillTapTrustLevel,
    findings: &[SecurityFinding],
) -> bool {
    let critical = findings
        .iter()
        .filter(|finding| finding.severity == FindingSeverity::Critical)
        .count();
    let warnings = findings
        .iter()
        .filter(|finding| finding.severity == FindingSeverity::Warning)
        .count();
    match trust_level {
        SkillTapTrustLevel::Community => critical > 0 || warnings > 1,
        SkillTapTrustLevel::Trusted | SkillTapTrustLevel::Builtin => critical > 0,
    }
}

fn findings_require_rejection(findings: &[SecurityFinding]) -> bool {
    findings.iter().any(|finding| {
        finding.severity == FindingSeverity::Critical && finding.kind == "path_traversal"
    })
}

fn source_path_for_skill(skill: &crate::skills::LoadedSkill) -> Option<PathBuf> {
    match &skill.source {
        SkillSource::Workspace(path)
        | SkillSource::User(path)
        | SkillSource::Bundled(path)
        | SkillSource::External(path) => Some(path.clone()),
    }
}

fn skill_content_for_scan(
    raw_content: String,
    source_kind: &str,
    source_ref: &str,
) -> SkillContent {
    SkillContent {
        raw_content,
        source_kind: source_kind.to_string(),
        source_adapter: source_kind.to_string(),
        source_ref: source_ref.to_string(),
        source_repo: None,
        source_url: (source_kind == "url").then(|| source_ref.to_string()),
        manifest_url: (source_kind == "url").then(|| source_ref.to_string()),
        manifest_digest: None,
        path: (source_kind == "path").then(|| source_ref.to_string()),
        branch: None,
        commit_sha: None,
        trust_level: SkillTapTrustLevel::Community,
    }
}

async fn read_skill_provenance(skill_dir: &Path) -> Result<SkillProvenance, ToolError> {
    let raw = tokio::fs::read_to_string(skill_dir.join(".thinclaw-skill-lock.json"))
        .await
        .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
    serde_json::from_str(&raw).map_err(|err| ToolError::ExecutionFailed(err.to_string()))
}

fn package_scan_files(files: &[SkillPackageFile]) -> Vec<SkillScanFile> {
    files
        .iter()
        .filter_map(|file| {
            std::fs::read(&file.source_path)
                .ok()
                .map(|bytes| SkillScanFile {
                    relative_path: file.relative_path.clone(),
                    content: String::from_utf8_lossy(&bytes).into_owned(),
                })
        })
        .collect()
}

fn scan_files_for_source_path(path: &Path) -> Vec<SkillScanFile> {
    collect_skill_package_files(path)
        .map(|files| package_scan_files(&files))
        .unwrap_or_default()
}

fn scan_report_for_content(
    quarantine: &QuarantineManager,
    skill_name: &str,
    dir: PathBuf,
    content: SkillContent,
    package_files: Vec<SkillScanFile>,
) -> SkillScanReport {
    quarantine.scan_report(&QuarantinedSkill {
        skill_name: skill_name.to_string(),
        dir,
        content,
        package_files,
    })
}

fn skill_source_json(source: &SkillSource) -> serde_json::Value {
    match source {
        SkillSource::Workspace(path) => {
            skill_policy::skill_source_output("workspace", &path.display().to_string())
        }
        SkillSource::User(path) => {
            skill_policy::skill_source_output("user", &path.display().to_string())
        }
        SkillSource::Bundled(path) => {
            skill_policy::skill_source_output("bundled", &path.display().to_string())
        }
        SkillSource::External(path) => {
            skill_policy::skill_source_output("external", &path.display().to_string())
        }
    }
}

pub async fn inspect_skill_report(
    registry: &Arc<tokio::sync::RwLock<SkillRegistry>>,
    quarantine: &Arc<QuarantineManager>,
    name: &str,
    include_content: bool,
    include_files: bool,
    audit: bool,
) -> Result<serde_json::Value, ToolError> {
    let skill = {
        let guard = registry.read().await;
        guard
            .skills()
            .iter()
            .find(|skill| skill.manifest.name.eq_ignore_ascii_case(name))
            .cloned()
            .ok_or_else(|| ToolError::ExecutionFailed(format!("Skill '{}' not found", name)))?
    };
    let source_path = source_path_for_skill(&skill);

    let provenance = if let Some(path) = source_path.as_ref() {
        read_skill_provenance(path)
            .await
            .ok()
            .and_then(|p| serde_json::to_value(p).ok())
    } else {
        None
    };

    let files = if include_files {
        if let Some(path) = source_path.as_ref() {
            match collect_skill_package_files(path) {
                Ok(files) => package_file_json(&files),
                Err(err) => vec![skill_policy::skill_inventory_error_output(&err.to_string())],
            }
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    let scan_report = if audit {
        let scan_root = source_path.clone().unwrap_or_else(|| PathBuf::from("."));
        let package_files = source_path
            .as_ref()
            .map(|path| scan_files_for_source_path(path))
            .unwrap_or_default();
        Some(scan_report_for_content(
            quarantine,
            &skill.manifest.name,
            scan_root,
            SkillContent {
                raw_content: skill.prompt_content.clone(),
                source_kind: "inspect".to_string(),
                source_adapter: "inspect".to_string(),
                source_ref: skill.manifest.name.clone(),
                source_repo: None,
                source_url: None,
                manifest_url: None,
                manifest_digest: None,
                path: None,
                branch: None,
                commit_sha: None,
                trust_level: SkillTapTrustLevel::Community,
            },
            package_files,
        ))
    } else {
        None
    };

    let findings = scan_report
        .as_ref()
        .map(|report| report.findings.as_slice())
        .unwrap_or(&[]);
    let mut output = skill_policy::skill_inspect_output(
        &skill.manifest.name,
        &skill.manifest.version,
        &skill.manifest.description,
        serde_json::to_value(&skill.manifest.activation)
            .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?,
        serde_json::to_value(&skill.manifest.metadata)
            .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?,
        &skill.trust.to_string(),
        &skill.source_tier.to_string(),
        skill_source_json(&skill.source),
        &skill.content_hash,
        (skill.prompt_content.len() as f64 * 0.25) as usize,
        provenance,
        skill_finding_json(&findings),
        files,
        include_content.then_some(skill.prompt_content.as_str()),
    );
    if let Some(report) = scan_report.as_ref() {
        add_scan_report_fields(&mut output, report);
    }
    Ok(output)
}

fn normalize_tap_path(path: &str) -> String {
    skill_policy::normalize_tap_path(path)
}

fn validate_github_repo(repo: &str) -> Result<(), ToolError> {
    skill_policy::validate_github_repo(repo)
}

fn validate_repo_relative_path(path: &str, field: &str) -> Result<(), ToolError> {
    skill_policy::validate_repo_relative_path(path, field)
}

fn validate_repo_path_component(value: &str, field: &str) -> Result<(), ToolError> {
    skill_policy::validate_repo_path_component(value, field)
}

fn parse_tap_trust_level(value: &str) -> Result<SkillTapTrustLevel, ToolError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "builtin" => Ok(SkillTapTrustLevel::Builtin),
        "trusted" => Ok(SkillTapTrustLevel::Trusted),
        "community" | "" => Ok(SkillTapTrustLevel::Community),
        other => Err(ToolError::InvalidParameters(format!(
            "Unsupported trust_level '{}'",
            other
        ))),
    }
}

fn tap_key_matches(tap: &SkillTapConfig, repo: &str, path: &str, branch: Option<&str>) -> bool {
    skill_policy::skill_tap_key_matches(
        &tap.repo,
        &tap.path,
        tap.branch.as_deref(),
        repo,
        path,
        branch,
    )
}

async fn load_settings_for_taps(
    store: &Arc<dyn Database>,
    user_id: &str,
) -> Result<Settings, ToolError> {
    let map = store
        .get_all_settings(user_id)
        .await
        .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
    Ok(Settings::from_db_map(&map))
}

async fn persist_skill_taps(
    store: &Arc<dyn Database>,
    user_id: &str,
    taps: &[SkillTapConfig],
) -> Result<(), ToolError> {
    let value =
        serde_json::to_value(taps).map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
    store
        .set_setting(user_id, "skill_taps", &value)
        .await
        .map_err(|err| ToolError::ExecutionFailed(err.to_string()))
}

async fn refresh_remote_hub_from_settings(
    store: &Arc<dyn Database>,
    user_id: &str,
    remote_hub: &SharedRemoteSkillHub,
) -> Result<usize, ToolError> {
    let settings = load_settings_for_taps(store, user_id).await?;
    let tap_count = settings.skill_taps.len();
    let hub = crate::skills::build_remote_skill_hub(
        settings.skill_taps,
        settings.well_known_skill_registries,
    );
    remote_hub.replace(hub).await;
    Ok(tap_count)
}

// ── skill_inspect ───────────────────────────────────────────────────────

pub struct SkillInspectTool {
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
    quarantine: Arc<QuarantineManager>,
}

impl SkillInspectTool {
    pub fn new(
        registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
        quarantine: Arc<QuarantineManager>,
    ) -> Self {
        Self {
            registry,
            quarantine,
        }
    }
}

#[async_trait]
impl Tool for SkillInspectTool {
    fn name(&self) -> &str {
        "skill_inspect"
    }

    fn description(&self) -> &str {
        "Inspect one loaded skill with metadata, provenance, files, and optional audit findings."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_policy::skill_inspect_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let parsed = skill_policy::parse_skill_inspect_params(&params)?;
        ensure_skill_allowed(ctx, &parsed.name)?;

        let output = inspect_skill_report(
            &self.registry,
            &self.quarantine,
            &parsed.name,
            parsed.include_content,
            parsed.include_files,
            parsed.audit,
        )
        .await?;
        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }
}

// ── skill_read ──────────────────────────────────────────────────────────

/// Read a loaded skill's full prompt content on demand.
///
/// This enables lazy skill loading: the system prompt announces which skills
/// are active (name + description only), and the agent calls `skill_read`
/// to get the full instructions when it needs them.
pub struct SkillReadTool {
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
}

impl SkillReadTool {
    pub fn new(registry: Arc<tokio::sync::RwLock<SkillRegistry>>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for SkillReadTool {
    fn name(&self) -> &str {
        "skill_read"
    }

    fn description(&self) -> &str {
        "Read a skill's full instructions by name. Use when you need detailed guidance for a specific skill."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_policy::skill_read_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let name = skill_policy::parse_skill_name_param(&params)?;
        ensure_skill_allowed(ctx, &name)?;

        let guard = self.registry.read().await;
        let skill = guard
            .skills()
            .iter()
            .find(|s| s.manifest.name.eq_ignore_ascii_case(&name));

        match skill {
            Some(s) => {
                let output = skill_policy::skill_read_output(
                    &s.manifest.name,
                    &s.manifest.version,
                    &s.manifest.description,
                    &s.trust.to_string(),
                    &s.source_tier.to_string(),
                    &s.prompt_content,
                );
                Ok(ToolOutput::success(output, start.elapsed()))
            }
            None => {
                let available: Vec<String> = guard
                    .skills()
                    .iter()
                    .map(|s| s.manifest.name.clone())
                    .collect();
                Err(ToolError::ExecutionFailed(format!(
                    "Skill '{}' not found. Available skills: {}",
                    name,
                    if available.is_empty() {
                        "none".to_string()
                    } else {
                        available.join(", ")
                    }
                )))
            }
        }
    }
}

// ── skill_list ──────────────────────────────────────────────────────────

pub struct SkillListTool {
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
}

impl SkillListTool {
    pub fn new(registry: Arc<tokio::sync::RwLock<SkillRegistry>>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for SkillListTool {
    fn name(&self) -> &str {
        "skill_list"
    }

    fn description(&self) -> &str {
        "List all loaded skills with their trust level, source, and activation keywords."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_policy::skill_list_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let parsed = skill_policy::parse_skill_list_params(&params);
        let allowed_skills = restricted_skill_names(ctx);

        let guard = self.registry.read().await;

        let skills: Vec<serde_json::Value> = guard
            .skills()
            .iter()
            .filter(|s| {
                allowed_skills
                    .as_ref()
                    .is_none_or(|allowed| allowed.contains(s.manifest.name.as_str()))
            })
            .map(|s| {
                let mut entry = skill_policy::skill_list_entry(
                    &s.manifest.name,
                    &s.manifest.description,
                    &s.trust.to_string(),
                    &s.source_tier.to_string(),
                    &format!("{:?}", s.source),
                    serde_json::json!(s.manifest.activation.keywords),
                );

                if parsed.verbose {
                    let mut provenance = None;
                    let mut lifecycle_status = None;
                    let mut outcome_score = None;
                    let mut reuse_count = None;
                    let mut activation_reason = None;
                    if let Some(openclaw) = s
                        .manifest
                        .metadata
                        .as_ref()
                        .and_then(|metadata| metadata.openclaw.as_ref())
                    {
                        provenance = Some(serde_json::json!(openclaw.provenance.clone()));
                        lifecycle_status =
                            Some(serde_json::json!(openclaw.lifecycle_status.clone()));
                        outcome_score = Some(serde_json::json!(openclaw.outcome_score));
                        reuse_count = Some(serde_json::json!(openclaw.reuse_count));
                        activation_reason =
                            Some(serde_json::json!(openclaw.activation_reason.clone()));
                    }
                    skill_policy::add_skill_list_verbose_fields(
                        &mut entry,
                        skill_policy::SkillListVerboseFields {
                            version: s.manifest.version.clone(),
                            tags: serde_json::json!(s.manifest.activation.tags),
                            content_hash: s.content_hash.clone(),
                            max_context_tokens: serde_json::json!(
                                s.manifest.activation.max_context_tokens
                            ),
                            provenance,
                            lifecycle_status,
                            outcome_score,
                            reuse_count,
                            activation_reason,
                        },
                    );
                }

                entry
            })
            .collect();

        let output = skill_policy::skill_list_output(skills);

        Ok(ToolOutput::success(output, start.elapsed()))
    }
}

// ── skill_search ────────────────────────────────────────────────────────

pub struct SkillSearchTool {
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
    catalog: Arc<SkillCatalog>,
    remote_hub: Option<SharedRemoteSkillHub>,
}

impl SkillSearchTool {
    pub fn new(
        registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
        catalog: Arc<SkillCatalog>,
        remote_hub: Option<SharedRemoteSkillHub>,
    ) -> Self {
        Self {
            registry,
            catalog,
            remote_hub,
        }
    }
}

#[async_trait]
impl Tool for SkillSearchTool {
    fn name(&self) -> &str {
        "skill_search"
    }

    fn description(&self) -> &str {
        "Search for skills in the ClawHub catalog and among locally loaded skills."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_policy::skill_search_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let parsed = skill_policy::parse_skill_search_params(&params)?;
        let query = parsed.query.as_str();
        let source_filter = parsed.source;

        // Search the ClawHub catalog (async, best-effort)
        let catalog_outcome = self.catalog.search(query).await;
        let catalog_error = catalog_outcome.error.clone();

        // Enrich top results with detail data (stars, downloads, owner)
        let mut catalog_entries = catalog_outcome.results;
        self.catalog
            .enrich_search_results(&mut catalog_entries, 5)
            .await;

        // IC-026: Single lock acquisition for both installed names and local search
        let (installed_names, local_matches): (Vec<String>, Vec<serde_json::Value>) = {
            let guard = self.registry.read().await;

            let names = guard
                .skills()
                .iter()
                .map(|s| s.manifest.name.clone())
                .collect();

            let query_lower = query.to_lowercase();
            let matches = guard
                .skills()
                .iter()
                .filter(|s| {
                    s.manifest.name.to_lowercase().contains(&query_lower)
                        || s.manifest.description.to_lowercase().contains(&query_lower)
                        || s.manifest
                            .activation
                            .keywords
                            .iter()
                            .any(|k| k.to_lowercase().contains(&query_lower))
                })
                .map(|s| {
                    skill_policy::skill_search_local_entry(
                        &s.manifest.name,
                        &s.manifest.description,
                        &s.trust.to_string(),
                        &s.source_tier.to_string(),
                    )
                })
                .collect();

            (names, matches)
        };

        // Mark catalog entries that are already installed
        let catalog_json: Vec<serde_json::Value> = catalog_entries
            .iter()
            .map(|entry| {
                let is_installed = installed_names.iter().any(|n| {
                    // Match by slug suffix or exact name
                    entry.slug.ends_with(n.as_str()) || entry.name == *n
                });
                skill_policy::skill_search_catalog_entry(
                    &entry.slug,
                    &entry.name,
                    &entry.description,
                    &entry.version,
                    entry.score,
                    is_installed,
                    entry.stars,
                    entry.downloads,
                    entry.owner.as_deref(),
                )
            })
            .collect();

        let remote_json = if let Some(ref hub) = self.remote_hub {
            hub.search(query)
                .await
                .into_iter()
                .map(|entry| {
                    let trust_level = format!("{:?}", entry.trust_level).to_lowercase();
                    skill_policy::skill_search_remote_entry(
                        &entry.slug,
                        &entry.name,
                        &entry.description,
                        &entry.version,
                        &entry.source_adapter,
                        &entry.source_label,
                        &entry.source_ref,
                        entry.manifest_url.as_deref(),
                        entry.manifest_digest.as_deref(),
                        entry.repo.as_deref(),
                        entry.path.as_deref(),
                        entry.branch.as_deref(),
                        &trust_level,
                    )
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let output = skill_policy::skill_search_output(
            source_filter.as_str(),
            catalog_json,
            remote_json,
            local_matches,
            &self.catalog.registry_url(),
            catalog_error,
        );

        Ok(ToolOutput::success(output, start.elapsed()))
    }
}

// ── skill_check ──────────────────────────────────────────────────────────

pub struct SkillCheckTool {
    quarantine: Arc<QuarantineManager>,
}

impl SkillCheckTool {
    pub fn new(quarantine: Arc<QuarantineManager>) -> Self {
        Self { quarantine }
    }

    async fn resolve_input(
        &self,
        params: &serde_json::Value,
    ) -> Result<(String, String, String), ToolError> {
        let input = skill_policy::parse_skill_check_input(params)?;
        if let Some(raw) = input.inline_content() {
            return Ok((
                raw.to_string(),
                input.source_kind().to_string(),
                input.source_ref(),
            ));
        }

        if let skill_policy::SkillCheckInput::Url(url) = &input {
            return Ok((
                fetch_skill_content(url).await?,
                input.source_kind().to_string(),
                input.source_ref(),
            ));
        }

        let path = input.source_ref();
        let skill_path = skill_policy::skill_check_path_for_read(&path);
        let raw = tokio::fs::read_to_string(&skill_path)
            .await
            .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
        Ok((raw, input.source_kind().to_string(), path))
    }
}

#[async_trait]
impl Tool for SkillCheckTool {
    fn name(&self) -> &str {
        "skill_check"
    }

    fn description(&self) -> &str {
        "Validate SKILL.md content, a local SKILL.md path, or a direct HTTPS SKILL.md URL without installing it."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_policy::skill_check_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let (raw_content, source_kind, source_ref) = self.resolve_input(&params).await?;
        let normalized = crate::skills::normalize_line_endings(&raw_content);
        let normalized_content_hash = crate::skills::registry::compute_hash(&normalized);
        let scan_content = skill_content_for_scan(raw_content, &source_kind, &source_ref);

        let source_path = if source_kind == "path" {
            PathBuf::from(&source_ref)
        } else {
            PathBuf::from(".")
        };
        let package_files = if source_kind == "path" {
            let package_root =
                if source_path.file_name().and_then(|name| name.to_str()) == Some("SKILL.md") {
                    source_path
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| PathBuf::from("."))
                } else {
                    source_path.clone()
                };
            collect_skill_package_files(&package_root)
                .map(|files| package_scan_files(&files))
                .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?
        } else {
            Vec::new()
        };
        let scan_report = scan_report_for_content(
            &self.quarantine,
            "(preflight)",
            self.quarantine.quarantine_dir().to_path_buf(),
            scan_content,
            package_files,
        );
        let findings = scan_report.findings.as_slice();

        let validation = if source_kind == "path" {
            crate::skills::registry::SkillRegistry::validate_skill_file(
                &source_path,
                SkillTrust::Installed,
                SkillSource::External(source_path.clone()),
            )
            .await
        } else {
            crate::skills::registry::SkillRegistry::validate_skill_content(
                &normalized,
                SkillTrust::Installed,
                SkillSource::External(source_path.clone()),
            )
            .await
        };

        let mut output = match validation {
            Ok((_name, loaded)) => skill_policy::skill_check_success_output(
                &source_kind,
                &source_ref,
                &loaded.manifest.name,
                &loaded.manifest.version,
                &loaded.manifest.description,
                serde_json::json!(loaded.manifest.activation),
                &loaded.trust.to_string(),
                &loaded.source_tier.to_string(),
                (loaded.prompt_content.len() as f64 * 0.25) as usize,
                loaded.manifest.activation.max_context_tokens,
                &loaded.content_hash,
                &normalized_content_hash,
                skill_finding_json(&findings),
            ),
            Err(err) => skill_policy::skill_check_error_output(
                &source_kind,
                &source_ref,
                &err.to_string(),
                &normalized_content_hash,
                skill_finding_json(&findings),
            ),
        };
        add_scan_report_fields(&mut output, &scan_report);

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }
}

// ── skill_install ───────────────────────────────────────────────────────

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
            crate::skills::catalog::skill_download_url(self.catalog.registry_url(), name);
        let content = fetch_skill_content(&download_url).await?;
        Ok(Some(SkillContent {
            raw_content: content,
            source_kind: "clawhub_catalog".to_string(),
            source_adapter: "clawhub_catalog".to_string(),
            source_ref: name.to_string(),
            source_repo: None,
            source_url: Some(download_url),
            manifest_url: Some(crate::skills::catalog::skill_download_url(
                self.catalog.registry_url(),
                name,
            )),
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
                let _ = crate::skills::registry::SkillRegistry::delete_skill_files(&path).await;
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
                        let _ =
                            crate::skills::registry::SkillRegistry::delete_skill_files(&orphan_dir)
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

/// Validate that a URL is safe to fetch (SSRF prevention).
///
/// Rejects:
/// - Non-HTTPS URLs (except in tests)
/// - URLs pointing to private, loopback, or link-local IP addresses
/// - URLs without a host
pub fn validate_fetch_url(url_str: &str) -> Result<(), ToolError> {
    skill_policy::validate_fetch_url(url_str)
}

/// Fetch SKILL.md content from a URL with SSRF protection.
///
/// The ClawHub registry returns skill downloads as ZIP archives containing
/// `SKILL.md` and `_meta.json`. This function detects ZIP responses (by the
/// `PK\x03\x04` magic bytes) and extracts `SKILL.md` automatically. Plain
/// text responses are returned as-is.
pub async fn fetch_skill_content(url: &str) -> Result<String, ToolError> {
    validate_fetch_url(url)?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("thinclaw/0.1")
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| ToolError::ExecutionFailed(format!("HTTP client error: {}", e)))?;

    let response = client.get(url).send().await.map_err(|e| {
        ToolError::ExecutionFailed(format!("Failed to fetch skill from {}: {}", url, e))
    })?;

    if !response.status().is_success() {
        // Provide a more helpful error for redirect responses (3xx).
        // Redirects are intentionally disabled (Policy::none) to prevent
        // redirect-based SSRF. Tell the user why and suggest the final URL.
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("unknown");
            return Err(ToolError::ExecutionFailed(format!(
                "URL returned HTTP {} redirect to '{}'. Redirects are blocked for security. \
                 Use the final destination URL directly.",
                response.status(),
                location
            )));
        }
        return Err(ToolError::ExecutionFailed(format!(
            "Skill fetch returned HTTP {}: {}",
            response.status(),
            url
        )));
    }

    // Limit download size to prevent memory exhaustion from large responses.
    const MAX_DOWNLOAD_BYTES: usize = 10 * 1024 * 1024; // 10 MB
    let bytes = response
        .bytes()
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read response body: {}", e)))?;
    if bytes.len() > MAX_DOWNLOAD_BYTES {
        return Err(ToolError::ExecutionFailed(format!(
            "Response too large: {} bytes (max {} bytes)",
            bytes.len(),
            MAX_DOWNLOAD_BYTES
        )));
    }

    // Detect ZIP archive (PK\x03\x04 magic) and extract SKILL.md
    let content = if bytes.starts_with(b"PK\x03\x04") {
        extract_skill_from_zip(&bytes)?
    } else {
        String::from_utf8(bytes.to_vec()).map_err(|e| {
            ToolError::ExecutionFailed(format!("Response is not valid UTF-8: {}", e))
        })?
    };

    // Basic size check
    if content.len() as u64 > crate::skills::MAX_PROMPT_FILE_SIZE {
        return Err(ToolError::ExecutionFailed(format!(
            "Skill content too large: {} bytes (max {} bytes)",
            content.len(),
            crate::skills::MAX_PROMPT_FILE_SIZE
        )));
    }

    Ok(content)
}

/// Extract `SKILL.md` from a ZIP archive returned by the ClawHub download API.
///
/// Walks ZIP local file headers looking for an entry named `SKILL.md`.
/// Supports Store (method 0) and Deflate (method 8) compression.
fn extract_skill_from_zip(data: &[u8]) -> Result<String, ToolError> {
    skill_policy::extract_skill_from_zip(data)
}

// ── skill_remove ────────────────────────────────────────────────────────

pub struct SkillAuditTool {
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
    quarantine: Arc<QuarantineManager>,
}

impl SkillAuditTool {
    pub fn new(
        registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
        quarantine: Arc<QuarantineManager>,
    ) -> Self {
        Self {
            registry,
            quarantine,
        }
    }
}

#[async_trait]
impl Tool for SkillAuditTool {
    fn name(&self) -> &str {
        "skill_audit"
    }

    fn description(&self) -> &str {
        "Audit loaded skills for risky patterns using the quarantine scanner without modifying or removing them."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_policy::skill_audit_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let target_name = skill_policy::parse_skill_audit_target_name(&params);
        let guard = self.registry.read().await;

        let audited = guard
            .skills()
            .iter()
            .filter(|skill| {
                target_name.is_none_or(|name| skill.manifest.name.eq_ignore_ascii_case(name))
            })
            .map(|skill| {
                let source_path =
                    source_path_for_skill(skill).unwrap_or_else(|| PathBuf::from("."));
                let package_files = scan_files_for_source_path(&source_path);
                let scan_report = scan_report_for_content(
                    &self.quarantine,
                    &skill.manifest.name,
                    source_path.clone(),
                    SkillContent {
                        raw_content: skill.prompt_content.clone(),
                        source_kind: "audit".to_string(),
                        source_adapter: "audit".to_string(),
                        source_ref: skill.manifest.name.clone(),
                        source_repo: None,
                        source_url: None,
                        manifest_url: None,
                        manifest_digest: None,
                        path: None,
                        branch: None,
                        commit_sha: None,
                        trust_level: SkillTapTrustLevel::Community,
                    },
                    package_files,
                );

                let mut entry = skill_policy::skill_audit_entry_output(
                    &skill.manifest.name,
                    &skill.trust.to_string(),
                    &skill.source_tier.to_string(),
                    &source_path.display().to_string(),
                    skill_finding_json(&scan_report.findings),
                );
                add_scan_report_fields(&mut entry, &scan_report);
                entry
            })
            .collect::<Vec<_>>();

        if audited.is_empty() {
            return Err(ToolError::ExecutionFailed(
                "No matching skills found to audit".to_string(),
            ));
        }

        Ok(ToolOutput::success(
            skill_policy::skill_audit_output(audited),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }
}

pub struct SkillUpdateTool {
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
    installer: SkillInstallTool,
}

impl SkillUpdateTool {
    pub fn new(
        registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
        catalog: Arc<SkillCatalog>,
        remote_hub: Option<SharedRemoteSkillHub>,
        quarantine: Arc<QuarantineManager>,
    ) -> Self {
        let installer =
            SkillInstallTool::new(Arc::clone(&registry), catalog, remote_hub, quarantine);
        Self {
            registry,
            installer,
        }
    }
}

#[async_trait]
impl Tool for SkillUpdateTool {
    fn name(&self) -> &str {
        "skill_update"
    }

    fn description(&self) -> &str {
        "Update an installed skill using its recorded provenance lock when available."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_policy::skill_update_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let parsed = skill_policy::parse_skill_update_params(&params)?;
        let name = parsed.name.as_str();
        let approve_risky = parsed.approve_risky;

        let (source_path, installed_name) = {
            let guard = self.registry.read().await;
            let skill = guard
                .skills()
                .iter()
                .find(|skill| skill.manifest.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| ToolError::ExecutionFailed(format!("Skill '{}' not found", name)))?;
            (
                source_path_for_skill(skill).ok_or_else(|| {
                    ToolError::ExecutionFailed(format!(
                        "Skill '{}' does not have a filesystem source path",
                        name
                    ))
                })?,
                skill.manifest.name.clone(),
            )
        };

        let provenance = read_skill_provenance(&source_path).await.map_err(|_| {
            ToolError::ExecutionFailed(format!(
                "Skill '{}' is missing a provenance lock and cannot be auto-updated",
                name
            ))
        })?;

        let mut install_params =
            skill_policy::skill_update_install_params(&installed_name, true, approve_risky);

        match provenance.source_adapter.as_str() {
            "clawhub_catalog" => {
                install_params["name"] = serde_json::Value::String(provenance.source_ref.clone());
            }
            "github_tap" | "well_known" => {
                install_params["name"] = serde_json::Value::String(installed_name);
            }
            "url" => {
                let url = provenance
                    .source_url
                    .clone()
                    .or(provenance.manifest_url.clone())
                    .ok_or_else(|| {
                        ToolError::ExecutionFailed(format!(
                            "Skill '{}' has URL provenance but no fetchable URL",
                            name
                        ))
                    })?;
                skill_policy::add_skill_update_url(&mut install_params, url);
            }
            _ => {
                if let Some(url) = provenance
                    .source_url
                    .clone()
                    .or(provenance.manifest_url.clone())
                {
                    skill_policy::add_skill_update_url(&mut install_params, url);
                } else {
                    return Err(ToolError::ExecutionFailed(format!(
                        "Skill '{}' does not have a supported update source",
                        name
                    )));
                }
            }
        }

        self.installer.execute(install_params, ctx).await
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

// ── skill_publish ───────────────────────────────────────────────────────

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

impl PublishPlan {
    fn json(&self, status: &str) -> serde_json::Value {
        let mut output = skill_policy::skill_publish_plan_output(
            status,
            &self.skill_name,
            &self.target_repo,
            &self.tap_path,
            &self.package_path,
            &self.branch,
            self.base_branch.as_deref(),
            &self.package_hash,
            package_file_json(&self.files),
            skill_finding_json(&self.findings),
            &self.trust,
            &self.source_tier,
            self.source.clone(),
        );
        add_scan_report_fields(&mut output, &self.scan_report);
        output
    }
}

async fn build_publish_plan(
    registry: &Arc<tokio::sync::RwLock<SkillRegistry>>,
    quarantine: &Arc<QuarantineManager>,
    store: Option<&Arc<dyn Database>>,
    user_id: &str,
    name: &str,
    target_repo: &str,
) -> Result<PublishPlan, ToolError> {
    validate_github_repo(target_repo)?;
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
    validate_repo_path_component(&skill.manifest.name, "skill name")?;
    let tap_path = normalize_tap_path(&tap.path);
    validate_repo_relative_path(&tap_path, "tap.path")?;
    let package_path = if tap_path.is_empty() {
        skill.manifest.name.clone()
    } else {
        format!("{}/{}", tap_path, skill.manifest.name)
    };
    validate_repo_relative_path(&package_path, "package_path")?;
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

    let mut output = plan.json("published");
    output["scratch_dir"] = serde_json::Value::String(scratch_dir.display().to_string());
    output["package_dir"] = serde_json::Value::String(package_dir.display().to_string());
    output["pr_url"] = serde_json::Value::String(pr_url);
    output["base_branch"] = serde_json::Value::String(base_branch);
    Ok(output)
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
            plan.json("dry_run")
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

pub struct SkillTapListTool {
    store: Option<Arc<dyn Database>>,
    remote_hub: Option<SharedRemoteSkillHub>,
}

pub struct SkillTapAddTool {
    store: Option<Arc<dyn Database>>,
    remote_hub: Option<SharedRemoteSkillHub>,
}

pub struct SkillTapRemoveTool {
    store: Option<Arc<dyn Database>>,
    remote_hub: Option<SharedRemoteSkillHub>,
}

pub struct SkillTapRefreshTool {
    store: Option<Arc<dyn Database>>,
    remote_hub: Option<SharedRemoteSkillHub>,
}

impl SkillTapListTool {
    pub fn new(store: Option<Arc<dyn Database>>, remote_hub: Option<SharedRemoteSkillHub>) -> Self {
        Self { store, remote_hub }
    }
}

impl SkillTapAddTool {
    pub fn new(store: Option<Arc<dyn Database>>, remote_hub: Option<SharedRemoteSkillHub>) -> Self {
        Self { store, remote_hub }
    }
}

impl SkillTapRemoveTool {
    pub fn new(store: Option<Arc<dyn Database>>, remote_hub: Option<SharedRemoteSkillHub>) -> Self {
        Self { store, remote_hub }
    }
}

impl SkillTapRefreshTool {
    pub fn new(store: Option<Arc<dyn Database>>, remote_hub: Option<SharedRemoteSkillHub>) -> Self {
        Self { store, remote_hub }
    }
}

fn tap_json(tap: &SkillTapConfig) -> serde_json::Value {
    let trust_level = format!("{:?}", tap.trust_level).to_lowercase();
    skill_policy::skill_tap_json(&tap.repo, &tap.path, tap.branch.as_deref(), &trust_level)
}

fn require_skill_tap_store<'a>(
    store: &'a Option<Arc<dyn Database>>,
    tool_name: &str,
) -> Result<&'a Arc<dyn Database>, ToolError> {
    store.as_ref().ok_or_else(|| {
        ToolError::ExecutionFailed(format!(
            "Tool '{}' requires the settings database",
            tool_name
        ))
    })
}

fn require_shared_remote_hub<'a>(
    remote_hub: &'a Option<SharedRemoteSkillHub>,
    tool_name: &str,
) -> Result<&'a SharedRemoteSkillHub, ToolError> {
    remote_hub.as_ref().ok_or_else(|| {
        ToolError::ExecutionFailed(format!(
            "Tool '{}' requires the skills remote hub",
            tool_name
        ))
    })
}

#[async_trait]
impl Tool for SkillTapListTool {
    fn name(&self) -> &str {
        "skill_tap_list"
    }

    fn description(&self) -> &str {
        "List configured GitHub skill taps."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_policy::skill_tap_list_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let include_health = params
            .get("include_health")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let store = require_skill_tap_store(&self.store, self.name())?;
        let settings = load_settings_for_taps(store, &ctx.user_id).await?;
        let hub_enabled = if include_health {
            match self.remote_hub.as_ref() {
                Some(hub) => Some(hub.is_enabled().await),
                None => Some(false),
            }
        } else {
            None
        };
        Ok(ToolOutput::success(
            skill_policy::skill_tap_list_output(
                settings.skill_taps.iter().map(tap_json).collect::<Vec<_>>(),
                hub_enabled,
            ),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }
}

#[async_trait]
impl Tool for SkillTapAddTool {
    fn name(&self) -> &str {
        "skill_tap_add"
    }

    fn description(&self) -> &str {
        "Persist a GitHub skill tap and refresh remote skill discovery."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_policy::skill_tap_add_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let store = require_skill_tap_store(&self.store, self.name())?;
        let remote_hub = require_shared_remote_hub(&self.remote_hub, self.name())?;
        let parsed = skill_policy::parse_skill_tap_add_params(&params)?;
        let repo = parsed.repo;
        let path = parsed.path;
        let branch = parsed.branch;
        let trust_level = parse_tap_trust_level(&parsed.trust_level)?;
        let replace = parsed.replace;
        let mut settings = load_settings_for_taps(store, &ctx.user_id).await?;
        let existing_idx = settings
            .skill_taps
            .iter()
            .position(|tap| tap_key_matches(tap, &repo, &path, branch.as_deref()));
        match (existing_idx, replace) {
            (Some(idx), true) => {
                settings.skill_taps[idx] = SkillTapConfig {
                    repo: repo.clone(),
                    path: path.clone(),
                    branch: branch.clone(),
                    trust_level,
                };
            }
            (Some(_), false) => {
                return Err(ToolError::ExecutionFailed(format!(
                    "Skill tap '{}:{}' already exists; use replace=true to update it",
                    repo, path
                )));
            }
            (None, _) => settings.skill_taps.push(SkillTapConfig {
                repo: repo.clone(),
                path: path.clone(),
                branch: branch.clone(),
                trust_level,
            }),
        }
        persist_skill_taps(store, &ctx.user_id, &settings.skill_taps).await?;
        let refreshed_count =
            refresh_remote_hub_from_settings(store, &ctx.user_id, remote_hub).await?;
        let replaced = existing_idx.is_some();
        Ok(ToolOutput::success(
            skill_policy::skill_tap_add_output(
                replaced,
                tap_json(&SkillTapConfig {
                    repo,
                    path,
                    branch,
                    trust_level,
                }),
                refreshed_count,
            ),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

#[async_trait]
impl Tool for SkillTapRemoveTool {
    fn name(&self) -> &str {
        "skill_tap_remove"
    }

    fn description(&self) -> &str {
        "Remove a persisted GitHub skill tap and refresh remote skill discovery."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_policy::skill_tap_remove_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let store = require_skill_tap_store(&self.store, self.name())?;
        let remote_hub = require_shared_remote_hub(&self.remote_hub, self.name())?;
        let parsed = skill_policy::parse_skill_tap_remove_params(&params)?;
        let repo = parsed.repo;
        let path = parsed.path;
        let branch = parsed.branch;
        let mut settings = load_settings_for_taps(store, &ctx.user_id).await?;
        let before = settings.skill_taps.len();
        settings
            .skill_taps
            .retain(|tap| !tap_key_matches(tap, &repo, &path, branch.as_deref()));
        if settings.skill_taps.len() == before {
            return Err(ToolError::ExecutionFailed(format!(
                "Skill tap '{}:{}' not found",
                repo, path
            )));
        }
        persist_skill_taps(store, &ctx.user_id, &settings.skill_taps).await?;
        let refreshed_count =
            refresh_remote_hub_from_settings(store, &ctx.user_id, remote_hub).await?;
        Ok(ToolOutput::success(
            skill_policy::skill_tap_remove_output(&repo, &path, branch.as_deref(), refreshed_count),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

#[async_trait]
impl Tool for SkillTapRefreshTool {
    fn name(&self) -> &str {
        "skill_tap_refresh"
    }

    fn description(&self) -> &str {
        "Rebuild remote skill discovery from persisted skill tap settings."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_policy::skill_tap_refresh_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let store = require_skill_tap_store(&self.store, self.name())?;
        let remote_hub = require_shared_remote_hub(&self.remote_hub, self.name())?;
        let parsed = skill_policy::parse_skill_tap_refresh_params(&params)?;
        let repo = parsed.repo;
        let path = parsed.path;

        if repo.is_some() || path.is_some() {
            let settings = load_settings_for_taps(store, &ctx.user_id).await?;
            let matches = settings.skill_taps.iter().any(|tap| {
                let repo_matches = match repo.as_ref() {
                    Some(repo) => tap.repo.eq_ignore_ascii_case(repo),
                    None => true,
                };
                let path_matches = match path.as_ref() {
                    Some(path) => normalize_tap_path(&tap.path) == *path,
                    None => true,
                };
                repo_matches && path_matches
            });
            if !matches {
                return Err(ToolError::ExecutionFailed(
                    "No configured skill tap matches the requested refresh filter".to_string(),
                ));
            }
        }

        let tap_count = refresh_remote_hub_from_settings(store, &ctx.user_id, remote_hub).await?;
        let hub_enabled = remote_hub.is_enabled().await;
        Ok(ToolOutput::success(
            skill_policy::skill_tap_refresh_output(
                tap_count,
                repo.as_deref(),
                path.as_deref(),
                hub_enabled,
            ),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

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
        let guard = self.registry.read().await;

        let snapshot = skill_policy::skill_snapshot_document(
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
        );

        let snapshot_dir = crate::platform::state_paths().skills_dir.join(".hub");
        tokio::fs::create_dir_all(&snapshot_dir)
            .await
            .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
        let snapshot_path = snapshot_dir.join(format!(
            "snapshot-{}.json",
            Utc::now().format("%Y%m%dT%H%M%SZ")
        ));
        tokio::fs::write(
            &snapshot_path,
            serde_json::to_vec_pretty(&snapshot)
                .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?,
        )
        .await
        .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;

        Ok(ToolOutput::success(
            skill_policy::skill_snapshot_output(
                &snapshot_path.display().to_string(),
                guard.count(),
            ),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

pub struct SkillRemoveTool {
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
}

pub struct SkillPromoteTrustTool {
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
}

impl SkillPromoteTrustTool {
    pub fn new(registry: Arc<tokio::sync::RwLock<SkillRegistry>>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for SkillPromoteTrustTool {
    fn name(&self) -> &str {
        "skill_trust_promote"
    }

    fn description(&self) -> &str {
        "Promote or demote a user-managed skill between installed and trusted trust ceilings."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_policy::skill_trust_promote_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let parsed = skill_policy::parse_skill_trust_promote_params(&params)?;
        let name = parsed.name.as_str();
        let target_trust = match parsed.target_trust.as_str() {
            "installed" => SkillTrust::Installed,
            "trusted" => SkillTrust::Trusted,
            _ => unreachable!("skill policy validates target_trust"),
        };

        let mut guard = self.registry.write().await;
        guard
            .promote_trust(name, target_trust)
            .await
            .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
        let source_tier = guard
            .find_by_name(name)
            .map(|skill| skill.source_tier.to_string())
            .unwrap_or_else(|| "community".to_string());

        Ok(ToolOutput::success(
            skill_policy::skill_trust_promote_output(name, &target_trust.to_string(), &source_tier),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

impl SkillRemoveTool {
    pub fn new(registry: Arc<tokio::sync::RwLock<SkillRegistry>>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for SkillRemoveTool {
    fn name(&self) -> &str {
        "skill_remove"
    }

    fn description(&self) -> &str {
        "Remove an installed skill by name. Only user-installed skills can be removed."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_policy::skill_remove_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let name = skill_policy::parse_skill_name_param(&params)?;
        let name = name.as_str();

        // ── TOCTOU fix ─────────────────────────────────────────────────
        // Hold the write lock for the entire validate → delete → commit
        // sequence. This prevents concurrent remove+install races where a
        // new install could land files that get incorrectly deleted.
        // The file I/O inside delete_skill_files is fast (single file +
        // rmdir) so lock contention is negligible.
        let mut guard = self.registry.write().await;

        let skill_path = guard
            .validate_remove(name)
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        // Delete files from disk (async I/O).
        crate::skills::registry::SkillRegistry::delete_skill_files(&skill_path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        // Remove from in-memory registry.
        guard
            .commit_remove(name)
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        drop(guard);

        let output = skill_policy::skill_remove_output(name);

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

// ── skill_reload ────────────────────────────────────────────────────────

/// Hot-reload a skill (or all skills) from disk without restarting.
///
/// Use after editing a SKILL.md file on disk so that changes take effect
/// in the current session. A single-skill reload is surgical and fast;
/// the `all` flag triggers a full re-discovery pass.
pub struct SkillReloadTool {
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
}

impl SkillReloadTool {
    pub fn new(registry: Arc<tokio::sync::RwLock<SkillRegistry>>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for SkillReloadTool {
    fn name(&self) -> &str {
        "skill_reload"
    }

    fn description(&self) -> &str {
        "Reload a skill (or all skills) from disk after editing SKILL.md files. \
         Use after making on-disk changes so they take effect immediately without restarting. \
         Provide a skill name to reload just that skill, or set all=true to rediscover all skills."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_policy::skill_reload_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let parsed = skill_policy::parse_skill_reload_params(&params);
        let reload_all = parsed.all;

        if reload_all {
            let mut guard = self.registry.write().await;
            let loaded = guard.reload().await;
            let output = skill_policy::skill_reload_all_output(loaded);
            return Ok(ToolOutput::success(output, start.elapsed()));
        }

        // Single-skill reload
        let name = parsed.name.as_deref().ok_or_else(|| {
            ToolError::InvalidParameters("missing required parameter: name".to_string())
        })?;
        let mut guard = self.registry.write().await;

        match guard.reload_skill(name).await {
            Ok(reloaded_name) => {
                let output = skill_policy::skill_reload_output(&reloaded_name);
                Ok(ToolOutput::success(output, start.elapsed()))
            }
            Err(e) => Err(ToolError::ExecutionFailed(format!(
                "Failed to reload skill '{}': {}",
                name, e
            ))),
        }
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        // Reloading changes agent behavior — require explicit approval
        // unless auto-approve is enabled.
        ApprovalRequirement::UnlessAutoApproved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_registry() -> Arc<tokio::sync::RwLock<SkillRegistry>> {
        let dir = tempfile::tempdir().unwrap();
        // Keep the tempdir so it lives for the test duration
        let path = dir.keep();
        Arc::new(tokio::sync::RwLock::new(SkillRegistry::new(path)))
    }

    fn test_catalog() -> Arc<SkillCatalog> {
        Arc::new(SkillCatalog::with_url("http://127.0.0.1:1"))
    }

    fn test_quarantine() -> Arc<QuarantineManager> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.keep();
        Arc::new(QuarantineManager::new(path))
    }

    async fn install_publishable_test_skill(
        registry: &Arc<tokio::sync::RwLock<SkillRegistry>>,
        name: &str,
    ) {
        registry
            .write()
            .await
            .install_skill(&format!(
                "---\nname: {name}\ndescription: Publishable skill\nactivation:\n  keywords: [\"publish\"]\n---\nUse this skill for publish tests.\n"
            ))
            .await
            .unwrap();

        let root = {
            let guard = registry.read().await;
            source_path_for_skill(guard.find_by_name(name).unwrap()).unwrap()
        };
        std::fs::write(root.join("README.md"), "supporting notes").unwrap();
    }

    #[test]
    fn test_skill_list_schema() {
        use crate::tools::tool::ApprovalRequirement;
        let tool = SkillListTool::new(test_registry());
        assert_eq!(tool.name(), "skill_list");
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::Never
        );
        let schema = tool.parameters_schema();
        assert!(schema.get("properties").is_some());
    }

    #[test]
    fn test_skill_search_schema() {
        use crate::tools::tool::ApprovalRequirement;
        let tool = SkillSearchTool::new(test_registry(), test_catalog(), None);
        assert_eq!(tool.name(), "skill_search");
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::Never
        );
        let schema = tool.parameters_schema();
        assert!(schema["properties"].get("query").is_some());
    }

    #[test]
    fn test_skill_install_schema() {
        use crate::tools::tool::ApprovalRequirement;
        let tool = SkillInstallTool::new(test_registry(), test_catalog(), None, test_quarantine());
        assert_eq!(tool.name(), "skill_install");
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::UnlessAutoApproved
        );
        let schema = tool.parameters_schema();
        assert!(schema["properties"].get("name").is_some());
        assert!(schema["properties"].get("url").is_some());
        assert!(schema["properties"].get("content").is_some());
    }

    #[test]
    fn test_skill_check_schema() {
        use crate::tools::tool::ApprovalRequirement;
        let tool = SkillCheckTool::new(test_quarantine());
        assert_eq!(tool.name(), "skill_check");
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::Never
        );
        let schema = tool.parameters_schema();
        assert!(schema["properties"].get("content").is_some());
        assert!(schema["properties"].get("path").is_some());
        assert!(schema["properties"].get("url").is_some());
    }

    #[test]
    fn test_skill_inspect_publish_and_tap_schemas() {
        use crate::tools::tool::ApprovalRequirement;

        let inspect = SkillInspectTool::new(test_registry(), test_quarantine());
        assert_eq!(inspect.name(), "skill_inspect");
        assert_eq!(
            inspect.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::Never
        );
        assert!(
            inspect.parameters_schema()["properties"]
                .get("include_files")
                .is_some()
        );

        let publish = SkillPublishTool::new(test_registry(), None, test_quarantine(), None);
        assert_eq!(publish.name(), "skill_publish");
        assert_eq!(
            publish.requires_approval(&serde_json::json!({"remote_write": false})),
            ApprovalRequirement::Never
        );
        assert_eq!(
            publish.requires_approval(&serde_json::json!({"remote_write": true})),
            ApprovalRequirement::UnlessAutoApproved
        );

        let tap_list = SkillTapListTool::new(None, None);
        let tap_add = SkillTapAddTool::new(None, None);
        let tap_remove = SkillTapRemoveTool::new(None, None);
        let tap_refresh = SkillTapRefreshTool::new(None, None);
        assert_eq!(tap_list.name(), "skill_tap_list");
        assert_eq!(tap_add.name(), "skill_tap_add");
        assert_eq!(tap_remove.name(), "skill_tap_remove");
        assert_eq!(tap_refresh.name(), "skill_tap_refresh");
        assert_eq!(
            tap_list.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::Never
        );
        assert_eq!(
            tap_add.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::UnlessAutoApproved
        );
    }

    #[test]
    fn test_skill_tap_path_validation_rejects_traversal() {
        assert!(validate_repo_relative_path("skills/community", "path").is_ok());
        assert!(validate_repo_relative_path("../outside", "path").is_err());
        assert!(validate_repo_relative_path("skills/../outside", "path").is_err());
        assert!(validate_github_repo("owner/repo").is_ok());
        assert!(validate_github_repo("owner/repo/extra").is_err());
    }

    #[tokio::test]
    async fn test_skill_publish_blocked_for_skill_restricted_contexts() {
        let tool = SkillPublishTool::new(test_registry(), None, test_quarantine(), None);
        let mut ctx = JobContext::default();
        ctx.metadata = serde_json::json!({
            "allowed_skills": ["github"]
        });

        let err = tool
            .execute(
                serde_json::json!({
                    "name": "anything",
                    "target_repo": "owner/repo"
                }),
                &ctx,
            )
            .await
            .unwrap_err();

        assert!(err.to_string().contains("not available"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_skill_publish_dry_run_reports_plan_inventory_and_source() {
        let (store, _guard) = crate::testing::test_db().await;
        let registry = test_registry();
        install_publishable_test_skill(&registry, "publishable-skill").await;
        let mut ctx = JobContext::default();
        ctx.user_id = "skill-publish-dry-run-user".to_string();
        store
            .set_setting(
                &ctx.user_id,
                "skill_taps",
                &serde_json::json!([{
                    "repo": "owner/skills",
                    "path": "community",
                    "branch": "main",
                    "trust_level": "community"
                }]),
            )
            .await
            .unwrap();

        let tool = SkillPublishTool::new(
            Arc::clone(&registry),
            None,
            test_quarantine(),
            Some(Arc::clone(&store)),
        );
        let output = tool
            .execute(
                serde_json::json!({
                    "name": "publishable-skill",
                    "target_repo": "owner/skills",
                    "dry_run": true
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(output.result["status"], "dry_run");
        assert_eq!(output.result["target_repo"], "owner/skills");
        assert_eq!(output.result["tap_path"], "community");
        assert_eq!(output.result["package_path"], "community/publishable-skill");
        assert!(
            output.result["package_hash"]
                .as_str()
                .unwrap()
                .starts_with("sha256:")
        );
        assert!(
            output.result["files"]
                .as_array()
                .unwrap()
                .iter()
                .any(|file| file["path"] == "SKILL.md")
        );
        assert!(
            output.result["files"]
                .as_array()
                .unwrap()
                .iter()
                .any(|file| file["path"] == "README.md")
        );
        assert_eq!(
            output.result["remote_write_plan"]["pull_request"]["draft"],
            true
        );
        assert!(output.result["source"].is_object());
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_skill_publish_remote_write_requires_configured_tap_and_confirmation() {
        let (store, _guard) = crate::testing::test_db().await;
        let registry = test_registry();
        install_publishable_test_skill(&registry, "remote-write-skill").await;
        let mut ctx = JobContext::default();
        ctx.user_id = "skill-publish-remote-write-user".to_string();
        let tool = SkillPublishTool::new(
            Arc::clone(&registry),
            None,
            test_quarantine(),
            Some(Arc::clone(&store)),
        );

        let missing_tap = tool
            .execute(
                serde_json::json!({
                    "name": "remote-write-skill",
                    "target_repo": "owner/missing",
                    "remote_write": true,
                    "dry_run": false,
                    "confirm_remote_write": true
                }),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(missing_tap.to_string().contains("not configured"));

        store
            .set_setting(
                &ctx.user_id,
                "skill_taps",
                &serde_json::json!([{
                    "repo": "owner/skills",
                    "path": "skills",
                    "branch": "main",
                    "trust_level": "community"
                }]),
            )
            .await
            .unwrap();

        let unconfirmed = tool
            .execute(
                serde_json::json!({
                    "name": "remote-write-skill",
                    "target_repo": "owner/skills",
                    "remote_write": true,
                    "dry_run": false
                }),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(
            unconfirmed
                .to_string()
                .contains("confirm_remote_write=true")
        );
    }

    #[tokio::test]
    async fn test_skill_check_valid_inline_content() {
        let tool = SkillCheckTool::new(test_quarantine());
        let output = tool
            .execute(
                serde_json::json!({
                    "content": "---\nname: checked-skill\ndescription: Checked\nactivation:\n  keywords: [\"check\"]\n---\nUse this skill for checking.\n"
                }),
                &JobContext::default(),
            )
            .await
            .unwrap();

        assert_eq!(output.result["ok"], true);
        assert_eq!(output.result["name"], "checked-skill");
        assert_eq!(output.result["source_kind"], "content");
        assert_eq!(output.result["finding_count"], 0);
        assert!(
            output.result["normalized_content_hash"]
                .as_str()
                .unwrap()
                .starts_with("sha256:")
        );
    }

    #[tokio::test]
    async fn test_skill_check_invalid_inline_content_returns_structured_failure() {
        let tool = SkillCheckTool::new(test_quarantine());
        let output = tool
            .execute(
                serde_json::json!({
                    "content": "---\nname: bad/name\n---\nBody.\n"
                }),
                &JobContext::default(),
            )
            .await
            .unwrap();

        assert_eq!(output.result["ok"], false);
        assert!(
            output.result["error"]
                .as_str()
                .unwrap()
                .contains("Invalid skill name")
        );
    }

    #[tokio::test]
    async fn test_skill_check_reports_quarantine_findings_without_installing() {
        let tool = SkillCheckTool::new(test_quarantine());
        let output = tool
            .execute(
                serde_json::json!({
                    "content": "---\nname: risky-skill\n---\nRun curl https://example.com and eval(x).\n"
                }),
                &JobContext::default(),
            )
            .await
            .unwrap();

        assert_eq!(output.result["ok"], true);
        assert_eq!(output.result["finding_count"], 2);
        assert!(
            output.result["findings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|finding| finding["kind"] == "network_fetch")
        );
        assert!(
            output.result["findings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|finding| finding["kind"] == "code_execution")
        );
    }

    #[tokio::test]
    async fn test_skill_check_requires_exactly_one_source() {
        let tool = SkillCheckTool::new(test_quarantine());
        let err = tool
            .execute(serde_json::json!({}), &JobContext::default())
            .await
            .unwrap_err();

        assert!(err.to_string().contains("exactly one"));
    }

    #[test]
    fn test_skill_remove_schema() {
        use crate::tools::tool::ApprovalRequirement;
        let tool = SkillRemoveTool::new(test_registry());
        assert_eq!(tool.name(), "skill_remove");
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::UnlessAutoApproved
        );
        let schema = tool.parameters_schema();
        assert!(schema["properties"].get("name").is_some());
    }

    #[tokio::test]
    async fn test_skill_audit_reports_findings() {
        let registry = test_registry();
        registry
            .write()
            .await
            .install_skill("---\nname: audited-skill\n---\nRun curl https://example.com\n")
            .await
            .unwrap();

        let tool = SkillAuditTool::new(Arc::clone(&registry), test_quarantine());
        let output = tool
            .execute(
                serde_json::json!({ "name": "audited-skill" }),
                &JobContext::default(),
            )
            .await
            .unwrap();

        assert_eq!(output.result["audited_count"], 1);
        assert_eq!(output.result["total_findings"], 1);
        assert_eq!(
            output.result["audited"][0]["findings"][0]["kind"],
            "network_fetch"
        );
    }

    #[tokio::test]
    async fn test_skill_inspect_reports_files_and_provenance() {
        let registry = test_registry();
        registry
            .write()
            .await
            .install_skill(
                "---\nname: inspectable-skill\nversion: 1.2.3\ndescription: Inspect me\nactivation:\n  keywords: [\"inspect\"]\n---\nInspect prompt.\n",
            )
            .await
            .unwrap();

        let root = {
            let guard = registry.read().await;
            source_path_for_skill(guard.find_by_name("inspectable-skill").unwrap()).unwrap()
        };
        std::fs::write(root.join("notes.md"), "support notes").unwrap();
        std::fs::write(
            root.join(".thinclaw-skill-lock.json"),
            serde_json::to_vec(&SkillProvenance {
                source_kind: "github_tap".to_string(),
                source_adapter: "github_tap".to_string(),
                source_ref: "github:owner/repo/inspectable-skill".to_string(),
                source_repo: Some("owner/repo".to_string()),
                source_url: None,
                manifest_url: None,
                manifest_digest: Some("sha".to_string()),
                path: Some("skills/inspectable-skill/SKILL.md".to_string()),
                branch: Some("main".to_string()),
                commit_sha: Some("sha".to_string()),
                trust_level: SkillTapTrustLevel::Community,
                downloaded_at: Utc::now().to_rfc3339(),
                findings: Vec::new(),
                scanner_version: Some(crate::skills::quarantine::SKILL_SCANNER_VERSION.to_string()),
                content_sha256: Some("sha256:test".to_string()),
                finding_summary: Some(FindingSummary::default()),
            })
            .unwrap(),
        )
        .unwrap();

        let quarantine = test_quarantine();
        let report = inspect_skill_report(
            &registry,
            &quarantine,
            "inspectable-skill",
            false,
            true,
            true,
        )
        .await
        .unwrap();

        assert_eq!(report["name"], "inspectable-skill");
        assert_eq!(report["provenance_lock"]["source_adapter"], "github_tap");
        assert!(
            report["files"]
                .as_array()
                .unwrap()
                .iter()
                .any(|file| file["path"] == "notes.md")
        );
        assert!(
            report["files"]
                .as_array()
                .unwrap()
                .iter()
                .any(|file| file["path"] == "SKILL.md")
        );
        assert!(report["source"]["kind"].as_str().is_some());
        assert!(report["inventory"].is_object());
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_skill_tap_add_list_refresh_and_remove_round_trip() {
        let (store, _guard) = crate::testing::test_db().await;
        let remote_hub = SharedRemoteSkillHub::default();
        let mut ctx = JobContext::default();
        ctx.user_id = "skill-tap-e2e-user".to_string();

        let add = SkillTapAddTool::new(Some(Arc::clone(&store)), Some(remote_hub.clone()));
        let list = SkillTapListTool::new(Some(Arc::clone(&store)), Some(remote_hub.clone()));
        let refresh = SkillTapRefreshTool::new(Some(Arc::clone(&store)), Some(remote_hub.clone()));
        let remove = SkillTapRemoveTool::new(Some(Arc::clone(&store)), Some(remote_hub.clone()));

        let added = add
            .execute(
                serde_json::json!({
                    "repo": "owner/tap",
                    "path": "/skills/community/",
                    "branch": "main",
                    "trust_level": "trusted"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(added.result["status"], "added");
        assert_eq!(added.result["tap"]["path"], "skills/community");
        assert_eq!(added.result["tap_count"], 1);
        assert!(remote_hub.is_enabled().await);

        let listed = list
            .execute(serde_json::json!({"include_health": true}), &ctx)
            .await
            .unwrap();
        assert_eq!(listed.result["count"], 1);
        assert_eq!(listed.result["taps"][0]["repo"], "owner/tap");
        assert_eq!(listed.result["hub_enabled"], true);

        let refreshed = refresh
            .execute(
                serde_json::json!({
                    "repo": "owner/tap",
                    "path": "skills/community"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(refreshed.result["status"], "refreshed");
        assert_eq!(refreshed.result["tap_count"], 1);

        let removed = remove
            .execute(
                serde_json::json!({
                    "repo": "owner/tap",
                    "path": "skills/community",
                    "branch": "main"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(removed.result["status"], "removed");
        assert_eq!(removed.result["tap_count"], 0);

        let listed_after_remove = list.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert_eq!(listed_after_remove.result["count"], 0);
    }

    #[test]
    fn test_skill_package_files_exclude_hidden_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("SKILL.md"),
            "---\nname: packaged\n---\nBody\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("README.md"), "readme").unwrap();
        std::fs::write(dir.path().join(".DS_Store"), "junk").unwrap();
        std::fs::write(dir.path().join(".secret"), "hidden").unwrap();

        let files = collect_skill_package_files(dir.path()).unwrap();
        let paths = files
            .iter()
            .map(|file| file.relative_path.as_str())
            .collect::<Vec<_>>();

        assert!(paths.contains(&"SKILL.md"));
        assert!(paths.contains(&"README.md"));
        assert!(!paths.contains(&".DS_Store"));
        assert!(!paths.contains(&".secret"));
    }

    #[cfg(unix)]
    #[test]
    fn test_skill_package_files_reject_symlink() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("SKILL.md"),
            "---\nname: packaged\n---\nBody\n",
        )
        .unwrap();
        std::os::unix::fs::symlink(dir.path().join("SKILL.md"), dir.path().join("linked.md"))
            .unwrap();

        let err = collect_skill_package_files(dir.path()).unwrap_err();
        assert!(err.to_string().contains("symlink"));
    }

    #[tokio::test]
    async fn test_skill_update_requires_provenance_lock() {
        let registry = test_registry();
        registry
            .write()
            .await
            .install_skill("---\nname: manual-skill\n---\n# Manual\n")
            .await
            .unwrap();

        let tool = SkillUpdateTool::new(
            Arc::clone(&registry),
            test_catalog(),
            None,
            test_quarantine(),
        );
        let err = tool
            .execute(
                serde_json::json!({ "name": "manual-skill" }),
                &JobContext::default(),
            )
            .await
            .unwrap_err();

        assert!(err.to_string().contains("missing a provenance lock"));
    }

    #[test]
    fn test_validate_fetch_url_allows_https() {
        assert!(super::validate_fetch_url("https://clawhub.ai/api/v1/download?slug=foo").is_ok());
    }

    #[test]
    fn test_validate_fetch_url_rejects_http() {
        let err = super::validate_fetch_url("http://example.com/skill.md").unwrap_err();
        assert!(err.to_string().contains("Only HTTPS"));
    }

    #[test]
    fn test_validate_fetch_url_rejects_private_ip() {
        let err = super::validate_fetch_url("https://192.168.1.1/skill.md").unwrap_err();
        assert!(err.to_string().contains("private"));
    }

    #[test]
    fn test_validate_fetch_url_rejects_loopback() {
        let err = super::validate_fetch_url("https://127.0.0.1/skill.md").unwrap_err();
        assert!(err.to_string().contains("private"));
    }

    #[test]
    fn test_validate_fetch_url_rejects_localhost() {
        let err = super::validate_fetch_url("https://localhost/skill.md").unwrap_err();
        assert!(err.to_string().contains("internal hostname"));
    }

    #[test]
    fn test_validate_fetch_url_rejects_metadata_endpoint() {
        let err =
            super::validate_fetch_url("https://169.254.169.254/latest/meta-data/").unwrap_err();
        assert!(err.to_string().contains("private"));
    }

    #[test]
    fn test_validate_fetch_url_rejects_internal_domain() {
        let err =
            super::validate_fetch_url("https://metadata.google.internal/something").unwrap_err();
        assert!(err.to_string().contains("internal hostname"));
    }

    #[test]
    fn test_validate_fetch_url_rejects_file_scheme() {
        let err = super::validate_fetch_url("file:///etc/passwd").unwrap_err();
        assert!(err.to_string().contains("Only HTTPS"));
    }

    #[test]
    fn test_extract_skill_from_zip_deflate() {
        // Build a real ZIP with flate2 + manual header construction.
        use flate2::Compression;
        use flate2::write::DeflateEncoder;
        use std::io::Write;

        let skill_md = b"---\nname: test\n---\n# Test Skill\n";
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(skill_md).unwrap();
        let compressed = encoder.finish().unwrap();

        let mut zip = Vec::new();
        // Local file header
        zip.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04]); // signature
        zip.extend_from_slice(&[0x14, 0x00]); // version needed (2.0)
        zip.extend_from_slice(&[0x00, 0x00]); // flags
        zip.extend_from_slice(&[0x08, 0x00]); // compression: deflate
        zip.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // mod time/date
        zip.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // crc32 (unused)
        zip.extend_from_slice(&(compressed.len() as u32).to_le_bytes()); // compressed size
        zip.extend_from_slice(&(skill_md.len() as u32).to_le_bytes()); // uncompressed size
        zip.extend_from_slice(&8u16.to_le_bytes()); // filename length
        zip.extend_from_slice(&0u16.to_le_bytes()); // extra field length
        zip.extend_from_slice(b"SKILL.md");
        zip.extend_from_slice(&compressed);

        let result = super::extract_skill_from_zip(&zip).unwrap();
        assert_eq!(result, "---\nname: test\n---\n# Test Skill\n");
    }

    #[test]
    fn test_extract_skill_from_zip_store() {
        let skill_md = b"---\nname: stored\n---\n# Stored\n";

        let mut zip = Vec::new();
        // Local file header
        zip.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04]);
        zip.extend_from_slice(&[0x0A, 0x00]); // version needed (1.0)
        zip.extend_from_slice(&[0x00, 0x00]); // flags
        zip.extend_from_slice(&[0x00, 0x00]); // compression: store
        zip.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // mod time/date
        zip.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // crc32
        zip.extend_from_slice(&(skill_md.len() as u32).to_le_bytes()); // compressed = uncompressed
        zip.extend_from_slice(&(skill_md.len() as u32).to_le_bytes());
        zip.extend_from_slice(&8u16.to_le_bytes()); // filename length
        zip.extend_from_slice(&0u16.to_le_bytes()); // extra field length
        zip.extend_from_slice(b"SKILL.md");
        zip.extend_from_slice(skill_md);

        let result = super::extract_skill_from_zip(&zip).unwrap();
        assert_eq!(result, "---\nname: stored\n---\n# Stored\n");
    }

    #[test]
    fn test_extract_skill_from_zip_missing_skill_md() {
        let mut zip = Vec::new();
        zip.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04]);
        zip.extend_from_slice(&[0x0A, 0x00]); // version
        zip.extend_from_slice(&[0x00, 0x00]); // flags
        zip.extend_from_slice(&[0x00, 0x00]); // compression: store
        zip.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // mod time/date
        zip.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // crc32
        zip.extend_from_slice(&2u32.to_le_bytes()); // compressed size
        zip.extend_from_slice(&2u32.to_le_bytes()); // uncompressed size
        zip.extend_from_slice(&10u16.to_le_bytes()); // filename length
        zip.extend_from_slice(&0u16.to_le_bytes()); // extra field length
        zip.extend_from_slice(b"_meta.json");
        zip.extend_from_slice(b"{}");

        let err = super::extract_skill_from_zip(&zip).unwrap_err();
        assert!(err.to_string().contains("does not contain SKILL.md"));
    }
}
