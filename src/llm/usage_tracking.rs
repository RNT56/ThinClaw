use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use rust_decimal::Decimal;

use crate::agent::cost_guard::CostGuard;
use crate::error::LlmError;
use crate::llm::cost_tracker::{CostEntry, CostTracker};
use crate::llm::costs;
use crate::llm::provider::{
    CompletionRequest, CompletionResponse, LlmProvider, ModelMetadata, StreamChunk,
    StreamChunkStream, ToolCompletionRequest, ToolCompletionResponse,
};

pub const USAGE_TRACKING_OWNER_KEY: &str = "thinclaw.usage_tracking.owner";
pub const USAGE_TRACKING_OWNER_REASONING: &str = "reasoning";
pub const USAGE_TRACKING_AGENT_KEY: &str = "thinclaw.usage_tracking.agent";

pub fn mark_reasoning_request(metadata: &mut HashMap<String, String>, agent_id: Option<&str>) {
    metadata.insert(
        USAGE_TRACKING_OWNER_KEY.to_string(),
        USAGE_TRACKING_OWNER_REASONING.to_string(),
    );
    if let Some(agent_id) = agent_id.filter(|value| !value.is_empty()) {
        metadata.insert(USAGE_TRACKING_AGENT_KEY.to_string(), agent_id.to_string());
    }
}

fn metadata_is_reasoning_owned(metadata: &HashMap<String, String>) -> bool {
    metadata.get(USAGE_TRACKING_OWNER_KEY).map(String::as_str)
        == Some(USAGE_TRACKING_OWNER_REASONING)
}

fn metadata_agent_id(metadata: &HashMap<String, String>) -> Option<String> {
    metadata
        .get(USAGE_TRACKING_AGENT_KEY)
        .filter(|value| !value.is_empty())
        .cloned()
}

fn fallback_cost_usd(model: &str, input_tokens: u32, output_tokens: u32) -> f64 {
    let (input_rate, output_rate) = costs::model_cost(model).unwrap_or_else(costs::default_cost);
    let total =
        input_rate * Decimal::from(input_tokens) + output_rate * Decimal::from(output_tokens);
    use rust_decimal::prelude::ToPrimitive;
    total.to_f64().unwrap_or(0.0)
}

/// Resolve model name from provider_model or fallback.
fn resolve_model(fallback_model: &str, provider_model: Option<&str>) -> String {
    provider_model
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback_model)
        .to_string()
}

/// Record to CostGuard only (no CostTracker). Used for reasoning-owned requests
/// where Reasoning handles its own CostTracker recording but budget enforcement
/// still needs accurate data.
async fn record_guard_only(
    guard: &Arc<CostGuard>,
    fallback_model: &str,
    provider_model: Option<&str>,
    cost_usd: Option<f64>,
    input_tokens: u32,
    output_tokens: u32,
) {
    let model = resolve_model(fallback_model, provider_model);
    let cost =
        cost_usd.unwrap_or_else(|| fallback_cost_usd(&model, input_tokens, output_tokens));
    use rust_decimal::prelude::FromPrimitive;
    let cost_decimal = rust_decimal::Decimal::from_f64(cost).unwrap_or_default();
    let _ = guard
        .record_llm_call_with_cost(&model, input_tokens, output_tokens, cost_decimal)
        .await;
}

async fn record_usage(
    tracker: &Arc<tokio::sync::Mutex<CostTracker>>,
    guard: Option<&Arc<CostGuard>>,
    metadata: &HashMap<String, String>,
    fallback_model: &str,
    provider_model: Option<&str>,
    cost_usd: Option<f64>,
    input_tokens: u32,
    output_tokens: u32,
) {
    let model = resolve_model(fallback_model, provider_model);
    let cost_usd =
        cost_usd.unwrap_or_else(|| fallback_cost_usd(&model, input_tokens, output_tokens));
    let provider = if let Some(idx) = model.find('/') {
        model[..idx].to_string()
    } else {
        "unknown".to_string()
    };
    let request_id = Some(uuid::Uuid::new_v4().to_string());
    let entry = CostEntry {
        timestamp: chrono::Utc::now().to_rfc3339(),
        agent_id: metadata_agent_id(metadata),
        provider,
        model: model.clone(),
        input_tokens,
        output_tokens,
        cost_usd,
        request_id,
    };
    tracker.lock().await.record(entry);
    if let Some(guard) = guard {
        use rust_decimal::prelude::FromPrimitive;
        let cost_decimal = rust_decimal::Decimal::from_f64(cost_usd).unwrap_or_default();
        let _ = guard
            .record_llm_call_with_cost(&model, input_tokens, output_tokens, cost_decimal)
            .await;
    }
}

pub struct UsageTrackingProvider {
    inner: Arc<dyn LlmProvider>,
    tracker: Arc<tokio::sync::Mutex<CostTracker>>,
    guard: Option<Arc<CostGuard>>,
}

