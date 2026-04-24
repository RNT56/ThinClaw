//! Agent-callable tools for managing skills (prompt-level extensions).
//!
//! Five tools for discovering, reading, installing, listing, and removing skills
//! entirely through conversation, following the extension_tools pattern.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;

use crate::context::JobContext;
use crate::settings::SkillTapTrustLevel;
use crate::skills::catalog::SkillCatalog;
use crate::skills::quarantine::{
    QuarantineManager, QuarantinedSkill, SecurityFinding, SkillContent, SkillProvenance,
};
use crate::skills::registry::SkillRegistry;
use crate::skills::{RemoteSkillHub, SkillSource, SkillTrust};
use crate::tools::ToolRegistry;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput, require_str};

fn restricted_skill_names(ctx: &JobContext) -> Option<std::collections::HashSet<String>> {
    ToolRegistry::metadata_string_list(&ctx.metadata, "allowed_skills")
        .map(|skills| skills.into_iter().collect())
}

fn ensure_skill_allowed(ctx: &JobContext, skill_name: &str) -> Result<(), ToolError> {
    if ToolRegistry::skill_name_allowed_by_metadata(&ctx.metadata, skill_name) {
        Ok(())
    } else {
        Err(ToolError::ExecutionFailed(format!(
            "Skill '{}' is not allowed in this agent context.",
            skill_name
        )))
    }
}

fn ensure_skill_admin_available(ctx: &JobContext, tool_name: &str) -> Result<(), ToolError> {
    if ToolRegistry::metadata_string_list(&ctx.metadata, "allowed_skills").is_some() {
        Err(ToolError::ExecutionFailed(format!(
            "Tool '{}' is not available when the current agent is restricted to a specific skill allowlist.",
            tool_name
        )))
    } else {
        Ok(())
    }
}

