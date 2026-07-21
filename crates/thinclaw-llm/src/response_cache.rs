//! In-memory LLM response cache with TTL and LRU eviction.
//!
//! Wraps any [`LlmProvider`] and caches [`complete()`] responses keyed
//! by a SHA-256 hash of the messages and model name. Tool-calling
//! requests are never cached since they can trigger side effects.
//!
//! ```text
//! ┌──────────────────────────────────────────────────┐
//! │               CachedProvider                      │
//! │  complete() ──► cache lookup ──► hit? return      │
//! │                                  miss? call inner │
//! │                                  store response   │
//! │                                                    │
//! │  complete_with_tools() ──► always call inner       │
//! └──────────────────────────────────────────────────┘
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use rust_decimal::Decimal;
use tokio::sync::Mutex;

use thinclaw_llm_core::provider::{
    CompletionRequest, CompletionResponse, FinishReason, LlmProvider, ModelMetadata, StreamSupport,
    TokenCaptureSupport, ToolCompletionRequest, ToolCompletionResponse,
    completion_request_cache_key,
};
use thinclaw_types::error::LlmError;

/// Configuration for the response cache.
#[derive(Debug, Clone)]
pub struct ResponseCacheConfig {
    /// Time-to-live for cache entries.
    pub ttl: Duration,
    /// Maximum number of cached entries before LRU eviction.
    pub max_entries: usize,
}

impl Default for ResponseCacheConfig {
    fn default() -> Self {
        Self {
            ttl: Duration::from_secs(3600), // 1 hour
            max_entries: 1000,
        }
    }
}

struct CacheEntry {
    response: CompletionResponse,
    created_at: Instant,
    last_accessed: Instant,
    hit_count: u64,
}

/// LLM provider wrapper that caches `complete()` responses.
///
/// Tool completion requests are always forwarded without caching since
/// tool calls can have side effects that should not be replayed.
pub struct CachedProvider {
    inner: Arc<dyn LlmProvider>,
    cache: Mutex<HashMap<String, CacheEntry>>,
    config: ResponseCacheConfig,
}

impl CachedProvider {
    /// Wrap an existing provider with response caching.
    pub fn new(inner: Arc<dyn LlmProvider>, config: ResponseCacheConfig) -> Self {
        Self {
            inner,
            cache: Mutex::new(HashMap::new()),
            config,
        }
    }

    /// Number of entries currently in the cache.
    pub async fn len(&self) -> usize {
        self.cache.lock().await.len()
    }

    /// Whether the cache is empty.
    pub async fn is_empty(&self) -> bool {
        self.cache.lock().await.is_empty()
    }

    /// Total cache hits across all entries.
    pub async fn total_hits(&self) -> u64 {
        self.cache.lock().await.values().map(|e| e.hit_count).sum()
    }

    /// Clear all cached entries.
    pub async fn clear(&self) {
        self.cache.lock().await.clear();
    }
}

