//! Compatibility adapter for the extracted heartbeat runner.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use thinclaw_agent::heartbeat::{
    HeartbeatLlmPort, HeartbeatOutcomeSummaryPort, HeartbeatRunner as ExtractedHeartbeatRunner,
};

pub use thinclaw_agent::heartbeat::{
    HeartbeatConfig, HeartbeatResult, build_daily_context, is_effectively_empty,
};

use crate::channels::OutgoingResponse;
use crate::db::Database;
use crate::llm::{CompletionRequest, LlmProvider, Reasoning};
use crate::safety::SafetyLayer;
use crate::workspace::Workspace;
use crate::workspace::hygiene::HygieneConfig;

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
        safety: Arc<SafetyLayer>,
    ) -> Self {
        let llm = Arc::new(RootHeartbeatLlm::new(llm, safety));
        let inner = ExtractedHeartbeatRunner::new(config, hygiene_config, workspace, llm.clone());
        Self { inner, llm }
    }

    /// Set the response channel for notifications.
    pub fn with_response_channel(
        mut self,
        tx: tokio::sync::mpsc::Sender<OutgoingResponse>,
    ) -> Self {
        self.inner = self.inner.with_response_channel(tx);
        self
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

    /// Run the heartbeat loop.
    pub async fn run(&mut self) {
        self.inner.run().await;
    }

    /// Run a single heartbeat check.
    pub async fn check_heartbeat(&self) -> HeartbeatResult {
        self.inner.check_heartbeat().await
    }
}

struct RootHeartbeatLlm {
    llm: Arc<dyn LlmProvider>,
    safety: Arc<SafetyLayer>,
    cost_tracker: Mutex<Option<Arc<tokio::sync::Mutex<crate::llm::cost_tracker::CostTracker>>>>,
}

impl RootHeartbeatLlm {
    fn new(llm: Arc<dyn LlmProvider>, safety: Arc<SafetyLayer>) -> Self {
        Self {
            llm,
            safety,
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
            .expect("cost tracker mutex poisoned") = Some(tracker);
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
        let mut reasoning = Reasoning::new(Arc::clone(&self.llm), Arc::clone(&self.safety));
        let tracker = self
            .cost_tracker
            .lock()
            .expect("cost tracker mutex poisoned")
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

/// Spawn the heartbeat runner as a background task.
pub fn spawn_heartbeat(
    config: HeartbeatConfig,
    hygiene_config: HygieneConfig,
    workspace: Arc<Workspace>,
    llm: Arc<dyn LlmProvider>,
    safety: Arc<SafetyLayer>,
    response_tx: Option<tokio::sync::mpsc::Sender<OutgoingResponse>>,
    cost_tracker: Option<Arc<tokio::sync::Mutex<crate::llm::cost_tracker::CostTracker>>>,
) -> tokio::task::JoinHandle<()> {
    let mut runner = HeartbeatRunner::new(config, hygiene_config, workspace, llm, safety);
    if let Some(tx) = response_tx {
        runner = runner.with_response_channel(tx);
    }
    if let Some(tracker) = cost_tracker {
        runner = runner.with_cost_tracker(tracker);
    }

    tokio::spawn(async move {
        runner.run().await;
    })
}
