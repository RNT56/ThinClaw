//! Mixture-of-Agents (MoA) tool.
//!
//! Fans a prompt out to multiple reference models in parallel, retries
//! transient failures, and then asks an aggregator model to synthesize the
//! strongest answer from the successful references.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use tokio::task::JoinSet;

use crate::context::JobContext;
use crate::llm::{ChatMessage, CompletionRequest, LlmProvider};
use crate::tools::builtin::llm_tools::wrap_model_spec_override;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput, require_str};

/// Maximum number of reference models the tool will fan out to in one call.
const MAX_REFERENCE_MODELS: usize = 6;

/// Maximum time to wait for any single model call.
const MODEL_TIMEOUT: Duration = Duration::from_secs(120);

/// Retry budget per model call.
const MAX_ATTEMPTS: usize = 3;

/// Base retry delay for transient model failures.
const RETRY_BASE_DELAY: Duration = Duration::from_millis(400);

#[derive(Clone)]
struct ReferenceTarget {
    label: String,
    provider: Arc<dyn LlmProvider>,
}

#[derive(Debug)]
struct ReferenceResponse {
    effective_model: String,
    content: String,
    cost: Option<Decimal>,
}

/// Mixture-of-Agents multi-model reasoning tool.
pub struct MoaTool {
    base_provider: Arc<dyn LlmProvider>,
    cheap_provider: Option<Arc<dyn LlmProvider>>,
    default_reference_models: Vec<String>,
    default_aggregator_model: Option<String>,
    default_min_successful: usize,
}

impl MoaTool {
    pub fn new(
        base_provider: Arc<dyn LlmProvider>,
        cheap_provider: Option<Arc<dyn LlmProvider>>,
        default_reference_models: Vec<String>,
        default_aggregator_model: Option<String>,
        default_min_successful: usize,
    ) -> Self {
        Self {
            base_provider,
            cheap_provider,
            default_reference_models,
            default_aggregator_model,
            default_min_successful: default_min_successful.max(1),
        }
    }

    /// Check if the tool has enough backing providers or configured model
    /// overrides to provide more than a single vanilla completion path.
    pub fn is_viable(&self) -> bool {
        !self.default_reference_models.is_empty() || self.cheap_provider.is_some()
    }

    fn parse_model_overrides(params: &serde_json::Value) -> Result<Option<Vec<String>>, ToolError> {
        let Some(models) = params.get("models") else {
            return Ok(None);
        };

        let arr = models.as_array().ok_or_else(|| {
            ToolError::InvalidParameters(
                "'models' must be an array of provider/model strings".into(),
            )
        })?;

        let parsed: Vec<String> = arr
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(str::to_string)
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| {
                ToolError::InvalidParameters(
                    "'models' entries must all be non-empty strings".into(),
                )
            })?;

