use std::sync::Arc;

use async_trait::async_trait;
use rust_decimal::Decimal;

use super::*;
use crate::error::LlmError;
use crate::llm::{
    CompletionRequest, CompletionResponse, FinishReason, ToolCompletionRequest,
    ToolCompletionResponse, ToolDefinition,
};
use crate::testing::StubLlm;

struct FinishReasonTestLlm {
    response: FinishReason,
}

struct PromptCachingCaptureLlm {
    last_request: Arc<tokio::sync::Mutex<Option<CompletionRequest>>>,
}

struct NonCachingCaptureLlm {
    last_request: Arc<tokio::sync::Mutex<Option<CompletionRequest>>>,
}

struct RoutingCaptureLlm {
    last_completion: Arc<tokio::sync::Mutex<Option<CompletionRequest>>>,
    last_tool_completion: Arc<tokio::sync::Mutex<Option<ToolCompletionRequest>>>,
}

#[async_trait]
impl LlmProvider for PromptCachingCaptureLlm {
    fn model_name(&self) -> &str {
        "prompt-caching-capture"
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        *self.last_request.lock().await = Some(request);
        Ok(CompletionResponse {
            content: "ok".to_string(),
            provider_model: Some(self.model_name().to_string()),
            cost_usd: Some(0.0),
            thinking_content: None,
            input_tokens: 1,
            output_tokens: 1,
            finish_reason: FinishReason::Stop,
            token_capture: None,
        })
    }

    async fn complete_with_tools(
        &self,
        _request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        Ok(ToolCompletionResponse {
            content: Some("ok".to_string()),
            provider_model: Some(self.model_name().to_string()),
            cost_usd: Some(0.0),
            tool_calls: Vec::new(),
            thinking_content: None,
            input_tokens: 1,
            output_tokens: 1,
            finish_reason: FinishReason::Stop,
            token_capture: None,
        })
    }

    fn supports_prompt_caching(&self) -> bool {
        true
    }
}

#[async_trait]
impl LlmProvider for NonCachingCaptureLlm {
    fn model_name(&self) -> &str {
        "non-caching-capture"
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        *self.last_request.lock().await = Some(request);
        Ok(CompletionResponse {
            content: "ok".to_string(),
            provider_model: Some(self.model_name().to_string()),
            cost_usd: Some(0.0),
            thinking_content: None,
            input_tokens: 1,
            output_tokens: 1,
            finish_reason: FinishReason::Stop,
            token_capture: None,
        })
    }

    async fn complete_with_tools(
        &self,
        _request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        Ok(ToolCompletionResponse {
            content: Some("ok".to_string()),
            provider_model: Some(self.model_name().to_string()),
            cost_usd: Some(0.0),
            tool_calls: Vec::new(),
            thinking_content: None,
            input_tokens: 1,
            output_tokens: 1,
            finish_reason: FinishReason::Stop,
            token_capture: None,
        })
    }
}

#[async_trait]
impl LlmProvider for FinishReasonTestLlm {
    fn model_name(&self) -> &str {
        "finish-reason-test"
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Ok(CompletionResponse {
            content: "text response".to_string(),
            provider_model: Some(self.model_name().to_string()),
            cost_usd: Some(0.0),
            thinking_content: None,
            input_tokens: 1,
            output_tokens: 1,
            finish_reason: self.response,
            token_capture: None,
        })
    }

    async fn complete_with_tools(
        &self,
        _request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        Ok(ToolCompletionResponse {
            content: Some("tool-capable response".to_string()),
            provider_model: Some(self.model_name().to_string()),
            cost_usd: Some(0.0),
            tool_calls: Vec::new(),
            thinking_content: None,
            input_tokens: 1,
            output_tokens: 1,
            finish_reason: self.response,
            token_capture: None,
        })
    }
}

#[async_trait]
impl LlmProvider for RoutingCaptureLlm {
    fn model_name(&self) -> &str {
        "routing-capture"
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        *self.last_completion.lock().await = Some(request);
        Ok(CompletionResponse {
            content: "authoritative tool unavailable".to_string(),
            provider_model: Some(self.model_name().to_string()),
            cost_usd: Some(0.0),
            thinking_content: None,
            input_tokens: 1,
            output_tokens: 1,
            finish_reason: FinishReason::Stop,
            token_capture: None,
        })
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        *self.last_tool_completion.lock().await = Some(request);
        Ok(ToolCompletionResponse {
            content: Some("tool route".to_string()),
            provider_model: Some(self.model_name().to_string()),
            cost_usd: Some(0.0),
            tool_calls: vec![ToolCall {
                id: "call_time".to_string(),
                name: "time".to_string(),
                arguments: serde_json::json!({"operation": "now"}),
            }],
            thinking_content: None,
            input_tokens: 1,
            output_tokens: 1,
            finish_reason: FinishReason::ToolUse,
            token_capture: None,
        })
    }
}

