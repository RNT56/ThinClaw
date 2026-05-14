//! Circuit breaker for LLM providers.
//!
//! Wraps any `LlmProvider` with a state machine that trips open after
//! consecutive transient failures, preventing request storms against a
//! degraded backend. Automatically probes for recovery via half-open state.
//!
//! ```text
//!   Closed ──(failures >= threshold)──► Open
//!     ▲                                   │
//!     │                          (recovery timeout)
//!     │                                   ▼
//!     └──(probe succeeds)──── HalfOpen ──(probe fails)──► Open
//! ```

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use rust_decimal::Decimal;
use tokio::sync::Mutex;

use thinclaw_llm_core::provider::{
    CompletionRequest, CompletionResponse, LlmProvider, ModelMetadata, StreamSupport,
    TokenCaptureSupport, ToolCompletionRequest, ToolCompletionResponse,
};
use thinclaw_types::error::LlmError;

/// Configuration for the circuit breaker.
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Consecutive transient failures before the circuit opens.
    pub failure_threshold: u32,
    /// How long the circuit stays open before allowing a probe.
    pub recovery_timeout: Duration,
    /// Successful probes needed in half-open to close the circuit.
    pub half_open_successes_needed: u32,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            recovery_timeout: Duration::from_secs(30),
            half_open_successes_needed: 2,
        }
    }
}

/// Circuit breaker states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Normal operation; tracking consecutive failures.
    Closed,
    /// Rejecting all calls; waiting for recovery timeout to elapse.
    Open,
    /// Allowing probe calls to test whether the backend recovered.
    HalfOpen,
}

/// Internal mutable state.
struct BreakerState {
    state: CircuitState,
    consecutive_failures: u32,
    opened_at: Option<Instant>,
    half_open_successes: u32,
}

impl BreakerState {
    fn new() -> Self {
        Self {
            state: CircuitState::Closed,
            consecutive_failures: 0,
            opened_at: None,
            half_open_successes: 0,
        }
    }
}

/// Wraps an `LlmProvider` with circuit breaker protection.
///
/// Tracks consecutive transient failures. After `failure_threshold` failures
/// the circuit opens and all requests are rejected for `recovery_timeout`.
/// After that timeout a probe call is allowed through (half-open); if it
/// succeeds the circuit closes, otherwise it reopens.
pub struct CircuitBreakerProvider {
    inner: Arc<dyn LlmProvider>,
    state: Arc<Mutex<BreakerState>>,
    config: CircuitBreakerConfig,
}

impl CircuitBreakerProvider {
    pub fn new(inner: Arc<dyn LlmProvider>, config: CircuitBreakerConfig) -> Self {
        Self {
            inner,
            state: Arc::new(Mutex::new(BreakerState::new())),
            config,
        }
    }

    /// Current circuit state (for observability / health checks).
    pub async fn circuit_state(&self) -> CircuitState {
        self.state.lock().await.state
    }

    /// Number of consecutive failures recorded so far.
    pub async fn consecutive_failures(&self) -> u32 {
        self.state.lock().await.consecutive_failures
    }

    /// Pre-flight: is a call allowed right now?
    async fn check_allowed(&self) -> Result<(), LlmError> {
        let mut state = self.state.lock().await;
        match state.state {
            CircuitState::Closed | CircuitState::HalfOpen => Ok(()),
            CircuitState::Open => {
                if let Some(opened_at) = state.opened_at {
                    if opened_at.elapsed() >= self.config.recovery_timeout {
                        state.state = CircuitState::HalfOpen;
                        state.half_open_successes = 0;
                        tracing::info!(
                            provider = self.inner.model_name(),
                            "Circuit breaker: Open -> HalfOpen, allowing probe"
                        );
                        Ok(())
                    } else {
                        let remaining = self
                            .config
                            .recovery_timeout
                            .checked_sub(opened_at.elapsed())
                            .unwrap_or(Duration::ZERO);
                        Err(LlmError::RequestFailed {
                            provider: self.inner.model_name().to_string(),
                            reason: format!(
                                "Circuit breaker open ({} consecutive failures, \
                                 recovery in {:.0}s)",
                                state.consecutive_failures,
                                remaining.as_secs_f64()
                            ),
                        })
                    }
                } else {
                    // opened_at should always be Some when Open; recover gracefully
                    state.state = CircuitState::Closed;
                    Ok(())
                }
            }
        }
    }

