//! Root-independent learning tool policy helpers.

use thinclaw_tools_core::ToolError;
use thinclaw_workspace::paths;
use uuid::Uuid;

use crate::registry::ToolRegistry;

pub const PROMPT_TARGETS: &[&str] = &[paths::SOUL, paths::SOUL_LOCAL, paths::AGENTS, paths::USER];
pub const SKILL_FILE_NAME: &str = "SKILL.md";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LearningOutcomesParams {
    pub contract_id: Option<Uuid>,
    pub status: Option<String>,
    pub contract_type: Option<String>,
    pub source_kind: Option<String>,
    pub thread_id: Option<String>,
    pub limit: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LearningHistoryParams {
    pub kind: String,
    pub limit: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptManageParams {
    pub operation: String,
    pub target: &'static str,
    pub scope: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptManageTargetResolution {
    pub resolved_target: String,
    pub timezone_sync_target: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillManageParams {
    pub operation: String,
    pub name: String,
    pub path: String,
    pub all: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LearningFeedbackParams {
    pub target_type: String,
    pub target_id: String,
    pub verdict: String,
    pub note: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LearningProposalReviewParams {
    pub proposal_id: Uuid,
    pub decision: String,
    pub note: Option<String>,
}

pub fn normalize_prompt_target(target: &str) -> Result<&'static str, ToolError> {
    let trimmed = target.trim().trim_start_matches('/');
    PROMPT_TARGETS
        .iter()
        .copied()
        .find(|candidate| trimmed.eq_ignore_ascii_case(candidate))
        .ok_or_else(|| {
            ToolError::InvalidParameters(format!(
                "target must be one of: {}, got '{}'",
                PROMPT_TARGETS.join(", "),
                target
            ))
        })
}

fn required_str<'a>(params: &'a serde_json::Value, key: &str) -> Result<&'a str, ToolError> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidParameters(format!("missing required parameter: {}", key)))
}

pub fn prompt_manage_parameters_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "operation": {
                "type": "string",
                "enum": ["replace", "upsert_section", "append_section", "remove_section"],
                "description": "Prompt mutation operation",
                "default": "replace"
            },
            "target": {
                "type": "string",
                "enum": PROMPT_TARGETS,
                "description": "Which prompt file to update"
            },
            "scope": {
                "type": "string",
                "enum": ["auto", "actor", "shared"],
                "description": "USER.md scope behavior. auto = actor USER.md in direct chats, shared USER.md in groups.",
                "default": "auto"
            },
            "content": {
                "type": "string",
                "description": "Replacement markdown content for operation=replace"
            },
            "heading": {
                "type": "string",
                "description": "Section heading for section-aware operations"
            },
            "section_content": {
                "type": "string",
                "description": "Section body for upsert_section or append_section"
            },
        },
        "required": ["target"]
    })
}

pub fn parse_prompt_manage_params(
    params: &serde_json::Value,
) -> Result<PromptManageParams, ToolError> {
    let operation = params
        .get("operation")
        .and_then(|v| v.as_str())
        .unwrap_or("replace")
        .to_ascii_lowercase();
    let target = normalize_prompt_target(required_str(params, "target")?)?;
    let scope = params
        .get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or("auto")
        .to_ascii_lowercase();
    if target != paths::USER && scope != "auto" {
        return Err(ToolError::InvalidParameters(
            "scope is only supported for target='USER.md'".to_string(),
        ));
    }
    Ok(PromptManageParams {
        operation,
        target,
        scope,
    })
}

pub fn prompt_manage_next_content(
    params: &serde_json::Value,
    before: &str,
    operation: &str,
) -> Result<String, ToolError> {
    match operation {
        "replace" => Ok(required_str(params, "content")?.to_string()),
        "upsert_section" => {
            let heading = required_str(params, "heading")?;
            let section_content = params
                .get("section_content")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            Ok(upsert_markdown_section(before, heading, section_content))
        }
        "append_section" => {
            let heading = required_str(params, "heading")?;
            let section_content = params
                .get("section_content")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            Ok(append_markdown_section(before, heading, section_content))
        }
        "remove_section" => {
            let heading = required_str(params, "heading")?;
            remove_markdown_section(before, heading)
        }
        other => Err(ToolError::InvalidParameters(format!(
            "unknown prompt_manage operation '{}'",
            other
        ))),
    }
}

