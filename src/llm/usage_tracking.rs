use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use async_trait::async_trait;
use futures::StreamExt;
use rust_decimal::Decimal;

use crate::agent::cost_guard::CostGuard;
use crate::db::Database;
use crate::error::LlmError;
use crate::experiments::ExperimentModelUsageRecord;
use crate::llm::cost_tracker::{CostEntry, CostTracker};
use crate::llm::costs;
use crate::llm::provider::{
    CompletionRequest, CompletionResponse, LlmProvider, ModelMetadata, StreamChunk,
    StreamChunkStream, StreamSupport, TokenCaptureSupport, ToolCompletionRequest,
    ToolCompletionResponse,
};

pub const USAGE_TRACKING_OWNER_KEY: &str = "thinclaw.usage_tracking.owner";
pub const USAGE_TRACKING_OWNER_REASONING: &str = "reasoning";
pub const USAGE_TRACKING_AGENT_KEY: &str = "thinclaw.usage_tracking.agent";
pub const USAGE_TRACKING_TELEMETRY_KEY: &str = "thinclaw.usage_tracking.telemetry_key";
pub const USAGE_TRACKING_ENDPOINT_TYPE_KEY: &str = "thinclaw.usage_tracking.endpoint_type";
pub const USAGE_TRACKING_WORKLOAD_TAG_KEY: &str = "thinclaw.usage_tracking.workload_tag";
pub const USAGE_TRACKING_PROMPT_ASSET_IDS_KEY: &str = "thinclaw.usage_tracking.prompt_asset_ids";
pub const USAGE_TRACKING_RETRIEVAL_ASSET_IDS_KEY: &str =
    "thinclaw.usage_tracking.retrieval_asset_ids";
pub const USAGE_TRACKING_TOOL_POLICY_IDS_KEY: &str = "thinclaw.usage_tracking.tool_policy_ids";
pub const USAGE_TRACKING_EVALUATOR_IDS_KEY: &str = "thinclaw.usage_tracking.evaluator_ids";
pub const USAGE_TRACKING_PARSER_IDS_KEY: &str = "thinclaw.usage_tracking.parser_ids";
pub const USAGE_TRACKING_EXPERIMENT_CAMPAIGN_ID_KEY: &str =
    "thinclaw.usage_tracking.experiment_campaign_id";
pub const USAGE_TRACKING_EXPERIMENT_TRIAL_ID_KEY: &str =
    "thinclaw.usage_tracking.experiment_trial_id";
pub const USAGE_TRACKING_EXPERIMENT_ROLE_KEY: &str = "thinclaw.usage_tracking.experiment_role";
pub const USAGE_TRACKING_EXPERIMENT_TARGET_IDS_KEY: &str =
    "thinclaw.usage_tracking.experiment_target_ids";

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
    let cost = cost_usd.unwrap_or_else(|| fallback_cost_usd(&model, input_tokens, output_tokens));
    use rust_decimal::prelude::FromPrimitive;
    let cost_decimal = rust_decimal::Decimal::from_f64(cost).unwrap_or_default();
    let _ = guard
        .record_llm_call_with_cost(&model, input_tokens, output_tokens, cost_decimal)
        .await;
}

