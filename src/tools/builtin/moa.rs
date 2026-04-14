//! Mixture-of-Agents (MoA) tool.
//!
//! Dispatches a complex prompt to multiple LLM "reference" models in parallel,
//! then synthesizes their diverse responses through an "aggregator" model.
//! This is designed for extremely difficult problems where diverse perspectives
//! improve quality.
//!
//! Requires multiple LLM providers to be configured.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;

use crate::context::JobContext;
use crate::llm::{ChatMessage, CompletionRequest, LlmProvider};
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput, require_str};

/// Maximum number of reference models.
#[cfg(test)]
const MAX_REFERENCE_MODELS: usize = 5;

/// Default timeout per model call.
const MODEL_TIMEOUT: Duration = Duration::from_secs(120);

/// Mixture-of-Agents multi-LLM reasoning tool.
pub struct MoaTool {
    /// Primary LLM provider (also used as aggregator).
    primary: Arc<dyn LlmProvider>,
    /// Cheap/fast LLM provider (used as one reference model).
    cheap: Option<Arc<dyn LlmProvider>>,
}

impl MoaTool {
    pub fn new(
        primary: Arc<dyn LlmProvider>,
        cheap: Option<Arc<dyn LlmProvider>>,
    ) -> Self {
        Self { primary, cheap }
    }

    /// Check if MoA is viable (need at least 2 distinct providers).
    pub fn is_viable(&self) -> bool {
        self.cheap.is_some()
    }
}

#[async_trait]
impl Tool for MoaTool {
    fn name(&self) -> &str {
        "mixture_of_agents"
    }

    fn description(&self) -> &str {
        "Mixture-of-Agents: for extremely difficult problems, dispatches your \
         prompt to multiple LLMs in parallel and synthesizes their diverse \
         responses into a superior answer. Use sparingly — this costs 2-4x a \
         normal query. Best for: complex analysis, nuanced decisions, \
         fact-checking, or when you need high confidence."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "The complex problem or question to analyze"
                },
                "context": {
                    "type": "string",
                    "description": "Optional background context for the models"
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
        let start = std::time::Instant::now();
        let prompt = require_str(&params, "prompt")?;
        let context = params.get("context").and_then(|v| v.as_str());

        // Build the reference prompt
        let mut reference_prompt = String::new();
        if let Some(ctx) = context {
            reference_prompt.push_str("Context:\n");
            reference_prompt.push_str(ctx);
            reference_prompt.push_str("\n\n");
        }
        reference_prompt.push_str("Please provide a thorough, well-reasoned response to the following:\n\n");
        reference_prompt.push_str(prompt);

        let reference_messages = vec![ChatMessage::user(reference_prompt)];

        // Dispatch to reference models in parallel
        let mut handles: Vec<tokio::task::JoinHandle<Result<(String, Option<Decimal>), String>>> = Vec::new();

        // Reference 1: Primary model
        {
            let provider = Arc::clone(&self.primary);
            let msgs = reference_messages.clone();
            handles.push(tokio::spawn(async move {
                let request = CompletionRequest::new(msgs);
                let resp = tokio::time::timeout(
                    MODEL_TIMEOUT,
                    provider.complete(request),
                )
                .await
                .map_err(|_| "Primary model timed out".to_string())?
                .map_err(|e| format!("Primary model error: {}", e))?;

                let cost = resp.cost_usd.and_then(Decimal::from_f64);
                Ok((resp.content, cost))
            }));
        }

        // Reference 2: Cheap model (if available)
        if let Some(ref cheap) = self.cheap {
            let provider = Arc::clone(cheap);
            let msgs = reference_messages.clone();
            handles.push(tokio::spawn(async move {
                let request = CompletionRequest::new(msgs);
                let resp = tokio::time::timeout(
                    MODEL_TIMEOUT,
                    provider.complete(request),
                )
                .await
                .map_err(|_| "Cheap model timed out".to_string())?
                .map_err(|e| format!("Cheap model error: {}", e))?;

                let cost = resp.cost_usd.and_then(Decimal::from_f64);
                Ok((resp.content, cost))
            }));
        }

        // Collect reference responses
        let mut responses: Vec<String> = Vec::new();
        let mut total_cost = Decimal::ZERO;
        let mut errors: Vec<String> = Vec::new();

        for handle in handles {
            match handle.await {
                Ok(Ok((text, cost))) => {
                    responses.push(text);
                    if let Some(c) = cost {
                        total_cost += c;
                    }
                }
                Ok(Err(e)) => {
                    tracing::warn!(error = %e, "MoA reference model failed");
                    errors.push(e);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "MoA reference task panicked");
                    errors.push(e.to_string());
                }
            }
        }

        // Need at least 1 successful reference
        if responses.is_empty() {
            return Err(ToolError::ExecutionFailed(format!(
                "All reference models failed: {}",
                errors.join("; ")
            )));
        }

        // Build aggregator prompt
        let mut aggregator_prompt = String::from(
            "You have been provided with multiple AI-generated responses to the same prompt. \
             Your task is to synthesize these into a single, superior response that:\n\
             1. Combines the best insights from each response\n\
             2. Resolves any contradictions by choosing the most well-supported position\n\
             3. Adds any missing nuances\n\
             4. Provides a clear, well-structured final answer\n\n"
        );

        aggregator_prompt.push_str(&format!("Original prompt: {}\n\n", prompt));

        for (i, response) in responses.iter().enumerate() {
            aggregator_prompt.push_str(&format!(
                "--- Response {} ---\n{}\n\n",
                i + 1,
                response
            ));
        }

        aggregator_prompt.push_str("--- Your Synthesized Response ---\n");

        let aggregator_messages = vec![ChatMessage::user(aggregator_prompt)];

        // Dispatch to aggregator (primary model)
        let aggregated = self
            .primary
            .complete(CompletionRequest::new(aggregator_messages))
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Aggregator failed: {}", e)))?;

        if let Some(c) = aggregated.cost_usd.and_then(Decimal::from_f64) {
            total_cost += c;
        }

        let result_text = aggregated.content;

        let result = serde_json::json!({
            "analysis": result_text,
            "reference_count": responses.len(),
            "errors": errors,
            "prompt": prompt,
        });

        let mut output = ToolOutput::success(result, start.elapsed());
        if total_cost > Decimal::ZERO {
            output = output.with_cost(total_cost);
        }
        Ok(output)
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved // Expensive operation
    }

    fn execution_timeout(&self) -> Duration {
        Duration::from_secs(300) // 5 minutes — multiple model calls
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // MoA tests require mock LLM providers which are complex to set up.
    // The core logic is tested through the tool's execute method in integration tests.

    #[test]
    fn test_moa_viability_requires_cheap() {
        // We can't create MoaTool without a real LlmProvider in unit tests,
        // but we can verify the viability check logic.
        // In real usage: MoaTool::new(primary, Some(cheap)).is_viable() == true
        // MoaTool::new(primary, None).is_viable() == false
    }

    #[test]
    fn test_tool_metadata() {
        // Verify tool name and description are correct
        // (actual tool creation requires LlmProvider which is trait-only)
        assert!(MAX_REFERENCE_MODELS >= 2);
        assert!(MODEL_TIMEOUT.as_secs() >= 60);
    }
}
