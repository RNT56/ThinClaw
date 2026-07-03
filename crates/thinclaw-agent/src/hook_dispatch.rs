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

impl NormalizedHookEvent {
    /// The externally-tagged serde variant name serde_json would emit for
    /// this event (matches `#[derive(Serialize)]`'s default external
    /// tagging: `{"<Variant>": {...}}`).
    fn variant_tag(&self) -> &'static str {
        match self {
            NormalizedHookEvent::Inbound { .. } => "Inbound",
            NormalizedHookEvent::ToolCall { .. } => "ToolCall",
            NormalizedHookEvent::Outbound { .. } => "Outbound",
            NormalizedHookEvent::SessionStart { .. } => "SessionStart",
            NormalizedHookEvent::SessionEnd { .. } => "SessionEnd",
            NormalizedHookEvent::ResponseTransform { .. } => "ResponseTransform",
            NormalizedHookEvent::AgentStart { .. } => "AgentStart",
            NormalizedHookEvent::MessageWrite { .. } => "MessageWrite",
            NormalizedHookEvent::LlmInput { .. } => "LlmInput",
            NormalizedHookEvent::LlmOutput { .. } => "LlmOutput",
            NormalizedHookEvent::TranscribeAudio { .. } => "TranscribeAudio",
        }
    }
}

/// The externally-tagged variant name a well-formed, already-normalized
/// payload must carry to be accepted for `point` without reconstruction.
fn expected_variant_tag(point: AgentHookPoint) -> &'static str {
    match point {
        AgentHookPoint::BeforeInbound => "Inbound",
        AgentHookPoint::BeforeToolCall => "ToolCall",
        AgentHookPoint::BeforeOutbound => "Outbound",
        AgentHookPoint::OnSessionStart => "SessionStart",
        AgentHookPoint::OnSessionEnd => "SessionEnd",
        AgentHookPoint::TransformResponse => "ResponseTransform",
        AgentHookPoint::BeforeAgentStart => "AgentStart",
        AgentHookPoint::BeforeMessageWrite => "MessageWrite",
        AgentHookPoint::BeforeLlmInput => "LlmInput",
        AgentHookPoint::AfterLlmOutput => "LlmOutput",
        AgentHookPoint::BeforeTranscribeAudio => "TranscribeAudio",
    }
}

/// Error returned when a hook event payload cannot be normalized for its
/// declared [`AgentHookPoint`].
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum HookNormalizationError {
    /// The payload was already shaped like a fully-formed
    /// [`NormalizedHookEvent`], but for a *different* variant than the one
    /// `event.point` declares. Re-routing to the payload's variant would
    /// silently dispatch to the wrong hook point, so this is rejected
    /// instead of guessed at.
    #[error(
        "hook event payload is shaped like `{payload_variant}` but declared hook point `{point:?}` expects `{expected_variant}`"
    )]
    PayloadPointMismatch {
        point: AgentHookPoint,
        expected_variant: &'static str,
        payload_variant: &'static str,
    },
    /// An identity-keyed field (currently: `user_id`) could not be resolved
    /// from either the payload or the invocation scope. Previously this
    /// silently defaulted to `""`, which let hook logic key off an empty
    /// user id as if it were a real (if unusual) identity.
    #[error("hook event for `{point:?}` is missing required identity field `{field}`")]
    MissingIdentity {
        point: AgentHookPoint,
        field: &'static str,
    },
}