    /// Record a successful call.
    async fn record_success(&self) {
        let mut state = self.state.lock().await;
        match state.state {
            CircuitState::Closed => {
                state.consecutive_failures = 0;
            }
            CircuitState::HalfOpen => {
                state.half_open_successes += 1;
                if state.half_open_successes >= self.config.half_open_successes_needed {
                    state.state = CircuitState::Closed;
                    state.consecutive_failures = 0;
                    state.opened_at = None;
                    tracing::info!(
                        provider = self.inner.model_name(),
                        "Circuit breaker: HalfOpen -> Closed (recovered)"
                    );
                }
            }
            CircuitState::Open => {
                // Shouldn't get here (check_allowed blocks Open), but recover
                state.state = CircuitState::Closed;
                state.consecutive_failures = 0;
                state.opened_at = None;
            }
        }
    }

    /// Record a failed call; only transient errors count toward the threshold.
    async fn record_failure(&self, err: &LlmError) {
        if !is_transient(err) {
            return;
        }

        let mut state = self.state.lock().await;
        match state.state {
            CircuitState::Closed => {
                state.consecutive_failures += 1;
                if state.consecutive_failures >= self.config.failure_threshold {
                    state.state = CircuitState::Open;
                    state.opened_at = Some(Instant::now());
                    tracing::warn!(
                        provider = self.inner.model_name(),
                        failures = state.consecutive_failures,
                        "Circuit breaker: Closed -> Open"
                    );
                }
            }
            CircuitState::HalfOpen => {
                state.state = CircuitState::Open;
                state.opened_at = Some(Instant::now());
                state.half_open_successes = 0;
                tracing::warn!(
                    provider = self.inner.model_name(),
                    "Circuit breaker: HalfOpen -> Open (probe failed)"
                );
            }
            CircuitState::Open => {}
        }
    }
}

async fn record_success_state(
    state: &Mutex<BreakerState>,
    config: &CircuitBreakerConfig,
    provider: &str,
) {
    let mut state = state.lock().await;
    match state.state {
        CircuitState::Closed => {
            state.consecutive_failures = 0;
        }
        CircuitState::HalfOpen => {
            state.half_open_successes += 1;
            if state.half_open_successes >= config.half_open_successes_needed {
                state.state = CircuitState::Closed;
                state.consecutive_failures = 0;
                state.opened_at = None;
                tracing::info!(provider, "Circuit breaker: HalfOpen -> Closed (recovered)");
            }
        }
        CircuitState::Open => {
            state.state = CircuitState::Closed;
            state.consecutive_failures = 0;
            state.opened_at = None;
        }
    }
}

async fn record_failure_state(
    state: &Mutex<BreakerState>,
    config: &CircuitBreakerConfig,
    provider: &str,
    err: &LlmError,
) {
    if !is_transient(err) {
        return;
    }

    let mut state = state.lock().await;
    match state.state {
        CircuitState::Closed => {
            state.consecutive_failures += 1;
            if state.consecutive_failures >= config.failure_threshold {
                state.state = CircuitState::Open;
                state.opened_at = Some(Instant::now());
                tracing::warn!(
                    provider,
                    failures = state.consecutive_failures,
                    "Circuit breaker: Closed -> Open"
                );
            }
        }
        CircuitState::HalfOpen => {
            state.state = CircuitState::Open;
            state.opened_at = Some(Instant::now());
            state.half_open_successes = 0;
            tracing::warn!(provider, "Circuit breaker: HalfOpen -> Open (probe failed)");
        }
        CircuitState::Open => {}
    }
}

