//! Chat inference backend trait and types.

pub mod cloud;
pub mod local;

use super::{BackendInfo, InferenceResult};
use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;

/// A streamed chat response chunk.
#[derive(Debug, Clone)]
pub enum ChatEvent {
    /// A content token.
    Content(String),
    /// Token usage info (sent once, at the end).
    Usage {
        prompt_tokens: u32,
        completion_tokens: u32,
    },
    /// The stream is done.
    Done,
}

/// Message role for chat backends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatRole {
    System,
    User,
    Assistant,
}

/// A chat message passed to the backend.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

/// Chat completion request.
#[derive(Debug, Clone)]
pub struct ChatRequest {
    /// The conversation messages (including system prompt).
    pub messages: Vec<ChatMessage>,
    /// Desired temperature (0.0–2.0).
    pub temperature: Option<f64>,
    /// Top-p sampling.
    pub top_p: Option<f64>,
    /// Maximum tokens to generate.
    pub max_tokens: Option<u32>,
    /// Model override (if not using the backend's default).
    pub model: Option<String>,
}

/// Chat completion backend — local or cloud.
#[async_trait]
pub trait ChatBackend: Send + Sync {
    /// Information about this backend.
    fn info(&self) -> BackendInfo;

    /// Run a streaming chat completion.  Returns a stream of `ChatEvent`s.
    async fn stream_chat(
        &self,
        request: ChatRequest,
    ) -> InferenceResult<Pin<Box<dyn Stream<Item = InferenceResult<ChatEvent>> + Send>>>;

    /// Run a non-streaming chat completion.  Returns the full response text.
    async fn complete(&self, request: ChatRequest) -> InferenceResult<String>;

    /// Estimate token count for the given messages.
    /// Implementations may use provider-specific tokenizers or fall back to
    /// a simple heuristic (chars / 4).
    async fn count_tokens(&self, messages: &[ChatMessage]) -> InferenceResult<u32> {
        let total_chars: usize = messages.iter().map(|m| m.content.len()).sum();
        Ok((total_chars / 4) as u32)
    }
}
