//! Core hook types and traits.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Points in the agent lifecycle where hooks can be attached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HookPoint {
    /// Before processing an inbound user message.
    BeforeInbound,
    /// Before executing a tool call.
    BeforeToolCall,
    /// Before sending an outbound response.
    BeforeOutbound,
    /// When a new session starts.
    OnSessionStart,
    /// When a session ends (pruned or expired).
    OnSessionEnd,
    /// Transform the final response before completing a turn.
    TransformResponse,
    /// Before the agent starts. Hooks can inspect the model/provider; a
    /// typed `HookPatch::AgentStart` can request overriding them, but that
    /// patch is not yet honored by any dispatcher call site (see
    /// [`HookPatch`] for the current consumption-point status).
    BeforeAgentStart,
    /// Before writing a message to a channel.
    BeforeMessageWrite,
    /// Before sending a request to the LLM.
    /// Hooks can inspect/modify the user message via the string-based
    /// `HookOutcome::Continue { modified }` channel. System message and
    /// model selection can be inspected, and a typed
    /// `HookPatch::LlmInput { system_message, .. }` can request overriding
    /// the system message once a dispatcher call site consumes
    /// `Hook::execute_patch` (see [`HookPatch`]).
    BeforeLlmInput,
    /// After receiving a response from the LLM.
    /// Hooks can inspect/modify the response content and track token usage.
    AfterLlmOutput,
    /// Before transcribing audio to text.
    /// Hooks can inspect/modify the audio source or skip transcription.
    BeforeTranscribeAudio,
}

impl HookPoint {
    /// Human-readable hook point identifier.
    pub fn as_str(&self) -> &'static str {
        match self {
            HookPoint::BeforeInbound => "beforeInbound",
            HookPoint::BeforeToolCall => "beforeToolCall",
            HookPoint::BeforeOutbound => "beforeOutbound",
            HookPoint::OnSessionStart => "onSessionStart",
            HookPoint::OnSessionEnd => "onSessionEnd",
            HookPoint::TransformResponse => "transformResponse",
            HookPoint::BeforeAgentStart => "beforeAgentStart",
            HookPoint::BeforeMessageWrite => "beforeMessageWrite",
            HookPoint::BeforeLlmInput => "beforeLlmInput",
            HookPoint::AfterLlmOutput => "afterLlmOutput",
            HookPoint::BeforeTranscribeAudio => "beforeTranscribeAudio",
        }
    }
}

/// Contextual data carried with each hook invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HookEvent {
    /// An inbound user message about to be processed.
    Inbound {
        user_id: String,
        channel: String,
        content: String,
        thread_id: Option<String>,
    },
    /// A tool call about to be executed.
    ToolCall {
        tool_name: String,
        parameters: serde_json::Value,
        user_id: String,
        /// "chat" for interactive, or a job ID string for autonomous jobs.
        context: String,
    },
    /// An outbound response about to be sent.
    Outbound {
        user_id: String,
        channel: String,
        content: String,
        thread_id: Option<String>,
    },
    /// A new session was created.
    SessionStart { user_id: String, session_id: String },
    /// A session was ended (pruned).
    SessionEnd { user_id: String, session_id: String },
    /// The final response is being transformed before completing a turn.
    ResponseTransform {
        user_id: String,
        thread_id: String,
        response: String,
    },
    /// The agent is about to start. Hooks can override model/provider.
    AgentStart {
        /// Current model name.
        model: String,
        /// Current provider name.
        provider: String,
    },
    /// A message is about to be written to a channel.
    MessageWrite {
        user_id: String,
        channel: String,
        content: String,
        thread_id: Option<String>,
    },
    /// A request is about to be sent to the LLM.
    LlmInput {
        /// The model being called.
        model: String,
        /// The system message (if any).
        system_message: Option<String>,
        /// The user prompt / last message being sent.
        user_message: String,
        /// Number of messages in the conversation context.
        message_count: usize,
        /// User who triggered this LLM call.
        user_id: String,
    },
    /// A response was received from the LLM.
    LlmOutput {
        /// The model that responded.
        model: String,
        /// The response content.
        content: String,
        /// Input tokens consumed.
        input_tokens: u32,
        /// Output tokens generated.
        output_tokens: u32,
        /// User who triggered this LLM call.
        user_id: String,
    },
    /// Audio is about to be transcribed to text.
    TranscribeAudio {
        /// User who sent the audio.
        user_id: String,
        /// Channel the audio was received on.
        channel: String,
        /// Audio file size in bytes.
        audio_size_bytes: u64,
        /// Audio MIME type (e.g. "audio/ogg", "audio/wav").
        mime_type: String,
        /// Duration in seconds, if known.
        duration_secs: Option<f32>,
    },
}

