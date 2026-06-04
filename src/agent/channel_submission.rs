//! Root agent adapter for the extracted channel-submission port.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use thinclaw_agent::ports::{
    ChannelSubmission, ChannelSubmissionAck, ChannelSubmissionPort, SubmissionStatus,
};

use crate::agent::Agent;
use crate::error::ChannelError;

pub struct RootChannelSubmissionPort {
    agent: Arc<Agent>,
}

impl RootChannelSubmissionPort {
    pub fn shared(agent: Arc<Agent>) -> Arc<dyn ChannelSubmissionPort> {
        Arc::new(Self { agent })
    }
}

#[async_trait]
impl ChannelSubmissionPort for RootChannelSubmissionPort {
    async fn submit(
        &self,
        submission: ChannelSubmission,
    ) -> Result<ChannelSubmissionAck, ChannelError> {
        let run_id = uuid::Uuid::new_v4();
        let thread_id = submission.parsed_thread_id();
        let agent = Arc::clone(&self.agent);
        let message = submission.message;

        tokio::spawn(async move {
            if let Err(error) = agent.handle_message_external(&message).await {
                tracing::warn!(run_id = %run_id, error = %error, "Channel submission failed");
            }
        });

        Ok(ChannelSubmissionAck {
            run_id,
            thread_id,
            accepted_at: Utc::now(),
            status: SubmissionStatus::Accepted,
        })
    }
}