pub fn prompt_manage_mirror_payload(
    target: &str,
    resolved_target: &str,
    scope: &str,
    operation: &str,
    content: &str,
) -> serde_json::Value {
    serde_json::json!({
        "tool": "prompt_manage",
        "target": target,
        "resolved_target": resolved_target,
        "scope": scope,
        "operation": operation,
        "content_preview": content.chars().take(240).collect::<String>(),
    })
}

pub fn prompt_manage_provenance(
    target: &str,
    resolved_target: &str,
    scope: &str,
    user_id: &str,
) -> serde_json::Value {
    serde_json::json!({
        "tool": "prompt_manage",
        "target": target,
        "resolved_target": resolved_target,
        "scope": scope,
        "user_id": user_id,
    })
}

pub fn prompt_manage_output(
    operation: &str,
    resolved_target: &str,
    bytes_written: usize,
    user_notification_required: bool,
    version_label: Option<String>,
    artifact_version_recorded: bool,
    artifact_version_error: Option<String>,
) -> serde_json::Value {
    serde_json::json!({
        "status": "updated",
        "operation": operation,
        "target": resolved_target,
        "bytes_written": bytes_written,
        "user_notification_required": user_notification_required,
        "version_label": version_label,
        "artifact_version_recorded": artifact_version_recorded,
        "artifact_version_error": artifact_version_error,
    })
}

pub fn skill_manage_parameters_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "operation": {
                "type": "string",
                "enum": ["create", "patch", "edit", "delete", "write_file", "remove_file", "reload"],
                "description": "What to do with the skill"
            },
            "name": {
                "type": "string",
                "description": "Skill name"
            },
            "path": {
                "type": "string",
                "description": "Relative file path inside the skill directory (defaults to SKILL.md)",
                "default": SKILL_FILE_NAME
            },
            "content": {
                "type": "string",
                "description": "New file content for create/write/edit/patch operations"
            },
            "all": {
                "type": "boolean",
                "description": "When operation=reload, reload every skill instead of one",
                "default": false
            }
        },
        "required": ["operation", "name"]
    })
}

pub fn parse_skill_manage_params(
    params: &serde_json::Value,
) -> Result<SkillManageParams, ToolError> {
    Ok(SkillManageParams {
        operation: required_str(params, "operation")?.to_ascii_lowercase(),
        name: required_str(params, "name")?.to_string(),
        path: params
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(SKILL_FILE_NAME)
            .to_string(),
        all: params.get("all").and_then(|v| v.as_bool()).unwrap_or(false),
    })
}

pub fn skill_manage_reload_all_output(loaded: Vec<String>) -> serde_json::Value {
    let count = loaded.len();
    serde_json::json!({
        "status": "reloaded_all",
        "skills": loaded,
        "count": count,
    })
}

pub fn skill_manage_reload_output(name: &str) -> serde_json::Value {
    serde_json::json!({
        "status": "reloaded",
        "name": name,
    })
}

pub fn skill_manage_created_output(
    name: &str,
    artifact_version_recorded: bool,
    artifact_version_error: Option<String>,
) -> serde_json::Value {
    serde_json::json!({
        "status": "created",
        "name": name,
        "path": SKILL_FILE_NAME,
        "artifact_version_recorded": artifact_version_recorded,
        "artifact_version_error": artifact_version_error,
    })
}

pub fn skill_manage_deleted_output(
    name: &str,
    artifact_version_recorded: bool,
    artifact_version_error: Option<String>,
) -> serde_json::Value {
    serde_json::json!({
        "status": "deleted",
        "name": name,
        "artifact_version_recorded": artifact_version_recorded,
        "artifact_version_error": artifact_version_error,
    })
}

pub fn skill_manage_removed_file_output(
    name: &str,
    path: impl serde::Serialize,
    artifact_version_recorded: bool,
    artifact_version_error: Option<String>,
) -> serde_json::Value {
    serde_json::json!({
        "status": "removed_file",
        "name": name,
        "path": path,
        "artifact_version_recorded": artifact_version_recorded,
        "artifact_version_error": artifact_version_error,
    })
}