#[async_trait]
impl LlmProvider for CachedProvider {
    fn model_name(&self) -> &str {
        self.inner.model_name()
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        self.inner.cost_per_token()
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let effective_model = self.inner.effective_model_name(request.model.as_deref());
        let key = completion_request_cache_key(&effective_model, &request);
        let now = Instant::now();

        // Check cache
        {
            let mut guard = self.cache.lock().await;
            if let Some(entry) = guard.get_mut(&key) {
                if now.duration_since(entry.created_at) < self.config.ttl {
                    entry.last_accessed = now;
                    entry.hit_count += 1;
                    tracing::debug!(hits = entry.hit_count, "response cache hit");
                    let mut response = entry.response.clone();
                    // A cache hit performs no provider inference. Returning the
                    // original usage/cost would make downstream accounting bill
                    // the same completion again on every hit.
                    response.input_tokens = 0;
                    response.output_tokens = 0;
                    response.cost_usd = Some(0.0);
                    response.token_capture = None;
                    return Ok(response);
                }
                // Expired, remove it
                guard.remove(&key);
            }
        }

        // Cache miss, call the real provider
        let response = self.inner.complete(request).await?;

        // Only complete, normally terminated responses are reusable. Caching a
        // length-truncated or filtered answer makes a transient partial result
        // look authoritative on every subsequent request.
        if response.finish_reason != FinishReason::Stop
            || self.config.max_entries == 0
            || response.provider_model.as_deref() != Some(effective_model.as_str())
        {
            return Ok(response);
        }

        // Store in cache
        {
            let mut guard = self.cache.lock().await;

            // Evict expired entries
            guard.retain(|_, entry| now.duration_since(entry.created_at) < self.config.ttl);

            // LRU eviction if over capacity
            while guard.len() >= self.config.max_entries {
                let oldest_key = guard
                    .iter()
                    .min_by_key(|(_, entry)| entry.last_accessed)
                    .map(|(k, _)| k.clone());

                if let Some(k) = oldest_key {
                    guard.remove(&k);
                } else {
                    break;
                }
            }

            guard.insert(
                key,
                CacheEntry {
                    response: response.clone(),
                    created_at: now,
                    last_accessed: now,
                    hit_count: 0,
                },
            );
        }

        Ok(response)
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        // Never cache tool calls; they can trigger side effects.
        self.inner.complete_with_tools(request).await
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        self.inner.list_models().await
    }

    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
        self.inner.model_metadata().await
    }

    fn effective_model_name(&self, requested_model: Option<&str>) -> String {
        self.inner.effective_model_name(requested_model)
    }

    fn active_model_name(&self) -> String {
        self.inner.active_model_name()
    }

    fn set_model(&self, model: &str) -> Result<(), LlmError> {
        self.inner.set_model(model)
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<thinclaw_llm_core::StreamChunkStream, LlmError> {
        // Streaming responses can't be cached (consumed incrementally).
        // Bypass the cache and forward to the inner provider's native streaming.
        self.inner.complete_stream(request).await
    }

    async fn complete_stream_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<thinclaw_llm_core::StreamChunkStream, LlmError> {
        self.inner.complete_stream_with_tools(request).await
    }

    fn supports_streaming(&self) -> bool {
        self.inner.supports_streaming()
    }

    fn stream_support(&self) -> StreamSupport {
        self.inner.stream_support()
    }

    fn stream_support_for_model(&self, requested_model: Option<&str>) -> StreamSupport {
        self.inner.stream_support_for_model(requested_model)
    }

    fn token_capture_support(&self) -> TokenCaptureSupport {
        self.inner.token_capture_support()
    }

    fn token_capture_support_for_model(
        &self,
        requested_model: Option<&str>,
    ) -> TokenCaptureSupport {
        self.inner.token_capture_support_for_model(requested_model)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use thinclaw_llm_core::ChatMessage;

    struct CountingProvider {
        calls: AtomicUsize,
        finish_reason: FinishReason,
        reported_model: Option<&'static str>,
    }

    #[async_trait]
    impl LlmProvider for CountingProvider {
        fn model_name(&self) -> &str {
            "counting"
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(CompletionResponse {
                content: format!("response-{call}"),
                provider_model: self.reported_model.map(str::to_string),
                cost_usd: None,
                thinking_content: None,
                input_tokens: 1,
                output_tokens: 1,
                finish_reason: self.finish_reason,
                token_capture: None,
            })
        }

        async fn complete_with_tools(
            &self,
            _request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            Ok(ToolCompletionResponse {
                content: Some("tools".to_string()),
                provider_model: self.reported_model.map(str::to_string),
                cost_usd: None,
                tool_calls: Vec::new(),
                thinking_content: None,
                input_tokens: 1,
                output_tokens: 1,
                finish_reason: self.finish_reason,
                token_capture: None,
            })
        }
    }

    #[tokio::test]
    async fn normally_finished_responses_are_reused() {
        let inner = Arc::new(CountingProvider {
            calls: AtomicUsize::new(0),
            finish_reason: FinishReason::Stop,
            reported_model: Some("counting"),
        });
        let cached = CachedProvider::new(inner.clone(), ResponseCacheConfig::default());
        let request = CompletionRequest::new(vec![ChatMessage::user("same")]);

        let first = cached.complete(request.clone()).await.unwrap();
        let second = cached.complete(request).await.unwrap();

        assert_eq!(first.content, second.content);
        assert_eq!(first.input_tokens, 1);
        assert_eq!(second.input_tokens, 0);
        assert_eq!(second.output_tokens, 0);
        assert_eq!(second.cost_usd, Some(0.0));
        assert_eq!(inner.calls.load(Ordering::SeqCst), 1);
        assert_eq!(cached.total_hits().await, 1);
    }

    #[tokio::test]
    async fn truncated_responses_are_never_reused() {
        let inner = Arc::new(CountingProvider {
            calls: AtomicUsize::new(0),
            finish_reason: FinishReason::Length,
            reported_model: Some("counting"),
        });
        let cached = CachedProvider::new(inner.clone(), ResponseCacheConfig::default());
        let request = CompletionRequest::new(vec![ChatMessage::user("same")]);

        let first = cached.complete(request.clone()).await.unwrap();
        let second = cached.complete(request).await.unwrap();

        assert_ne!(first.content, second.content);
        assert_eq!(inner.calls.load(Ordering::SeqCst), 2);
        assert!(cached.is_empty().await);
    }

    #[tokio::test]
    async fn zero_capacity_disables_storage() {
        let inner = Arc::new(CountingProvider {
            calls: AtomicUsize::new(0),
            finish_reason: FinishReason::Stop,
            reported_model: Some("counting"),
        });
        let cached = CachedProvider::new(
            inner.clone(),
            ResponseCacheConfig {
                max_entries: 0,
                ..ResponseCacheConfig::default()
            },
        );
        let request = CompletionRequest::new(vec![ChatMessage::user("same")]);

        cached.complete(request.clone()).await.unwrap();
        cached.complete(request).await.unwrap();

        assert_eq!(inner.calls.load(Ordering::SeqCst), 2);
        assert!(cached.is_empty().await);
    }

    #[tokio::test]
    async fn unknown_servicing_model_is_never_reused() {
        let inner = Arc::new(CountingProvider {
            calls: AtomicUsize::new(0),
            finish_reason: FinishReason::Stop,
            reported_model: None,
        });
        let cached = CachedProvider::new(inner.clone(), ResponseCacheConfig::default());
        let request = CompletionRequest::new(vec![ChatMessage::user("same")]);

        let first = cached.complete(request.clone()).await.unwrap();
        let second = cached.complete(request).await.unwrap();

        assert_ne!(first.content, second.content);
        assert_eq!(inner.calls.load(Ordering::SeqCst), 2);
        assert!(cached.is_empty().await);
    }
}