fn summarize_findings(findings: &[SecurityFinding]) -> String {
    findings
        .iter()
        .map(|finding| {
            format!(
                "{} ({:?}): {}",
                finding.kind, finding.severity, finding.excerpt
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn findings_require_approval(
    trust_level: SkillTapTrustLevel,
    findings: &[SecurityFinding],
) -> bool {
    trust_level == SkillTapTrustLevel::Community && !findings.is_empty()
}

fn source_path_for_skill(skill: &crate::skills::LoadedSkill) -> Option<PathBuf> {
    match &skill.source {
        SkillSource::Workspace(path)
        | SkillSource::User(path)
        | SkillSource::Bundled(path)
        | SkillSource::External(path) => Some(path.clone()),
    }
}

async fn read_skill_provenance(skill_dir: &Path) -> Result<SkillProvenance, ToolError> {
    let raw = tokio::fs::read_to_string(skill_dir.join(".thinclaw-skill-lock.json"))
        .await
        .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
    serde_json::from_str(&raw).map_err(|err| ToolError::ExecutionFailed(err.to_string()))
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
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Name of the skill to read (from skill_list or the Skills section)"
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let name = require_str(&params, "name")?;
        ensure_skill_allowed(ctx, name)?;

        let guard = self.registry.read().await;
        let skill = guard
            .skills()
            .iter()
            .find(|s| s.manifest.name.eq_ignore_ascii_case(name));

        match skill {
            Some(s) => {
                let output = serde_json::json!({
                    "name": s.manifest.name,
                    "version": s.manifest.version,
                    "description": s.manifest.description,
                    "trust": s.trust.to_string(),
                    "source_tier": s.source_tier.to_string(),
                    "content": s.prompt_content,
                });
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
        serde_json::json!({
            "type": "object",
            "properties": {
                "verbose": {
                    "type": "boolean",
                    "description": "Include extra detail (tags, content_hash, version)",
                    "default": false
                }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let verbose = params
            .get("verbose")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
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
                let mut entry = serde_json::json!({
                    "name": s.manifest.name,
                    "description": s.manifest.description,
                    "trust": s.trust.to_string(),
                    "source_tier": s.source_tier.to_string(),
                    "source": format!("{:?}", s.source),
                    "keywords": s.manifest.activation.keywords,
                });

                if verbose && let Some(obj) = entry.as_object_mut() {
                    obj.insert(
                        "version".to_string(),
                        serde_json::Value::String(s.manifest.version.clone()),
                    );
                    obj.insert(
                        "tags".to_string(),
                        serde_json::json!(s.manifest.activation.tags),
                    );
                    obj.insert(
                        "content_hash".to_string(),
                        serde_json::Value::String(s.content_hash.clone()),
                    );
                    obj.insert(
                        "max_context_tokens".to_string(),
                        serde_json::json!(s.manifest.activation.max_context_tokens),
                    );
                    if let Some(openclaw) = s
                        .manifest
                        .metadata
                        .as_ref()
                        .and_then(|metadata| metadata.openclaw.as_ref())
                    {
                        obj.insert(
                            "provenance".to_string(),
                            serde_json::json!(openclaw.provenance.clone()),
                        );
                        obj.insert(
                            "lifecycle_status".to_string(),
                            serde_json::json!(openclaw.lifecycle_status.clone()),
                        );
                        obj.insert(
                            "outcome_score".to_string(),
                            serde_json::json!(openclaw.outcome_score),
                        );
                        obj.insert(
                            "reuse_count".to_string(),
                            serde_json::json!(openclaw.reuse_count),
                        );
                        obj.insert(
                            "activation_reason".to_string(),
                            serde_json::json!(openclaw.activation_reason.clone()),
                        );
                    }
                }

                entry
            })
            .collect();

        let output = serde_json::json!({
            "skills": skills,
            "count": skills.len(),
        });

        Ok(ToolOutput::success(output, start.elapsed()))
    }
}

// ── skill_search ────────────────────────────────────────────────────────

pub struct SkillSearchTool {
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
    catalog: Arc<SkillCatalog>,
    remote_hub: Option<Arc<RemoteSkillHub>>,
}

impl SkillSearchTool {
    pub fn new(
        registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
        catalog: Arc<SkillCatalog>,
        remote_hub: Option<Arc<RemoteSkillHub>>,
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
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query (name, keyword, or description fragment)"
                },
                "source": {
                    "type": "string",
                    "enum": ["all", "clawhub", "github", "well_known"],
                    "description": "Optional source filter.",
                    "default": "all"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let query = require_str(&params, "query")?;
        let source_filter = params
            .get("source")
            .and_then(|value| value.as_str())
            .unwrap_or("all")
            .to_ascii_lowercase();

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
                    serde_json::json!({
                        "name": s.manifest.name,
                        "description": s.manifest.description,
                        "trust": s.trust.to_string(),
                        "source_tier": s.source_tier.to_string(),
                    })
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
                serde_json::json!({
                    "slug": entry.slug,
                    "name": entry.name,
                    "description": entry.description,
                    "version": entry.version,
                    "score": entry.score,
                    "installed": is_installed,
                    "stars": entry.stars,
                    "downloads": entry.downloads,
                    "owner": entry.owner,
                })
            })
            .collect();

        let remote_json = if let Some(ref hub) = self.remote_hub {
            hub.search(query)
                .await
                .into_iter()
                .map(|entry| {
                    serde_json::json!({
                        "slug": entry.slug,
                        "name": entry.name,
                        "description": entry.description,
                        "version": entry.version,
                        "source": entry.source_adapter,
                        "source_label": entry.source_label,
                        "source_ref": entry.source_ref,
                        "manifest_url": entry.manifest_url,
                        "manifest_digest": entry.manifest_digest,
                        "repo": entry.repo,
                        "path": entry.path,
                        "branch": entry.branch,
                        "trust_level": format!("{:?}", entry.trust_level).to_lowercase(),
                    })
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let github_json: Vec<serde_json::Value> = remote_json
            .iter()
            .filter(|entry| entry.get("source").and_then(|v| v.as_str()) == Some("github_tap"))
            .cloned()
            .collect();
        let well_known_json: Vec<serde_json::Value> = remote_json
            .iter()
            .filter(|entry| entry.get("source").and_then(|v| v.as_str()) == Some("well_known"))
            .cloned()
            .collect();

        let mut output = match source_filter.as_str() {
            "clawhub" => serde_json::json!({
                "catalog": catalog_json,
                "catalog_count": catalog_json.len(),
                "registry_url": self.catalog.registry_url(),
            }),
            "github" => serde_json::json!({
                "github": github_json,
                "github_count": github_json.len(),
            }),
            "well_known" => serde_json::json!({
                "well_known": well_known_json,
                "well_known_count": well_known_json.len(),
            }),
            _ => serde_json::json!({
                "catalog": catalog_json,
                "catalog_count": catalog_json.len(),
                "remote": remote_json,
                "remote_count": remote_json.len(),
                "github": github_json,
                "github_count": github_json.len(),
                "well_known": well_known_json,
                "well_known_count": well_known_json.len(),
                "installed": local_matches,
                "installed_count": local_matches.len(),
                "registry_url": self.catalog.registry_url(),
            }),
        };
        if let Some(err) = catalog_error {
            output["catalog_error"] = serde_json::Value::String(err);
        }

        Ok(ToolOutput::success(output, start.elapsed()))
    }
}

// ── skill_install ───────────────────────────────────────────────────────

pub struct SkillInstallTool {
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
    catalog: Arc<SkillCatalog>,
    remote_hub: Option<Arc<RemoteSkillHub>>,
    quarantine: Arc<QuarantineManager>,
}

impl SkillInstallTool {
    pub fn new(
        registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
        catalog: Arc<SkillCatalog>,
        remote_hub: Option<Arc<RemoteSkillHub>>,
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
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Skill name or slug (from search results). Used as the catalog lookup key if neither url nor content is provided."
                },
                "url": {
                    "type": "string",
                    "description": "Optional: direct URL to a SKILL.md file (skips catalog lookup)"
                },
                "content": {
                    "type": "string",
                    "description": "Optional: raw SKILL.md content to install directly (skips fetch)"
                },
                "force": {
                    "type": "boolean",
                    "description": "If true, removes the existing skill before installing the new version (update/upgrade)",
                    "default": false
                },
                "approve_risky": {
                    "type": "boolean",
                    "description": "Approve installation even when the quarantine scan finds risky patterns in a community skill.",
                    "default": false
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let name = require_str(&params, "name")?;
        let force = params
            .get("force")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let approve_risky = params
            .get("approve_risky")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

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

        let (skill_name, loaded_skill, findings) = if let Some(remote) = external_content {
            let quarantined = self
                .quarantine
                .quarantine_skill(&skill_name_from_parse, &remote)
                .await
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
            let findings = self.quarantine.scan_quarantined(&quarantined);

            if findings_require_approval(remote.trust_level, &findings) && !approve_risky {
                self.quarantine.cleanup(&quarantined).await;
                return Err(ToolError::ExecutionFailed(format!(
                    "Skill '{}' was quarantined with findings: {}. Re-run with approve_risky=true to install anyway.",
                    skill_name_from_parse,
                    summarize_findings(&findings)
                )));
            }

            let installed_dir = self
                .quarantine
                .approve_and_install(&quarantined, &user_dir, &findings)
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
            (loaded.0, loaded.1, findings)
        } else {
            let loaded = crate::skills::registry::SkillRegistry::prepare_install_to_disk(
                &user_dir,
                &skill_name_from_parse,
                &normalized,
            )
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
            (loaded.0, loaded.1, Vec::new())
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

        let action = if force { "updated" } else { "installed" };
        let output = serde_json::json!({
            "name": installed_name,
            "status": action,
            "trust": "installed",
            "findings": findings.iter().map(|finding| serde_json::json!({
                "kind": finding.kind,
                "severity": format!("{:?}", finding.severity).to_lowercase(),
                "excerpt": finding.excerpt,
            })).collect::<Vec<_>>(),
            "message": format!(
                "Skill '{}' {} successfully. It will activate when matching keywords are detected.",
                installed_name, action
            ),
        });

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
    let parsed = url::Url::parse(url_str)
        .map_err(|e| ToolError::ExecutionFailed(format!("Invalid URL '{}': {}", url_str, e)))?;

    // Require HTTPS
    if parsed.scheme() != "https" {
        return Err(ToolError::ExecutionFailed(format!(
            "Only HTTPS URLs are allowed for skill fetching, got scheme '{}'",
            parsed.scheme()
        )));
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| ToolError::ExecutionFailed("URL has no host".to_string()))?;

    // Check if host is an IP address and reject private ranges.
    // Unwrap IPv4-mapped IPv6 addresses (e.g. ::ffff:192.168.1.1) to catch
    // SSRF bypasses that encode private IPv4 addresses as IPv6.
    if let Ok(raw_ip) = host.parse::<std::net::IpAddr>() {
        let ip = match raw_ip {
            std::net::IpAddr::V6(v6) => v6
                .to_ipv4_mapped()
                .map(std::net::IpAddr::V4)
                .unwrap_or(std::net::IpAddr::V6(v6)),
            other => other,
        };
        if ip.is_loopback() || ip.is_unspecified() || is_private_ip(&ip) || is_link_local_ip(&ip) {
            return Err(ToolError::ExecutionFailed(format!(
                "URL points to a private/loopback/link-local address: {}",
                host
            )));
        }
    }

    // Reject common internal hostnames
    let host_lower = host.to_lowercase();
    if host_lower == "localhost"
        || host_lower == "metadata.google.internal"
        || host_lower.ends_with(".internal")
        || host_lower.ends_with(".local")
    {
        return Err(ToolError::ExecutionFailed(format!(
            "URL points to an internal hostname: {}",
            host
        )));
    }

    Ok(())
}

fn is_private_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, 169.254.0.0/16
            v4.is_private() || v4.is_link_local()
        }
        std::net::IpAddr::V6(v6) => {
            // Unique local (fc00::/7)
            let segments = v6.segments();
            (segments[0] & 0xfe00) == 0xfc00
        }
    }
}

fn is_link_local_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => v4.is_link_local(),
        std::net::IpAddr::V6(v6) => {
            // fe80::/10
            let segments = v6.segments();
            (segments[0] & 0xffc0) == 0xfe80
        }
    }
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
    use flate2::read::DeflateDecoder;
    use std::io::Read;

    // SKILL.md files should never be larger than 1 MB.
    const MAX_DECOMPRESSED: usize = 1_024 * 1_024;

    let mut offset = 0;
    while offset + 30 <= data.len() {
        // Local file header signature = PK\x03\x04
        if data[offset..offset + 4] != [0x50, 0x4B, 0x03, 0x04] {
            break;
        }

        let compression = u16::from_le_bytes([data[offset + 8], data[offset + 9]]);
        let compressed_size = u32::from_le_bytes([
            data[offset + 18],
            data[offset + 19],
            data[offset + 20],
            data[offset + 21],
        ]) as usize;
        let uncompressed_size = u32::from_le_bytes([
            data[offset + 22],
            data[offset + 23],
            data[offset + 24],
            data[offset + 25],
        ]) as usize;
        let name_len = u16::from_le_bytes([data[offset + 26], data[offset + 27]]) as usize;
        let extra_len = u16::from_le_bytes([data[offset + 28], data[offset + 29]]) as usize;

        let name_start = offset + 30;
        let name_end = name_start + name_len;
        if name_end > data.len() {
            break;
        }
        let file_name = std::str::from_utf8(&data[name_start..name_end]).unwrap_or("");

        let data_start = name_end
            .checked_add(extra_len)
            .ok_or_else(|| ToolError::ExecutionFailed("ZIP header offset overflow".to_string()))?;
        let data_end = data_start
            .checked_add(compressed_size)
            .ok_or_else(|| ToolError::ExecutionFailed("ZIP header size overflow".to_string()))?;

        if file_name == "SKILL.md" {
            if data_end > data.len() {
                return Err(ToolError::ExecutionFailed(
                    "ZIP archive truncated".to_string(),
                ));
            }

            if uncompressed_size > MAX_DECOMPRESSED {
                return Err(ToolError::ExecutionFailed(
                    "ZIP entry too large to decompress safely".to_string(),
                ));
            }

            let raw = &data[data_start..data_end];
            let decompressed = match compression {
                0 => raw.to_vec(), // Store
                8 => {
                    // Deflate -- wrap with a read limit to guard against ZIP bombs
                    // where the declared size is small but decompressed output is huge.
                    let mut decoder = DeflateDecoder::new(raw).take(MAX_DECOMPRESSED as u64);
                    let mut buf = Vec::with_capacity(uncompressed_size.min(MAX_DECOMPRESSED));
                    decoder.read_to_end(&mut buf).map_err(|e| {
                        ToolError::ExecutionFailed(format!("Failed to decompress SKILL.md: {}", e))
                    })?;
                    buf
                }
                other => {
                    return Err(ToolError::ExecutionFailed(format!(
                        "Unsupported ZIP compression method: {}",
                        other
                    )));
                }
            };

            return String::from_utf8(decompressed).map_err(|e| {
                ToolError::ExecutionFailed(format!("SKILL.md in archive is not valid UTF-8: {}", e))
            });
        }

        // Skip to next entry
        offset = data_end;
    }

    Err(ToolError::ExecutionFailed(
        "ZIP archive does not contain SKILL.md".to_string(),
    ))
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
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Optional single skill name to audit. Omit to audit all loaded skills."
                }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let target_name = params.get("name").and_then(|value| value.as_str());
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
                let findings = self.quarantine.scan_quarantined(&QuarantinedSkill {
                    skill_name: skill.manifest.name.clone(),
                    dir: source_path.clone(),
                    content: SkillContent {
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
                });

                serde_json::json!({
                    "name": skill.manifest.name,
                    "trust": skill.trust.to_string(),
                    "source_tier": skill.source_tier.to_string(),
                    "source_path": source_path.display().to_string(),
                    "finding_count": findings.len(),
                    "findings": findings.iter().map(|finding| serde_json::json!({
                        "kind": finding.kind,
                        "severity": format!("{:?}", finding.severity).to_lowercase(),
                        "excerpt": finding.excerpt,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect::<Vec<_>>();

        if audited.is_empty() {
            return Err(ToolError::ExecutionFailed(
                "No matching skills found to audit".to_string(),
            ));
        }

        let total_findings = audited
            .iter()
            .map(|entry| {
                entry
                    .get("finding_count")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0)
            })
            .sum::<u64>();

        Ok(ToolOutput::success(
            serde_json::json!({
                "audited": audited,
                "audited_count": audited.len(),
                "total_findings": total_findings,
            }),
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
        remote_hub: Option<Arc<RemoteSkillHub>>,
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
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Skill name to update."
                },
                "approve_risky": {
                    "type": "boolean",
                    "description": "Approve update even when the quarantine scan reports risky patterns.",
                    "default": false
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let name = require_str(&params, "name")?;
        let approve_risky = params
            .get("approve_risky")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);

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

        let mut install_params = serde_json::json!({
            "name": installed_name,
            "force": true,
            "approve_risky": approve_risky,
        });

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
                install_params["url"] = serde_json::Value::String(url);
            }
            _ => {
                if let Some(url) = provenance
                    .source_url
                    .clone()
                    .or(provenance.manifest_url.clone())
                {
                    install_params["url"] = serde_json::Value::String(url);
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
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let guard = self.registry.read().await;

        let snapshot = serde_json::json!({
            "generated_at": Utc::now().to_rfc3339(),
            "skills": guard.skills().iter().map(|skill| serde_json::json!({
                "name": skill.manifest.name,
                "version": skill.manifest.version,
                "trust": skill.trust.to_string(),
                "source_tier": skill.source_tier.to_string(),
                "content_hash": skill.content_hash,
                "source_path": source_path_for_skill(skill).map(|path| path.display().to_string()),
            })).collect::<Vec<_>>(),
        });

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
            serde_json::json!({
                "path": snapshot_path.display().to_string(),
                "count": guard.count(),
            }),
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
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Skill name to move between trust ceilings."
                },
                "target_trust": {
                    "type": "string",
                    "enum": ["installed", "trusted"],
                    "description": "Target trust ceiling."
                }
            },
            "required": ["name", "target_trust"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let name = require_str(&params, "name")?;
        let target_trust = match require_str(&params, "target_trust")?
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "installed" => SkillTrust::Installed,
            "trusted" => SkillTrust::Trusted,
            other => {
                return Err(ToolError::InvalidParameters(format!(
                    "Unsupported target_trust '{}'",
                    other
                )));
            }
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
            serde_json::json!({
                "name": name,
                "trust": target_trust.to_string(),
                "source_tier": source_tier,
                "status": "updated",
            }),
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
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Name of the skill to remove"
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let name = require_str(&params, "name")?;

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

        let output = serde_json::json!({
            "name": name,
            "status": "removed",
            "message": format!("Skill '{}' has been removed.", name),
        });

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
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Name of the specific skill to reload from disk. \
                                   Required unless all=true."
                },
                "all": {
                    "type": "boolean",
                    "description": "When true, reload ALL skills (full re-discovery). \
                                   Use after adding new skill files on disk.",
                    "default": false
                }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let reload_all = params.get("all").and_then(|v| v.as_bool()).unwrap_or(false);

        if reload_all {
            let mut guard = self.registry.write().await;
            let loaded = guard.reload().await;
            let output = serde_json::json!({
                "status": "reloaded_all",
                "skills": loaded,
                "count": loaded.len(),
                "message": format!("Reloaded all skills: {}", loaded.join(", ")),
            });
            return Ok(ToolOutput::success(output, start.elapsed()));
        }

        // Single-skill reload
        let name = require_str(&params, "name")?;
        let mut guard = self.registry.write().await;

        match guard.reload_skill(name).await {
            Ok(reloaded_name) => {
                let output = serde_json::json!({
                    "status": "reloaded",
                    "name": reloaded_name,
                    "message": format!(
                        "Skill '{}' has been reloaded from disk. \
                         Updated keywords, descriptions, and prompt content are now active.",
                        reloaded_name
                    ),
                });
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
