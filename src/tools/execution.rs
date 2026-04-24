//! Shared tool execution pipeline used across chat, workers, schedulers, and subagents.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::context::JobContext;
use crate::error::Error;
use crate::hooks::{HookError, HookEvent, HookOutcome, HookRegistry};
use crate::safety::SafetyLayer;
use crate::tools::policy::ToolPolicyManager;
use crate::tools::rate_limiter::RateLimitResult;
use crate::tools::{
    ApprovalRequirement, Tool, ToolApprovalClass, ToolDescriptor, ToolExecutionLane, ToolProfile,
    ToolRegistry,
};

/// Hook execution context for a tool preparation request.
pub struct ToolHookConfig<'a> {
    pub registry: &'a HookRegistry,
    pub user_id: &'a str,
    pub context: &'a str,
}

/// How approval should be enforced for a tool call.
#[derive(Debug, Clone, Copy)]
pub enum ToolApprovalMode {
    Interactive {
        auto_approve_tools: bool,
        session_auto_approved: bool,
    },
    Autonomous,
    Bypass,
}

/// Inputs for shared tool preparation.
pub struct ToolPrepareRequest<'a> {
    pub tools: &'a ToolRegistry,
    pub safety: &'a SafetyLayer,
    pub job_ctx: &'a JobContext,
    pub tool_name: &'a str,
    pub params: &'a serde_json::Value,
    pub lane: ToolExecutionLane,
    pub default_profile: ToolProfile,
    pub profile_override: Option<ToolProfile>,
    pub approval_mode: ToolApprovalMode,
    pub hooks: Option<ToolHookConfig<'a>>,
}

/// Prepared tool invocation ready to execute.
pub struct PreparedToolCall {
    pub tool: Arc<dyn Tool>,
    pub descriptor: ToolDescriptor,
    pub params: serde_json::Value,
    pub lane: ToolExecutionLane,
    pub profile: ToolProfile,
}

/// Approval result from tool preparation.
pub struct PendingToolApproval {
    pub tool: Arc<dyn Tool>,
    pub descriptor: ToolDescriptor,
    pub params: serde_json::Value,
    pub lane: ToolExecutionLane,
    pub profile: ToolProfile,
}

/// Outcome of tool preparation.
pub enum ToolPrepareOutcome {
    Ready(PreparedToolCall),
    NeedsApproval(PendingToolApproval),
}

/// Result of a shared tool execution.
pub struct ToolExecutionOutput {
    pub result_json: serde_json::Value,
    pub sanitized_content: String,
    pub sanitized_value: serde_json::Value,
    pub was_modified: bool,
    pub warnings: Vec<String>,
    pub elapsed: Duration,
    pub sanitized_bytes: usize,
    pub sanitized_hash: String,
}

/// Prepare a tool call by resolving policy, hooks, approval, rate limits, and validation.
pub async fn prepare_tool_call(
    request: ToolPrepareRequest<'_>,
) -> Result<ToolPrepareOutcome, Error> {
    let tool_policies = ToolPolicyManager::load_from_settings();
    if let Some(reason) =
        tool_policies.denial_reason_for_metadata(request.tool_name, &request.job_ctx.metadata)
    {
        return Err(tool_execution_failed(request.tool_name, reason));
    }

    let tool = request
        .tools
        .get(request.tool_name)
        .await
        .ok_or_else(|| tool_not_found(request.tool_name))?;
    let descriptor = tool.descriptor();
    let profile = request.profile_override.unwrap_or(request.default_profile);

    if let Some(reason) = deny_reason_for_lane(tool.as_ref(), &descriptor, request.lane) {
        return Err(tool_execution_failed(request.tool_name, reason));
    }

    if let Some(reason) = deny_reason_for_profile(
        &descriptor,
        request.lane,
        profile,
        &request.job_ctx.metadata,
    ) {
        return Err(tool_execution_failed(request.tool_name, reason));
    }

    let mut params = request.params.clone();
    if let Some(hook_config) = request.hooks {
        params = run_tool_hook(
            hook_config.registry,
            request.tool_name,
            &params,
            hook_config.user_id,
            hook_config.context,
        )
        .await
        .map_err(|reason| tool_execution_failed(request.tool_name, reason))?;
    }

    let approval = tool.requires_approval(&params);
    if approval_required(approval, request.approval_mode) {
        return Ok(ToolPrepareOutcome::NeedsApproval(PendingToolApproval {
            tool,
            descriptor,
            params,
            lane: request.lane,
            profile,
        }));
    }

    if let Some(config) = tool.rate_limit_config()
        && let RateLimitResult::Limited { retry_after, .. } = request
            .tools
            .rate_limiter()
            .check_and_record(&request.job_ctx.user_id, request.tool_name, &config)
            .await
    {
        return Err(Error::Tool(crate::error::ToolError::RateLimited {
            name: request.tool_name.to_string(),
            retry_after: Some(retry_after),
        }));
    }

    let schema_validation = request
        .safety
        .validator()
        .validate_tool_params_against_schema(&descriptor.parameters, &params);
    if !schema_validation.is_valid {
        return Err(tool_invalid_params(
            request.tool_name,
            format_validation_details(&schema_validation),
        ));
    }

    let validation = request.safety.validator().validate_tool_params(&params);
    if !validation.is_valid {
        return Err(tool_invalid_params(
            request.tool_name,
            format_validation_details(&validation),
        ));
    }

    Ok(ToolPrepareOutcome::Ready(PreparedToolCall {
        tool,
        descriptor,
        params,
        lane: request.lane,
        profile,
    }))
}

