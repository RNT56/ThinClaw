//! Root hook-registry adapter for the extracted agent hook-dispatch port.

use std::sync::Arc;

use async_trait::async_trait;
use thinclaw_agent::ports::{
    AgentHookContext, AgentHookEvent, AgentHookOutcome, AgentHookPoint, HookDispatchPort,
    HookPortError,
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
        let hook_event = hook_event_from_agent(event, context)?;
        self.hooks
            .run(&hook_event)
            .await
            .map(hook_outcome_to_agent)
            .map_err(hook_error_to_agent)
    }
}

fn hook_event_from_agent(
    event: AgentHookEvent,
    context: AgentHookContext,
) -> Result<HookEvent, HookPortError> {
    if let Ok(root_event) = serde_json::from_value::<HookEvent>(event.payload.clone()) {
        return Ok(root_event);
    }

    let payload = event.payload;
    match event.point {
        AgentHookPoint::BeforeInbound => Ok(HookEvent::Inbound {
            user_id: string_field(
                &payload,
                "user_id",
                context.scope.as_ref().map(|s| s.principal_id.as_str()),
            ),
            channel: string_field(
                &payload,
                "channel",
                context.scope.as_ref().and_then(|s| s.channel.as_deref()),
            ),
            content: string_field(&payload, "content", None),
            thread_id: optional_string_field(&payload, "thread_id")
                .or_else(|| context.scope.and_then(|s| s.external_thread_id)),
        }),
        AgentHookPoint::BeforeToolCall => Ok(HookEvent::ToolCall {
            tool_name: string_field(&payload, "tool_name", None),
            parameters: payload
                .get("parameters")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            user_id: string_field(
                &payload,
                "user_id",
                context.scope.as_ref().map(|s| s.principal_id.as_str()),
            ),
            context: string_field(&payload, "context", Some("chat")),
        }),
        AgentHookPoint::BeforeOutbound => Ok(HookEvent::Outbound {
            user_id: string_field(
                &payload,
                "user_id",
                context.scope.as_ref().map(|s| s.principal_id.as_str()),
            ),
            channel: string_field(
                &payload,
                "channel",
                context.scope.as_ref().and_then(|s| s.channel.as_deref()),
            ),
            content: string_field(&payload, "content", None),
            thread_id: optional_string_field(&payload, "thread_id")
                .or_else(|| context.scope.and_then(|s| s.external_thread_id)),
        }),
        AgentHookPoint::OnSessionStart => Ok(HookEvent::SessionStart {
            user_id: string_field(
                &payload,
                "user_id",
                context.scope.as_ref().map(|s| s.principal_id.as_str()),
            ),
            session_id: string_field(&payload, "session_id", None),
        }),
        AgentHookPoint::OnSessionEnd => Ok(HookEvent::SessionEnd {
            user_id: string_field(
                &payload,
                "user_id",
                context.scope.as_ref().map(|s| s.principal_id.as_str()),
            ),
            session_id: string_field(&payload, "session_id", None),
        }),
        AgentHookPoint::TransformResponse => Ok(HookEvent::ResponseTransform {
            user_id: string_field(
                &payload,
                "user_id",
                context.scope.as_ref().map(|s| s.principal_id.as_str()),
            ),
            thread_id: string_field(
                &payload,
                "thread_id",
                context
                    .scope
                    .as_ref()
                    .and_then(|s| s.external_thread_id.as_deref()),
            ),
            response: string_field(&payload, "response", None),
        }),
        AgentHookPoint::BeforeAgentStart => Ok(HookEvent::AgentStart {
            model: string_field(&payload, "model", None),
            provider: string_field(&payload, "provider", None),
        }),
        AgentHookPoint::BeforeMessageWrite => Ok(HookEvent::MessageWrite {
            user_id: string_field(
                &payload,
                "user_id",
                context.scope.as_ref().map(|s| s.principal_id.as_str()),
            ),
            channel: string_field(
                &payload,
                "channel",
                context.scope.as_ref().and_then(|s| s.channel.as_deref()),
            ),
            content: string_field(&payload, "content", None),
            thread_id: optional_string_field(&payload, "thread_id")
                .or_else(|| context.scope.and_then(|s| s.external_thread_id)),
        }),
        AgentHookPoint::BeforeLlmInput => Ok(HookEvent::LlmInput {
            model: string_field(&payload, "model", None),
            system_message: optional_string_field(&payload, "system_message"),
            user_message: string_field(&payload, "user_message", None),
            message_count: payload
                .get("message_count")
                .and_then(|value| value.as_u64())
                .unwrap_or_default() as usize,
            user_id: string_field(
                &payload,
                "user_id",
                context.scope.as_ref().map(|s| s.principal_id.as_str()),
            ),
        }),
        AgentHookPoint::AfterLlmOutput => Ok(HookEvent::LlmOutput {
            model: string_field(&payload, "model", None),
            content: string_field(&payload, "content", None),
            input_tokens: u32_field(&payload, "input_tokens"),
            output_tokens: u32_field(&payload, "output_tokens"),
            user_id: string_field(
                &payload,
                "user_id",
                context.scope.as_ref().map(|s| s.principal_id.as_str()),
            ),
        }),
        AgentHookPoint::BeforeTranscribeAudio => Ok(HookEvent::TranscribeAudio {
            user_id: string_field(
                &payload,
                "user_id",
                context.scope.as_ref().map(|s| s.principal_id.as_str()),
            ),
            channel: string_field(
                &payload,
                "channel",
                context.scope.as_ref().and_then(|s| s.channel.as_deref()),
            ),
            audio_size_bytes: payload
                .get("audio_size_bytes")
                .and_then(|value| value.as_u64())
                .unwrap_or_default(),
            mime_type: string_field(&payload, "mime_type", None),
            duration_secs: payload
                .get("duration_secs")
                .and_then(|value| value.as_f64())
                .map(|value| value as f32),
        }),
    }
}

fn string_field(payload: &serde_json::Value, key: &str, fallback: Option<&str>) -> String {
    payload
        .get(key)
        .and_then(|value| value.as_str())
        .or(fallback)
        .unwrap_or_default()
        .to_string()
}

fn optional_string_field(payload: &serde_json::Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
}

fn u32_field(payload: &serde_json::Value, key: &str) -> u32 {
    payload
        .get(key)
        .and_then(|value| value.as_u64())
        .unwrap_or_default()
        .min(u32::MAX as u64) as u32
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

#[cfg(test)]
mod tests {
    use super::*;
    use thinclaw_agent::ports::AgentScope;

    #[test]
    fn hook_event_adapter_uses_scope_fallbacks() {
        let event = hook_event_from_agent(
            AgentHookEvent {
                point: AgentHookPoint::BeforeOutbound,
                payload: serde_json::json!({"content": "done"}),
            },
            AgentHookContext {
                metadata: serde_json::Value::Null,
                scope: Some(
                    AgentScope::new("user-1", "actor-1")
                        .with_channel("web")
                        .with_external_thread("thread-1"),
                ),
            },
        )
        .expect("hook event");

        match event {
            HookEvent::Outbound {
                user_id,
                channel,
                content,
                thread_id,
            } => {
                assert_eq!(user_id, "user-1");
                assert_eq!(channel, "web");
                assert_eq!(content, "done");
                assert_eq!(thread_id.as_deref(), Some("thread-1"));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
