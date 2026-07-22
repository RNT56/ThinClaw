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
        let channel_name = message.channel.clone();

        let accepted = self
            .agent
            .spawn_external_submission(async move {
                agent.channels().record_received(&message.channel).await;
                match agent.handle_message_external(&message).await {
                    Ok(Some(response)) if !response.is_empty() => {
                        if let Err(error) = agent
                            .channels()
                            .respond(
                                &message,
                                crate::channels::OutgoingResponse::text(response.content)
                                    .with_attachments(response.attachments),
                            )
                            .await
                        {
                            tracing::warn!(
                                run_id = %run_id,
                                %error,
                                "Channel submission response delivery failed"
                            );
                        }
                    }
                    Ok(_) => {}
                    Err(error) => {
                        tracing::warn!(run_id = %run_id, %error, "Channel submission failed");
                        let _ = agent
                            .channels()
                            .send_status(
                                &message.channel,
                                crate::channels::StatusUpdate::Error {
                                    message: error.to_string(),
                                    code: Some("submission_failed".to_string()),
                                },
                                &message.metadata,
                            )
                            .await;
                    }
                }
            })
            .await;
        if !accepted {
            return Err(ChannelError::Disconnected {
                name: channel_name,
                reason: "agent submission queue is full or shutting down".to_string(),
            });
        }

        Ok(ChannelSubmissionAck {
            run_id,
            thread_id,
            accepted_at: Utc::now(),
            status: SubmissionStatus::Accepted,
        })
    }
}