/// Execute a prepared tool call and return sanitized output.
pub async fn execute_tool_call(
    prepared: &PreparedToolCall,
    safety: &SafetyLayer,
    job_ctx: &JobContext,
) -> Result<ToolExecutionOutput, Error> {
    tracing::debug!(
        tool = %prepared.descriptor.name,
        lane = %prepared.lane.as_str(),
        profile = %prepared.profile.as_str(),
        params = %prepared.params,
        "Tool call started"
    );

    let timeout = prepared.tool.execution_timeout();
    let start = Instant::now();
    let result = tokio::time::timeout(timeout, async {
        prepared
            .tool
            .execute(prepared.params.clone(), job_ctx)
            .await
    })
    .await;
    let elapsed = start.elapsed();

    match result {
        Ok(Ok(output)) => {
            let raw = serde_json::to_string_pretty(&output.result).map_err(|err| {
                tool_execution_failed(
                    &prepared.descriptor.name,
                    format!("Failed to serialize result: {err}"),
                )
            })?;
            let sanitized = safety.sanitize_tool_output(&prepared.descriptor.name, &raw);
            let sanitized_value = parse_sanitized_value(&sanitized.content);
            let preview = preview(&sanitized.content, 240);
            let hash = blake3::hash(sanitized.content.as_bytes())
                .to_hex()
                .to_string();
            let sanitized_len = sanitized.content.len();
            let warnings = sanitized
                .warnings
                .iter()
                .map(|warning| warning.description.clone())
                .collect::<Vec<_>>();

            tracing::debug!(
                tool = %prepared.descriptor.name,
                lane = %prepared.lane.as_str(),
                profile = %prepared.profile.as_str(),
                elapsed_ms = elapsed.as_millis() as u64,
                bytes = sanitized.content.len(),
                was_modified = sanitized.was_modified,
                content_hash = %hash,
                preview = %preview,
                "Tool call succeeded"
            );

            Ok(ToolExecutionOutput {
                sanitized_value,
                result_json: output.result,
                sanitized_content: sanitized.content,
                was_modified: sanitized.was_modified,
                warnings,
                elapsed,
                sanitized_bytes: sanitized_len,
                sanitized_hash: hash,
            })
        }
        Ok(Err(err)) => {
            tracing::debug!(
                tool = %prepared.descriptor.name,
                lane = %prepared.lane.as_str(),
                profile = %prepared.profile.as_str(),
                elapsed_ms = elapsed.as_millis() as u64,
                error = %err,
                "Tool call failed"
            );
            Err(tool_execution_failed(
                &prepared.descriptor.name,
                err.to_string(),
            ))
        }
        Err(_) => {
            tracing::debug!(
                tool = %prepared.descriptor.name,
                lane = %prepared.lane.as_str(),
                profile = %prepared.profile.as_str(),
                elapsed_ms = elapsed.as_millis() as u64,
                timeout_secs = timeout.as_secs(),
                "Tool call timed out"
            );
            Err(Error::Tool(crate::error::ToolError::Timeout {
                name: prepared.descriptor.name.clone(),
                timeout,
            }))
        }
    }
}