pub fn skill_manage_updated_output(
    name: &str,
    path: impl serde::Serialize,
    artifact_version_recorded: bool,
    artifact_version_error: Option<String>,
) -> serde_json::Value {
    serde_json::json!({
        "status": "updated",
        "name": name,
        "path": path,
        "artifact_version_recorded": artifact_version_recorded,
        "artifact_version_error": artifact_version_error,
    })
}

pub fn skill_manage_provenance(
    operation: &str,
    path: Option<impl serde::Serialize>,
    reloaded_name: Option<&str>,
) -> serde_json::Value {
    let mut provenance = serde_json::json!({
        "tool": "skill_manage",
        "agent_generated": true,
        "operation": operation,
    });
    if let Some(path) = path {
        provenance["path"] = serialize_value(path);
    }
    if let Some(reloaded_name) = reloaded_name {
        provenance["reloaded_name"] = serde_json::Value::String(reloaded_name.to_string());
    }
    provenance
}

pub fn learning_status_parameters_schema() -> serde_json::Value {
    serde_json::json!({"type": "object", "properties": {}})
}

pub fn learning_outcomes_parameters_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "contract_id": {
                "type": "string",
                "description": "Optional outcome contract UUID for detailed inspection"
            },
            "status": { "type": "string" },
            "contract_type": { "type": "string" },
            "source_kind": { "type": "string" },
            "thread_id": { "type": "string" },
            "limit": {
                "type": "integer",
                "minimum": 1,
                "maximum": 100
            }
        }
    })
}

pub fn parse_learning_outcomes_params(
    params: &serde_json::Value,
) -> Result<LearningOutcomesParams, ToolError> {
    let contract_id = params
        .get("contract_id")
        .and_then(|value| value.as_str())
        .map(|value| {
            Uuid::parse_str(value).map_err(|_| {
                ToolError::InvalidParameters("contract_id must be a valid UUID".to_string())
            })
        })
        .transpose()?;
    let limit = params
        .get("limit")
        .and_then(|value| value.as_u64())
        .unwrap_or(25)
        .clamp(1, 100) as i64;

    Ok(LearningOutcomesParams {
        contract_id,
        status: params
            .get("status")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        contract_type: params
            .get("contract_type")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        source_kind: params
            .get("source_kind")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        thread_id: params
            .get("thread_id")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        limit,
    })
}

pub fn learning_history_parameters_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "kind": {
                "type": "string",
                "enum": ["all", "events", "evaluations", "candidates", "artifact_versions", "feedback", "rollbacks", "code_proposals"],
                "default": "all"
            },
            "limit": {
                "type": "integer",
                "default": 20,
                "minimum": 1,
                "maximum": 100
            }
        }
    })
}

pub fn parse_learning_history_params(params: &serde_json::Value) -> LearningHistoryParams {
    let kind = params
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("all")
        .to_ascii_lowercase();
    let limit = params
        .get("limit")
        .and_then(|v| v.as_i64())
        .unwrap_or(20)
        .clamp(1, 100);
    LearningHistoryParams { kind, limit }
}

pub fn serialize_value<T: serde::Serialize>(value: T) -> serde_json::Value {
    serde_json::to_value(value).unwrap_or_else(|_| serde_json::json!({}))
}

pub fn recent_items_output<T: serde::Serialize>(items: Vec<T>) -> serde_json::Value {
    let count = items.len();
    serde_json::json!({
        "count": count,
        "items": items,
    })
}

pub fn learning_status_output(
    settings: serde_json::Value,
    provider_health: serde_json::Value,
    outcomes_enabled: bool,
    outcome_summary: serde_json::Value,
    recent_outcomes: serde_json::Value,
    recent_activity: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "settings": settings,
        "provider_health": provider_health,
        "outcomes": {
            "enabled": outcomes_enabled,
            "summary": outcome_summary,
            "recent": recent_outcomes,
        },
        "recent_activity": recent_activity,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn learning_recent_activity_output(
    events: serde_json::Value,
    evaluations: serde_json::Value,
    candidates: serde_json::Value,
    artifact_versions: serde_json::Value,
    feedback: serde_json::Value,
    rollbacks: serde_json::Value,
    code_proposals: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "events": events,
        "evaluations": evaluations,
        "candidates": candidates,
        "artifact_versions": artifact_versions,
        "feedback": feedback,
        "rollbacks": rollbacks,
        "code_proposals": code_proposals,
    })
}

