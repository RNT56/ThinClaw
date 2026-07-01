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
use thinclaw_tools::ports::{
    SkillInstallToolHostPort, SkillPublishToolHostPort, SkillSearchToolHostPort,
    SkillTapToolHostPort, SkillToolHostPort, ToolHostError, ToolOperationScope,
    ToolSkillCheckRequest, ToolSkillCheckResult, ToolSkillInstallActionRequest,
    ToolSkillInstallRequest, ToolSkillMutationActionResult, ToolSkillPublishRequest,
    ToolSkillPublishResult, ToolSkillQuery, ToolSkillRead, ToolSkillRemoveResult,
    ToolSkillSearchCatalogEntry, ToolSkillSearchLocalEntry, ToolSkillSearchRemoteEntry,
    ToolSkillSearchRequest, ToolSkillSearchResult, ToolSkillSnapshotResult, ToolSkillSummary,
    ToolSkillTap, ToolSkillTapAddRequest, ToolSkillTapList, ToolSkillTapMutationResult,
    ToolSkillTapQuery, ToolSkillTapRefreshRequest, ToolSkillTapRefreshResult,
    ToolSkillTapRemoveRequest, ToolSkillTapTrust, ToolSkillTrust, ToolSkillTrustMutationRequest,
    ToolSkillTrustMutationResult, ToolSkillUpdateActionRequest, job_context_from_tool_scope,
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

fn finding_severity_label(severity: FindingSeverity) -> &'static str {
    match severity {
        FindingSeverity::Info => "info",
        FindingSeverity::Warning => "warning",
        FindingSeverity::Critical => "critical",
    }
}

fn skill_policy_findings(
    findings: &[SecurityFinding],
) -> Vec<skill_policy::SkillFindingPolicy<'_>> {
    findings
        .iter()
        .map(|finding| skill_policy::SkillFindingPolicy {
            kind: &finding.kind,
            severity: finding_severity_label(finding.severity),
            excerpt: &finding.excerpt,
            rule_id: finding.rule_id.as_deref(),
            file: finding.file.as_deref(),
            line: finding.line,
            recommendation: finding.recommendation.as_deref(),
            scanner_version: finding.scanner_version.as_deref(),
        })
        .collect()
}

fn skill_finding_json(findings: &[SecurityFinding]) -> Vec<serde_json::Value> {
    skill_policy::skill_finding_detail_outputs(skill_policy_findings(findings))
}

fn summarize_findings(findings: &[SecurityFinding]) -> String {
    skill_policy::skill_findings_detail_summary(skill_policy_findings(findings))
}

fn finding_summary_policy(summary: &FindingSummary) -> skill_policy::SkillFindingSummary {
    skill_policy::SkillFindingSummary {
        total: summary.total,
        warnings: summary.warnings,
        critical: summary.critical,
        categories: summary.categories.clone(),
    }
}

fn add_scan_report_fields(output: &mut serde_json::Value, report: &SkillScanReport) {
    skill_policy::add_skill_scan_report_fields(
        output,
        &report.scanner_version,
        &report.content_sha256,
        finding_summary_policy(&report.summary),
    );
}

fn tap_trust_label(value: SkillTapTrustLevel) -> &'static str {
    match value {
        SkillTapTrustLevel::Community => "community",
        SkillTapTrustLevel::Trusted => "trusted",
        SkillTapTrustLevel::Builtin => "builtin",
    }
}

fn findings_require_approval(
    trust_level: SkillTapTrustLevel,
    findings: &[SecurityFinding],
) -> bool {
    skill_policy::skill_findings_require_approval_for_details(
        tap_trust_label(trust_level),
        skill_policy_findings(findings),
    )
}