#[test]
fn merge_streamed_tool_calls_dedupes_full_and_delta_versions() {
    let tool_calls = vec![ToolCall {
        id: "call_1".to_string(),
        name: "memory_read".to_string(),
        arguments: serde_json::json!({"target": "daily_log"}),
    }];

    let mut partial_tool_calls = std::collections::HashMap::new();
    partial_tool_calls.insert(
        0,
        (
            "call_1".to_string(),
            "memory_read".to_string(),
            r#"{"target":"daily_log"}"#.to_string(),
        ),
    );

    let merged = merge_streamed_tool_calls(tool_calls, partial_tool_calls);

    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].id, "call_1");
    assert_eq!(merged[0].name, "memory_read");
    assert_eq!(
        merged[0].arguments,
        serde_json::json!({"target": "daily_log"})
    );
}

#[test]
fn merge_streamed_tool_calls_preserves_delta_only_calls() {
    let mut partial_tool_calls = std::collections::HashMap::new();
    partial_tool_calls.insert(
        0,
        (
            "call_2".to_string(),
            "memory_read".to_string(),
            r#"{"target":"USER.md"}"#.to_string(),
        ),
    );

    let merged = merge_streamed_tool_calls(Vec::new(), partial_tool_calls);

    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].id, "call_2");
    assert_eq!(merged[0].name, "memory_read");
    assert_eq!(
        merged[0].arguments,
        serde_json::json!({"target": "USER.md"})
    );
}

#[tokio::test]
async fn respond_with_tools_preserves_non_streaming_finish_reason() {
    let reasoning = Reasoning::new(Arc::new(FinishReasonTestLlm {
        response: FinishReason::Length,
    }));
    let context = ReasoningContext::new()
        .with_messages(vec![ChatMessage::user("hello")])
        .with_tools(vec![ToolDefinition {
            name: "demo".to_string(),
            description: "demo tool".to_string(),
            parameters: serde_json::json!({"type":"object"}),
        }]);

    let output = reasoning
        .respond_with_tools(&context)
        .await
        .expect("non-streaming response should succeed");

    assert_eq!(output.finish_reason, FinishReason::Length);
}

#[tokio::test]
async fn respond_with_tools_streaming_preserves_finish_reason() {
    let reasoning = Reasoning::new(Arc::new(FinishReasonTestLlm {
        response: FinishReason::ContentFilter,
    }));
    let context = ReasoningContext::new().with_messages(vec![ChatMessage::user("hello")]);
    let mut streamed = String::new();

    let output = reasoning
        .respond_with_tools_streaming(&context, |chunk| streamed.push_str(chunk))
        .await
        .expect("streaming response should succeed");

    assert_eq!(streamed, "text response");
    assert_eq!(output.finish_reason, FinishReason::ContentFilter);
}

#[test]
fn conversation_prompt_includes_selective_transparency_guidance() {
    let reasoning = Reasoning::new(Arc::new(StubLlm::new("done")));
    let context = ReasoningContext::new().with_tools(vec![ToolDefinition {
        name: "emit_user_message".to_string(),
        description: "Send a progress message.".to_string(),
        parameters: serde_json::json!({"type":"object"}),
    }]);

    let prompt = reasoning.build_conversation_prompt(&context);

    assert!(prompt.contains("Narrate meaningful milestones"));
    assert!(prompt.contains("Avoid noisy play-by-play updates"));
    assert!(prompt.contains("Use `emit_user_message` for durable checkpoints"));
    assert!(!prompt.contains("Don't narrate routine tool calls"));
}

#[test]
fn conversation_prompt_includes_spawn_subagent_guidance_when_available() {
    let reasoning = Reasoning::new(Arc::new(StubLlm::new("done")));
    let context = ReasoningContext::new().with_tools(vec![ToolDefinition {
        name: "spawn_subagent".to_string(),
        description: "Delegate work to a focused sub-agent.".to_string(),
        parameters: serde_json::json!({"type":"object"}),
    }]);

    let prompt = reasoning.build_conversation_prompt(&context);

    assert!(prompt.contains("Use `spawn_subagent` when work can be cleanly delegated"));
    assert!(prompt.contains("Do not delegate tiny tasks"));
}

#[test]
fn conversation_prompt_includes_consult_advisor_guidance_when_available() {
    let reasoning = Reasoning::new(Arc::new(StubLlm::new("done")));
    let context = ReasoningContext::new().with_tools(vec![ToolDefinition {
        name: "consult_advisor".to_string(),
        description: "Consult the advisor lane.".to_string(),
        parameters: serde_json::json!({"type":"object"}),
    }]);

    let prompt = reasoning.build_conversation_prompt(&context);

    assert!(prompt.contains("Use `consult_advisor` for strategic uncertainty"));
    assert!(prompt.contains("Do not spend advisor budget on routine tool calls"));
}