pub fn learning_contract_detail_output(
    contract: impl serde::Serialize,
    observations: impl serde::Serialize,
) -> serde_json::Value {
    serde_json::json!({
        "contract": contract,
        "observations": observations,
    })
}

pub fn learning_items_output<T: serde::Serialize>(items: Vec<T>) -> serde_json::Value {
    let count = items.len();
    serde_json::json!({
        "count": count,
        "items": items,
    })
}

pub fn learning_history_single_output(
    kind: &str,
    items: impl serde::Serialize,
) -> serde_json::Value {
    serde_json::json!({
        "kind": kind,
        "items": serialize_value(items),
    })
}

#[allow(clippy::too_many_arguments)]
pub fn learning_history_all_output(
    kind: &str,
    events: impl serde::Serialize,
    evaluations: impl serde::Serialize,
    candidates: impl serde::Serialize,
    artifact_versions: impl serde::Serialize,
    feedback: impl serde::Serialize,
    rollbacks: impl serde::Serialize,
    code_proposals: impl serde::Serialize,
) -> serde_json::Value {
    serde_json::json!({
        "kind": kind,
        "events": serialize_value(events),
        "evaluations": serialize_value(evaluations),
        "candidates": serialize_value(candidates),
        "artifact_versions": serialize_value(artifact_versions),
        "feedback": serialize_value(feedback),
        "rollbacks": serialize_value(rollbacks),
        "code_proposals": serialize_value(code_proposals),
    })
}

pub fn learning_feedback_parameters_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "target_type": {
                "type": "string",
                "description": "Type of target (for example: candidate, code_proposal, prompt, skill)"
            },
            "target_id": {
                "type": "string",
                "description": "Identifier for the target"
            },
            "verdict": {
                "type": "string",
                "description": "Feedback verdict (for example: helpful, harmful, reject, dont_learn)"
            },
            "note": {
                "type": "string",
                "description": "Optional note explaining the verdict"
            },
            "metadata": {
                "type": "object",
                "description": "Optional extra metadata"
            }
        },
        "required": ["target_type", "target_id", "verdict"]
    })
}

pub fn parse_learning_feedback_params(
    params: &serde_json::Value,
) -> Result<LearningFeedbackParams, ToolError> {
    Ok(LearningFeedbackParams {
        target_type: required_str(params, "target_type")?.to_string(),
        target_id: required_str(params, "target_id")?.to_string(),
        verdict: required_str(params, "verdict")?.to_string(),
        note: params
            .get("note")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        metadata: params.get("metadata").cloned(),
    })
}

pub fn learning_feedback_output(
    id: impl serde::Serialize,
    target_type: &str,
    target_id: &str,
    verdict: &str,
) -> serde_json::Value {
    serde_json::json!({
        "status": "recorded",
        "id": id,
        "target_type": target_type,
        "target_id": target_id,
        "verdict": verdict,
    })
}

pub fn learning_proposal_review_parameters_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "proposal_id": {
                "type": "string",
                "description": "UUID of the learning code proposal"
            },
            "decision": {
                "type": "string",
                "enum": ["approve", "reject"],
                "description": "Review decision"
            },
            "note": {
                "type": "string",
                "description": "Optional reviewer note"
            }
        },
        "required": ["proposal_id", "decision"]
    })
}

pub fn parse_learning_proposal_review_params(
    params: &serde_json::Value,
) -> Result<LearningProposalReviewParams, ToolError> {
    let proposal_id = Uuid::parse_str(required_str(params, "proposal_id")?)
        .map_err(|err| ToolError::InvalidParameters(format!("invalid proposal_id: {}", err)))?;
    Ok(LearningProposalReviewParams {
        proposal_id,
        decision: required_str(params, "decision")?.to_string(),
        note: params
            .get("note")
            .and_then(|value| value.as_str())
            .map(str::to_string),
    })
}