impl HookEvent {
    /// Returns the [`HookPoint`] this event corresponds to.
    pub fn hook_point(&self) -> HookPoint {
        match self {
            HookEvent::Inbound { .. } => HookPoint::BeforeInbound,
            HookEvent::ToolCall { .. } => HookPoint::BeforeToolCall,
            HookEvent::Outbound { .. } => HookPoint::BeforeOutbound,
            HookEvent::SessionStart { .. } => HookPoint::OnSessionStart,
            HookEvent::SessionEnd { .. } => HookPoint::OnSessionEnd,
            HookEvent::ResponseTransform { .. } => HookPoint::TransformResponse,
            HookEvent::AgentStart { .. } => HookPoint::BeforeAgentStart,
            HookEvent::MessageWrite { .. } => HookPoint::BeforeMessageWrite,
            HookEvent::LlmInput { .. } => HookPoint::BeforeLlmInput,
            HookEvent::LlmOutput { .. } => HookPoint::AfterLlmOutput,
            HookEvent::TranscribeAudio { .. } => HookPoint::BeforeTranscribeAudio,
        }
    }

    /// Apply a modification string to the event's primary content field.
    pub fn apply_modification(&mut self, modified: &str) {
        match self {
            HookEvent::Inbound { content, .. } | HookEvent::Outbound { content, .. } => {
                *content = modified.to_string();
            }
            HookEvent::ToolCall { parameters, .. } => match serde_json::from_str(modified) {
                Ok(parsed) => *parameters = parsed,
                Err(e) => {
                    tracing::warn!(
                        "Hook returned non-JSON modification for ToolCall, ignoring: {}",
                        e
                    );
                }
            },
            HookEvent::ResponseTransform { response, .. } => {
                *response = modified.to_string();
            }
            HookEvent::MessageWrite { content, .. } => {
                *content = modified.to_string();
            }
            HookEvent::LlmInput { user_message, .. } => {
                *user_message = modified.to_string();
            }
            HookEvent::LlmOutput { content, .. } => {
                *content = modified.to_string();
            }
            HookEvent::SessionStart { .. }
            | HookEvent::SessionEnd { .. }
            | HookEvent::AgentStart { .. }
            | HookEvent::TranscribeAudio { .. } => {
                // These events don't have modifiable content
            }
        }
    }
}

/// The result of executing a hook.
#[derive(Debug, Clone)]
pub enum HookOutcome {
    /// Continue processing, optionally with modified content.
    Continue {
        /// If `Some`, replace the event's primary content with this value.
        modified: Option<String>,
    },
    /// Reject the event entirely.
    Reject {
        /// Human-readable reason for the rejection.
        reason: String,
    },
}

impl HookOutcome {
    /// Shorthand for `Continue { modified: None }`.
    pub fn ok() -> Self {
        HookOutcome::Continue { modified: None }
    }

    /// Shorthand for `Continue { modified: Some(value) }`.
    pub fn modify(value: String) -> Self {
        HookOutcome::Continue {
            modified: Some(value),
        }
    }

    /// Shorthand for `Reject { reason }`.
    pub fn reject(reason: impl Into<String>) -> Self {
        HookOutcome::Reject {
            reason: reason.into(),
        }
    }
}