/// Normalize a portable [`AgentHookEvent`]/[`AgentHookContext`] pair into a
/// [`NormalizedHookEvent`].
///
/// Normalization is **point-driven**: the payload is only accepted as an
/// already-normalized [`NormalizedHookEvent`] if it deserializes to the
/// variant matching `event.point`. A payload shaped like a different
/// variant (e.g. a `ToolCall`-shaped payload declared under
/// `BeforeOutbound`) is rejected via
/// [`HookNormalizationError::PayloadPointMismatch`] rather than silently
/// re-routed to whatever hook point the payload happens to match — that
/// mismatch previously caused hooks registered for one point to run against
/// a different point's semantics.
///
/// Identity-keyed fields (`user_id`) that cannot be resolved from either
/// the payload or `context.scope` are reported via
/// [`HookNormalizationError::MissingIdentity`] instead of silently
/// defaulting to an empty string.
pub fn normalize_agent_hook_event(
    event: AgentHookEvent,
    context: AgentHookContext,
) -> Result<NormalizedHookEvent, HookNormalizationError> {
    let point = event.point;
    let expected_tag = expected_variant_tag(point);

    // Only accept the fast path (payload is already a fully-formed
    // NormalizedHookEvent) when it matches the declared point. Trying every
    // variant and taking whichever happens to parse is exactly the bug this
    // guards against: a `ToolCall`-shaped payload declared under
    // `BeforeOutbound` must not silently become a `ToolCall` event.
    if let Ok(normalized) = serde_json::from_value::<NormalizedHookEvent>(event.payload.clone()) {
        let payload_tag = normalized.variant_tag();
        if payload_tag == expected_tag {
            return Ok(normalized);
        }
        tracing::warn!(
            point = ?point,
            expected_variant = expected_tag,
            payload_variant = payload_tag,
            "Hook event payload shape does not match declared hook point; rejecting instead of \
             re-routing to the payload's variant"
        );
        return Err(HookNormalizationError::PayloadPointMismatch {
            point,
            expected_variant: expected_tag,
            payload_variant: payload_tag,
        });
    }

    let payload = event.payload;
    let scope = context.scope;

    let user_id = |payload: &serde_json::Value| -> Result<String, HookNormalizationError> {
        optional_string_field(payload, "user_id")
            .or_else(|| scope_principal(scope.as_ref()).map(ToString::to_string))
            .ok_or(HookNormalizationError::MissingIdentity {
                point,
                field: "user_id",
            })
    };

    Ok(match point {
        AgentHookPoint::BeforeInbound => NormalizedHookEvent::Inbound {
            user_id: user_id(&payload)?,
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
            user_id: user_id(&payload)?,
            context: string_field(&payload, "context", Some("chat")),
        },
        AgentHookPoint::BeforeOutbound => NormalizedHookEvent::Outbound {
            user_id: user_id(&payload)?,
            channel: string_field(&payload, "channel", scope_channel(scope.as_ref())),
            content: string_field(&payload, "content", None),
            thread_id: optional_string_field(&payload, "thread_id")
                .or_else(|| scope_external_thread(scope.as_ref()).map(ToString::to_string)),
        },
        AgentHookPoint::OnSessionStart => NormalizedHookEvent::SessionStart {
            user_id: user_id(&payload)?,
            session_id: string_field(&payload, "session_id", None),
        },
        AgentHookPoint::OnSessionEnd => NormalizedHookEvent::SessionEnd {
            user_id: user_id(&payload)?,
            session_id: string_field(&payload, "session_id", None),
        },
        AgentHookPoint::TransformResponse => NormalizedHookEvent::ResponseTransform {
            user_id: user_id(&payload)?,
            thread_id: string_field(&payload, "thread_id", scope_external_thread(scope.as_ref())),
            response: string_field(&payload, "response", None),
        },
        AgentHookPoint::BeforeAgentStart => NormalizedHookEvent::AgentStart {
            model: string_field(&payload, "model", None),
            provider: string_field(&payload, "provider", None),
        },
        AgentHookPoint::BeforeMessageWrite => NormalizedHookEvent::MessageWrite {
            user_id: user_id(&payload)?,
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
            user_id: user_id(&payload)?,
        },
        AgentHookPoint::AfterLlmOutput => NormalizedHookEvent::LlmOutput {
            model: string_field(&payload, "model", None),
            content: string_field(&payload, "content", None),
            input_tokens: u32_field(&payload, "input_tokens"),
            output_tokens: u32_field(&payload, "output_tokens"),
            user_id: user_id(&payload)?,
        },
        AgentHookPoint::BeforeTranscribeAudio => NormalizedHookEvent::TranscribeAudio {
            user_id: user_id(&payload)?,
            channel: string_field(&payload, "channel", scope_channel(scope.as_ref())),
            audio_size_bytes: u64_field(&payload, "audio_size_bytes"),
            mime_type: string_field(&payload, "mime_type", None),
            duration_secs: f32_field(&payload, "duration_secs"),
        },
    })
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
        )
        .expect("normalize");

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
        )
        .expect("normalize");

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
        )
        .expect("normalize");

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
                payload: serde_json::json!({"mime_type": "audio/wav", "user_id": "user-1"}),
            },
            AgentHookContext {
                metadata: serde_json::Value::Null,
                scope: None,
            },
        )
        .expect("normalize");

        assert_eq!(
            event,
            NormalizedHookEvent::TranscribeAudio {
                user_id: "user-1".to_string(),
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
                point: AgentHookPoint::BeforeToolCall,
                payload,
            },
            AgentHookContext {
                metadata: serde_json::Value::Null,
                scope: Some(scope()),
            },
        )
        .expect("normalize");

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

    /// Regression test: a payload shaped like a fully-formed
    /// `NormalizedHookEvent::ToolCall` (e.g. produced by a caller that
    /// mistakenly re-used a serialized event from a different call site)
    /// must NOT silently re-route a `BeforeOutbound`-declared event into a
    /// `ToolCall` event. Previously `normalize_agent_hook_event` tried
    /// every `NormalizedHookEvent` variant against the payload before ever
    /// consulting `event.point`, so this exact input yielded
    /// `NormalizedHookEvent::ToolCall` even though the hook point said
    /// `BeforeOutbound`. It must now be rejected instead.
    #[test]
    fn hook_dispatch_conversion_rejects_payload_shaped_for_a_different_point() {
        let payload = serde_json::to_value(NormalizedHookEvent::ToolCall {
            tool_name: "search".to_string(),
            parameters: serde_json::json!({"q": "status"}),
            user_id: "payload-user".to_string(),
            context: "job-7".to_string(),
        })
        .expect("serialize event");

        let result = normalize_agent_hook_event(
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
            result,
            Err(HookNormalizationError::PayloadPointMismatch {
                point: AgentHookPoint::BeforeOutbound,
                expected_variant: "Outbound",
                payload_variant: "ToolCall",
            })
        );
    }

    #[test]
    fn hook_dispatch_conversion_errors_on_missing_identity_without_scope() {
        let result = normalize_agent_hook_event(
            AgentHookEvent {
                point: AgentHookPoint::BeforeInbound,
                payload: serde_json::json!({"content": "hi", "channel": "web"}),
            },
            AgentHookContext {
                metadata: serde_json::Value::Null,
                scope: None,
            },
        );

        assert_eq!(
            result,
            Err(HookNormalizationError::MissingIdentity {
                point: AgentHookPoint::BeforeInbound,
                field: "user_id",
            })
        );
    }

    #[test]
    fn hook_dispatch_conversion_agent_start_does_not_require_identity() {
        // AgentStart carries no user_id field at all, so it must not be
        // affected by identity-field validation.
        let event = normalize_agent_hook_event(
            AgentHookEvent {
                point: AgentHookPoint::BeforeAgentStart,
                payload: serde_json::json!({"model": "gpt-test", "provider": "openai"}),
            },
            AgentHookContext {
                metadata: serde_json::Value::Null,
                scope: None,
            },
        )
        .expect("normalize");

        assert_eq!(
            event,
            NormalizedHookEvent::AgentStart {
                model: "gpt-test".to_string(),
                provider: "openai".to_string(),
            }
        );
    }

    #[test]
    fn hook_dispatch_conversion_metadata_is_not_dropped_by_caller_contract() {
        // normalize_agent_hook_event only produces a NormalizedHookEvent;
        // metadata propagation to the hook itself is the root adapter's
        // responsibility (see src/agent/hook_dispatch.rs), but this test
        // documents that `context.metadata` is accepted here without
        // panicking or being required, so future refactors don't
        // accidentally start rejecting non-null metadata.
        let event = normalize_agent_hook_event(
            AgentHookEvent {
                point: AgentHookPoint::BeforeInbound,
                payload: serde_json::json!({"content": "hi", "channel": "web"}),
            },
            AgentHookContext {
                metadata: serde_json::json!({"trace_id": "abc-123"}),
                scope: Some(scope()),
            },
        )
        .expect("normalize");

        assert_eq!(
            event,
            NormalizedHookEvent::Inbound {
                user_id: "user-1".to_string(),
                channel: "web".to_string(),
                content: "hi".to_string(),
                thread_id: Some("thread-1".to_string()),
            }
        );
    }
}