impl UsageTrackingProvider {
    pub fn new(
        inner: Arc<dyn LlmProvider>,
        tracker: Arc<tokio::sync::Mutex<CostTracker>>,
        guard: Option<Arc<CostGuard>>,
    ) -> Self {
        Self {
            inner,
            tracker,
            guard,
        }
    }

    async fn track_completion(
        &self,
        metadata: &HashMap<String, String>,
        provider_model: Option<&str>,
        cost_usd: Option<f64>,
        input_tokens: u32,
        output_tokens: u32,
    ) {
        if metadata_is_reasoning_owned(metadata) {
            // Reasoning records to CostTracker itself, but we must still update
            // CostGuard for accurate daily budget enforcement.
            if let Some(ref guard) = self.guard {
                record_guard_only(
                    guard,
                    &self.inner.active_model_name(),
                    provider_model,
                    cost_usd,
                    input_tokens,
                    output_tokens,
                )
                .await;
            }
            return;
        }
        record_usage(
            &self.tracker,
            self.guard.as_ref(),
            metadata,
            &self.inner.active_model_name(),
            provider_model,
            cost_usd,
            input_tokens,
            output_tokens,
        )
        .await;
    }
}

#[async_trait]
impl LlmProvider for UsageTrackingProvider {
    fn model_name(&self) -> &str {
        self.inner.model_name()
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        self.inner.cost_per_token()
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let metadata = request.metadata.clone();
        let response = self.inner.complete(request).await?;
        self.track_completion(
            &metadata,
            response.provider_model.as_deref(),
            response.cost_usd,
            response.input_tokens,
            response.output_tokens,
        )
        .await;
        Ok(response)
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let metadata = request.metadata.clone();
        let response = self.inner.complete_with_tools(request).await?;
        self.track_completion(
            &metadata,
            response.provider_model.as_deref(),
            response.cost_usd,
            response.input_tokens,
            response.output_tokens,
        )
        .await;
        Ok(response)
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<StreamChunkStream, LlmError> {
        let metadata = request.metadata.clone();
        let mut stream = self.inner.complete_stream(request).await?;
        if metadata_is_reasoning_owned(&metadata) {
            // Still need CostGuard recording for budget enforcement, even
            // though Reasoning handles CostTracker itself.
            if let Some(ref guard) = self.guard {
                let guard = Arc::clone(guard);
                let fallback_model = self.inner.active_model_name();
                let wrapped = async_stream::stream! {
                    while let Some(chunk) = stream.next().await {
                        match chunk {
                            Ok(StreamChunk::Done {
                                provider_model,
                                cost_usd,
                                input_tokens,
                                output_tokens,
                                finish_reason,
                            }) => {
                                record_guard_only(
                                    &guard,
                                    &fallback_model,
                                    provider_model.as_deref(),
                                    cost_usd,
                                    input_tokens,
                                    output_tokens,
                                ).await;
                                yield Ok(StreamChunk::Done {
                                    provider_model,
                                    cost_usd,
                                    input_tokens,
                                    output_tokens,
                                    finish_reason,
                                });
                            }
                            other => yield other,
                        }
                    }
                };
                return Ok(Box::pin(wrapped));
            }
            return Ok(stream);
        }
        let tracker = Arc::clone(&self.tracker);
        let guard = self.guard.as_ref().map(Arc::clone);
        let fallback_model = self.inner.active_model_name();
        let wrapped = async_stream::stream! {
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(StreamChunk::Done {
                        provider_model,
                        cost_usd,
                        input_tokens,
                        output_tokens,
                        finish_reason,
                    }) => {
                        record_usage(
                            &tracker,
                            guard.as_ref(),
                            &metadata,
                            &fallback_model,
                            provider_model.as_deref(),
                            cost_usd,
                            input_tokens,
                            output_tokens,
                        ).await;
                        yield Ok(StreamChunk::Done {
                            provider_model,
                            cost_usd,
                            input_tokens,
                            output_tokens,
                            finish_reason,
                        });
                    }
                    other => yield other,
                }
            }
        };
        Ok(Box::pin(wrapped))
    }

    async fn complete_stream_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<StreamChunkStream, LlmError> {
        let metadata = request.metadata.clone();
        let mut stream = self.inner.complete_stream_with_tools(request).await?;
        if metadata_is_reasoning_owned(&metadata) {
            if let Some(ref guard) = self.guard {
                let guard = Arc::clone(guard);
                let fallback_model = self.inner.active_model_name();
                let wrapped = async_stream::stream! {
                    while let Some(chunk) = stream.next().await {
                        match chunk {
                            Ok(StreamChunk::Done {
                                provider_model,
                                cost_usd,
                                input_tokens,
                                output_tokens,
                                finish_reason,
                            }) => {
                                record_guard_only(
                                    &guard,
                                    &fallback_model,
                                    provider_model.as_deref(),
                                    cost_usd,
                                    input_tokens,
                                    output_tokens,
                                ).await;
                                yield Ok(StreamChunk::Done {
                                    provider_model,
                                    cost_usd,
                                    input_tokens,
                                    output_tokens,
                                    finish_reason,
                                });
                            }
                            other => yield other,
                        }
                    }
                };
                return Ok(Box::pin(wrapped));
            }
            return Ok(stream);
        }
        let tracker = Arc::clone(&self.tracker);
        let guard = self.guard.as_ref().map(Arc::clone);
        let fallback_model = self.inner.active_model_name();
        let wrapped = async_stream::stream! {
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(StreamChunk::Done {
                        provider_model,
                        cost_usd,
                        input_tokens,
                        output_tokens,
                        finish_reason,
                    }) => {
                        record_usage(
                            &tracker,
                            guard.as_ref(),
                            &metadata,
                            &fallback_model,
                            provider_model.as_deref(),
                            cost_usd,
                            input_tokens,
                            output_tokens,
                        ).await;
                        yield Ok(StreamChunk::Done {
                            provider_model,
                            cost_usd,
                            input_tokens,
                            output_tokens,
                            finish_reason,
                        });
                    }
                    other => yield other,
                }
            }
        };
        Ok(Box::pin(wrapped))
    }

    fn supports_streaming(&self) -> bool {
        self.inner.supports_streaming()
    }

    fn supports_streaming_for_model(&self, requested_model: Option<&str>) -> bool {
        self.inner.supports_streaming_for_model(requested_model)
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::StubLlm;
    use futures::StreamExt;

    #[tokio::test]
    async fn direct_completion_is_recorded() {
        let tracker = Arc::new(tokio::sync::Mutex::new(
            CostTracker::new(Default::default()),
        ));
        let guard = Arc::new(CostGuard::new(Default::default()));
        let provider: Arc<dyn LlmProvider> = Arc::new(UsageTrackingProvider::new(
            Arc::new(StubLlm::new("ok").with_model_name("openai/gpt-5.4-mini")),
            Arc::clone(&tracker),
            Some(Arc::clone(&guard)),
        ));

        let response = provider
            .complete(CompletionRequest::new(vec![]))
            .await
            .expect("completion should succeed");

        assert_eq!(response.content, "ok");
        let summary = tracker.lock().await.summary("2026-04-05", "2026-04");
        assert_eq!(summary.total_requests, 1);
        assert_eq!(summary.model_details.len(), 1);
        assert_eq!(summary.model_details[0].model, "openai/gpt-5.4-mini");
        assert_eq!(guard.actions_this_hour().await, 1);
    }

    #[tokio::test]
    async fn reasoning_owned_skips_tracker_but_updates_guard() {
        let tracker = Arc::new(tokio::sync::Mutex::new(
            CostTracker::new(Default::default()),
        ));
        let guard = Arc::new(CostGuard::new(Default::default()));
        let provider: Arc<dyn LlmProvider> = Arc::new(UsageTrackingProvider::new(
            Arc::new(StubLlm::new("ok").with_model_name("openai/gpt-5.4")),
            Arc::clone(&tracker),
            Some(Arc::clone(&guard)),
        ));

        let mut request = CompletionRequest::new(vec![]);
        mark_reasoning_request(&mut request.metadata, Some("gateway"));
        provider
            .complete(request)
            .await
            .expect("completion should succeed");

        // CostTracker is NOT updated (Reasoning handles that separately)
        let summary = tracker.lock().await.summary("2026-04-05", "2026-04");
        assert_eq!(summary.total_requests, 0);
        // CostGuard IS updated for accurate budget enforcement
        assert_eq!(guard.actions_this_hour().await, 1);
    }

    #[tokio::test]
    async fn direct_stream_is_recorded_on_done() {
        let tracker = Arc::new(tokio::sync::Mutex::new(
            CostTracker::new(Default::default()),
        ));
        let provider: Arc<dyn LlmProvider> = Arc::new(UsageTrackingProvider::new(
            Arc::new(StubLlm::new("streamed").with_model_name("openai/gpt-5.4-mini")),
            Arc::clone(&tracker),
            None,
        ));

        let mut stream = provider
            .complete_stream(CompletionRequest::new(vec![]))
            .await
            .expect("stream should succeed");

        while let Some(chunk) = stream.next().await {
            let _ = chunk.expect("stream chunk should be ok");
        }

        let summary = tracker.lock().await.summary("2026-04-05", "2026-04");
        assert_eq!(summary.total_requests, 1);
        assert_eq!(summary.model_details[0].model, "openai/gpt-5.4-mini");
    }
}