        Ok(Some(parsed))
    }

    fn resolve_reference_targets(
        &self,
        requested_models: Option<Vec<String>>,
    ) -> Result<Vec<ReferenceTarget>, ToolError> {
        let configured = requested_models.unwrap_or_else(|| self.default_reference_models.clone());

        let mut targets = Vec::new();
        let mut seen = HashSet::new();

        if configured.is_empty() {
            targets.push(ReferenceTarget {
                label: self.base_provider.active_model_name(),
                provider: Arc::clone(&self.base_provider),
            });

            if let Some(ref cheap) = self.cheap_provider {
                let cheap_label = cheap.active_model_name();
                if seen.insert(cheap_label.clone()) {
                    targets.push(ReferenceTarget {
                        label: cheap_label,
                        provider: Arc::clone(cheap),
                    });
                }
            }
        } else {
            for spec in configured {
                if !seen.insert(spec.clone()) {
                    continue;
                }
                targets.push(ReferenceTarget {
                    label: spec.clone(),
                    provider: wrap_model_spec_override(Arc::clone(&self.base_provider), spec),
                });
            }
        }

        if targets.is_empty() {
            return Err(ToolError::ExecutionFailed(
                "Mixture-of-Agents is not configured. Add providers.cheap_model or providers.moa_reference_models first.".to_string(),
            ));
        }

        if targets.len() > MAX_REFERENCE_MODELS {
            return Err(ToolError::InvalidParameters(format!(
                "Too many reference models requested ({}). Max supported is {}.",
                targets.len(),
                MAX_REFERENCE_MODELS
            )));
        }

        Ok(targets)
    }

    async fn call_reference_model(
        target: ReferenceTarget,
        request: CompletionRequest,
    ) -> Result<ReferenceResponse, String> {
        let mut last_error = None;

        for attempt in 0..MAX_ATTEMPTS {
            let req = request.clone();
            match tokio::time::timeout(MODEL_TIMEOUT, target.provider.complete(req)).await {
                Ok(Ok(response)) => {
                    return Ok(ReferenceResponse {
                        effective_model: response
                            .provider_model
                            .clone()
                            .unwrap_or_else(|| target.provider.active_model_name()),
                        content: response.content,
                        cost: response.cost_usd.and_then(Decimal::from_f64),
                    });
                }
                Ok(Err(error)) => {
                    last_error = Some(format!("{} failed: {}", target.label, error));
                }
                Err(_) => {
                    last_error = Some(format!(
                        "{} timed out after {:?}",
                        target.label, MODEL_TIMEOUT
                    ));
                }
            }

            if attempt + 1 < MAX_ATTEMPTS {
                let delay = RETRY_BASE_DELAY.mul_f64(3f64.powi(attempt as i32));
                tokio::time::sleep(delay).await;
            }
        }

        Err(last_error.unwrap_or_else(|| format!("{} failed without an error", target.label)))
    }

    fn build_reference_request(prompt: &str, context: Option<&str>) -> CompletionRequest {
        let mut body = String::from(
            "You are one expert in a Mixture-of-Agents ensemble. Solve the task independently.\n\
             Focus on accuracy, concrete reasoning, and useful detail. Do not mention the ensemble.\n\n",
        );

        if let Some(context) = context
            && !context.trim().is_empty()
        {
            body.push_str("Context:\n");
            body.push_str(context.trim());
            body.push_str("\n\n");
        }

        body.push_str("Task:\n");
        body.push_str(prompt.trim());

        CompletionRequest::new(vec![ChatMessage::user(body)])
    }

    fn build_aggregator_request(
        prompt: &str,
        responses: &[ReferenceResponse],
        context: Option<&str>,
    ) -> CompletionRequest {
        let mut body = String::from(
            "You are synthesizing several independently-produced expert answers into one final response.\n\
             Combine the strongest points, discard weak or redundant claims, resolve contradictions carefully,\n\
             and produce the best possible answer for the original task.\n\n",
        );

        if let Some(context) = context
            && !context.trim().is_empty()
        {
            body.push_str("Original context:\n");
            body.push_str(context.trim());
            body.push_str("\n\n");
        }

        body.push_str("Original task:\n");
        body.push_str(prompt.trim());
        body.push_str("\n\nReference responses:\n");

        for (index, response) in responses.iter().enumerate() {
            body.push_str(&format!(
                "\n--- Reference {} ({}) ---\n{}\n",
                index + 1,
                response.effective_model,
                response.content.trim()
            ));
        }

        body.push_str("\nReturn only the synthesized final answer.\n");

        CompletionRequest::new(vec![ChatMessage::user(body)])
    }
}

#[async_trait]
impl Tool for MoaTool {
    fn name(&self) -> &str {
        "mixture_of_agents"
    }

