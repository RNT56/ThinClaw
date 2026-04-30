use std::sync::Arc;
use std::time::Duration;

use crate::config::Config;
use crate::error::LlmError;
#[cfg(test)]
use crate::llm::{
    CompletionRequest, CompletionResponse, FinishReason, ToolCompletionRequest,
    ToolCompletionResponse,
};
use crate::llm::{LlmProvider, build_provider_chain};
#[cfg(test)]
use async_trait::async_trait;
#[cfg(test)]
use rust_decimal::Decimal;

pub use thinclaw_tools::smart_approve::{ApprovalDecision, SmartApprovalMode};

pub struct SmartApprover {
    inner: thinclaw_tools::smart_approve::SmartApprover,
}

impl SmartApprover {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self {
            inner: thinclaw_tools::smart_approve::SmartApprover::new(provider),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.inner = self.inner.with_timeout(timeout);
        self
    }

    #[cfg(test)]
    pub(crate) fn from_test_response(response: impl Into<String>) -> Self {
        Self::new(Arc::new(TestResponseProvider {
            response: response.into(),
        }))
    }

    pub async fn from_env() -> Result<Self, String> {
        let config = Config::from_env().await.map_err(|e| e.to_string())?;
        let (primary, cheap) = build_provider_chain(&config.llm, None).map_err(map_llm_error)?;
        let provider = cheap.unwrap_or(primary);
        Ok(Self::new(provider))
    }

    pub async fn assess_command(
        &self,
        command: &str,
        description: &str,
        working_dir: &str,
    ) -> ApprovalDecision {
        self.inner
            .assess_command(command, description, working_dir)
            .await
    }
}

fn map_llm_error(err: LlmError) -> String {
    err.to_string()
}

#[cfg(test)]
struct TestResponseProvider {
    response: String,
}

#[cfg(test)]
#[async_trait]
impl LlmProvider for TestResponseProvider {
    fn model_name(&self) -> &str {
        "smart-approval-test"
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Ok(CompletionResponse {
            content: self.response.clone(),
            provider_model: Some(self.model_name().to_string()),
            cost_usd: None,
            thinking_content: None,
            input_tokens: 1,
            output_tokens: 1,
            finish_reason: FinishReason::Stop,
            token_capture: None,
        })
    }

    async fn complete_with_tools(
        &self,
        _request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        Err(LlmError::RequestFailed {
            provider: "test".to_string(),
            reason: "tool use not supported".to_string(),
        })
    }
}