fn findings_require_rejection(findings: &[SecurityFinding]) -> bool {
    skill_policy::skill_findings_require_rejection_for_details(skill_policy_findings(findings))
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

fn parse_tap_trust_level(value: &str) -> Result<SkillTapTrustLevel, ToolError> {
    match skill_policy::parse_skill_tap_trust_level(value)?.as_str() {
        "builtin" => Ok(SkillTapTrustLevel::Builtin),
        "trusted" => Ok(SkillTapTrustLevel::Trusted),
        "community" => Ok(SkillTapTrustLevel::Community),
        other => Err(ToolError::InvalidParameters(format!(
            "Unsupported trust_level '{}'",
            other
        ))),
    }
}

fn tap_trust_to_port(value: SkillTapTrustLevel) -> ToolSkillTapTrust {
    match value {
        SkillTapTrustLevel::Builtin => ToolSkillTapTrust::Builtin,
        SkillTapTrustLevel::Trusted => ToolSkillTapTrust::Trusted,
        SkillTapTrustLevel::Community => ToolSkillTapTrust::Community,
    }
}

fn tap_trust_from_port(value: ToolSkillTapTrust) -> SkillTapTrustLevel {
    match value {
        ToolSkillTapTrust::Builtin => SkillTapTrustLevel::Builtin,
        ToolSkillTapTrust::Trusted => SkillTapTrustLevel::Trusted,
        ToolSkillTapTrust::Community => SkillTapTrustLevel::Community,
    }
}

fn tool_host_error_from_tool(error: ToolError) -> ToolHostError {
    match error {
        ToolError::InvalidParameters(reason) => ToolHostError::InvalidRequest { reason },
        ToolError::NotAuthorized(reason) => ToolHostError::PermissionDenied { reason },
        ToolError::ExternalService(service) => ToolHostError::Unavailable { service },
        other => ToolHostError::OperationFailed {
            reason: other.to_string(),
        },
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

fn tool_scope_user_id(scope: &ToolOperationScope) -> &str {
    &scope.principal_id
}

fn skill_trust_to_port(trust: SkillTrust) -> ToolSkillTrust {
    match trust {
        SkillTrust::Installed => ToolSkillTrust::Installed,
        SkillTrust::Trusted => ToolSkillTrust::Trusted,
    }
}

async fn resolve_skill_check_input(
    input: &skill_policy::SkillCheckInput,
) -> Result<(String, String, String), ToolError> {
    if let Some(raw) = input.inline_content() {
        return Ok((
            raw.to_string(),
            input.source_kind().to_string(),
            input.source_ref(),
        ));
    }

    if let skill_policy::SkillCheckInput::Url(url) = input {
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

async fn skill_check_output_for_input(
    quarantine: &QuarantineManager,
    input: skill_policy::SkillCheckInput,
) -> Result<serde_json::Value, ToolError> {
    let (raw_content, source_kind, source_ref) = resolve_skill_check_input(&input).await?;
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
        quarantine,
        "(preflight)",
        quarantine.quarantine_dir().to_path_buf(),
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
            skill_finding_json(findings),
        ),
        Err(err) => skill_policy::skill_check_error_output(
            &source_kind,
            &source_ref,
            &err.to_string(),
            &normalized_content_hash,
            skill_finding_json(findings),
        ),
    };
    add_scan_report_fields(&mut output, &scan_report);
    Ok(output)
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

async fn audit_skills_for_registry(
    registry: &Arc<tokio::sync::RwLock<SkillRegistry>>,
    quarantine: &Arc<QuarantineManager>,
    target_name: Option<&str>,
) -> Result<Vec<serde_json::Value>, ToolError> {
    let guard = registry.read().await;
    let audited = guard
        .skills()
        .iter()
        .filter(|skill| {
            target_name.is_none_or(|name| skill.manifest.name.eq_ignore_ascii_case(name))
        })
        .map(|skill| {
            let source_path = source_path_for_skill(skill).unwrap_or_else(|| PathBuf::from("."));
            let package_files = scan_files_for_source_path(&source_path);
            let scan_report = scan_report_for_content(
                quarantine,
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

    Ok(audited)
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

async fn promote_skill_trust_in_registry(
    registry: &Arc<tokio::sync::RwLock<SkillRegistry>>,
    name: &str,
    target_trust: SkillTrust,
) -> Result<String, ToolError> {
    let mut guard = registry.write().await;
    guard
        .promote_trust(name, target_trust)
        .await
        .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
    Ok(guard
        .find_by_name(name)
        .map(|skill| skill.source_tier.to_string())
        .unwrap_or_else(|| "community".to_string()))
}

async fn remove_skill_from_registry(
    registry: &Arc<tokio::sync::RwLock<SkillRegistry>>,
    name: &str,
) -> Result<(), ToolError> {
    // Hold the write lock for the entire validate/delete/commit sequence so a
    // concurrent install cannot land files that this remove deletes afterward.
    let mut guard = registry.write().await;
    let skill_path = guard
        .validate_remove(name)
        .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
    crate::skills::registry::SkillRegistry::delete_skill_files(&skill_path)
        .await
        .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
    guard
        .commit_remove(name)
        .map_err(|err| ToolError::ExecutionFailed(err.to_string()))
}

mod audit;
mod check;
mod hosts;
mod inspect;
mod install;
mod list;
mod publish;
mod read;
mod reload;
mod remove;
mod search;
mod snapshot;
mod tap;
mod update;

pub use audit::*;
pub use check::*;
pub use hosts::*;
pub use inspect::*;
pub use install::*;
pub use list::*;
pub use publish::*;
pub use read::*;
pub use reload::*;
pub use remove::*;
pub use search::*;
pub use snapshot::*;
pub use tap::*;
pub use update::*;

#[cfg(test)]
mod tests;