fn approval_required(requirement: ApprovalRequirement, mode: ToolApprovalMode) -> bool {
    match mode {
        ToolApprovalMode::Bypass => false,
        ToolApprovalMode::Autonomous => matches!(requirement, ApprovalRequirement::Always),
        ToolApprovalMode::Interactive {
            auto_approve_tools,
            session_auto_approved,
        } => {
            if auto_approve_tools {
                matches!(requirement, ApprovalRequirement::Always)
            } else {
                match requirement {
                    ApprovalRequirement::Never => false,
                    ApprovalRequirement::UnlessAutoApproved => !session_auto_approved,
                    ApprovalRequirement::Always => true,
                }
            }
        }
    }
}

async fn run_tool_hook(
    hooks: &HookRegistry,
    tool_name: &str,
    params: &serde_json::Value,
    user_id: &str,
    context: &str,
) -> Result<serde_json::Value, String> {
    let event = HookEvent::ToolCall {
        tool_name: tool_name.to_string(),
        parameters: params.clone(),
        user_id: user_id.to_string(),
        context: context.to_string(),
    };

    match hooks.run(&event).await {
        Err(HookError::Rejected { reason }) => Err(format!("Blocked by hook: {reason}")),
        Err(err) => Err(format!("Blocked by hook failure mode: {err}")),
        Ok(HookOutcome::Continue {
            modified: Some(new_params),
        }) => serde_json::from_str(&new_params)
            .map_err(|err| format!("Hook returned non-JSON modification for tool call: {err}")),
        Ok(HookOutcome::Continue { modified: None }) => Ok(params.clone()),
        Ok(HookOutcome::Reject { reason }) => Err(format!("Blocked by hook: {reason}")),
    }
}

fn deny_reason_for_profile(
    descriptor: &ToolDescriptor,
    lane: ToolExecutionLane,
    profile: ToolProfile,
    metadata: &serde_json::Value,
) -> Option<String> {
    if !ToolRegistry::tool_name_allowed_by_metadata(metadata, &descriptor.name) {
        return Some("Tool is not permitted in this agent context".to_string());
    }

    let explicit_tools = ToolRegistry::metadata_string_list(metadata, "allowed_tools");
    if descriptor.is_coordination_tool() {
        return None;
    }

    if let Some(explicit_tools) = explicit_tools {
        if explicit_tools.iter().any(|name| name == &descriptor.name) {
            return None;
        }

        return Some(format!(
            "Tool '{}' is not granted in this delegated context. Add it to allowed_tools or keep this step in the main agent.",
            descriptor.name
        ));
    }

    let implicitly_allowed = match profile {
        ToolProfile::Standard => true,
        ToolProfile::Restricted => descriptor.is_safe_read_only_orchestrator(),
        ToolProfile::ExplicitOnly => false,
        ToolProfile::Acp => descriptor_allowed_for_acp(descriptor),
    };

    if implicitly_allowed {
        None
    } else {
        Some(format!(
            "Tool '{}' is blocked in the {} lane under the '{}' tool profile. Grant it explicitly via allowed_tools or keep this work in the main agent.",
            descriptor.name,
            lane.as_str(),
            profile.as_str()
        ))
    }
}

fn descriptor_allowed_for_acp(descriptor: &ToolDescriptor) -> bool {
    let name = descriptor.name.as_str();
    if descriptor.is_coordination_tool() {
        return true;
    }

    matches!(
        name,
        "read_file"
            | "write_file"
            | "list_dir"
            | "apply_patch"
            | "grep"
            | "search_files"
            | "shell"
            | "process"
            | "execute_code"
            | "session_search"
            | "browser"
            | "vision_analyze"
            | "llm_list_models"
            | "llm_select"
    ) || name.starts_with("memory_")
        || name.starts_with("external_memory_")
        || name.starts_with("skill_")
}

fn deny_reason_for_lane(
    tool: &dyn Tool,
    descriptor: &ToolDescriptor,
    lane: ToolExecutionLane,
) -> Option<String> {
    if !matches!(
        lane,
        ToolExecutionLane::Scheduler
            | ToolExecutionLane::Worker
            | ToolExecutionLane::WorkerRuntime
            | ToolExecutionLane::Subagent
    ) {
        return None;
    }

    const DISPATCHER_ONLY_TOOLS: &[&str] = &["spawn_subagent", "list_subagents", "cancel_subagent"];
    if DISPATCHER_ONLY_TOOLS.contains(&descriptor.name.as_str()) {
        return Some(format!(
            "Tool '{}' requires dispatcher interception and is not available in the {} lane.",
            descriptor.name,
            lane.as_str()
        ));
    }

    if tool.requires_approval(&serde_json::json!({})) == ApprovalRequirement::Always {
        return Some(format!(
            "Tool '{}' requires explicit human approval and cannot run in the {} lane.",
            descriptor.name,
            lane.as_str()
        ));
    }

    None
}

