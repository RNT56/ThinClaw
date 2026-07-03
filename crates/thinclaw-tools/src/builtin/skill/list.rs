//! Skill tool policy: list.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use thinclaw_tools_core::{ApprovalRequirement, Tool, ToolError, ToolOutput};
use thinclaw_types::JobContext;

use crate::ports::{
    SkillToolHostPort, ToolSkillQuery, ToolSkillSummary, tool_scope_from_job_context,
};

use super::*;

pub fn skill_list_parameters_schema() -> serde_json::Value {
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

pub fn parse_skill_list_params(params: &serde_json::Value) -> SkillListParams {
    SkillListParams {
        verbose: params
            .get("verbose")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
    }
}

pub fn skill_list_output(skills: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({
        "skills": skills,
        "count": skills.len(),
    })
}

pub fn skill_list_entry(
    name: &str,
    description: &str,
    trust: &str,
    source_tier: &str,
    source: &str,
    keywords: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "description": description,
        "trust": trust,
        "source_tier": source_tier,
        "source": source,
        "keywords": keywords,
    })
}

#[derive(Debug, Clone)]
pub struct SkillListVerboseFields {
    pub version: String,
    pub tags: serde_json::Value,
    pub content_hash: String,
    pub max_context_tokens: serde_json::Value,
    pub provenance: Option<serde_json::Value>,
    pub lifecycle_status: Option<serde_json::Value>,
    pub outcome_score: Option<serde_json::Value>,
    pub reuse_count: Option<serde_json::Value>,
    pub activation_reason: Option<serde_json::Value>,
}

pub fn add_skill_list_verbose_fields(
    entry: &mut serde_json::Value,
    fields: SkillListVerboseFields,
) {
    if let Some(obj) = entry.as_object_mut() {
        obj.insert(
            "version".to_string(),
            serde_json::Value::String(fields.version),
        );
        obj.insert("tags".to_string(), fields.tags);
        obj.insert(
            "content_hash".to_string(),
            serde_json::Value::String(fields.content_hash),
        );
        obj.insert("max_context_tokens".to_string(), fields.max_context_tokens);
        if let Some(value) = fields.provenance {
            obj.insert("provenance".to_string(), value);
        }
        if let Some(value) = fields.lifecycle_status {
            obj.insert("lifecycle_status".to_string(), value);
        }
        if let Some(value) = fields.outcome_score {
            obj.insert("outcome_score".to_string(), value);
        }
        if let Some(value) = fields.reuse_count {
            obj.insert("reuse_count".to_string(), value);
        }
        if let Some(value) = fields.activation_reason {
            obj.insert("activation_reason".to_string(), value);
        }
    }
}

fn metadata_string<'a>(metadata: &'a serde_json::Value, key: &str, fallback: &'a str) -> &'a str {
    metadata
        .get(key)
        .and_then(|value| value.as_str())
        .unwrap_or(fallback)
}

fn metadata_value_or(
    metadata: &serde_json::Value,
    key: &str,
    fallback: serde_json::Value,
) -> serde_json::Value {
    metadata.get(key).cloned().unwrap_or(fallback)
}

fn tool_skill_list_entry(summary: &ToolSkillSummary, verbose: bool) -> serde_json::Value {
    let metadata = &summary.metadata;
    let mut entry = skill_list_entry(
        &summary.name,
        summary.description.as_deref().unwrap_or_default(),
        tool_skill_trust_label(summary.trust),
        metadata_string(metadata, "source_tier", "community"),
        metadata_string(metadata, "source", ""),
        metadata_value_or(metadata, "keywords", serde_json::json!([])),
    );

    if verbose {
        add_skill_list_verbose_fields(
            &mut entry,
            SkillListVerboseFields {
                version: metadata_string(metadata, "version", "").to_string(),
                tags: metadata_value_or(metadata, "tags", serde_json::json!([])),
                content_hash: metadata_string(metadata, "content_hash", "").to_string(),
                max_context_tokens: metadata_value_or(
                    metadata,
                    "max_context_tokens",
                    serde_json::Value::Null,
                ),
                provenance: metadata.get("provenance").cloned(),
                lifecycle_status: metadata.get("lifecycle_status").cloned(),
                outcome_score: metadata.get("outcome_score").cloned(),
                reuse_count: metadata.get("reuse_count").cloned(),
                activation_reason: metadata.get("activation_reason").cloned(),
            },
        );
    }

    entry
}

pub struct SkillListHostTool {
    host: Arc<dyn SkillToolHostPort>,
}

impl SkillListHostTool {
    pub fn new(host: Arc<dyn SkillToolHostPort>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Tool for SkillListHostTool {
    fn name(&self) -> &str {
        "skill_list"
    }

    fn description(&self) -> &str {
        "List all loaded skills with their trust level, source, and activation keywords."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_list_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let parsed = parse_skill_list_params(&params);
        let summaries = self
            .host
            .list_skills(ToolSkillQuery {
                scope: tool_scope_from_job_context(ctx),
                query: None,
                source: None,
            })
            .await
            .map_err(tool_host_error)?;
        let skills = summaries
            .iter()
            .map(|summary| tool_skill_list_entry(summary, parsed.verbose))
            .collect();
        Ok(ToolOutput::success(
            skill_list_output(skills),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }
}
