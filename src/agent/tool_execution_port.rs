//! Root tool execution adapter for the extracted agent tool port.

use std::sync::Arc;

use async_trait::async_trait;
use thinclaw_agent::ports::{
    PortableApprovalRequirement, ToolApprovalMode as AgentToolApprovalMode, ToolExecutionPort,
    ToolExecutionRequest, ToolExecutionResult, ToolPreparation,
};
use thinclaw_tools_core::{ApprovalRequirement, ToolOutput};

use crate::error::{Error, ToolError};
use crate::safety::SafetyLayer;
use crate::tools::execution::{
    PreparedToolCall, ToolApprovalMode, ToolPrepareOutcome, ToolPrepareRequest, execute_tool_call,
    prepare_tool_call,
};
use crate::tools::{ToolProfile, ToolRegistry};

pub struct RootToolExecutionPort {
    tools: Arc<ToolRegistry>,
    safety: Arc<SafetyLayer>,
}

impl RootToolExecutionPort {
    pub fn shared(
        tools: Arc<ToolRegistry>,
        safety: Arc<SafetyLayer>,
    ) -> Arc<dyn ToolExecutionPort> {
        Arc::new(Self { tools, safety })
    }
}

#[async_trait]
impl ToolExecutionPort for RootToolExecutionPort {
    async fn list_tools(&self) -> Result<Vec<thinclaw_tools_core::ToolDescriptor>, ToolError> {
        Ok(self.tools.tool_descriptors().await)
    }

    async fn get_tool(
        &self,
        name: &str,
    ) -> Result<Option<thinclaw_tools_core::ToolDescriptor>, ToolError> {
        Ok(self.tools.tool_descriptor(name).await)
    }

    async fn prepare_tool(
        &self,
        request: ToolExecutionRequest,
    ) -> Result<ToolPreparation, ToolError> {
        let outcome = prepare_tool_call(ToolPrepareRequest {
            tools: &self.tools,
            safety: &self.safety,
            job_ctx: &request.job_ctx,
            tool_name: &request.tool_name,
            params: &request.params,
            lane: request.lane,
            default_profile: request.profile,
            profile_override: None::<ToolProfile>,
            approval_mode: approval_mode_from_agent(request.approval_mode),
            hooks: None,
        })
        .await
        .map_err(tool_error_from_root)?;

        Ok(match outcome {
            ToolPrepareOutcome::Ready(prepared) => ToolPreparation::Ready {
                descriptor: prepared.descriptor,
                params: prepared.params,
                lane: prepared.lane,
                profile: prepared.profile,
            },
            ToolPrepareOutcome::NeedsApproval(pending) => ToolPreparation::NeedsApproval {
                request_id: uuid::Uuid::new_v4(),
                approval: approval_requirement_from_root(
                    pending.tool.requires_approval(&pending.params),
                ),
                description: pending.descriptor.description.clone(),
                descriptor: pending.descriptor,
                params: pending.params,
                lane: pending.lane,
                profile: pending.profile,
            },
        })
    }

    async fn execute_tool(
        &self,
        request: ToolExecutionRequest,
    ) -> Result<ToolExecutionResult, ToolError> {
        let prepared = match prepare_tool_call(ToolPrepareRequest {
            tools: &self.tools,
            safety: &self.safety,
            job_ctx: &request.job_ctx,
            tool_name: &request.tool_name,
            params: &request.params,
            lane: request.lane,
            default_profile: request.profile,
            profile_override: None::<ToolProfile>,
            approval_mode: approval_mode_from_agent(request.approval_mode),
            hooks: None,
        })
        .await
        .map_err(tool_error_from_root)?
        {
            ToolPrepareOutcome::Ready(prepared) => prepared,
            ToolPrepareOutcome::NeedsApproval(_) => {
                return Err(ToolError::ExecutionFailed {
                    name: request.tool_name,
                    reason: "tool requires approval".to_string(),
                });
            }
        };

        execute_prepared_tool(prepared, &self.safety, &request.job_ctx).await
    }
}

async fn execute_prepared_tool(
    prepared: PreparedToolCall,
    safety: &SafetyLayer,
    job_ctx: &thinclaw_types::JobContext,
) -> Result<ToolExecutionResult, ToolError> {
    let output = execute_tool_call(&prepared, safety, job_ctx)
        .await
        .map_err(tool_error_from_root)?;
    let tool_output = ToolOutput::success(output.result_json.clone(), output.elapsed);
    Ok(ToolExecutionResult {
        output: tool_output,
        sanitized_content: output.sanitized_content,
        sanitized_value: output.sanitized_value,
        was_modified: output.was_modified,
        warnings: output.warnings,
        elapsed: output.elapsed,
        sanitized_bytes: output.sanitized_bytes,
        sanitized_hash: output.sanitized_hash,
    })
}

fn approval_mode_from_agent(mode: AgentToolApprovalMode) -> ToolApprovalMode {
    match mode {
        AgentToolApprovalMode::Interactive {
            auto_approve_tools,
            session_auto_approved,
        } => ToolApprovalMode::Interactive {
            auto_approve_tools,
            session_auto_approved,
        },
        AgentToolApprovalMode::Autonomous => ToolApprovalMode::Autonomous,
        AgentToolApprovalMode::Bypass => ToolApprovalMode::Bypass,
    }
}

fn approval_requirement_from_root(requirement: ApprovalRequirement) -> PortableApprovalRequirement {
    match requirement {
        ApprovalRequirement::Never => PortableApprovalRequirement::Never,
        ApprovalRequirement::UnlessAutoApproved => PortableApprovalRequirement::UnlessAutoApproved,
        ApprovalRequirement::Always => PortableApprovalRequirement::Always,
    }
}

fn tool_error_from_root(error: Error) -> ToolError {
    match error {
        Error::Tool(error) => error,
        other => ToolError::ExecutionFailed {
            name: "tool".to_string(),
            reason: other.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_requirement_adapter_preserves_variants() {
        assert_eq!(
            approval_requirement_from_root(ApprovalRequirement::Never),
            PortableApprovalRequirement::Never
        );
        assert_eq!(
            approval_requirement_from_root(ApprovalRequirement::UnlessAutoApproved),
            PortableApprovalRequirement::UnlessAutoApproved
        );
        assert_eq!(
            approval_requirement_from_root(ApprovalRequirement::Always),
            PortableApprovalRequirement::Always
        );
    }
}
