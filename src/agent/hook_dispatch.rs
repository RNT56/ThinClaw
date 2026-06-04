//! Root hook-registry adapter for the extracted agent hook-dispatch port.

use std::sync::Arc;

use async_trait::async_trait;
use thinclaw_agent::hook_dispatch::{NormalizedHookEvent, normalize_agent_hook_event};
use thinclaw_agent::ports::{
    AgentHookContext, AgentHookEvent, AgentHookOutcome, HookDispatchPort, HookPortError,
};

use crate::hooks::{HookError, HookEvent, HookOutcome, HookRegistry};

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
        let hook_event = hook_event_from_agent(event, context);
        self.hooks
            .run(&hook_event)
            .await
            .map(hook_outcome_to_agent)
            .map_err(hook_error_to_agent)
    }
}

fn hook_event_from_agent(event: AgentHookEvent, context: AgentHookContext) -> HookEvent {
    match normalize_agent_hook_event(event, context) {
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
    }
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
