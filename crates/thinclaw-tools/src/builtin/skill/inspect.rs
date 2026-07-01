//! Skill tool policy: inspect.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use thinclaw_tools_core::{ApprovalRequirement, Tool, ToolError, ToolOutput};
use thinclaw_types::JobContext;

use crate::ports::{SkillToolHostPort, ToolSkillRead, tool_scope_from_job_context};

use super::*;

pub fn skill_inspect_parameters_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "description": "Name of the loaded skill to inspect."
            },
            "include_content": {
                "type": "boolean",
                "description": "Include full prompt content in the response.",
                "default": false
            },
            "include_files": {
                "type": "boolean",
                "description": "Include regular publishable files in the skill directory.",
                "default": true
            },
            "audit": {
                "type": "boolean",
                "description": "Run the quarantine scanner over the skill prompt.",
                "default": true
            }
        },
        "required": ["name"]
    })
}

pub fn parse_skill_inspect_params(
    params: &serde_json::Value,
) -> Result<SkillInspectParams, ToolError> {
    Ok(SkillInspectParams {
        name: required_str(params, "name")?.to_string(),
        include_content: params
            .get("include_content")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        include_files: params
            .get("include_files")
            .and_then(|value| value.as_bool())
            .unwrap_or(true),
        audit: params
            .get("audit")
            .and_then(|value| value.as_bool())
            .unwrap_or(true),
    })
}

pub fn skill_read_parameters_schema() -> serde_json::Value {
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

pub fn skill_read_output(
    name: &str,
    version: &str,
    description: &str,
    trust: &str,
    source_tier: &str,
    content: &str,
) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "version": version,
        "description": description,
        "trust": trust,
        "source_tier": source_tier,
        "content": content,
    })
}

pub fn skill_inventory_error_output(error: &str) -> serde_json::Value {
    serde_json::json!({
        "error": error,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn skill_inspect_output(
    name: &str,
    version: &str,
    description: &str,
    activation: serde_json::Value,
    metadata: serde_json::Value,
    trust: &str,
    source_tier: &str,
    source: serde_json::Value,
    content_hash: &str,
    prompt_tokens_approx: usize,
    provenance_lock: Option<serde_json::Value>,
    findings: Vec<serde_json::Value>,
    files: Vec<serde_json::Value>,
    content: Option<&str>,
) -> serde_json::Value {
    let finding_count = findings.len();
    let file_count = files.len();
    let mut output = serde_json::json!({
        "name": name,
        "version": version,
        "description": description,
        "activation": activation,
        "metadata": metadata,
        "trust": trust,
        "source_tier": source_tier,
        "source": source,
        "content_hash": content_hash,
        "prompt_tokens_approx": prompt_tokens_approx,
        "provenance_lock": provenance_lock,
        "finding_count": finding_count,
        "findings": findings,
        "inventory": {
            "file_count": file_count,
            "files": files.clone(),
        },
        "files": files,
    });
    if let Some(content) = content {
        output["content"] = serde_json::Value::String(content.to_string());
    }
    output
}

fn tool_skill_read_output(read: &ToolSkillRead) -> serde_json::Value {
    skill_read_output(
        &read.name,
        &read.version,
        &read.description,
        tool_skill_trust_label(read.trust),
        &read.source_tier,
        &read.content,
    )
}

pub struct SkillReadHostTool {
    host: Arc<dyn SkillToolHostPort>,
}

impl SkillReadHostTool {
    pub fn new(host: Arc<dyn SkillToolHostPort>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Tool for SkillReadHostTool {
    fn name(&self) -> &str {
        "skill_read"
    }

    fn description(&self) -> &str {
        "Read a skill's full instructions by name. Use when you need detailed guidance for a specific skill."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_read_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let name = parse_skill_name_param(&params)?;
        ensure_skill_allowed(&ctx.metadata, &name)?;
        let output = self
            .host
            .read_skill(tool_scope_from_job_context(ctx), name)
            .await
            .map_err(tool_host_error)?;
        Ok(ToolOutput::success(
            tool_skill_read_output(&output),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }
}

pub struct SkillInspectHostTool {
    host: Arc<dyn SkillToolHostPort>,
}

impl SkillInspectHostTool {
    pub fn new(host: Arc<dyn SkillToolHostPort>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Tool for SkillInspectHostTool {
    fn name(&self) -> &str {
        "skill_inspect"
    }

    fn description(&self) -> &str {
        "Inspect one loaded skill with metadata, provenance, files, and optional audit findings."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_inspect_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let parsed = parse_skill_inspect_params(&params)?;
        ensure_skill_allowed(&ctx.metadata, &parsed.name)?;
        let output = self
            .host
            .inspect_skill(
                tool_scope_from_job_context(ctx),
                parsed.name,
                parsed.include_content,
                parsed.include_files,
                parsed.audit,
            )
            .await
            .map_err(tool_host_error)?;
        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }
}