    fn description(&self) -> &str {
        "For especially hard problems, consult multiple reference models in parallel and synthesize their answers into one stronger result. Use sparingly: this is slower and more expensive than a normal response."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "The difficult problem or question to analyze."
                },
                "context": {
                    "type": "string",
                    "description": "Optional background context shared with every reference model."
                },
                "models": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of explicit reference models in provider/model format."
                },
                "aggregator_model": {
                    "type": "string",
                    "description": "Optional provider/model override for the synthesis pass."
                },
                "min_successful": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Minimum successful reference responses required before aggregation."
                }
            },
            "required": ["prompt"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let prompt = require_str(&params, "prompt")?;
        let context = params.get("context").and_then(|value| value.as_str());
        let requested_models = Self::parse_model_overrides(&params)?;
        let reference_targets = self.resolve_reference_targets(requested_models)?;

        let min_successful = params
            .get("min_successful")
            .and_then(|value| value.as_u64())
            .map(|value| value as usize)
            .unwrap_or(self.default_min_successful)
            .clamp(1, reference_targets.len());

        let aggregator_model = params
            .get("aggregator_model")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| self.default_aggregator_model.clone());

        let reference_request = Self::build_reference_request(prompt, context);

        let mut join_set = JoinSet::new();
        for target in reference_targets.clone() {
            let request = reference_request.clone();
            join_set.spawn(async move { Self::call_reference_model(target, request).await });
        }

        let mut successes = Vec::new();
        let mut failures = Vec::new();
        let mut total_cost = Decimal::ZERO;

        while let Some(joined) = join_set.join_next().await {
            match joined {
                Ok(Ok(response)) => {
                    if let Some(cost) = response.cost {
                        total_cost += cost;
                    }
                    successes.push(response);
                }
                Ok(Err(error)) => {
                    tracing::warn!(error = %error, "MoA reference call failed");
                    failures.push(error);
                }
                Err(error) => {
                    let message = format!("Reference task panicked: {}", error);
                    tracing::warn!(error = %message, "MoA reference task panicked");
                    failures.push(message);
                }
            }
        }

        if successes.len() < min_successful {
            return Err(ToolError::ExecutionFailed(format!(
                "Mixture-of-Agents required {} successful reference responses but only {} succeeded. Errors: {}",
                min_successful,
                successes.len(),
                if failures.is_empty() {
                    "none".to_string()
                } else {
                    failures.join("; ")
                }
            )));
        }

        let aggregator_provider: Arc<dyn LlmProvider> = if let Some(ref model) = aggregator_model {
            wrap_model_spec_override(Arc::clone(&self.base_provider), model.clone())
        } else {
            Arc::clone(&self.base_provider)
        };

        let aggregator_request = Self::build_aggregator_request(prompt, &successes, context);
        let aggregated = aggregator_provider
            .complete(aggregator_request)
            .await
            .map_err(|error| ToolError::ExecutionFailed(format!("Aggregator failed: {}", error)))?;

        if let Some(cost) = aggregated.cost_usd.and_then(Decimal::from_f64) {
            total_cost += cost;
        }

        let output = serde_json::json!({
            "analysis": aggregated.content,
            "reference_count": successes.len(),
            "reference_models": successes.iter().map(|item| item.effective_model.clone()).collect::<Vec<_>>(),
            "requested_reference_models": reference_targets.iter().map(|item| item.label.clone()).collect::<Vec<_>>(),
            "aggregator_model": aggregated.provider_model.unwrap_or_else(|| aggregator_provider.active_model_name()),
            "failures": failures,
            "min_successful": min_successful,
        });

        let mut tool_output = ToolOutput::success(output, start.elapsed());
        if total_cost > Decimal::ZERO {
            tool_output = tool_output.with_cost(total_cost);
        }
        Ok(tool_output)
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn execution_timeout(&self) -> Duration {
        Duration::from_secs(300)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    use crate::error::LlmError;
    use crate::llm::{
        CompletionResponse, FinishReason, StreamChunkStream, ToolCompletionRequest,
        ToolCompletionResponse,
    };

    struct DummyProvider;

    #[async_trait]
    impl LlmProvider for DummyProvider {
        fn model_name(&self) -> &str {
            "dummy"
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            Ok(CompletionResponse {
                content: "ok".to_string(),
                provider_model: Some("dummy".to_string()),
                cost_usd: None,
                thinking_content: None,
                input_tokens: 1,
                output_tokens: 1,
                finish_reason: FinishReason::Stop,
            })
        }

        async fn complete_with_tools(
            &self,
            _request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            unimplemented!("tool completions are not used in these tests")
        }

        async fn complete_stream(
            &self,
            _request: CompletionRequest,
        ) -> Result<StreamChunkStream, LlmError> {
            unimplemented!("streaming is not used in these tests")
        }
    }

    #[test]
    fn moa_is_viable_with_explicit_reference_models() {
        let tool = MoaTool {
            base_provider: Arc::new(DummyProvider),
            cheap_provider: None,
            default_reference_models: vec!["openai/gpt-4o".to_string()],
            default_aggregator_model: None,
            default_min_successful: 1,
        };

        assert!(tool.is_viable());
    }

    #[test]
    fn parse_model_overrides_rejects_non_strings() {
        let params = serde_json::json!({
            "models": ["openai/gpt-4o", 42]
        });

        assert!(MoaTool::parse_model_overrides(&params).is_err());
    }

    #[test]
    fn schema_exposes_model_controls() {
        let tool = MoaTool {
            base_provider: Arc::new(DummyProvider),
            cheap_provider: None,
            default_reference_models: Vec::new(),
            default_aggregator_model: None,
            default_min_successful: 1,
        };

        let schema = tool.parameters_schema();
        let properties = schema["properties"].as_object().unwrap();
        assert!(properties.contains_key("models"));
        assert!(properties.contains_key("aggregator_model"));
        assert!(properties.contains_key("min_successful"));
    }
}
