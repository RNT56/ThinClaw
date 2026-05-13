//! Agent environment compatibility facade.
//!
//! The reusable environment/eval framework lives in `thinclaw-agent`. The root
//! module only adapts the concrete root `Agent` into the extracted env port.

use async_trait::async_trait;
use thinclaw_channels_core::IncomingMessage;
use thinclaw_llm_core::{ProviderTokenCapture, TokenCaptureSupport};

pub use thinclaw_agent::env::*;

#[async_trait]
impl AgentEnvAgent for crate::agent::Agent {
    async fn handle_env_message(
        &self,
        message: &IncomingMessage,
    ) -> anyhow::Result<Option<String>> {
        self.handle_message_external(message)
            .await
            .map_err(Into::into)
    }

    async fn latest_token_capture_for_env_message(
        &self,
        message: &IncomingMessage,
    ) -> Option<ProviderTokenCapture> {
        self.latest_token_capture_for_message(message).await
    }

    fn env_llm_token_capture_support(&self) -> TokenCaptureSupport {
        self.llm_token_capture_support()
    }

    fn env_llm_provider_name(&self) -> String {
        self.llm_provider_name()
    }
}
