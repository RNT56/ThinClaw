//! Cloud chat backend — wraps the existing UnifiedProvider.

use crate::inference::chat::{ChatBackend, ChatEvent, ChatMessage, ChatRequest, ChatRole};
use crate::inference::{BackendInfo, InferenceError, InferenceResult};
use crate::rig_lib::unified_provider::{ProviderEvent, ProviderKind, UnifiedProvider};
use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;
use thinclaw_runtime_contracts::{ApiStyle, ProviderEndpoint};

/// Cloud chat backend for any supported provider.
///
/// Wraps `UnifiedProvider` with the appropriate API key and base URL,
/// looked up from the shared provider catalog.
pub struct CloudChatBackend {
    /// Provider keychain slug (e.g. "anthropic", "openai").
    pub provider_id: String,
    /// API key (read from SecretStore at construction time).
    pub api_key: String,
    /// Base URL for the provider API.
    pub base_url: String,
    /// Display name.
    pub display_name: String,
    /// Active model.
    pub model_name: String,
    /// Context window size.
    pub context_size: u32,
    /// API compatibility mode.
    pub api_style: ApiStyle,
}

impl CloudChatBackend {
    /// Create from a provider endpoint + live API key.
    pub fn from_endpoint(
        provider_id: &str,
        endpoint: &ProviderEndpoint,
        api_key: String,
        model_override: Option<String>,
        context_size_override: Option<u32>,
    ) -> Self {
        Self {
            provider_id: provider_id.to_string(),
            api_key,
            base_url: endpoint.base_url.to_string(),
            display_name: endpoint.display_name.to_string(),
            model_name: model_override.unwrap_or_else(|| endpoint.default_model.to_string()),
            context_size: context_size_override.unwrap_or(endpoint.default_context_size),
            api_style: endpoint.api_style,
        }
    }

    /// Map shared ApiStyle to UnifiedProvider's ProviderKind.
    fn provider_kind(&self) -> ProviderKind {
        match self.api_style {
            ApiStyle::OpenAi => ProviderKind::OpenAI,
            ApiStyle::Anthropic => ProviderKind::Anthropic,
            ApiStyle::OpenAiCompatible => ProviderKind::OpenAI,
            ApiStyle::Ollama => ProviderKind::OpenAI,
        }
    }

    /// Build the UnifiedProvider for this backend.
    fn build_provider(&self) -> UnifiedProvider {
        UnifiedProvider::new(
            self.provider_kind(),
            &self.base_url,
            &self.api_key,
            &self.model_name,
            None,
        )
    }

    /// Convert our ChatMessages to serde_json::Value messages for UnifiedProvider.
    fn messages_to_json(messages: &[ChatMessage]) -> Vec<serde_json::Value> {
        messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    ChatRole::System => "system",
                    ChatRole::User => "user",
                    ChatRole::Assistant => "assistant",
                };
                serde_json::json!({
                    "role": role,
                    "content": m.content
                })
            })
            .collect()
    }
}

#[async_trait]
impl ChatBackend for CloudChatBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            id: self.provider_id.clone(),
            display_name: self.display_name.clone(),
            is_local: false,
            model_id: Some(self.model_name.clone()),
            available: true,
        }
    }

    async fn stream_chat(
        &self,
        request: ChatRequest,
    ) -> InferenceResult<Pin<Box<dyn Stream<Item = InferenceResult<ChatEvent>> + Send>>> {
        let provider = self.build_provider();
        let messages = Self::messages_to_json(&request.messages);
        let temperature = request.temperature;

        let raw_stream = provider
            .stream_raw_completion(messages, temperature)
            .await
            .map_err(|e| InferenceError::provider(e))?;

        // Map ProviderEvent -> ChatEvent
        use futures::StreamExt;
        let mapped = raw_stream.map(|result| {
            result
                .map(|event| match event {
                    ProviderEvent::Content(text) => ChatEvent::Content(text),
                    ProviderEvent::Usage(usage) => ChatEvent::Usage {
                        prompt_tokens: usage.prompt_tokens,
                        completion_tokens: usage.completion_tokens,
                    },
                    ProviderEvent::ContextUpdate(_) => {
                        // Context updates are agent-level concerns, not chat-level
                        ChatEvent::Content(String::new())
                    }
                })
                .map_err(|e| InferenceError::provider(e))
        });

        Ok(Box::pin(mapped))
    }

    async fn complete(&self, request: ChatRequest) -> InferenceResult<String> {
        let provider = self.build_provider();

        // Build rig CompletionRequest from our ChatRequest
        let mut history = Vec::new();
        let mut system_preamble = None;
        let mut prompt = String::new();

        for (i, msg) in request.messages.iter().enumerate() {
            if i == request.messages.len() - 1 {
                // Last message is the prompt
                prompt = msg.content.clone();
            } else {
                match msg.role {
                    ChatRole::System => {
                        system_preamble = Some(msg.content.clone());
                    }
                    _ => {
                        let role = match msg.role {
                            ChatRole::User => "user",
                            ChatRole::Assistant => "assistant",
                            _ => "user",
                        };
                        history.push(rig::completion::Message {
                            role: role.to_string(),
                            content: msg.content.clone(),
                        });
                    }
                }
            }
        }

        let rig_request = rig::completion::CompletionRequest {
            preamble: system_preamble,
            chat_history: history,
            prompt,
            documents: vec![],
            tools: Vec::new(),
            temperature: request.temperature,
            max_tokens: request.max_tokens.map(|t| t as u64),
            additional_params: None,
        };

        use rig::completion::CompletionModel;
        let response = provider
            .completion(rig_request)
            .await
            .map_err(|e| InferenceError::provider(format!("Completion failed: {}", e)))?;

        match response.choice {
            rig::completion::ModelChoice::Message(content) => Ok(content),
            _ => Err(InferenceError::provider(
                "Received tool call instead of message",
            )),
        }
    }
}
