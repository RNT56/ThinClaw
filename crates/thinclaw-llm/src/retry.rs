//! Shared retry helpers and composable `RetryProvider` decorator for LLM providers.
//!
//! Provides:
//! - `is_retryable()` — `LlmError`-level retryability classification (shared with `failover.rs`)
//! - `retry_backoff_delay()` — exponential backoff with jitter
//! - `RetryProvider` — decorator that wraps any `LlmProvider` with automatic retries

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rand::Rng;
use rust_decimal::Decimal;

use thinclaw_llm_core::provider::{
    CompletionRequest, CompletionResponse, LlmProvider, ModelMetadata, StreamSupport,
    TokenCaptureSupport, ToolCompletionRequest, ToolCompletionResponse,
};
use thinclaw_types::error::LlmError;

/// Returns `true` if the `LlmError` is transient and the request should be retried.
///
/// Used by `RetryProvider` (retry the same provider) and `FailoverProvider`
/// (try the next provider). The question is: "could this exact same request
/// succeed if we try again?"
///
/// Retryable: `RequestFailed`, `RateLimited`, `InvalidResponse`,
/// `SessionRenewalFailed`, `Http`, `Io`.
///
/// Non-retryable: `AuthFailed`, `SessionExpired`, `ContextLengthExceeded`,
/// `ModelNotAvailable`, `Json`.
/// - `SessionExpired` — handled by session renewal layer, not by retry
/// - `ModelNotAvailable` — the model won't appear between attempts
/// - `Json` — a serde parse bug, not a transient failure
///
/// See also `circuit_breaker::is_transient()` which answers a different
/// question: "does this error indicate the backend is degraded?"
pub(crate) fn is_retryable(err: &LlmError) -> bool {
    match err {
        LlmError::RequestFailed { reason, .. } => !is_deterministic_request_failure(reason),
        LlmError::RateLimited { .. }
        | LlmError::InvalidResponse { .. }
        | LlmError::SessionRenewalFailed { .. }
        | LlmError::Http(_)
        | LlmError::Io(_) => true,
        _ => false,
    }
}

fn is_deterministic_request_failure(reason: &str) -> bool {
    let normalized = reason.trim().to_ascii_lowercase();
    normalized.contains("message conversion error")
        || normalized.contains("only supports pdf documents")
}

/// Calculate exponential backoff delay with random jitter.
///
/// Base delay is 1 second, doubled each attempt, with +/-25% jitter.
/// - attempt 0: ~1s (0.75s - 1.25s)
/// - attempt 1: ~2s (1.5s - 2.5s)
/// - attempt 2: ~4s (3.0s - 5.0s)
pub(crate) fn retry_backoff_delay(attempt: u32) -> Duration {
    let base_ms: u64 = 1000u64.saturating_mul(2u64.saturating_pow(attempt));
    let jitter_range = base_ms / 4; // 25%
    let jitter = if jitter_range > 0 {
        let offset = rand::thread_rng().gen_range(0..=jitter_range * 2);
        offset as i64 - jitter_range as i64
    } else {
        0
    };
    let delay_ms = (base_ms as i64 + jitter).max(100) as u64;
    Duration::from_millis(delay_ms)
}

/// Configuration for the retry decorator.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (not counting the initial attempt).
    /// Default: 3.
    pub max_retries: u32,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self { max_retries: 3 }
    }
}

/// Composable decorator that wraps any `LlmProvider` with automatic retries.
///
/// On transient errors, sleeps using exponential backoff and retries.
/// On non-transient errors (`AuthFailed`, `ContextLengthExceeded`, `SessionExpired`),
/// returns immediately.
///
/// Special handling for `RateLimited { retry_after }`: uses the provider-suggested
/// duration if available, otherwise falls back to standard backoff.
pub struct RetryProvider {
    inner: Arc<dyn LlmProvider>,
    config: RetryConfig,
}

impl RetryProvider {
    pub fn new(inner: Arc<dyn LlmProvider>, config: RetryConfig) -> Self {
        Self { inner, config }
    }
}

#[async_trait]
impl LlmProvider for RetryProvider {
    fn model_name(&self) -> &str {
        self.inner.model_name()
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        self.inner.cost_per_token()
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let mut last_error: Option<LlmError> = None;

        for attempt in 0..=self.config.max_retries {
            let req = request.clone();
            match self.inner.complete(req).await {
                Ok(resp) => return Ok(resp),
                Err(err) => {
                    if !is_retryable(&err) || attempt == self.config.max_retries {
                        return Err(err);
                    }

                    let delay = match &err {
                        LlmError::RateLimited {
                            retry_after: Some(duration),
                            ..
                        } => *duration,
                        _ => retry_backoff_delay(attempt),
                    };

                    tracing::warn!(
                        provider = %self.inner.model_name(),
                        attempt = attempt + 1,
                        max_retries = self.config.max_retries,
                        delay_ms = delay.as_millis() as u64,
                        error = %err,
                        "Retrying after transient error"
                    );

                    last_error = Some(err);
                    tokio::time::sleep(delay).await;
                }
            }
        }

        Err(last_error.unwrap_or_else(|| LlmError::RequestFailed {
            provider: self.inner.model_name().to_string(),
            reason: "retry loop exited unexpectedly".to_string(),
        }))
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let mut last_error: Option<LlmError> = None;

        for attempt in 0..=self.config.max_retries {
            let req = request.clone();
            match self.inner.complete_with_tools(req).await {
                Ok(resp) => return Ok(resp),
                Err(err) => {
                    if !is_retryable(&err) || attempt == self.config.max_retries {
                        return Err(err);
                    }

                    let delay = match &err {
                        LlmError::RateLimited {
                            retry_after: Some(duration),
                            ..
                        } => *duration,
                        _ => retry_backoff_delay(attempt),
                    };

                    tracing::warn!(
                        provider = %self.inner.model_name(),
                        attempt = attempt + 1,
                        max_retries = self.config.max_retries,
                        delay_ms = delay.as_millis() as u64,
                        error = %err,
                        "Retrying after transient error (tools)"
                    );

                    last_error = Some(err);
                    tokio::time::sleep(delay).await;
                }
            }
        }

        Err(last_error.unwrap_or_else(|| LlmError::RequestFailed {
            provider: self.inner.model_name().to_string(),
            reason: "retry loop exited unexpectedly".to_string(),
        }))
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        self.inner.list_models().await
    }

    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
        self.inner.model_metadata().await
    }

    fn active_model_name(&self) -> String {
        self.inner.active_model_name()
    }

    fn set_model(&self, model: &str) -> Result<(), LlmError> {
        self.inner.set_model(model)
    }

    fn calculate_cost(&self, input_tokens: u32, output_tokens: u32) -> Decimal {
        self.inner.calculate_cost(input_tokens, output_tokens)
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<thinclaw_llm_core::StreamChunkStream, LlmError> {
        // Streaming doesn't support retry (can't restart mid-stream).
        // Forward directly to inner provider's native streaming.
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
