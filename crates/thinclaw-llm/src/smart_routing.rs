//! Smart routing provider that routes requests to cheap or primary models based on task complexity.
//!
//! Inspired by RelayPlane's cost-reduction approach: simple tasks (status checks, greetings,
//! short questions) go to a cheap model (e.g. Haiku), while complex tasks (code generation,
//! analysis, multi-step reasoning) go to the primary model (e.g. Sonnet/Opus).
//!
//! This is a decorator that wraps two `LlmProvider`s and implements `LlmProvider` itself,
//! following the same pattern as `RetryProvider`, `CachedProvider`, and `CircuitBreakerProvider`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use rust_decimal::Decimal;

use thinclaw_llm_core::provider::{
    CompletionRequest, CompletionResponse, LlmProvider, ModelMetadata, Role, StreamSupport,
    TokenCaptureSupport, ToolCompletionRequest, ToolCompletionResponse,
};
pub use thinclaw_llm_core::smart_routing::{SmartRoutingConfig, TaskComplexity, classify_message};
use thinclaw_types::error::LlmError;

/// Atomic counters for routing observability.
struct SmartRoutingStats {
    total_requests: AtomicU64,
    cheap_requests: AtomicU64,
    primary_requests: AtomicU64,
    cascade_escalations: AtomicU64,
}

impl SmartRoutingStats {
    fn new() -> Self {
        Self {
            total_requests: AtomicU64::new(0),
            cheap_requests: AtomicU64::new(0),
            primary_requests: AtomicU64::new(0),
            cascade_escalations: AtomicU64::new(0),
        }
    }
}

/// Snapshot of routing statistics for external consumption.
#[derive(Debug, Clone)]
pub struct SmartRoutingSnapshot {
    pub total_requests: u64,
    pub cheap_requests: u64,
    pub primary_requests: u64,
    pub cascade_escalations: u64,
}

/// Smart routing provider that classifies task complexity and routes to the appropriate model.
///
/// - `complete()` — classifies and routes to cheap or primary model
/// - `complete_with_tools()` — always routes to primary (tool use requires reliable structured output)
pub struct SmartRoutingProvider {
    primary: Arc<dyn LlmProvider>,
    cheap: Arc<dyn LlmProvider>,
    config: SmartRoutingConfig,
    stats: SmartRoutingStats,
}

impl SmartRoutingProvider {
    /// Create a new smart routing provider wrapping a primary and cheap provider.
    pub fn new(
        primary: Arc<dyn LlmProvider>,
        cheap: Arc<dyn LlmProvider>,
        config: SmartRoutingConfig,
    ) -> Self {
        Self {
            primary,
            cheap,
            config,
            stats: SmartRoutingStats::new(),
        }
    }

    /// Get a snapshot of routing statistics.
    pub fn stats(&self) -> SmartRoutingSnapshot {
        SmartRoutingSnapshot {
            total_requests: self.stats.total_requests.load(Ordering::Relaxed),
            cheap_requests: self.stats.cheap_requests.load(Ordering::Relaxed),
            primary_requests: self.stats.primary_requests.load(Ordering::Relaxed),
            cascade_escalations: self.stats.cascade_escalations.load(Ordering::Relaxed),
        }
    }

    /// Classify the complexity of a request based on its last user message.
    fn classify(&self, request: &CompletionRequest) -> TaskComplexity {
        let last_user_msg = request
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .map(|m| m.content.as_str())
            .unwrap_or("");

        classify_message(last_user_msg, &self.config)
    }

    /// Check if a response from the cheap model shows uncertainty, warranting escalation.
    ///
    /// Bug 7 fix: replaced the previous string-pattern heuristic (which matched
    /// uncertainty phrases like "I need more context" or "could you clarify" and
    /// produced high false-positive escalation rates) with a more conservative
    /// approach:
    ///   1. Empty responses are always uncertain.
    ///   2. Very short responses (< 30 chars) are likely incomplete.
    ///   3. Explicit refusal patterns only (not clarification requests).
    ///
    /// This avoids escalating confident but brief or contextual answers.
    fn response_is_uncertain(response: &CompletionResponse) -> bool {
        let content = response.content.trim();

        // Empty response is always uncertain
        if content.is_empty() {
            return true;
        }

        // Very short response from cheap model likely means incomplete/truncated
        // output. Note: legitimate short answers like "Yes." or "42" are only
        // 3–4 chars; we use 10 chars as the cutoff to avoid escalating those
        // while still catching single-word fragments or error stubs.
        if content.len() < 10 {
            return true;
        }

        let lower = content.to_lowercase();

        // Only escalate on explicit inability signals, not clarification requests
        // (which are valid, confident responses). Bug 7: previous list included
        // "could you clarify" etc. which escalated many legitimate answers.
        let hard_refusal_patterns = [
            "i'm not able to",
            "i am not able to",
            "i cannot complete",
            "i can't complete",
            "beyond my capabilities",
            "i don't have access",
            "i do not have access",
        ];

        hard_refusal_patterns.iter().any(|p| lower.contains(p))
    }
}

