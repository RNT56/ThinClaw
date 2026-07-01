//! Skill tool policy: lifecycle.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use thinclaw_tools_core::{ApprovalRequirement, Tool, ToolError, ToolOutput};
use thinclaw_types::JobContext;

use crate::ports::{SkillToolHostPort, tool_scope_from_job_context};

use super::*;

pub struct SkillReloadHostTool {
    host: Arc<dyn SkillToolHostPort>,
}

impl SkillReloadHostTool {
    pub fn new(host: Arc<dyn SkillToolHostPort>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Tool for SkillReloadHostTool {
    fn name(&self) -> &str {
        "skill_reload"
    }

    fn description(&self) -> &str {
        "Reload a skill (or all skills) from disk after editing SKILL.md files. \
         Use after making on-disk changes so they take effect immediately without restarting. \
         Provide a skill name to reload just that skill, or set all=true to rediscover all skills."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_reload_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(&ctx.metadata, self.name())?;
        let start = Instant::now();
        let parsed = parse_skill_reload_params(&params);
        let scope = tool_scope_from_job_context(ctx);

        if parsed.all {
            let loaded = self
                .host
                .reload_skills(scope, None)
                .await
                .map_err(tool_host_error)?
                .into_iter()
                .map(|summary| summary.name)
                .collect::<Vec<_>>();
            return Ok(ToolOutput::success(
                skill_reload_all_output(loaded),
                start.elapsed(),
            ));
        }

        let name = parsed.name.ok_or_else(|| {
            ToolError::InvalidParameters("missing required parameter: name".to_string())
        })?;
        let mut loaded = self
            .host
            .reload_skills(scope, Some(name.clone()))
            .await
            .map_err(tool_host_error)?;
        let reloaded_name = loaded.pop().map(|summary| summary.name).unwrap_or(name);
        Ok(ToolOutput::success(
            skill_reload_output(&reloaded_name),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

pub struct SkillSnapshotHostTool {
    host: Arc<dyn SkillToolHostPort>,
}

impl SkillSnapshotHostTool {
    pub fn new(host: Arc<dyn SkillToolHostPort>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Tool for SkillSnapshotHostTool {
    fn name(&self) -> &str {
        "skill_snapshot"
    }

    fn description(&self) -> &str {
        "Write a JSON snapshot of loaded skills, hashes, and provenance tiers to the local skills state directory."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_snapshot_parameters_schema()
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(&ctx.metadata, self.name())?;
        let start = Instant::now();
        let result = self
            .host
            .snapshot_skills(tool_scope_from_job_context(ctx))
            .await
            .map_err(tool_host_error)?;
        Ok(ToolOutput::success(
            skill_snapshot_output(&result.path, result.count),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

pub struct SkillRemoveHostTool {
    host: Arc<dyn SkillToolHostPort>,
}

impl SkillRemoveHostTool {
    pub fn new(host: Arc<dyn SkillToolHostPort>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl Tool for SkillRemoveHostTool {
    fn name(&self) -> &str {
        "skill_remove"
    }

    fn description(&self) -> &str {
        "Remove an installed skill by name. Only user-installed skills can be removed."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_remove_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(&ctx.metadata, self.name())?;
        let start = Instant::now();
        let name = parse_skill_name_param(&params)?;
        let result = self
            .host
            .remove_skill(tool_scope_from_job_context(ctx), name)
            .await
            .map_err(tool_host_error)?;
        Ok(ToolOutput::success(
            skill_remove_output(&result.name),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

pub fn skill_snapshot_parameters_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {}
    })
}

pub fn skill_snapshot_document(
    generated_at: String,
    skills: Vec<serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "generated_at": generated_at,
        "skills": skills,
    })
}

pub fn skill_snapshot_entry(
    name: &str,
    version: &str,
    trust: &str,
    source_tier: &str,
    content_hash: &str,
    source_path: Option<String>,
) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "version": version,
        "trust": trust,
        "source_tier": source_tier,
        "content_hash": content_hash,
        "source_path": source_path,
    })
}

pub fn skill_snapshot_output(path: &str, count: usize) -> serde_json::Value {
    serde_json::json!({
        "path": path,
        "count": count,
    })
}

pub fn skill_remove_parameters_schema() -> serde_json::Value {
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

pub fn skill_remove_output(name: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "status": "removed",
        "message": format!("Skill '{}' has been removed.", name),
    })
}

pub fn skill_reload_parameters_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "description": "Name of the specific skill to reload from disk. Required unless all=true."
            },
            "all": {
                "type": "boolean",
                "description": "When true, reload ALL skills (full re-discovery). Use after adding new skill files on disk.",
                "default": false
            }
        }
    })
}

pub fn parse_skill_reload_params(params: &serde_json::Value) -> SkillReloadParams {
    SkillReloadParams {
        name: params
            .get("name")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        all: params
            .get("all")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
    }
}

pub fn skill_reload_all_output(loaded: Vec<String>) -> serde_json::Value {
    serde_json::json!({
        "status": "reloaded_all",
        "skills": loaded,
        "count": loaded.len(),
        "message": format!("Reloaded all skills: {}", loaded.join(", ")),
    })
}

pub fn skill_reload_output(name: &str) -> serde_json::Value {
    serde_json::json!({
        "status": "reloaded",
        "name": name,
        "message": format!(
            "Skill '{}' has been reloaded from disk. \
             Updated keywords, descriptions, and prompt content are now active.",
            name
        ),
    })
}