pub fn learning_proposal_review_output(
    status: impl serde::Serialize,
    proposal: impl serde::Serialize,
) -> serde_json::Value {
    serde_json::json!({
        "status": status,
        "proposal": serialize_value(proposal),
    })
}

pub fn validate_prompt_content(content: &str) -> Result<(), ToolError> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err(ToolError::InvalidParameters(
            "prompt content cannot be empty".to_string(),
        ));
    }
    if !trimmed.contains('#') {
        return Err(ToolError::InvalidParameters(
            "prompt content must include markdown headings".to_string(),
        ));
    }
    let lowered = trimmed.to_ascii_lowercase();
    let suspicious_markers = ["role: user", "role: assistant", "tool_result", "<tool_call"];
    if suspicious_markers
        .iter()
        .any(|marker| lowered.contains(marker))
    {
        return Err(ToolError::InvalidParameters(
            "prompt content appears to include transcript/tool residue".to_string(),
        ));
    }
    Ok(())
}

pub fn validate_agents_prompt_safety(content: &str) -> Result<(), ToolError> {
    let lowered = content.to_ascii_lowercase();
    let required_markers = ["red lines", "ask first", "don't"];
    if required_markers
        .iter()
        .all(|marker| !lowered.contains(marker))
    {
        return Err(ToolError::InvalidParameters(format!(
            "{} update rejected: core safety guidance appears to be missing",
            paths::AGENTS
        )));
    }
    Ok(())
}

pub fn validate_skill_admin_available(
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

pub fn validate_prompt_manage_available(metadata: &serde_json::Value) -> Result<(), ToolError> {
    if ToolRegistry::metadata_string_list(metadata, "allowed_skills").is_some() {
        Err(ToolError::NotAuthorized(
            "prompt_manage is not available when the current agent is restricted to a specific skill allowlist.".to_string(),
        ))
    } else {
        Ok(())
    }
}

pub fn prompt_manage_user_target(
    scope: &str,
    metadata: &serde_json::Value,
    actor_id: Option<&str>,
) -> Result<String, ToolError> {
    let actor_id = metadata
        .get("actor_id")
        .and_then(|v| v.as_str())
        .or(actor_id);
    let conversation_kind = metadata
        .get("conversation_kind")
        .or_else(|| metadata.get("chat_type"))
        .and_then(|v| v.as_str())
        .unwrap_or("direct")
        .to_ascii_lowercase();
    let is_group = matches!(
        conversation_kind.as_str(),
        "group" | "channel" | "supergroup"
    );

    match scope {
        "shared" => Ok(paths::USER.to_string()),
        "actor" => {
            let Some(actor_id) = actor_id else {
                return Err(ToolError::InvalidParameters(
                    "scope='actor' requires actor_id context".to_string(),
                ));
            };
            Ok(paths::actor_user(actor_id))
        }
        "auto" => {
            if !is_group && let Some(actor_id) = actor_id {
                return Ok(paths::actor_user(actor_id));
            }
            Ok(paths::USER.to_string())
        }
        other => Err(ToolError::InvalidParameters(format!(
            "unsupported scope '{}'; expected auto, actor, or shared",
            other
        ))),
    }
}

pub fn resolve_prompt_manage_target(
    target: &str,
    scope: &str,
    metadata: &serde_json::Value,
    actor_id: Option<&str>,
    user_id: &str,
) -> Result<PromptManageTargetResolution, ToolError> {
    let resolved_target = if target == paths::USER {
        prompt_manage_user_target(scope, metadata, actor_id)?
    } else {
        target.to_string()
    };
    let owner_actor_user = if target == paths::USER {
        Some(paths::actor_user(user_id))
    } else {
        None
    };
    let timezone_sync_target = target == paths::USER
        && (resolved_target == paths::USER
            || owner_actor_user
                .as_deref()
                .is_some_and(|path| resolved_target == path));

    Ok(PromptManageTargetResolution {
        resolved_target,
        timezone_sync_target,
    })
}

pub fn validate_relative_skill_path(path: &str) -> Result<std::path::PathBuf, ToolError> {
    let trimmed = path.trim().trim_start_matches('/');
    if trimmed.is_empty() {
        return Err(ToolError::InvalidParameters(
            "path cannot be empty".to_string(),
        ));
    }

    let path = std::path::Path::new(trimmed);
    if path.is_absolute() {
        return Err(ToolError::InvalidParameters(format!(
            "skill file path must be relative, got '{}'",
            path.display()
        )));
    }

    let mut clean = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => clean.push(part),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => {
                return Err(ToolError::InvalidParameters(format!(
                    "skill file path '{}' must not contain path traversal components",
                    path.display()
                )));
            }
        }
    }

    if clean.as_os_str().is_empty() {
        return Err(ToolError::InvalidParameters(
            "path cannot resolve to an empty location".to_string(),
        ));
    }

    Ok(clean)
}

