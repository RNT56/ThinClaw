//! Local chat backend — wraps existing InferenceEngine + SidecarManager.

use crate::inference::chat::{ChatBackend, ChatEvent, ChatMessage, ChatRequest, ChatRole};
use crate::inference::{BackendInfo, InferenceError, InferenceResult};
use crate::rig_lib::unified_provider::{ProviderEvent, ProviderKind, UnifiedProvider};
use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;

/// Local chat backend using llama.cpp sidecar or MLX engine.
///
/// This is a thin adapter over the existing `SidecarManager` / `EngineManager`
/// infrastructure. It delegates to the running local inference server via
/// `UnifiedProvider` with `ProviderKind::Local`.
pub struct LocalChatBackend {
    /// Base URL of the local inference server (e.g. `http://127.0.0.1:8080/v1`).
    pub base_url: String,
    /// Auth token for the local server.
    pub token: String,
    /// Model ID reported by the server.
    pub model_name: String,
    /// Context size.
    pub context_size: u32,
    /// Model family hint (e.g. "chatml", "llama3", "gemma").
    pub model_family: Option<String>,
}

impl LocalChatBackend {
    /// Build a UnifiedProvider targeting the local server.
    fn build_provider(&self) -> UnifiedProvider {
        UnifiedProvider::new(
            ProviderKind::Local,
            &self.base_url,
            &self.token,
            &self.model_name,
            self.model_family.clone(),
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
impl ChatBackend for LocalChatBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            id: "local".to_string(),
            display_name: format!(
                "Local ({})",
                self.model_family.as_deref().unwrap_or("llama.cpp")
            ),
            is_local: true,
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
            .map_err(|e| InferenceError::server_not_running(e))?;

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
                    ProviderEvent::ContextUpdate(_) => ChatEvent::Content(String::new()),
                })
                .map_err(|e| InferenceError::server_not_running(e))
        });

        Ok(Box::pin(mapped))
    }

    async fn complete(&self, request: ChatRequest) -> InferenceResult<String> {
        let provider = self.build_provider();

        let mut history = Vec::new();
        let mut system_preamble = None;
        let mut prompt = String::new();

        for (i, msg) in request.messages.iter().enumerate() {
            if i == request.messages.len() - 1 {
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
        let response = provider.completion(rig_request).await.map_err(|e| {
            InferenceError::server_not_running(format!("Local completion failed: {}", e))
        })?;

        match response.choice {
            rig::completion::ModelChoice::Message(content) => Ok(content),
            _ => Err(InferenceError::provider(
                "Received tool call instead of message",
            )),
        }
    }
}
