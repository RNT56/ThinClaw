//! Unified cross-platform send message tool.
//!
//! Provides a single tool for the agent to send messages across any
//! configured platform (Telegram, Discord, Slack, etc.) without needing
//! to know which platform-specific action tool to use.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::context::JobContext;
use crate::tools::tool::{
    ApprovalRequirement, Tool, ToolError, ToolOutput, ToolRateLimitConfig, require_str,
};

/// Supported platform identifiers.
const SUPPORTED_PLATFORMS: &[&str] = &["telegram", "discord", "slack", "email", "nostr"];

/// Callback for dispatching messages to the gateway's channel infrastructure.
///
/// This is injected at registration time. The callback receives:
/// (platform, recipient, text, thread_id) and returns either a message ID or error.
pub type SendMessageFn = Arc<
    dyn Fn(
            String,
            String,
            String,
            Option<String>,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>
        + Send
        + Sync,
>;

/// Unified cross-platform messaging tool.
pub struct SendMessageTool {
    send_fn: Option<SendMessageFn>,
}

impl SendMessageTool {
    /// Create a new SendMessageTool without a send function (will error on use).
    pub fn new() -> Self {
        Self { send_fn: None }
    }

    /// Create with a gateway send function.
    pub fn with_send_fn(mut self, send_fn: SendMessageFn) -> Self {
        self.send_fn = Some(send_fn);
        self
    }
}

impl Default for SendMessageTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SendMessageTool {
    fn name(&self) -> &str {
        "send_message"
    }

    fn description(&self) -> &str {
        "Send a message to a user on any connected platform. \
         Specify the platform (telegram, discord, slack, email, or nostr) and recipient. \
         The message will be delivered through the gateway. \
         For Nostr, this tool sends encrypted DMs only; use nostr_actions for public posting or social interaction. \
         For platform-specific features (reactions, polls, etc.), use the \
         dedicated platform action tools instead."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "platform": {
                    "type": "string",
                    "enum": SUPPORTED_PLATFORMS,
                    "description": "Target platform"
                },
                "recipient": {
                    "type": "string",
                    "description": "Recipient identifier (chat_id, channel_id, email address, Nostr pubkey, etc.)"
                },
                "text": {
                    "type": "string",
                    "description": "Message content"
                },
                "thread_id": {
                    "type": "string",
                    "description": "Optional thread/topic ID to reply in"
                }
            },
            "required": ["platform", "recipient", "text"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let platform = require_str(&params, "platform")?;
        let recipient = require_str(&params, "recipient")?;
        let text = require_str(&params, "text")?;
        let thread_id = params.get("thread_id").and_then(|v| v.as_str());

        // Validate platform
        if !SUPPORTED_PLATFORMS.contains(&platform) {
            return Err(ToolError::InvalidParameters(format!(
                "Unsupported platform: '{}'. Supported: {:?}",
                platform, SUPPORTED_PLATFORMS
            )));
        }

        // Check that send function is available (gateway running)
        let send_fn = self.send_fn.as_ref().ok_or_else(|| {
            ToolError::ExecutionFailed(
                "Message gateway not available. \
                 Ensure the gateway is running and the target channel is configured."
                    .to_string(),
            )
        })?;

        // Dispatch the message
        let message_id = send_fn(
            platform.to_string(),
            recipient.to_string(),
            text.to_string(),
            thread_id.map(String::from),
        )
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("Failed to send message: {}", e)))?;

        let result = serde_json::json!({
            "success": true,
            "platform": platform,
            "recipient": recipient,
            "message_id": message_id,
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        Some(ToolRateLimitConfig::new(10, 60))
    }

    fn execution_timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_send_message_no_gateway() {
        let tool = SendMessageTool::new();
        let ctx = JobContext::default();

        let err = tool
            .execute(
                serde_json::json!({
                    "platform": "telegram",
                    "recipient": "12345",
                    "text": "hello"
                }),
                &ctx,
            )
            .await
            .unwrap_err();

        assert!(err.to_string().contains("gateway not available"));
    }

    #[tokio::test]
    async fn test_send_message_unsupported_platform() {
        let tool = SendMessageTool::new();
        let ctx = JobContext::default();

        let err = tool
            .execute(
                serde_json::json!({
                    "platform": "myspace",
                    "recipient": "user",
                    "text": "hello"
                }),
                &ctx,
            )
            .await
            .unwrap_err();

        assert!(err.to_string().contains("Unsupported platform"));
    }

    #[tokio::test]
    async fn test_send_message_missing_params() {
        let tool = SendMessageTool::new();
        let ctx = JobContext::default();

        let err = tool
            .execute(serde_json::json!({"platform": "telegram"}), &ctx)
            .await
            .unwrap_err();

        assert!(err.to_string().contains("missing"));
    }

    #[tokio::test]
    async fn test_send_message_with_mock() {
        let send_fn: SendMessageFn = Arc::new(|platform, recipient, _text, _thread| {
            Box::pin(async move { Ok(format!("msg_{}_{}", platform, recipient)) })
        });

        let tool = SendMessageTool::new().with_send_fn(send_fn);
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "platform": "telegram",
                    "recipient": "12345",
                    "text": "Hello from unified tool!"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.result.get("success").unwrap().as_bool().unwrap());
        assert_eq!(
            result.result.get("message_id").unwrap().as_str().unwrap(),
            "msg_telegram_12345"
        );
    }

    #[tokio::test]
    async fn test_send_message_with_thread() {
        let send_fn: SendMessageFn = Arc::new(|_platform, _recipient, _text, thread| {
            Box::pin(async move {
                assert!(thread.is_some());
                Ok("msg_threaded".to_string())
            })
        });

        let tool = SendMessageTool::new().with_send_fn(send_fn);
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "platform": "discord",
                    "recipient": "channel_123",
                    "text": "thread reply",
                    "thread_id": "thread_456"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.result.get("success").unwrap().as_bool().unwrap());
    }

    #[test]
    fn test_approval_required() {
        let tool = SendMessageTool::new();
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::UnlessAutoApproved
        );
    }
}