/// A structured, typed modification a hook can request in addition to (or
/// instead of) the legacy string-based [`HookOutcome::Continue`] `modified`
/// field.
///
/// This is an **additive** mechanism: [`HookOutcome`] cannot grow new
/// variants or fields without breaking exhaustive `match` expressions at
/// call sites across the codebase (dispatcher/agent-loop hook call sites in
/// particular), so typed patches travel through a separate, optional
/// channel (see [`Hook::execute_patch`]) rather than through
/// `HookOutcome::Continue`.
///
/// Only the variants and fields documented as "honored" below are actually
/// applied by [`HookPatch::apply_to`]. Unhandled variants/fields are
/// reserved for future wiring and are currently no-ops when applied.
///
/// Consumption point: `HookRegistry::run_with_context` requests a patch
/// from each hook (via [`Hook::execute_patch`]) after its `execute`
/// succeeds and applies it to the evolving event; the dispatcher's
/// `BeforeLlmInput` site (`src/agent/dispatcher/llm_turn.rs`) reads the
/// final event back through `HookRegistry::run_returning_event` and honors
/// `LlmInput` user/system-message overrides. `AgentStart` patches are
/// accepted structurally but not yet honored (see below).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HookPatch {
    /// Patch fields on a [`HookEvent::LlmInput`] event.
    ///
    /// Honored fields: `user_message`, `system_message` (both applied by
    /// [`HookPatch::apply_to`]).
    LlmInput {
        /// If `Some`, replaces the user message sent to the LLM.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user_message: Option<String>,
        /// If `Some`, replaces the system message sent to the LLM.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        system_message: Option<String>,
    },
    /// Patch fields on a [`HookEvent::AgentStart`] event.
    ///
    /// Documented as overridable in [`HookPoint::BeforeAgentStart`], but not
    /// yet honored by [`HookPatch::apply_to`] — reserved for follow-up work.
    AgentStart {
        /// If `Some`, requests overriding the model used at startup.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        /// If `Some`, requests overriding the provider used at startup.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
    },
}

impl HookPatch {
    /// Apply this patch to a [`HookEvent`], mutating only the fields the
    /// patch explicitly sets and only for the variants documented as
    /// honored on [`HookPatch`].
    ///
    /// Applying a patch whose variant does not match `event`'s variant, or
    /// whose fields are not yet honored for that variant, is a no-op.
    pub fn apply_to(&self, event: &mut HookEvent) {
        match (self, event) {
            (
                HookPatch::LlmInput {
                    user_message,
                    system_message,
                },
                HookEvent::LlmInput {
                    user_message: event_user_message,
                    system_message: event_system_message,
                    ..
                },
            ) => {
                if let Some(value) = user_message {
                    *event_user_message = value.clone();
                }
                if let Some(value) = system_message {
                    *event_system_message = Some(value.clone());
                }
            }
            // AgentStart patches are not yet honored — see the doc comment
            // on `HookPatch::AgentStart` and the module-level consumption
            // note above.
            (HookPatch::AgentStart { .. }, _) => {}
            // Patch variant does not match the event variant; nothing to do.
            (HookPatch::LlmInput { .. }, _) => {}
        }
    }
}

/// How to handle hook execution failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookFailureMode {
    /// On error/timeout, continue processing as if the hook returned `ok()`.
    FailOpen,
    /// On error/timeout, reject the event.
    FailClosed,
}

/// Hook execution errors.
#[derive(Debug, thiserror::Error)]
pub enum HookError {
    #[error("Hook execution failed: {reason}")]
    ExecutionFailed { reason: String },

    #[error("Hook timed out after {timeout:?}")]
    Timeout { timeout: Duration },

    #[error("Hook rejected: {reason}")]
    Rejected { reason: String },
}

/// Context passed to hooks alongside the event.
pub struct HookContext {
    /// Arbitrary metadata hooks can use.
    pub metadata: serde_json::Value,
}

impl Default for HookContext {
    fn default() -> Self {
        Self {
            metadata: serde_json::Value::Null,
        }
    }
}

/// Trait for implementing lifecycle hooks.
///
/// Hooks intercept and can modify agent operations at well-defined points.
#[async_trait]
pub trait Hook: Send + Sync {
    /// A unique name for this hook.
    fn name(&self) -> &str;

    /// The lifecycle points this hook should be called at.
    fn hook_points(&self) -> &[HookPoint];

    /// How to handle failures in this hook.
    ///
    /// Default: `FailOpen` (continue on error).
    fn failure_mode(&self) -> HookFailureMode {
        HookFailureMode::FailOpen
    }

    /// Maximum time this hook is allowed to run.
    ///
    /// Default: 5 seconds.
    fn timeout(&self) -> Duration {
        Duration::from_secs(5)
    }

    /// Execute the hook.
    async fn execute(&self, event: &HookEvent, ctx: &HookContext)
    -> Result<HookOutcome, HookError>;

