//! Root-independent hook dispatch conversion helpers.

use serde::{Deserialize, Serialize};

use crate::ports::{AgentHookContext, AgentHookEvent, AgentHookPoint, AgentScope};

/// Hook event shape normalized from portable agent hook DTOs.
///
/// Root adapters can map this into their concrete hook registry event type
/// without owning fallback/default-field behavior.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NormalizedHookEvent {
    Inbound {
        user_id: String,
        channel: String,
        content: String,
        thread_id: Option<String>,
    },
    ToolCall {
        tool_name: String,
        parameters: serde_json::Value,
        user_id: String,
        context: String,
    },
    Outbound {
        user_id: String,
        channel: String,
        content: String,
        thread_id: Option<String>,
    },
    SessionStart {
        user_id: String,
        session_id: String,
    },
    SessionEnd {
        user_id: String,
        session_id: String,
    },
    ResponseTransform {
        user_id: String,
        thread_id: String,
        response: String,
    },
    AgentStart {
        model: String,
        provider: String,
    },
    MessageWrite {
        user_id: String,
        channel: String,
        content: String,
        thread_id: Option<String>,
    },
    LlmInput {
        model: String,
        system_message: Option<String>,
        user_message: String,
        message_count: usize,
        user_id: String,
    },
    LlmOutput {
        model: String,
        content: String,
        input_tokens: u32,
        output_tokens: u32,
        user_id: String,
    },
    TranscribeAudio {
        user_id: String,
        channel: String,
        audio_size_bytes: u64,
        mime_type: String,
        duration_secs: Option<f32>,
    },
}

pub fn normalize_agent_hook_event(
    event: AgentHookEvent,
    context: AgentHookContext,
) -> NormalizedHookEvent {
    if let Ok(normalized) = serde_json::from_value::<NormalizedHookEvent>(event.payload.clone()) {
        return normalized;
    }

    let payload = event.payload;
    let scope = context.scope;

    match event.point {
        AgentHookPoint::BeforeInbound => NormalizedHookEvent::Inbound {
            user_id: string_field(&payload, "user_id", scope_principal(scope.as_ref())),
            channel: string_field(&payload, "channel", scope_channel(scope.as_ref())),
            content: string_field(&payload, "content", None),
            thread_id: optional_string_field(&payload, "thread_id")
                .or_else(|| scope_external_thread(scope.as_ref()).map(ToString::to_string)),
        },
        AgentHookPoint::BeforeToolCall => NormalizedHookEvent::ToolCall {
            tool_name: string_field(&payload, "tool_name", None),
            parameters: payload
                .get("parameters")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            user_id: string_field(&payload, "user_id", scope_principal(scope.as_ref())),
            context: string_field(&payload, "context", Some("chat")),
        },
        AgentHookPoint::BeforeOutbound => NormalizedHookEvent::Outbound {
            user_id: string_field(&payload, "user_id", scope_principal(scope.as_ref())),
            channel: string_field(&payload, "channel", scope_channel(scope.as_ref())),
            content: string_field(&payload, "content", None),
            thread_id: optional_string_field(&payload, "thread_id")
                .or_else(|| scope_external_thread(scope.as_ref()).map(ToString::to_string)),
        },
        AgentHookPoint::OnSessionStart => NormalizedHookEvent::SessionStart {
            user_id: string_field(&payload, "user_id", scope_principal(scope.as_ref())),
            session_id: string_field(&payload, "session_id", None),
        },
        AgentHookPoint::OnSessionEnd => NormalizedHookEvent::SessionEnd {
            user_id: string_field(&payload, "user_id", scope_principal(scope.as_ref())),
            session_id: string_field(&payload, "session_id", None),
        },
        AgentHookPoint::TransformResponse => NormalizedHookEvent::ResponseTransform {
            user_id: string_field(&payload, "user_id", scope_principal(scope.as_ref())),
            thread_id: string_field(&payload, "thread_id", scope_external_thread(scope.as_ref())),
            response: string_field(&payload, "response", None),
        },
        AgentHookPoint::BeforeAgentStart => NormalizedHookEvent::AgentStart {
            model: string_field(&payload, "model", None),
            provider: string_field(&payload, "provider", None),
        },
        AgentHookPoint::BeforeMessageWrite => NormalizedHookEvent::MessageWrite {
            user_id: string_field(&payload, "user_id", scope_principal(scope.as_ref())),
            channel: string_field(&payload, "channel", scope_channel(scope.as_ref())),
            content: string_field(&payload, "content", None),
            thread_id: optional_string_field(&payload, "thread_id")
                .or_else(|| scope_external_thread(scope.as_ref()).map(ToString::to_string)),
        },
        AgentHookPoint::BeforeLlmInput => NormalizedHookEvent::LlmInput {
            model: string_field(&payload, "model", None),
            system_message: optional_string_field(&payload, "system_message"),
            user_message: string_field(&payload, "user_message", None),
            message_count: usize_field(&payload, "message_count"),
            user_id: string_field(&payload, "user_id", scope_principal(scope.as_ref())),
        },
        AgentHookPoint::AfterLlmOutput => NormalizedHookEvent::LlmOutput {
            model: string_field(&payload, "model", None),
            content: string_field(&payload, "content", None),
            input_tokens: u32_field(&payload, "input_tokens"),
            output_tokens: u32_field(&payload, "output_tokens"),
            user_id: string_field(&payload, "user_id", scope_principal(scope.as_ref())),
        },
        AgentHookPoint::BeforeTranscribeAudio => NormalizedHookEvent::TranscribeAudio {
            user_id: string_field(&payload, "user_id", scope_principal(scope.as_ref())),
            channel: string_field(&payload, "channel", scope_channel(scope.as_ref())),
            audio_size_bytes: u64_field(&payload, "audio_size_bytes"),
            mime_type: string_field(&payload, "mime_type", None),
            duration_secs: f32_field(&payload, "duration_secs"),
        },
    }
}

