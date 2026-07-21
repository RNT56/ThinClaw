//! Compatibility adapter for the extracted heartbeat runner.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use thinclaw_agent::heartbeat::{
    HeartbeatLlmPort, HeartbeatOutcomeSummaryPort, HeartbeatRunner as ExtractedHeartbeatRunner,
};

#[allow(unused_imports)] // compatibility re-export for downstream root-crate callers
pub use thinclaw_agent::heartbeat::{
    HeartbeatConfig, HeartbeatResult, build_daily_context, is_effectively_empty,
};

use crate::db::Database;
use crate::llm::{CompletionRequest, LlmProvider, Reasoning};
use crate::workspace::hygiene::HygieneConfig;
use crate::workspace::{AuthorizedWorkspace, Workspace};

/// Heartbeat runner preserving the root crate constructor and builder API.
pub struct HeartbeatRunner {
    inner: ExtractedHeartbeatRunner,
    llm: Arc<RootHeartbeatLlm>,
}

impl HeartbeatRunner {
    /// Create a new heartbeat runner.
    pub fn new(
        config: HeartbeatConfig,
        hygiene_config: HygieneConfig,
        workspace: Arc<Workspace>,
        llm: Arc<dyn LlmProvider>,
    ) -> Self {
        let llm = Arc::new(RootHeartbeatLlm::new(llm));
        let inner = ExtractedHeartbeatRunner::new(config, hygiene_config, workspace, llm.clone());
        Self { inner, llm }
    }

    /// Create a standalone heartbeat bound to the caller's exact memory scope.
    pub fn new_authorized(
        config: HeartbeatConfig,
        hygiene_config: HygieneConfig,
        workspace: Arc<AuthorizedWorkspace>,
        llm: Arc<dyn LlmProvider>,
    ) -> Self {
        let llm = Arc::new(RootHeartbeatLlm::new(llm));
        let inner = ExtractedHeartbeatRunner::new_authorized(
            config,
            hygiene_config,
            workspace,
            llm.clone(),
        );
        Self { inner, llm }
    }

    /// Attach a shared cost tracker so heartbeat LLM calls are recorded.
    pub fn with_cost_tracker(
        self,
        tracker: Arc<tokio::sync::Mutex<crate::llm::cost_tracker::CostTracker>>,
    ) -> Self {
        self.llm.set_cost_tracker(tracker);
        self
    }

    /// Attach DB context for outcome-review summaries in standalone heartbeat mode.
    pub fn with_outcome_context(
        mut self,
        store: Arc<dyn Database>,
        user_id: impl Into<String>,
    ) -> Self {
        self.inner = self
            .inner
            .with_outcome_summary(Arc::new(RootHeartbeatOutcomeSummary {
                store,
                user_id: user_id.into(),
            }));
        self
    }

    /// Run a single heartbeat check.
    pub async fn check_heartbeat(&self) -> HeartbeatResult {
        self.inner.check_heartbeat().await
    }
}

struct RootHeartbeatLlm {
    llm: Arc<dyn LlmProvider>,
    cost_tracker: Mutex<Option<Arc<tokio::sync::Mutex<crate::llm::cost_tracker::CostTracker>>>>,
}

impl RootHeartbeatLlm {
    fn new(llm: Arc<dyn LlmProvider>) -> Self {
        Self {
            llm,
            cost_tracker: Mutex::new(None),
        }
    }

    fn set_cost_tracker(
        &self,
        tracker: Arc<tokio::sync::Mutex<crate::llm::cost_tracker::CostTracker>>,
    ) {
        *self
            .cost_tracker
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(tracker);
    }
}

#[async_trait]
impl HeartbeatLlmPort for RootHeartbeatLlm {
    async fn context_length(&self) -> Result<Option<u32>, String> {
        self.llm
            .model_metadata()
            .await
            .map(|metadata| metadata.context_length)
            .map_err(|err| err.to_string())
    }

    async fn complete_heartbeat(&self, request: CompletionRequest) -> Result<String, String> {
        let mut reasoning = Reasoning::new(Arc::clone(&self.llm));
        let tracker = self
            .cost_tracker
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(tracker) = tracker {
            reasoning = reasoning.with_cost_tracker(tracker);
        }
        reasoning
            .complete(request)
            .await
            .map(|(content, _usage)| content)
            .map_err(|err| err.to_string())
    }
}

struct RootHeartbeatOutcomeSummary {
    store: Arc<dyn Database>,
    user_id: String,
}

#[async_trait]
impl HeartbeatOutcomeSummaryPort for RootHeartbeatOutcomeSummary {
    async fn heartbeat_review_summary(&self) -> Result<Option<String>, String> {
        crate::agent::outcomes::heartbeat_review_summary(&self.store, &self.user_id)
            .await
            .map_err(|err| err.to_string())
    }
}