fn normalize_heading_name(raw: &str) -> String {
    raw.trim()
        .trim_start_matches('#')
        .trim()
        .to_ascii_lowercase()
}

fn parse_markdown_heading(line: &str) -> Option<(usize, String)> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return None;
    }
    let level = trimmed.chars().take_while(|ch| *ch == '#').count();
    if level == 0 {
        return None;
    }
    let title = trimmed[level..].trim();
    if title.is_empty() {
        return None;
    }
    Some((level, title.to_string()))
}

pub fn find_section_byte_range(
    doc: &str,
    heading_name: &str,
) -> Option<(usize, usize, usize, String)> {
    let target = normalize_heading_name(heading_name);
    let mut offset = 0usize;
    let mut start: Option<(usize, usize, usize, String)> = None;

    for line in doc.split_inclusive('\n') {
        let line_start = offset;
        let line_end = offset + line.len();
        offset = line_end;

        if let Some((level, title)) = parse_markdown_heading(line) {
            if let Some((start_offset, current_level, _, current_title)) = &start
                && level <= *current_level
            {
                return Some((
                    *start_offset,
                    line_start,
                    *current_level,
                    current_title.clone(),
                ));
            }

            if normalize_heading_name(&title) == target {
                start = Some((line_start, level, line_end, title));
            }
        }
    }

    start.map(|(start_offset, level, _, title)| (start_offset, doc.len(), level, title))
}

pub fn upsert_markdown_section(doc: &str, heading: &str, section_content: &str) -> String {
    let normalized_content = section_content.trim();
    let body = if normalized_content.is_empty() {
        String::new()
    } else {
        format!("\n{}\n", normalized_content)
    };

    if let Some((start, end, level, title)) = find_section_byte_range(doc, heading) {
        let heading_line = format!("{} {}", "#".repeat(level.max(1)), title.trim());
        let replacement = format!("{heading_line}{body}");
        let mut merged = String::with_capacity(doc.len() + replacement.len());
        merged.push_str(&doc[..start]);
        merged.push_str(replacement.trim_end_matches('\n'));
        merged.push('\n');
        merged.push_str(doc[end..].trim_start_matches('\n'));
        return merged.trim().to_string() + "\n";
    }

    let mut merged = doc.trim().to_string();
    if !merged.is_empty() {
        merged.push_str("\n\n");
    }
    merged.push_str(&format!("## {}\n", heading.trim()));
    if !normalized_content.is_empty() {
        merged.push_str(normalized_content);
        merged.push('\n');
    }
    merged
}

pub fn append_markdown_section(doc: &str, heading: &str, section_content: &str) -> String {
    let mut merged = doc.trim().to_string();
    if !merged.is_empty() {
        merged.push_str("\n\n");
    }
    merged.push_str(&format!("## {}\n", heading.trim()));
    let content = section_content.trim();
    if !content.is_empty() {
        merged.push_str(content);
        merged.push('\n');
    }
    merged
}

pub fn remove_markdown_section(doc: &str, heading: &str) -> Result<String, ToolError> {
    let Some((start, end, _, _)) = find_section_byte_range(doc, heading) else {
        return Err(ToolError::ExecutionFailed(format!(
            "section '{}' not found",
            heading
        )));
    };

    let mut merged = String::with_capacity(doc.len());
    merged.push_str(&doc[..start]);
    merged.push_str(doc[end..].trim_start_matches('\n'));
    Ok(merged.trim().to_string() + "\n")
}