#[test]
fn conversation_prompt_includes_model_guidance_for_gpt_family() {
    let reasoning = Reasoning::new(Arc::new(StubLlm::new("done").with_model_name("gpt-4o")))
        .with_model_name("gpt-4o");

    let prompt = reasoning.build_conversation_prompt(&ReasoningContext::new());

    assert!(prompt.contains("## Model-Specific Guidance"));
    assert!(prompt.contains("GPT-family models:"));
}

#[test]
fn conversation_prompt_skips_model_guidance_when_disabled() {
    let reasoning = Reasoning::new(Arc::new(StubLlm::new("done").with_model_name("gpt-4o")))
        .with_model_name("gpt-4o")
        .with_model_guidance_enabled(false);

    let prompt = reasoning.build_conversation_prompt(&ReasoningContext::new());

    assert!(!prompt.contains("## Model-Specific Guidance"));
}

#[test]
fn conversation_prompt_includes_personality_overlay_after_identity() {
    let reasoning = Reasoning::new(Arc::new(StubLlm::new("done")))
        .with_system_prompt("## Identity\n\nBase identity".to_string())
        .with_personality_overlay("## Temporary Personality\n\nBe extra concise.".to_string());

    let prompt = reasoning.build_conversation_prompt(&ReasoningContext::new());

    assert!(prompt.contains("## Identity\n\nBase identity"));
    assert!(prompt.contains("## Temporary Personality\n\nBe extra concise."));
    assert!(prompt.contains("Base identity\n\n---\n\n## Temporary Personality"));
}

#[test]
fn conversation_prompt_matches_identity_tool_paths() {
    let reasoning = Reasoning::new(Arc::new(StubLlm::new("done")));

    let prompt = reasoning.build_conversation_prompt(&ReasoningContext::new());

    assert!(prompt.contains(
        "For identity/personality updates to SOUL.md, SOUL.local.md, USER.md, or AGENTS.md, use `prompt_manage`."
    ));
    assert!(
        prompt.contains(
            "Use `prompt_manage` for SOUL.md / SOUL.local.md / AGENTS.md / USER.md updates"
        )
    );
    assert!(!prompt.contains("For memory/identity writes (`memory_write`), just do it"));
}

#[test]
fn conversation_prompt_falls_back_to_home_soul_without_workspace_prompt() {
    let temp_home = tempfile::tempdir().expect("temp home");
    let previous_home = std::env::var_os("THINCLAW_HOME");
    unsafe {
        std::env::set_var("THINCLAW_HOME", temp_home.path());
    }
    crate::identity::soul_store::write_home_soul(
        &crate::identity::soul::compose_seeded_soul("balanced").expect("seeded soul"),
    )
    .expect("write home soul");

    let reasoning = Reasoning::new(Arc::new(StubLlm::new("done")));

    let prompt = reasoning.build_conversation_prompt(&ReasoningContext::new());
    assert!(prompt.contains("## Soul"));
    assert!(prompt.contains("Full canonical soul: `memory_read SOUL.md`"));

    if let Some(previous_home) = previous_home {
        unsafe {
            std::env::set_var("THINCLAW_HOME", previous_home);
        }
    } else {
        unsafe {
            std::env::remove_var("THINCLAW_HOME");
        }
    }
}

#[test]
fn conversation_prompt_uses_injected_channel_formatting_hints() {
    let reasoning = Reasoning::new(Arc::new(StubLlm::new("done")))
        .with_channel("custom_channel")
        .with_channel_formatting_hints(
            "- Custom channel uses plain text only.\n- Keep replies to one paragraph.",
        );

    let prompt = reasoning.build_conversation_prompt(&ReasoningContext::new());

    assert!(prompt.contains("## Platform Formatting (custom_channel)"));
    assert!(prompt.contains("Custom channel uses plain text only."));
    assert!(prompt.contains("Keep replies to one paragraph."));
}

#[tokio::test]
async fn respond_attaches_prompt_cache_hint_on_system_message_when_supported() {
    let last_request = Arc::new(tokio::sync::Mutex::new(None));
    let reasoning = Reasoning::new(Arc::new(PromptCachingCaptureLlm {
        last_request: Arc::clone(&last_request),
    }));

    let context = ReasoningContext::new().with_messages(vec![ChatMessage::user("hello")]);
    let _ = reasoning
        .respond_with_tools(&context)
        .await
        .expect("response");

    let request = last_request
        .lock()
        .await
        .clone()
        .expect("request should be captured");
    let system = request
        .messages
        .first()
        .expect("system message should be present");
    let hint = system
        .provider_metadata
        .get("anthropic")
        .and_then(|metadata| metadata.get("cache_control"))
        .and_then(|cache| cache.get("type"))
        .and_then(|value| value.as_str());

    assert_eq!(hint, Some("ephemeral"));
}