#[async_trait]
impl LlmProvider for SmartRoutingProvider {
    fn model_name(&self) -> &str {
        self.primary.model_name()
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        self.primary.cost_per_token()
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.stats.total_requests.fetch_add(1, Ordering::Relaxed);

        let complexity = self.classify(&request);

        match complexity {
            TaskComplexity::Simple => {
                tracing::debug!(
                    model = %self.cheap.model_name(),
                    "Smart routing: Simple task -> cheap model"
                );
                self.stats.cheap_requests.fetch_add(1, Ordering::Relaxed);
                self.cheap.complete(request).await
            }
            TaskComplexity::Complex => {
                tracing::debug!(
                    model = %self.primary.model_name(),
                    "Smart routing: Complex task -> primary model"
                );
                self.stats.primary_requests.fetch_add(1, Ordering::Relaxed);
                self.primary.complete(request).await
            }
            TaskComplexity::Moderate => {
                if self.config.cascade_enabled {
                    tracing::debug!(
                        model = %self.cheap.model_name(),
                        "Smart routing: Moderate task -> cheap model (cascade enabled)"
                    );
                    self.stats.cheap_requests.fetch_add(1, Ordering::Relaxed);

                    let response = self.cheap.complete(request.clone()).await?;

                    if Self::response_is_uncertain(&response) {
                        tracing::info!(
                            cheap_model = %self.cheap.model_name(),
                            primary_model = %self.primary.model_name(),
                            "Smart routing: Escalating to primary (cheap model response uncertain)"
                        );
                        self.stats
                            .cascade_escalations
                            .fetch_add(1, Ordering::Relaxed);
                        self.stats.primary_requests.fetch_add(1, Ordering::Relaxed);
                        self.primary.complete(request).await
                    } else {
                        Ok(response)
                    }
                } else {
                    // Without cascade, moderate tasks go to cheap model
                    tracing::debug!(
                        model = %self.cheap.model_name(),
                        "Smart routing: Moderate task -> cheap model (cascade disabled)"
                    );
                    self.stats.cheap_requests.fetch_add(1, Ordering::Relaxed);
                    self.cheap.complete(request).await
                }
            }
        }
    }

    /// Tool use always goes to the primary model for reliable structured output.
    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        self.stats.total_requests.fetch_add(1, Ordering::Relaxed);
        self.stats.primary_requests.fetch_add(1, Ordering::Relaxed);
        tracing::debug!(
            model = %self.primary.model_name(),
            "Smart routing: Tool use -> primary model (always)"
        );
        self.primary.complete_with_tools(request).await
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        self.primary.list_models().await
    }

    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
        self.primary.model_metadata().await
    }

    fn active_model_name(&self) -> String {
        self.primary.active_model_name()
    }

    fn set_model(&self, model: &str) -> Result<(), LlmError> {
        self.primary.set_model(model)
    }

    fn calculate_cost(&self, input_tokens: u32, output_tokens: u32) -> Decimal {
        self.primary.calculate_cost(input_tokens, output_tokens)
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<thinclaw_llm_core::StreamChunkStream, LlmError> {
        // Streaming always uses primary model (interactive streaming
        // is used for complex, user-facing responses).
        self.primary.complete_stream(request).await
    }

    async fn complete_stream_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<thinclaw_llm_core::StreamChunkStream, LlmError> {
        self.primary.complete_stream_with_tools(request).await
    }

    fn supports_streaming(&self) -> bool {
        self.primary.supports_streaming()
    }

    fn stream_support(&self) -> StreamSupport {
        self.primary.stream_support()
    }

    fn stream_support_for_model(&self, requested_model: Option<&str>) -> StreamSupport {
        self.primary.stream_support_for_model(requested_model)
    }

    fn token_capture_support(&self) -> TokenCaptureSupport {
        self.primary.token_capture_support()
    }

    fn token_capture_support_for_model(
        &self,
        requested_model: Option<&str>,
    ) -> TokenCaptureSupport {
        self.primary
            .token_capture_support_for_model(requested_model)
    }
}