/// Check whether a tool descriptor is usable for the given lane/profile/metadata tuple.
pub fn descriptor_allowed_for_profile(
    descriptor: &ToolDescriptor,
    lane: ToolExecutionLane,
    profile: ToolProfile,
    metadata: &serde_json::Value,
) -> bool {
    deny_reason_for_profile(descriptor, lane, profile, metadata).is_none()
}

/// Check whether a concrete tool may be exposed/executed in the given lane at all.
pub fn tool_allowed_for_lane(
    tool: &dyn Tool,
    descriptor: &ToolDescriptor,
    lane: ToolExecutionLane,
) -> bool {
    deny_reason_for_lane(tool, descriptor, lane).is_none()
}

fn preview(content: &str, max_chars: usize) -> String {
    let char_count = content.chars().count();
    if char_count <= max_chars {
        return content.to_string();
    }

    let truncated: String = content.chars().take(max_chars.saturating_sub(3)).collect();
    format!("{truncated}...")
}

fn parse_sanitized_value(content: &str) -> serde_json::Value {
    serde_json::from_str(content).unwrap_or_else(|_| serde_json::Value::String(content.to_string()))
}

fn format_validation_details(result: &crate::safety::ValidationResult) -> String {
    result
        .errors
        .iter()
        .map(|error| format!("{}: {}", error.field, error.message))
        .collect::<Vec<_>>()
        .join("; ")
}

fn tool_not_found(name: &str) -> Error {
    Error::Tool(crate::error::ToolError::NotFound {
        name: name.to_string(),
    })
}

fn tool_invalid_params(name: &str, reason: String) -> Error {
    Error::Tool(crate::error::ToolError::InvalidParameters {
        name: name.to_string(),
        reason,
    })
}

fn tool_execution_failed(name: &str, reason: String) -> Error {
    Error::Tool(crate::error::ToolError::ExecutionFailed {
        name: name.to_string(),
        reason,
    })
}

/// Map a descriptor's metadata to an approval class when no explicit annotation exists.
pub fn approval_class_from_requirement(requirement: ApprovalRequirement) -> ToolApprovalClass {
    match requirement {
        ApprovalRequirement::Never => ToolApprovalClass::Never,
        ApprovalRequirement::UnlessAutoApproved => ToolApprovalClass::Conditional,
        ApprovalRequirement::Always => ToolApprovalClass::Always,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{ToolDomain, ToolMetadata, ToolSideEffectLevel};

    fn descriptor(name: &str) -> ToolDescriptor {
        ToolDescriptor {
            name: name.to_string(),
            description: String::new(),
            parameters: serde_json::json!({ "type": "object" }),
            domain: ToolDomain::Container,
            metadata: ToolMetadata {
                authoritative_source: false,
                live_data: false,
                side_effect_level: ToolSideEffectLevel::Write,
                approval_class: ToolApprovalClass::Conditional,
                parallel_safe: false,
                route_intents: Vec::new(),
            },
        }
    }

    #[test]
    fn acp_profile_allows_editor_tools_and_blocks_messaging() {
        assert!(descriptor_allowed_for_profile(
            &descriptor("read_file"),
            ToolExecutionLane::Chat,
            ToolProfile::Acp,
            &serde_json::json!({})
        ));
        assert!(descriptor_allowed_for_profile(
            &descriptor("skill_search"),
            ToolExecutionLane::Chat,
            ToolProfile::Acp,
            &serde_json::json!({})
        ));
        assert!(!descriptor_allowed_for_profile(
            &descriptor("spawn_subagent"),
            ToolExecutionLane::Chat,
            ToolProfile::Acp,
            &serde_json::json!({})
        ));
        assert!(!descriptor_allowed_for_profile(
            &descriptor("send_message"),
            ToolExecutionLane::Chat,
            ToolProfile::Acp,
            &serde_json::json!({})
        ));
        assert!(!descriptor_allowed_for_profile(
            &descriptor("routine_create"),
            ToolExecutionLane::Chat,
            ToolProfile::Acp,
            &serde_json::json!({})
        ));
    }
}