#[tokio::test]
async fn respond_omits_prompt_cache_hint_when_provider_does_not_support_it() {
    let last_request = Arc::new(tokio::sync::Mutex::new(None));
    let reasoning = Reasoning::new(Arc::new(NonCachingCaptureLlm {
        last_request: Arc::clone(&last_request),
    }));

    let context = ReasoningContext::new().with_messages(vec![ChatMessage::user("hello")]);
    let _ = reasoning
        .respond_with_tools(&context)
        .await
        .expect("response");

    let request = last_request
        .lock()
        .await
        .clone()
        .expect("request should be captured");
    let system = request
        .messages
        .first()
        .expect("system message should be present");

    assert!(system.provider_metadata.is_empty());
}

#[tokio::test]
async fn respond_prompt_keeps_channel_hints_model_guidance_and_cache_hint_together() {
    let last_request = Arc::new(tokio::sync::Mutex::new(None));
    let reasoning = Reasoning::new(Arc::new(PromptCachingCaptureLlm {
        last_request: Arc::clone(&last_request),
    }))
    .with_model_name("gpt-4o")
    .with_channel("telegram")
    .with_channel_formatting_hints("Use Telegram HTML tags only.");

    let context = ReasoningContext::new().with_messages(vec![ChatMessage::user("hello")]);
    let _ = reasoning
        .respond_with_tools(&context)
        .await
        .expect("response");

    let request = last_request
        .lock()
        .await
        .clone()
        .expect("request should be captured");
    let system = request
        .messages
        .first()
        .expect("system message should be present");

    assert!(system.content.contains("## Model-Specific Guidance"));
    assert!(system.content.contains("## Platform Formatting (telegram)"));
    assert!(
        system
            .provider_metadata
            .get("anthropic")
            .and_then(|metadata| metadata.get("cache_control"))
            .is_some()
    );
}

#[tokio::test]
async fn select_tools_routes_current_time_to_required_time_tool() {
    let llm = Arc::new(RoutingCaptureLlm {
        last_completion: Arc::new(tokio::sync::Mutex::new(None)),
        last_tool_completion: Arc::new(tokio::sync::Mutex::new(None)),
    });
    let reasoning = Reasoning::new(llm.clone());

    let context = ReasoningContext::new()
        .with_messages(vec![ChatMessage::user("What time is it right now?")])
        .with_tools(vec![
            ToolDefinition {
                name: "memory_search".to_string(),
                description: "Search memory".to_string(),
                parameters: serde_json::json!({"type":"object"}),
            },
            ToolDefinition {
                name: "time".to_string(),
                description: "Current time".to_string(),
                parameters: serde_json::json!({
                    "type":"object",
                    "properties":{"operation":{"type":"string"}},
                    "required":["operation"]
                }),
            },
        ]);

    let selections = reasoning
        .select_tools(&context)
        .await
        .expect("tool routing should succeed");

    assert_eq!(selections.len(), 1);
    assert_eq!(selections[0].tool_name, "time");

    let request = llm
        .last_tool_completion
        .lock()
        .await
        .clone()
        .expect("tool request should be captured");
    assert_eq!(request.tool_choice.as_deref(), Some("required"));
    assert_eq!(request.tools.len(), 1);
    assert_eq!(request.tools[0].name, "time");
}

#[tokio::test]
async fn respond_with_tools_refuses_current_time_when_authoritative_tool_missing() {
    let llm = Arc::new(RoutingCaptureLlm {
        last_completion: Arc::new(tokio::sync::Mutex::new(None)),
        last_tool_completion: Arc::new(tokio::sync::Mutex::new(None)),
    });
    let reasoning = Reasoning::new(llm.clone());

    let context = ReasoningContext::new()
        .with_messages(vec![ChatMessage::user("What time is it right now?")])
        .with_tools(vec![ToolDefinition {
            name: "memory_search".to_string(),
            description: "Search memory".to_string(),
            parameters: serde_json::json!({"type":"object"}),
        }]);

    let output = reasoning
        .respond_with_tools(&context)
        .await
        .expect("response should succeed");

    assert!(matches!(output.result, RespondResult::Text(_)));
    assert!(llm.last_tool_completion.lock().await.is_none());
    let request = llm
        .last_completion
        .lock()
        .await
        .clone()
        .expect("text request should be captured");
    assert!(
        request
            .messages
            .iter()
            .any(|message| message.content.contains("Do not guess or fabricate"))
    );
}