fn scope_principal(scope: Option<&AgentScope>) -> Option<&str> {
    scope.map(|scope| scope.principal_id.as_str())
}

fn scope_channel(scope: Option<&AgentScope>) -> Option<&str> {
    scope.and_then(|scope| scope.channel.as_deref())
}

fn scope_external_thread(scope: Option<&AgentScope>) -> Option<&str> {
    scope.and_then(|scope| scope.external_thread_id.as_deref())
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

fn usize_field(payload: &serde_json::Value, key: &str) -> usize {
    payload
        .get(key)
        .and_then(|value| value.as_u64())
        .unwrap_or_default()
        .min(usize::MAX as u64) as usize
}

fn u32_field(payload: &serde_json::Value, key: &str) -> u32 {
    payload
        .get(key)
        .and_then(|value| value.as_u64())
        .unwrap_or_default()
        .min(u32::MAX as u64) as u32
}

fn u64_field(payload: &serde_json::Value, key: &str) -> u64 {
    payload
        .get(key)
        .and_then(|value| value.as_u64())
        .unwrap_or_default()
}

fn f32_field(payload: &serde_json::Value, key: &str) -> Option<f32> {
    payload
        .get(key)
        .and_then(|value| value.as_f64())
        .map(|value| value as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope() -> AgentScope {
        AgentScope::new("user-1", "actor-1")
            .with_channel("web")
            .with_external_thread("thread-1")
    }

    #[test]
    fn hook_dispatch_conversion_uses_scope_fallbacks() {
        let event = normalize_agent_hook_event(
            AgentHookEvent {
                point: AgentHookPoint::BeforeOutbound,
                payload: serde_json::json!({"content": "done"}),
            },
            AgentHookContext {
                metadata: serde_json::Value::Null,
                scope: Some(scope()),
            },
        );

        assert_eq!(
            event,
            NormalizedHookEvent::Outbound {
                user_id: "user-1".to_string(),
                channel: "web".to_string(),
                content: "done".to_string(),
                thread_id: Some("thread-1".to_string()),
            }
        );
    }

    #[test]
    fn hook_dispatch_conversion_prefers_payload_over_scope() {
        let event = normalize_agent_hook_event(
            AgentHookEvent {
                point: AgentHookPoint::BeforeMessageWrite,
                payload: serde_json::json!({
                    "user_id": "payload-user",
                    "channel": "sms",
                    "content": "queued",
                    "thread_id": "payload-thread"
                }),
            },
            AgentHookContext {
                metadata: serde_json::Value::Null,
                scope: Some(scope()),
            },
        );

        assert_eq!(
            event,
            NormalizedHookEvent::MessageWrite {
                user_id: "payload-user".to_string(),
                channel: "sms".to_string(),
                content: "queued".to_string(),
                thread_id: Some("payload-thread".to_string()),
            }
        );
    }

    #[test]
    fn hook_dispatch_conversion_applies_numeric_defaults_and_saturation() {
        let event = normalize_agent_hook_event(
            AgentHookEvent {
                point: AgentHookPoint::AfterLlmOutput,
                payload: serde_json::json!({
                    "model": "gpt-test",
                    "content": "answer",
                    "input_tokens": u64::from(u32::MAX) + 1,
                    "output_tokens": 42
                }),
            },
            AgentHookContext {
                metadata: serde_json::Value::Null,
                scope: Some(scope()),
            },
        );

        assert_eq!(
            event,
            NormalizedHookEvent::LlmOutput {
                model: "gpt-test".to_string(),
                content: "answer".to_string(),
                input_tokens: u32::MAX,
                output_tokens: 42,
                user_id: "user-1".to_string(),
            }
        );

        let event = normalize_agent_hook_event(
            AgentHookEvent {
                point: AgentHookPoint::BeforeTranscribeAudio,
                payload: serde_json::json!({"mime_type": "audio/wav"}),
            },
            AgentHookContext {
                metadata: serde_json::Value::Null,
                scope: None,
            },
        );

        assert_eq!(
            event,
            NormalizedHookEvent::TranscribeAudio {
                user_id: String::new(),
                channel: String::new(),
                audio_size_bytes: 0,
                mime_type: "audio/wav".to_string(),
                duration_secs: None,
            }
        );
    }

    #[test]
    fn hook_dispatch_conversion_preserves_serialized_normalized_event() {
        let payload = serde_json::to_value(NormalizedHookEvent::ToolCall {
            tool_name: "search".to_string(),
            parameters: serde_json::json!({"q": "status"}),
            user_id: "payload-user".to_string(),
            context: "job-7".to_string(),
        })
        .expect("serialize event");

        let event = normalize_agent_hook_event(
            AgentHookEvent {
                point: AgentHookPoint::BeforeOutbound,
                payload,
            },
            AgentHookContext {
                metadata: serde_json::Value::Null,
                scope: Some(scope()),
            },
        );

        assert_eq!(
            event,
            NormalizedHookEvent::ToolCall {
                tool_name: "search".to_string(),
                parameters: serde_json::json!({"q": "status"}),
                user_id: "payload-user".to_string(),
                context: "job-7".to_string(),
            }
        );
    }
}