async fn record_usage(
    tracker: &Arc<tokio::sync::Mutex<CostTracker>>,
    db: Option<&Arc<dyn Database>>,
    guard: Option<&Arc<CostGuard>>,
    metadata: &HashMap<String, String>,
    fallback_model: &str,
    provider_model: Option<&str>,
    cost_usd: Option<f64>,
    input_tokens: u32,
    output_tokens: u32,
    latency_ms: Option<u64>,
    success: bool,
) {
    let model = resolve_model(fallback_model, provider_model);
    let cost_usd =
        cost_usd.unwrap_or_else(|| fallback_cost_usd(&model, input_tokens, output_tokens));
    let (provider, model_name, route_key, logical_role) =
        usage_identity(metadata, &model, provider_model);
    let request_id = Some(uuid::Uuid::new_v4().to_string());
    let entry = CostEntry {
        timestamp: chrono::Utc::now().to_rfc3339(),
        agent_id: metadata_agent_id(metadata),
        provider: provider.clone(),
        model: model_name.clone(),
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
    if let Some(db) = db {
        let record = ExperimentModelUsageRecord {
            id: uuid::Uuid::new_v4(),
            provider,
            model: model_name,
            route_key,
            logical_role,
            endpoint_type: metadata.get(USAGE_TRACKING_ENDPOINT_TYPE_KEY).cloned(),
            workload_tag: metadata.get(USAGE_TRACKING_WORKLOAD_TAG_KEY).cloned(),
            latency_ms,
            cost_usd: Some(cost_usd),
            success,
            prompt_asset_ids: metadata_csv_list(metadata, USAGE_TRACKING_PROMPT_ASSET_IDS_KEY),
            retrieval_asset_ids: metadata_csv_list(
                metadata,
                USAGE_TRACKING_RETRIEVAL_ASSET_IDS_KEY,
            ),
            tool_policy_ids: metadata_csv_list(metadata, USAGE_TRACKING_TOOL_POLICY_IDS_KEY),
            evaluator_ids: metadata_csv_list(metadata, USAGE_TRACKING_EVALUATOR_IDS_KEY),
            parser_ids: metadata_csv_list(metadata, USAGE_TRACKING_PARSER_IDS_KEY),
            metadata: usage_record_metadata(metadata),
            created_at: chrono::Utc::now(),
        };
        if let Err(error) = db.create_experiment_model_usage(&record).await {
            tracing::debug!(%error, "Failed to persist experiment model usage record");
        }
    }
}

#[derive(Clone)]
enum StreamUsageMode {
    Full {
        tracker: Arc<tokio::sync::Mutex<CostTracker>>,
        db: Option<Arc<dyn Database>>,
        guard: Option<Arc<CostGuard>>,
        metadata: HashMap<String, String>,
        fallback_model: String,
    },
    GuardOnly {
        guard: Arc<CostGuard>,
        fallback_model: String,
    },
}

impl StreamUsageMode {
    async fn record(
        &self,
        provider_model: Option<String>,
        cost_usd: Option<f64>,
        input_tokens: u32,
        output_tokens: u32,
        latency_ms: Option<u64>,
        success: bool,
    ) {
        match self {
            Self::Full {
                tracker,
                db,
                guard,
                metadata,
                fallback_model,
            } => {
                record_usage(
                    tracker,
                    db.as_ref(),
                    guard.as_ref(),
                    metadata,
                    fallback_model,
                    provider_model.as_deref(),
                    cost_usd,
                    input_tokens,
                    output_tokens,
                    latency_ms,
                    success,
                )
                .await;
            }
            Self::GuardOnly {
                guard,
                fallback_model,
            } => {
                record_guard_only(
                    guard,
                    fallback_model,
                    provider_model.as_deref(),
                    cost_usd,
                    input_tokens,
                    output_tokens,
                )
                .await;
            }
        }
    }
}

struct StreamUsageRecorder {
    mode: StreamUsageMode,
    started: Instant,
    recorded: AtomicBool,
}

impl StreamUsageRecorder {
    fn new(mode: StreamUsageMode, started: Instant) -> Self {
        Self {
            mode,
            started,
            recorded: AtomicBool::new(false),
        }
    }

    async fn record(
        &self,
        provider_model: Option<String>,
        cost_usd: Option<f64>,
        input_tokens: u32,
        output_tokens: u32,
        success: bool,
    ) {
        if self.recorded.swap(true, Ordering::AcqRel) {
            return;
        }
        self.mode
            .record(
                provider_model,
                cost_usd,
                input_tokens,
                output_tokens,
                Some(self.started.elapsed().as_millis() as u64),
                success,
            )
            .await;
    }
}

impl Drop for StreamUsageRecorder {
    fn drop(&mut self) {
        if self.recorded.swap(true, Ordering::AcqRel) {
            return;
        }
        let mode = self.mode.clone();
        let latency_ms = Some(self.started.elapsed().as_millis() as u64);
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        handle.spawn(async move {
            mode.record(None, None, 0, 0, latency_ms, false).await;
        });
    }
}

fn usage_record_metadata(metadata: &HashMap<String, String>) -> serde_json::Value {
    let mut payload = serde_json::Map::new();
    if let Some(agent_id) = metadata_agent_id(metadata) {
        payload.insert("agent_id".to_string(), serde_json::json!(agent_id));
    }
    if let Some(owner) = metadata
        .get(USAGE_TRACKING_OWNER_KEY)
        .filter(|value| !value.is_empty())
    {
        payload.insert("owner".to_string(), serde_json::json!(owner));
    }
    for (input_key, output_key) in [
        (
            USAGE_TRACKING_EXPERIMENT_CAMPAIGN_ID_KEY,
            "experiment_campaign_id",
        ),
        (
            USAGE_TRACKING_EXPERIMENT_TRIAL_ID_KEY,
            "experiment_trial_id",
        ),
        (USAGE_TRACKING_EXPERIMENT_ROLE_KEY, "experiment_role"),
        (
            USAGE_TRACKING_EXPERIMENT_TARGET_IDS_KEY,
            "experiment_target_ids",
        ),
    ] {
        if let Some(value) = metadata.get(input_key).filter(|value| !value.is_empty()) {
            payload.insert(output_key.to_string(), serde_json::json!(value));
        }
    }
    serde_json::Value::Object(payload)
}

fn metadata_csv_list(metadata: &HashMap<String, String>, key: &str) -> Vec<String> {
    metadata
        .get(key)
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn usage_identity(
    metadata: &HashMap<String, String>,
    resolved_model: &str,
    provider_model: Option<&str>,
) -> (String, String, Option<String>, Option<String>) {
    if let Some(key) = metadata.get(USAGE_TRACKING_TELEMETRY_KEY) {
        let mut parts = key.splitn(3, '|');
        let logical_role = parts
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let provider = parts
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| provider_from_model(resolved_model));
        let model = parts
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| resolve_model(resolved_model, provider_model));
        return (provider, model, Some(key.clone()), logical_role);
    }
    (
        provider_from_model(resolved_model),
        resolve_model(resolved_model, provider_model),
        None,
        None,
    )
}