fn wrap_breaker_stream(
    mut stream: thinclaw_llm_core::StreamChunkStream,
    state: Arc<Mutex<BreakerState>>,
    config: CircuitBreakerConfig,
    provider: String,
) -> thinclaw_llm_core::StreamChunkStream {
    Box::pin(async_stream::stream! {
        use futures::StreamExt;
        let mut finished = false;
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(thinclaw_llm_core::StreamChunk::Done {
                    provider_model,
                    cost_usd,
                    input_tokens,
                    output_tokens,
                    finish_reason,
                    token_capture,
                }) => {
                    finished = true;
                    record_success_state(&state, &config, &provider).await;
                    yield Ok(thinclaw_llm_core::StreamChunk::Done {
                        provider_model,
                        cost_usd,
                        input_tokens,
                        output_tokens,
                        finish_reason,
                        token_capture,
                    });
                }
                Err(error) => {
                    record_failure_state(&state, &config, &provider, &error).await;
                    yield Err(error);
                    break;
                }
                other => yield other,
            }
        }

        if !finished {
            let error = LlmError::RequestFailed {
                provider: provider.clone(),
                reason: "stream ended before Done chunk".to_string(),
            };
            record_failure_state(&state, &config, &provider, &error).await;
        }
    })
}

/// Returns `true` for errors that indicate the provider is degraded
/// (server errors, rate limits, network failures, auth infrastructure down).
///
/// This answers: "should this error count toward tripping the circuit breaker?"
///
/// Includes `SessionExpired` because repeated session failures signal backend
/// auth infrastructure trouble.
///
/// Excludes client errors that are the caller's problem, not backend trouble:
/// `AuthFailed`, `ContextLengthExceeded`, `ModelNotAvailable`, `Json`.
///
/// See also `retry::is_retryable()` which answers a different question:
/// "could retrying this exact request succeed?"
fn is_transient(err: &LlmError) -> bool {
    matches!(
        err,
        LlmError::RequestFailed { .. }
            | LlmError::RateLimited { .. }
            | LlmError::InvalidResponse { .. }
            | LlmError::SessionExpired { .. }
            | LlmError::SessionRenewalFailed { .. }
            | LlmError::Http(_)
            | LlmError::Io(_)
    )
}

#[async_trait]
impl LlmProvider for CircuitBreakerProvider {
    fn model_name(&self) -> &str {
        self.inner.model_name()
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        self.inner.cost_per_token()
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.check_allowed().await?;
        match self.inner.complete(request).await {
            Ok(resp) => {
                self.record_success().await;
                Ok(resp)
            }
            Err(err) => {
                self.record_failure(&err).await;
                Err(err)
            }
        }
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        self.check_allowed().await?;
        match self.inner.complete_with_tools(request).await {
            Ok(resp) => {
                self.record_success().await;
                Ok(resp)
            }
            Err(err) => {
                self.record_failure(&err).await;
                Err(err)
            }
        }
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

    fn calculate_cost(&self, input_tokens: u32, output_tokens: u32) -> Decimal {
        self.inner.calculate_cost(input_tokens, output_tokens)
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<thinclaw_llm_core::StreamChunkStream, LlmError> {
        self.check_allowed().await?;
        // Start failures count immediately; stream success/failure is recorded
        // by the wrapper once the caller consumes Done or a midstream error.
        match self.inner.complete_stream(request).await {
            Ok(stream) => Ok(wrap_breaker_stream(
                stream,
                Arc::clone(&self.state),
                self.config.clone(),
                self.inner.model_name().to_string(),
            )),
            Err(err) => {
                self.record_failure(&err).await;
                Err(err)
            }
        }
    }

    async fn complete_stream_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<thinclaw_llm_core::StreamChunkStream, LlmError> {
        self.check_allowed().await?;
        match self.inner.complete_stream_with_tools(request).await {
            Ok(stream) => Ok(wrap_breaker_stream(
                stream,
                Arc::clone(&self.state),
                self.config.clone(),
                self.inner.model_name().to_string(),
            )),
            Err(err) => {
                self.record_failure(&err).await;
                Err(err)
            }
        }
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
