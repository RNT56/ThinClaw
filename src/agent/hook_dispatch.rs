//! Root hook-registry adapter for the extracted agent hook-dispatch port.

use std::sync::Arc;

use async_trait::async_trait;
use thinclaw_agent::hook_dispatch::{
    HookNormalizationError, NormalizedHookEvent, normalize_agent_hook_event,
};
use thinclaw_agent::ports::{
    AgentHookContext, AgentHookEvent, AgentHookOutcome, HookDispatchPort, HookPortError,
};

use crate::hooks::{HookContext, HookError, HookEvent, HookOutcome, HookRegistry};

pub struct RootHookDispatchPort {
    hooks: Arc<HookRegistry>,
}

impl RootHookDispatchPort {
    pub fn shared(hooks: Arc<HookRegistry>) -> Arc<dyn HookDispatchPort> {
        Arc::new(Self { hooks })
    }
}

#[async_trait]
impl HookDispatchPort for RootHookDispatchPort {
    async fn dispatch_hook(
        &self,
        event: AgentHookEvent,
        context: AgentHookContext,
    ) -> Result<AgentHookOutcome, HookPortError> {
        // Real invocation metadata (e.g. trace ids, routine-run context)
        // travels alongside the event and must reach the hook instead of
        // being dropped in favor of an always-empty `HookContext::default()`.
        let metadata = context.metadata.clone();
        let hook_event = hook_event_from_agent(event, context)?;
        let hook_ctx = HookContext { metadata };
        self.hooks
            .run_with_context(&hook_event, &hook_ctx)
            .await
            .map(hook_outcome_to_agent)
            .map_err(hook_error_to_agent)
    }
}

fn hook_event_from_agent(
    event: AgentHookEvent,
    context: AgentHookContext,
) -> Result<HookEvent, HookPortError> {
    let normalized =
        normalize_agent_hook_event(event, context).map_err(hook_normalization_error_to_agent)?;

    Ok(match normalized {
        NormalizedHookEvent::Inbound {
            user_id,
            channel,
            content,
            thread_id,
        } => HookEvent::Inbound {
            user_id,
            channel,
            content,
            thread_id,
        },
        NormalizedHookEvent::ToolCall {
            tool_name,
            parameters,
            user_id,
            context,
        } => HookEvent::ToolCall {
            tool_name,
            parameters,
            user_id,
            context,
        },
        NormalizedHookEvent::Outbound {
            user_id,
            channel,
            content,
            thread_id,
        } => HookEvent::Outbound {
            user_id,
            channel,
            content,
            thread_id,
        },
        NormalizedHookEvent::SessionStart {
            user_id,
            session_id,
        } => HookEvent::SessionStart {
            user_id,
            session_id,
        },
        NormalizedHookEvent::SessionEnd {
            user_id,
            session_id,
        } => HookEvent::SessionEnd {
            user_id,
            session_id,
        },
        NormalizedHookEvent::ResponseTransform {
            user_id,
            thread_id,
            response,
        } => HookEvent::ResponseTransform {
            user_id,
            thread_id,
            response,
        },
        NormalizedHookEvent::AgentStart { model, provider } => {
            HookEvent::AgentStart { model, provider }
        }
        NormalizedHookEvent::MessageWrite {
            user_id,
            channel,
            content,
            thread_id,
        } => HookEvent::MessageWrite {
            user_id,
            channel,
            content,
            thread_id,
        },
        NormalizedHookEvent::LlmInput {
            model,
            system_message,
            user_message,
            message_count,
            user_id,
        } => HookEvent::LlmInput {
            model,
            system_message,
            user_message,
            message_count,
            user_id,
        },
        NormalizedHookEvent::LlmOutput {
            model,
            content,
            input_tokens,
            output_tokens,
            user_id,
        } => HookEvent::LlmOutput {
            model,
            content,
            input_tokens,
            output_tokens,
            user_id,
        },
        NormalizedHookEvent::TranscribeAudio {
            user_id,
            channel,
            audio_size_bytes,
            mime_type,
            duration_secs,
        } => HookEvent::TranscribeAudio {
            user_id,
            channel,
            audio_size_bytes,
            mime_type,
            duration_secs,
        },
    })
}

fn hook_outcome_to_agent(outcome: HookOutcome) -> AgentHookOutcome {
    match outcome {
        HookOutcome::Continue { modified } => AgentHookOutcome::Continue { modified },
        HookOutcome::Reject { reason } => AgentHookOutcome::Reject { reason },
    }
}

fn hook_error_to_agent(error: HookError) -> HookPortError {
    match error {
        HookError::ExecutionFailed { reason } => HookPortError::ExecutionFailed { reason },
        HookError::Timeout { timeout } => HookPortError::Timeout {
            timeout_ms: timeout.as_millis().min(u128::from(u64::MAX)) as u64,
        },
        HookError::Rejected { reason } => HookPortError::Rejected { reason },
    }
}

/// Map a normalization failure (payload/point mismatch or a missing
/// identity-keyed field) into the existing `HookPortError::ExecutionFailed`
/// variant, so callers of `dispatch_hook` see a normal hook-dispatch error
/// rather than a new, additional error surface.
fn hook_normalization_error_to_agent(error: HookNormalizationError) -> HookPortError {
    HookPortError::ExecutionFailed {
        reason: error.to_string(),
    }
}
