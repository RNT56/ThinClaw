//! Skill tool policy: common.

use thinclaw_tools_core::ToolError;

use crate::ports::{ToolHostError, ToolSkillTrust};
use crate::registry::ToolRegistry;

pub(crate) fn required_str<'a>(
    params: &'a serde_json::Value,
    key: &str,
) -> Result<&'a str, ToolError> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidParameters(format!("missing required parameter: {}", key)))
}

pub fn restricted_skill_names(
    metadata: &serde_json::Value,
) -> Option<std::collections::HashSet<String>> {
    ToolRegistry::metadata_string_list(metadata, "allowed_skills")
        .map(|skills| skills.into_iter().collect())
}

pub fn ensure_skill_allowed(
    metadata: &serde_json::Value,
    skill_name: &str,
) -> Result<(), ToolError> {
    if ToolRegistry::skill_name_allowed_by_metadata(metadata, skill_name) {
        Ok(())
    } else {
        Err(ToolError::ExecutionFailed(format!(
            "Skill '{}' is not allowed in this agent context.",
            skill_name
        )))
    }
}

pub fn ensure_skill_admin_available(
    metadata: &serde_json::Value,
    tool_name: &str,
) -> Result<(), ToolError> {
    if ToolRegistry::metadata_string_list(metadata, "allowed_skills").is_some() {
        Err(ToolError::ExecutionFailed(format!(
            "Tool '{}' is not available when the current agent is restricted to a specific skill allowlist.",
            tool_name
        )))
    } else {
        Ok(())
    }
}

pub fn parse_skill_name_param(params: &serde_json::Value) -> Result<String, ToolError> {
    Ok(required_str(params, "name")?.to_string())
}

pub fn skill_source_output(kind: &str, path: &str) -> serde_json::Value {
    serde_json::json!({
        "kind": kind,
        "path": path,
    })
}

pub(crate) fn tool_host_error(error: ToolHostError) -> ToolError {
    ToolError::ExecutionFailed(error.to_string())
}

pub(crate) fn tool_skill_trust_label(trust: ToolSkillTrust) -> &'static str {
    match trust {
        ToolSkillTrust::Installed => "installed",
        ToolSkillTrust::Trusted => "trusted",
        ToolSkillTrust::Community => "community",
    }
}
