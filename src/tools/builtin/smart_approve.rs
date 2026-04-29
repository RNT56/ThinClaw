use std::sync::Arc;
use std::time::Duration;

#[cfg(test)]
use async_trait::async_trait;
#[cfg(test)]
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tokio::time::timeout;

use crate::config::Config;
use crate::error::LlmError;
use crate::llm::{ChatMessage, CompletionRequest, LlmProvider, build_provider_chain};

/// Decision returned by the smart approver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalDecision {
    Approve,
    Deny,
    Escalate,
}

/// Smart approval operating mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SmartApprovalMode {
    Off,
    Smart,
    AlwaysAsk,
}

impl SmartApprovalMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Smart => "smart",
            Self::AlwaysAsk => "always_ask",
        }
    }
}

impl std::str::FromStr for SmartApprovalMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" | "disabled" | "false" | "0" => Ok(Self::Off),
            "smart" | "auto" | "llm" => Ok(Self::Smart),
            "always_ask" | "always-ask" | "alwaysask" | "ask" => Ok(Self::AlwaysAsk),
            other => Err(format!(
                "invalid smart approval mode '{other}', expected off, smart, or always_ask"
            )),
        }
    }
}

/// Cheap LLM-backed triage for shell approval.
pub struct SmartApprover {
    provider: Arc<dyn LlmProvider>,
    timeout: Duration,
}

impl SmartApprover {
    /// Construct a smart approver from an already-selected provider.
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self {
            provider,
            timeout: Duration::from_secs(8),
        }
    }

    /// Set the request timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Build a smart approver from the current environment/configuration.
    ///
    /// Uses the existing LLM provider chain and prefers the cheap model when
    /// one is configured. In test builds, `SAFETY_SMART_APPROVAL_TEST_RESPONSE`
    /// can force a deterministic mock provider response.
    pub async fn from_env() -> Result<Self, String> {
        #[cfg(test)]
        if let Some(response) =
            crate::config::helpers::optional_env("SAFETY_SMART_APPROVAL_TEST_RESPONSE")
                .map_err(|e| e.to_string())?
        {
            return Ok(Self::new(Arc::new(MockResponseProvider::new(response))));
        }

        let config = Config::from_env().await.map_err(|e| e.to_string())?;
        let (primary, cheap) = build_provider_chain(&config.llm, None).map_err(map_llm_error)?;
        let provider = cheap.unwrap_or(primary);
        Ok(Self::new(provider))
    }

    /// Assess whether a command is genuinely dangerous.
    pub async fn assess_command(
        &self,
        command: &str,
        description: &str,
        working_dir: &str,
    ) -> ApprovalDecision {
        let prompt = build_prompt(command, description, working_dir);
        let request = CompletionRequest::new(vec![
            ChatMessage::system(
                "You are a security assessor. Evaluate whether this shell command is genuinely dangerous. \
                 Respond with exactly one word: APPROVE (safe false positive), DENY (genuinely dangerous), \
                 or ESCALATE (uncertain).",
            ),
            ChatMessage::user(prompt),
        ])
        .with_max_tokens(8)
        .with_temperature(0.0);

        match timeout(self.timeout, self.provider.complete(request)).await {
            Ok(Ok(response)) => parse_decision(&response.content),
            _ => ApprovalDecision::Escalate,
        }
    }
}

fn build_prompt(command: &str, description: &str, working_dir: &str) -> String {
    format!(
        "Command: {command}\nDescription: {description}\nWorking directory: {working_dir}\n\n\
         Return exactly one word."
    )
}

fn parse_decision(response: &str) -> ApprovalDecision {
    let upper = response.to_ascii_uppercase();
    if upper.contains("APPROVE") {
        ApprovalDecision::Approve
    } else if upper.contains("DENY") {
        ApprovalDecision::Deny
    } else {
        ApprovalDecision::Escalate
    }
}

fn map_llm_error(err: LlmError) -> String {
    err.to_string()
}

#[cfg(test)]
struct MockResponseProvider {
    response: String,
}

#[cfg(test)]
impl MockResponseProvider {
    fn new(response: impl Into<String>) -> Self {
        Self {
            response: response.into(),
        }
    }
}

#[cfg(test)]
#[async_trait]
impl LlmProvider for MockResponseProvider {
    fn model_name(&self) -> &str {
        "smart-approval-test"
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(
        &self,
        _request: CompletionRequest,
    ) -> Result<crate::llm::CompletionResponse, LlmError> {
        Ok(crate::llm::CompletionResponse {
            content: self.response.clone(),
            provider_model: Some(self.model_name().to_string()),
            cost_usd: None,
            thinking_content: None,
            input_tokens: 1,
            output_tokens: 1,
            finish_reason: crate::llm::FinishReason::Stop,
            token_capture: None,
        })
    }

    async fn complete_with_tools(
        &self,
        _request: crate::llm::ToolCompletionRequest,
    ) -> Result<crate::llm::ToolCompletionResponse, LlmError> {
        Err(LlmError::RequestFailed {
            provider: "test".to_string(),
            reason: "tool use not supported".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use rust_decimal::Decimal;

    struct ErrorProvider;

    #[async_trait]
    impl LlmProvider for ErrorProvider {
        fn model_name(&self) -> &str {
            "error-provider"
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<crate::llm::CompletionResponse, LlmError> {
            Err(LlmError::RequestFailed {
                provider: "test".to_string(),
                reason: "boom".to_string(),
            })
        }

        async fn complete_with_tools(
            &self,
            _request: crate::llm::ToolCompletionRequest,
        ) -> Result<crate::llm::ToolCompletionResponse, LlmError> {
            Err(LlmError::RequestFailed {
                provider: "test".to_string(),
                reason: "boom".to_string(),
            })
        }
    }

    #[test]
    fn parse_mode_variants() {
        assert_eq!(
            "off".parse::<SmartApprovalMode>().unwrap(),
            SmartApprovalMode::Off
        );
        assert_eq!(
            "smart".parse::<SmartApprovalMode>().unwrap(),
            SmartApprovalMode::Smart
        );
        assert_eq!(
            "always-ask".parse::<SmartApprovalMode>().unwrap(),
            SmartApprovalMode::AlwaysAsk
        );
    }

    #[tokio::test]
    async fn approve_response_maps_to_approve() {
        let approver = SmartApprover::new(Arc::new(MockResponseProvider::new("APPROVE")));
        assert_eq!(
            approver
                .assess_command("ls -la", "list files", "/tmp")
                .await,
            ApprovalDecision::Approve
        );
    }

    #[tokio::test]
    async fn deny_response_maps_to_deny() {
        let approver = SmartApprover::new(Arc::new(MockResponseProvider::new("DENY")));
        assert_eq!(
            approver
                .assess_command("rm -rf /tmp", "remove files", "/tmp")
                .await,
            ApprovalDecision::Deny
        );
    }

    #[tokio::test]
    async fn llm_error_escalates() {
        let approver = SmartApprover::new(Arc::new(ErrorProvider));
        assert_eq!(
            approver
                .assess_command("sudo echo hi", "test", "/tmp")
                .await,
            ApprovalDecision::Escalate
        );
    }
}