pub fn artifact_name_for_skill(skill_name: &str, path: &std::path::Path) -> String {
    let path_str = path.to_string_lossy();
    if path_str.eq_ignore_ascii_case(SKILL_FILE_NAME) {
        skill_name.to_string()
    } else {
        format!("{}/{}", skill_name, path_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_target_normalization_accepts_known_targets() {
        assert_eq!(normalize_prompt_target("/SOUL.md").unwrap(), paths::SOUL);
        assert_eq!(normalize_prompt_target("agents.md").unwrap(), paths::AGENTS);
        assert!(normalize_prompt_target("unknown.md").is_err());
    }

    #[test]
    fn prompt_content_validation_rejects_empty_or_transcript_residue() {
        assert!(validate_prompt_content("").is_err());
        assert!(validate_prompt_content("plain text").is_err());
        assert!(validate_prompt_content("# Notes\nrole: user\nhello").is_err());
        assert!(validate_prompt_content("# Notes\nKeep this concise.").is_ok());
    }

    #[test]
    fn prompt_manage_user_target_resolves_actor_and_group_scope() {
        assert_eq!(
            prompt_manage_user_target(
                "auto",
                &serde_json::json!({ "conversation_kind": "direct" }),
                Some("actor-1")
            )
            .unwrap(),
            paths::actor_user("actor-1")
        );
        assert_eq!(
            prompt_manage_user_target(
                "auto",
                &serde_json::json!({ "conversation_kind": "group" }),
                Some("actor-1")
            )
            .unwrap(),
            paths::USER
        );
        assert!(prompt_manage_user_target("actor", &serde_json::json!({}), None).is_err());
    }

    #[test]
    fn prompt_manage_target_resolution_marks_timezone_targets() {
        let direct = serde_json::json!({ "conversation_kind": "direct" });
        let actor_user =
            resolve_prompt_manage_target(paths::USER, "auto", &direct, Some("user-1"), "user-1")
                .unwrap();
        assert_eq!(actor_user.resolved_target, paths::actor_user("user-1"));
        assert!(actor_user.timezone_sync_target);

        let other_actor =
            resolve_prompt_manage_target(paths::USER, "auto", &direct, Some("actor-2"), "user-1")
                .unwrap();
        assert_eq!(other_actor.resolved_target, paths::actor_user("actor-2"));
        assert!(!other_actor.timezone_sync_target);

        let soul =
            resolve_prompt_manage_target(paths::SOUL, "auto", &direct, Some("user-1"), "user-1")
                .unwrap();
        assert_eq!(soul.resolved_target, paths::SOUL);
        assert!(!soul.timezone_sync_target);
    }

    #[test]
    fn prompt_and_skill_admin_gates_respect_skill_restrictions() {
        let metadata = serde_json::json!({ "allowed_skills": ["github"] });
        assert!(validate_skill_admin_available(&metadata, "skill_manage").is_err());
        assert!(validate_prompt_manage_available(&metadata).is_err());
        assert!(validate_skill_admin_available(&serde_json::json!({}), "skill_manage").is_ok());
        assert!(validate_prompt_manage_available(&serde_json::json!({})).is_ok());
    }

    #[test]
    fn relative_skill_paths_reject_traversal() {
        assert_eq!(
            validate_relative_skill_path("./docs/NOTE.md").unwrap(),
            std::path::PathBuf::from("docs/NOTE.md")
        );
        assert_eq!(
            validate_relative_skill_path("/absolute").unwrap(),
            std::path::PathBuf::from("absolute")
        );
        assert!(validate_relative_skill_path("../secret").is_err());
        assert!(validate_relative_skill_path("").is_err());
    }

    #[test]
    fn learning_tool_schemas_and_params_are_root_independent() {
        assert_eq!(prompt_manage_parameters_schema()["required"][0], "target");
        assert_eq!(skill_manage_parameters_schema()["required"][0], "operation");
        assert_eq!(
            learning_status_parameters_schema()["type"].as_str(),
            Some("object")
        );
        assert_eq!(
            learning_outcomes_parameters_schema()["properties"]["limit"]["maximum"],
            100
        );
        assert_eq!(
            learning_history_parameters_schema()["properties"]["kind"]["default"],
            "all"
        );
        assert_eq!(
            learning_feedback_parameters_schema()["required"][2],
            "verdict"
        );
        assert_eq!(
            learning_proposal_review_parameters_schema()["properties"]["decision"]["enum"][0],
            "approve"
        );

        let outcomes = parse_learning_outcomes_params(&serde_json::json!({
            "limit": 500,
            "status": "active"
        }))
        .unwrap();
        assert_eq!(outcomes.limit, 100);
        assert_eq!(outcomes.status.as_deref(), Some("active"));
        assert!(
            parse_learning_outcomes_params(&serde_json::json!({
                "contract_id": "not-a-uuid"
            }))
            .is_err()
        );

        let history = parse_learning_history_params(&serde_json::json!({
            "kind": "EVENTS",
            "limit": -4
        }));
        assert_eq!(history.kind, "events");
        assert_eq!(history.limit, 1);

        let prompt = parse_prompt_manage_params(&serde_json::json!({
            "target": "USER.md",
            "scope": "ACTOR"
        }))
        .unwrap();
        assert_eq!(prompt.target, paths::USER);
        assert_eq!(prompt.operation, "replace");
        assert_eq!(prompt.scope, "actor");
        assert!(
            parse_prompt_manage_params(&serde_json::json!({
                "target": "AGENTS.md",
                "scope": "shared"
            }))
            .is_err()
        );
        let next = prompt_manage_next_content(
            &serde_json::json!({
                "operation": "upsert_section",
                "heading": "Preferences",
                "section_content": "- concise"
            }),
            "# Root\n",
            "upsert_section",
        )
        .unwrap();
        assert!(next.contains("## Preferences\n- concise"));

        let skill = parse_skill_manage_params(&serde_json::json!({
            "operation": "RELOAD",
            "name": "docs",
            "all": true
        }))
        .unwrap();
        assert_eq!(skill.operation, "reload");
        assert_eq!(skill.name, "docs");
        assert_eq!(skill.path, SKILL_FILE_NAME);
        assert!(skill.all);

        let feedback = parse_learning_feedback_params(&serde_json::json!({
            "target_type": "candidate",
            "target_id": "abc",
            "verdict": "helpful",
            "metadata": { "source": "test" }
        }))
        .unwrap();
        assert_eq!(feedback.target_type, "candidate");
        assert_eq!(feedback.metadata.unwrap()["source"].as_str(), Some("test"));

        let proposal_id = Uuid::new_v4();
        let review = parse_learning_proposal_review_params(&serde_json::json!({
            "proposal_id": proposal_id.to_string(),
            "decision": "approve",
            "note": "looks good"
        }))
        .unwrap();
        assert_eq!(review.proposal_id, proposal_id);
        assert_eq!(review.note.as_deref(), Some("looks good"));
        assert!(
            parse_learning_proposal_review_params(&serde_json::json!({
                "proposal_id": "bad",
                "decision": "approve"
            }))
            .is_err()
        );
    }

    #[test]
    fn upsert_section_replaces_existing_block() {
        let source = "# Root\n\n## Preferences\nold\n\n## Next\nstay\n";
        let updated = upsert_markdown_section(source, "Preferences", "new");
        assert!(updated.contains("## Preferences\nnew"));
        assert!(updated.contains("## Next\nstay"));
        assert!(!updated.contains("old"));
    }

    #[test]
    fn append_section_adds_new_block() {
        let source = "# Root\n\nBody\n";
        let updated = append_markdown_section(source, "Habits", "- concise");
        assert!(updated.contains("## Habits\n- concise"));
    }

    #[test]
    fn remove_section_drops_target_only() {
        let source = "# Root\n\n## A\none\n\n## B\ntwo\n";
        let updated = remove_markdown_section(source, "A").expect("section A should exist");
        assert!(!updated.contains("## A"));
        assert!(updated.contains("## B\ntwo"));
    }
}