    /// Optionally return a structured [`HookPatch`] alongside (or instead
    /// of) the string-based modification in [`HookOutcome`].
    ///
    /// This is an additive extension point: the default implementation
    /// returns `None`, so existing hooks that only implement `execute` are
    /// unaffected. Hooks that want to request a typed modification (e.g.
    /// patching `LlmInput.system_message`, which the string-based
    /// `HookOutcome::Continue { modified }` channel cannot express) can
    /// override this method.
    ///
    /// Called by `HookRegistry` after each successful `execute`; the
    /// returned patch is applied to the evolving event (see [`HookPatch`]
    /// for which fields each consumer honors).
    fn execute_patch(&self, _event: &HookEvent, _ctx: &HookContext) -> Option<HookPatch> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_patch_llm_input_applies_system_and_user_message() {
        let mut event = HookEvent::LlmInput {
            model: "gpt-test".to_string(),
            system_message: Some("original system".to_string()),
            user_message: "original user".to_string(),
            message_count: 3,
            user_id: "user-1".to_string(),
        };

        let patch = HookPatch::LlmInput {
            user_message: Some("patched user".to_string()),
            system_message: Some("patched system".to_string()),
        };
        patch.apply_to(&mut event);

        match event {
            HookEvent::LlmInput {
                user_message,
                system_message,
                ..
            } => {
                assert_eq!(user_message, "patched user");
                assert_eq!(system_message, Some("patched system".to_string()));
            }
            other => panic!("expected LlmInput event, got {other:?}"),
        }
    }

    #[test]
    fn hook_patch_llm_input_partial_fields_leave_unset_fields_untouched() {
        let mut event = HookEvent::LlmInput {
            model: "gpt-test".to_string(),
            system_message: Some("original system".to_string()),
            user_message: "original user".to_string(),
            message_count: 1,
            user_id: "user-1".to_string(),
        };

        let patch = HookPatch::LlmInput {
            user_message: None,
            system_message: Some("patched system only".to_string()),
        };
        patch.apply_to(&mut event);

        match event {
            HookEvent::LlmInput {
                user_message,
                system_message,
                ..
            } => {
                assert_eq!(user_message, "original user");
                assert_eq!(system_message, Some("patched system only".to_string()));
            }
            other => panic!("expected LlmInput event, got {other:?}"),
        }
    }

    #[test]
    fn hook_patch_mismatched_variant_is_a_no_op() {
        let mut event = HookEvent::Inbound {
            user_id: "user-1".to_string(),
            channel: "test".to_string(),
            content: "hello".to_string(),
            thread_id: None,
        };

        let patch = HookPatch::LlmInput {
            user_message: Some("should not apply".to_string()),
            system_message: None,
        };
        patch.apply_to(&mut event);

        match event {
            HookEvent::Inbound { content, .. } => assert_eq!(content, "hello"),
            other => panic!("expected Inbound event, got {other:?}"),
        }
    }

    #[test]
    fn hook_patch_agent_start_is_not_yet_honored() {
        let mut event = HookEvent::AgentStart {
            model: "original-model".to_string(),
            provider: "original-provider".to_string(),
        };

        let patch = HookPatch::AgentStart {
            model: Some("new-model".to_string()),
            provider: Some("new-provider".to_string()),
        };
        patch.apply_to(&mut event);

        // Documented as not-yet-honored: applying the patch is a no-op
        // until a dispatcher call site consumes `Hook::execute_patch`.
        match event {
            HookEvent::AgentStart { model, provider } => {
                assert_eq!(model, "original-model");
                assert_eq!(provider, "original-provider");
            }
            other => panic!("expected AgentStart event, got {other:?}"),
        }
    }

    #[test]
    fn hook_patch_serializes_with_kind_tag() {
        let patch = HookPatch::LlmInput {
            user_message: Some("hi".to_string()),
            system_message: None,
        };
        let value = serde_json::to_value(&patch).expect("serialize");
        assert_eq!(value["kind"], "llm_input");
        assert_eq!(value["user_message"], "hi");
        assert!(value.get("system_message").is_none());
    }

    #[test]
    fn default_execute_patch_returns_none() {
        struct NoPatchHook;

        #[async_trait]
        impl Hook for NoPatchHook {
            fn name(&self) -> &str {
                "no-patch"
            }
            fn hook_points(&self) -> &[HookPoint] {
                &[HookPoint::BeforeLlmInput]
            }
            async fn execute(
                &self,
                _event: &HookEvent,
                _ctx: &HookContext,
            ) -> Result<HookOutcome, HookError> {
                Ok(HookOutcome::ok())
            }
        }

        let hook = NoPatchHook;
        let event = HookEvent::LlmInput {
            model: "m".to_string(),
            system_message: None,
            user_message: "hi".to_string(),
            message_count: 0,
            user_id: "u".to_string(),
        };
        let ctx = HookContext::default();
        assert!(hook.execute_patch(&event, &ctx).is_none());
    }
}
