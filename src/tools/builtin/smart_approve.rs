use std::sync::Arc;
use std::time::Duration;

use crate::config::Config;
use crate::error::LlmError;
use crate::llm::{LlmProvider, build_provider_chain};

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
