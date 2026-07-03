//! Skill tool policy: search.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use thinclaw_tools_core::{ApprovalRequirement, Tool, ToolError, ToolOutput};
use thinclaw_types::JobContext;

use crate::ports::{
    SkillSearchToolHostPort, ToolSkillSearchCatalogEntry, ToolSkillSearchLocalEntry,
    ToolSkillSearchRemoteEntry, ToolSkillSearchRequest, ToolSkillSearchResult,
    tool_scope_from_job_context,
};

use super::*;

pub fn skill_search_parameters_schema() -> serde_json::Value {
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

fn tool_skill_search_catalog_entry(entry: &ToolSkillSearchCatalogEntry) -> serde_json::Value {
    skill_search_catalog_entry(
        &entry.slug,
        &entry.name,
        &entry.description,
        &entry.version,
        entry.score,
        entry.installed,
        entry.stars,
        entry.downloads,
        entry.owner.as_deref(),
    )
}

fn tool_skill_search_remote_entry(entry: &ToolSkillSearchRemoteEntry) -> serde_json::Value {
    skill_search_remote_entry(
        &entry.slug,
        &entry.name,
        &entry.description,
        &entry.version,
        &entry.source,
        &entry.source_label,
        &entry.source_ref,
        entry.manifest_url.as_deref(),
        entry.manifest_digest.as_deref(),
        entry.repo.as_deref(),
        entry.path.as_deref(),
        entry.branch.as_deref(),
        &entry.trust_level,
    )
}

fn tool_skill_search_local_entry(entry: &ToolSkillSearchLocalEntry) -> serde_json::Value {
    skill_search_local_entry(
        &entry.name,
        &entry.description,
        &entry.trust,
        &entry.source_tier,
    )
}

fn tool_skill_search_output(
    source_filter: &str,
    result: &ToolSkillSearchResult,
) -> serde_json::Value {
    skill_search_output(
        source_filter,
        result
            .catalog
            .iter()
            .map(tool_skill_search_catalog_entry)
            .collect(),
        result
            .remote
            .iter()
            .map(tool_skill_search_remote_entry)
            .collect(),
        result
            .local
            .iter()
            .map(tool_skill_search_local_entry)
            .collect(),
        &result.registry_url,
        result.catalog_error.clone(),
    )
}

pub struct SkillSearchHostTool {
    host: Arc<dyn SkillSearchToolHostPort>,
}

impl SkillSearchHostTool {
    pub fn new(host: Arc<dyn SkillSearchToolHostPort>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Tool for SkillSearchHostTool {
    fn name(&self) -> &str {
        "skill_search"
    }

    fn description(&self) -> &str {
        "Search for skills in the ClawHub catalog and among locally loaded skills."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_search_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(&ctx.metadata, self.name())?;
        let start = Instant::now();
        let parsed = parse_skill_search_params(&params)?;
        let result = self
            .host
            .search_skills(ToolSkillSearchRequest {
                scope: tool_scope_from_job_context(ctx),
                query: parsed.query,
                source: parsed.source.clone(),
            })
            .await
            .map_err(tool_host_error)?;
        Ok(ToolOutput::success(
            tool_skill_search_output(&parsed.source, &result),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }
}

pub fn parse_skill_search_params(
    params: &serde_json::Value,
) -> Result<SkillSearchParams, ToolError> {
    Ok(SkillSearchParams {
        query: required_str(params, "query")?.to_string(),
        source: params
            .get("source")
            .and_then(|value| value.as_str())
            .unwrap_or("all")
            .to_ascii_lowercase(),
    })
}

pub fn skill_search_local_entry(
    name: &str,
    description: &str,
    trust: &str,
    source_tier: &str,
) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "description": description,
        "trust": trust,
        "source_tier": source_tier,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn skill_search_catalog_entry(
    slug: &str,
    name: &str,
    description: &str,
    version: &str,
    score: f64,
    installed: bool,
    stars: Option<u64>,
    downloads: Option<u64>,
    owner: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "slug": slug,
        "name": name,
        "description": description,
        "version": version,
        "score": score,
        "installed": installed,
        "stars": stars,
        "downloads": downloads,
        "owner": owner,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn skill_search_remote_entry(
    slug: &str,
    name: &str,
    description: &str,
    version: &str,
    source: &str,
    source_label: &str,
    source_ref: &str,
    manifest_url: Option<&str>,
    manifest_digest: Option<&str>,
    repo: Option<&str>,
    path: Option<&str>,
    branch: Option<&str>,
    trust_level: &str,
) -> serde_json::Value {
    serde_json::json!({
        "slug": slug,
        "name": name,
        "description": description,
        "version": version,
        "source": source,
        "source_label": source_label,
        "source_ref": source_ref,
        "manifest_url": manifest_url,
        "manifest_digest": manifest_digest,
        "repo": repo,
        "path": path,
        "branch": branch,
        "trust_level": trust_level,
    })
}

pub fn skill_search_output(
    source_filter: &str,
    catalog_json: Vec<serde_json::Value>,
    remote_json: Vec<serde_json::Value>,
    local_matches: Vec<serde_json::Value>,
    registry_url: &str,
    catalog_error: Option<String>,
) -> serde_json::Value {
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

    let mut output = match source_filter {
        "clawhub" => serde_json::json!({
            "catalog": catalog_json,
            "catalog_count": catalog_json.len(),
            "registry_url": registry_url,
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
            "registry_url": registry_url,
        }),
    };
    if let Some(err) = catalog_error {
        output["catalog_error"] = serde_json::Value::String(err);
    }
    output
}
