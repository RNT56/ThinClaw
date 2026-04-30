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
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use thinclaw_llm_core::provider::{
    CompletionRequest, CompletionResponse, LlmProvider, ModelMetadata, StreamSupport,
    TokenCaptureSupport, ToolCompletionRequest, ToolCompletionResponse,
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

/// Build a deterministic cache key from a completion request.
///
/// Hashes the model name, messages, and response-affecting parameters
/// (max_tokens, temperature, stop_sequences) via SHA-256. Two requests
/// with identical content and parameters produce the same key.
///
/// Note: The system prompt is included implicitly — it is prepended to
/// `messages` as a system-role `ChatMessage` before the request is built,
/// so it is captured by the `serde_json::to_string(&request.messages)` hash.
fn cache_key(model: &str, request: &CompletionRequest) -> String {
    let mut hasher = Sha256::new();
    hasher.update(model.as_bytes());
    hasher.update(b"|");

    // Messages are Serialize, so we can deterministically hash them.
    // serde_json produces stable output for the same input structure.
    if let Ok(json) = serde_json::to_string(&request.messages) {
        hasher.update(json.as_bytes());
    }
    if !request.context_documents.is_empty() {
        hasher.update(b"|docs:");
        if let Ok(json) = serde_json::to_string(&request.context_documents) {
            hasher.update(json.as_bytes());
        }
    }

    // Include response-affecting parameters so different temperatures,
    // max_tokens, or stop sequences produce distinct cache keys.
    hasher.update(b"|");
    if let Some(max_tokens) = request.max_tokens {
        hasher.update(max_tokens.to_le_bytes());
    }
    hasher.update(b"|");
    if let Some(temp) = request.temperature {
        hasher.update(temp.to_le_bytes());
    }
    hasher.update(b"|");
    if let Some(ref stops) = request.stop_sequences {
        for s in stops {
            hasher.update(s.as_bytes());
            hasher.update(b"\x00");
        }
    }

    // Include thinking config so requests with different reasoning budgets
    // (or enabled vs disabled) produce distinct cache keys.
    hasher.update(b"|thinking:");
    match request.thinking {
        thinclaw_llm_core::ThinkingConfig::Disabled => hasher.update(b"off"),
        thinclaw_llm_core::ThinkingConfig::Enabled { budget_tokens } => {
            hasher.update(b"on:");
            hasher.update(budget_tokens.to_le_bytes());
        }
    }

    // Include metadata so requests with different thread/context IDs
    // don't collide.
    if !request.metadata.is_empty() {
        hasher.update(b"|meta:");
        // Sort keys for deterministic ordering.
        let mut keys: Vec<&String> = request.metadata.keys().collect();
        keys.sort();
        for key in keys {
            if let Some(val) = request.metadata.get(key) {
                hasher.update(key.as_bytes());
                hasher.update(b"=");
                hasher.update(val.as_bytes());
                hasher.update(b"\x00");
            }
        }
    }

    format!("{:x}", hasher.finalize())
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
        let key = cache_key(&effective_model, &request);
        let now = Instant::now();

        // Check cache
        {
            let mut guard = self.cache.lock().await;
            if let Some(entry) = guard.get_mut(&key) {
                if now.duration_since(entry.created_at) < self.config.ttl {
                    entry.last_accessed = now;
                    entry.hit_count += 1;
                    tracing::debug!(hits = entry.hit_count, "response cache hit");
                    return Ok(entry.response.clone());
                }
                // Expired, remove it
                guard.remove(&key);
            }
        }

        // Cache miss, call the real provider
        let response = self.inner.complete(request).await?;

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