fn provider_from_model(model: &str) -> String {
    model.split('/').next().unwrap_or("unknown").to_string()
}

pub struct UsageTrackingProvider {
    inner: Arc<dyn LlmProvider>,
    tracker: Arc<tokio::sync::Mutex<CostTracker>>,
    db: Option<Arc<dyn Database>>,
    guard: Option<Arc<CostGuard>>,
}

impl UsageTrackingProvider {
    pub fn new(
        inner: Arc<dyn LlmProvider>,
        tracker: Arc<tokio::sync::Mutex<CostTracker>>,
        db: Option<Arc<dyn Database>>,
        guard: Option<Arc<CostGuard>>,
    ) -> Self {
        Self {
            inner,
            tracker,
            db,
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
        latency_ms: Option<u64>,
        success: bool,
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
            self.db.as_ref(),
            self.guard.as_ref(),
            metadata,
            &self.inner.active_model_name(),
            provider_model,
            cost_usd,
            input_tokens,
            output_tokens,
            latency_ms,
            success,
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
        let started = std::time::Instant::now();
        let response = self.inner.complete(request).await;
        match response {
            Ok(response) => {
                self.track_completion(
                    &metadata,
                    response.provider_model.as_deref(),
                    response.cost_usd,
                    response.input_tokens,
                    response.output_tokens,
                    Some(started.elapsed().as_millis() as u64),
                    true,
                )
                .await;
                Ok(response)
            }
            Err(error) => {
                self.track_completion(
                    &metadata,
                    None,
                    None,
                    0,
                    0,
                    Some(started.elapsed().as_millis() as u64),
                    false,
                )
                .await;
                Err(error)
            }
        }
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let metadata = request.metadata.clone();
        let started = std::time::Instant::now();
        let response = self.inner.complete_with_tools(request).await;
        match response {
            Ok(response) => {
                self.track_completion(
                    &metadata,
                    response.provider_model.as_deref(),
                    response.cost_usd,
                    response.input_tokens,
                    response.output_tokens,
                    Some(started.elapsed().as_millis() as u64),
                    true,
                )
                .await;
                Ok(response)
            }
            Err(error) => {
                self.track_completion(
                    &metadata,
                    None,
                    None,
                    0,
                    0,
                    Some(started.elapsed().as_millis() as u64),
                    false,
                )
                .await;
                Err(error)
            }
        }
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<StreamChunkStream, LlmError> {
        let metadata = request.metadata.clone();
        let started = Instant::now();
        let mut stream = match self.inner.complete_stream(request).await {
            Ok(stream) => stream,
            Err(error) => {
                self.track_completion(
                    &metadata,
                    None,
                    None,
                    0,
                    0,
                    Some(started.elapsed().as_millis() as u64),
                    false,
                )
                .await;
                return Err(error);
            }
        };
        if metadata_is_reasoning_owned(&metadata) {
            // Still need CostGuard recording for budget enforcement, even
            // though Reasoning handles CostTracker itself.
            if let Some(ref guard) = self.guard {
                let recorder = StreamUsageRecorder::new(
                    StreamUsageMode::GuardOnly {
                        guard: Arc::clone(guard),
                        fallback_model: self.inner.active_model_name(),
                    },
                    started,
                );
                let wrapped = async_stream::stream! {
                    let recorder = recorder;
                    while let Some(chunk) = stream.next().await {
                        match chunk {
                            Ok(StreamChunk::Done {
                                provider_model,
                                cost_usd,
                                input_tokens,
                                output_tokens,
                                finish_reason,
                            }) => {
                                recorder.record(
                                    provider_model.clone(),
                                    cost_usd,
                                    input_tokens,
                                    output_tokens,
                                    true,
                                ).await;
                                yield Ok(StreamChunk::Done {
                                    provider_model,
                                    cost_usd,
                                    input_tokens,
                                    output_tokens,
                                    finish_reason,
                                });
                            }
                            Err(error) => {
                                recorder.record(None, None, 0, 0, false).await;
                                yield Err(error);
                                break;
                            }
                            other => yield other,
                        }
                    }
                };
                return Ok(Box::pin(wrapped));
            }
            return Ok(stream);
        }
        let recorder = StreamUsageRecorder::new(
            StreamUsageMode::Full {
                tracker: Arc::clone(&self.tracker),
                db: self.db.as_ref().map(Arc::clone),
                guard: self.guard.as_ref().map(Arc::clone),
                metadata,
                fallback_model: self.inner.active_model_name(),
            },
            started,
        );
        let wrapped = async_stream::stream! {
            let recorder = recorder;
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(StreamChunk::Done {
                        provider_model,
                        cost_usd,
                        input_tokens,
                        output_tokens,
                        finish_reason,
                    }) => {
                        recorder.record(
                            provider_model.clone(),
                            cost_usd,
                            input_tokens,
                            output_tokens,
                            true,
                        )
                        .await;
                        yield Ok(StreamChunk::Done {
                            provider_model,
                            cost_usd,
                            input_tokens,
                            output_tokens,
                            finish_reason,
                        });
                    }
                    Err(error) => {
                        recorder.record(None, None, 0, 0, false).await;
                        yield Err(error);
                        break;
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
        let started = Instant::now();
        let mut stream = match self.inner.complete_stream_with_tools(request).await {
            Ok(stream) => stream,
            Err(error) => {
                self.track_completion(
                    &metadata,
                    None,
                    None,
                    0,
                    0,
                    Some(started.elapsed().as_millis() as u64),
                    false,
                )
                .await;
                return Err(error);
            }
        };
        if metadata_is_reasoning_owned(&metadata) {
            if let Some(ref guard) = self.guard {
                let recorder = StreamUsageRecorder::new(
                    StreamUsageMode::GuardOnly {
                        guard: Arc::clone(guard),
                        fallback_model: self.inner.active_model_name(),
                    },
                    started,
                );
                let wrapped = async_stream::stream! {
                    let recorder = recorder;
                    while let Some(chunk) = stream.next().await {
                        match chunk {
                            Ok(StreamChunk::Done {
                                provider_model,
                                cost_usd,
                                input_tokens,
                                output_tokens,
                                finish_reason,
                            }) => {
                                recorder.record(
                                    provider_model.clone(),
                                    cost_usd,
                                    input_tokens,
                                    output_tokens,
                                    true,
                                ).await;
                                yield Ok(StreamChunk::Done {
                                    provider_model,
                                    cost_usd,
                                    input_tokens,
                                    output_tokens,
                                    finish_reason,
                                });
                            }
                            Err(error) => {
                                recorder.record(None, None, 0, 0, false).await;
                                yield Err(error);
                                break;
                            }
                            other => yield other,
                        }
                    }
                };
                return Ok(Box::pin(wrapped));
            }
            return Ok(stream);
        }
        let recorder = StreamUsageRecorder::new(
            StreamUsageMode::Full {
                tracker: Arc::clone(&self.tracker),
                db: self.db.as_ref().map(Arc::clone),
                guard: self.guard.as_ref().map(Arc::clone),
                metadata,
                fallback_model: self.inner.active_model_name(),
            },
            started,
        );
        let wrapped = async_stream::stream! {
            let recorder = recorder;
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(StreamChunk::Done {
                        provider_model,
                        cost_usd,
                        input_tokens,
                        output_tokens,
                        finish_reason,
                    }) => {
                        recorder.record(
                            provider_model.clone(),
                            cost_usd,
                            input_tokens,
                            output_tokens,
                            true,
                        )
                        .await;
                        yield Ok(StreamChunk::Done {
                            provider_model,
                            cost_usd,
                            input_tokens,
                            output_tokens,
                            finish_reason,
                        });
                    }
                    Err(error) => {
                        recorder.record(None, None, 0, 0, false).await;
                        yield Err(error);
                        break;
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
            None,
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
            None,
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
