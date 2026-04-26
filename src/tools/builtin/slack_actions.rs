//! Slack moderation actions tool.
//!
//! Provides the agent with Slack-specific moderation capabilities:
//! - Send/update/delete messages
//! - Set channel topic/purpose
//! - Invite/kick users from channels
//! - Get channel/user info
//! - Pin/unpin messages
//! - List channels and users
//!
//! Requires the bot to have appropriate OAuth scopes.

use std::time::Instant;

use async_trait::async_trait;
use reqwest::Client;

use crate::context::JobContext;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolDomain, ToolError, ToolOutput};

/// Slack actions tool for workspace moderation.
pub struct SlackActionsTool {
    client: Client,
    bot_token: String,
}

impl std::fmt::Debug for SlackActionsTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlackActionsTool")
            .field("bot_token", &"<redacted>")
            .finish()
    }
}

impl SlackActionsTool {
    /// Create a new Slack actions tool.
    pub fn new(bot_token: &str) -> Self {
        Self {
            client: Client::new(),
            bot_token: bot_token.to_string(),
        }
    }

    /// Call a Slack Web API method.
    async fn api(
        &self,
        method: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let resp = self
            .client
            .post(format!("https://slack.com/api/{method}"))
            .bearer_auth(&self.bot_token)
            .json(&body)
            .send()
            .await
            .map_err(|e| ToolError::ExternalService(format!("Slack API: {e}")))?;

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ToolError::ExternalService(format!("Response parse: {e}")))?;

        if json.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            let error = json
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            return Err(ToolError::ExternalService(format!(
                "Slack {method}: {error}"
            )));
        }

        Ok(json)
    }
}

#[async_trait]
impl Tool for SlackActionsTool {
    fn name(&self) -> &str {
        "slack_actions"
    }

    fn description(&self) -> &str {
        "Perform live Slack actions such as posting or moderating messages, managing \
         channels, and reading user or channel info. Use this for real Slack-side changes, \
         not for searching prior Slack transcript history."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "send_message", "update_message", "delete_message",
                        "pin_message", "unpin_message",
                        "set_topic", "set_purpose",
                        "invite_to_channel", "kick_from_channel",
                        "get_channel_info", "get_user_info",
                        "list_channels", "list_users",
                        "add_reaction", "remove_reaction"
                    ],
                    "description": "The Slack action to perform"
                },
                "channel": {
                    "type": "string",
                    "description": "Channel ID"
                },
                "user": {
                    "type": "string",
                    "description": "User ID"
                },
                "ts": {
                    "type": "string",
                    "description": "Message timestamp (for update/delete/pin/unpin/reaction)"
                },
                "text": {
                    "type": "string",
                    "description": "Message text, topic, purpose, or reaction name"
                },
                "thread_ts": {
                    "type": "string",
                    "description": "Thread timestamp (for threaded replies)"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();

        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("Missing 'action'".into()))?;

        let get_str = |key: &str| -> Result<&str, ToolError> {
            params
                .get(key)
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidParameters(format!("Missing '{key}'")))
        };

        let result = match action {
            "send_message" => {
                let channel = get_str("channel")?;
                let text = get_str("text")?;
                let mut body = serde_json::json!({
                    "channel": channel,
                    "text": text,
                });
                if let Some(ts) = params.get("thread_ts").and_then(|v| v.as_str()) {
                    body["thread_ts"] = serde_json::Value::String(ts.to_string());
                }
                self.api("chat.postMessage", body).await?
            }

            "update_message" => {
                let channel = get_str("channel")?;
                let ts = get_str("ts")?;
                let text = get_str("text")?;
                self.api(
                    "chat.update",
                    serde_json::json!({
                        "channel": channel,
                        "ts": ts,
                        "text": text,
                    }),
                )
                .await?
            }

            "delete_message" => {
                let channel = get_str("channel")?;
                let ts = get_str("ts")?;
                self.api(
                    "chat.delete",
                    serde_json::json!({"channel": channel, "ts": ts}),
                )
                .await?
            }

            "pin_message" => {
                let channel = get_str("channel")?;
                let ts = get_str("ts")?;
                self.api(
                    "pins.add",
                    serde_json::json!({"channel": channel, "timestamp": ts}),
                )
                .await?
            }

            "unpin_message" => {
                let channel = get_str("channel")?;
                let ts = get_str("ts")?;
                self.api(
                    "pins.remove",
                    serde_json::json!({"channel": channel, "timestamp": ts}),
                )
                .await?
            }

            "set_topic" => {
                let channel = get_str("channel")?;
                let text = get_str("text")?;
                self.api(
                    "conversations.setTopic",
                    serde_json::json!({"channel": channel, "topic": text}),
                )
                .await?
            }

            "set_purpose" => {
                let channel = get_str("channel")?;
                let text = get_str("text")?;
                self.api(
                    "conversations.setPurpose",
                    serde_json::json!({"channel": channel, "purpose": text}),
                )
                .await?
            }

            "invite_to_channel" => {
                let channel = get_str("channel")?;
                let user = get_str("user")?;
                self.api(
                    "conversations.invite",
                    serde_json::json!({"channel": channel, "users": user}),
                )
                .await?
            }

            "kick_from_channel" => {
                let channel = get_str("channel")?;
                let user = get_str("user")?;
                self.api(
                    "conversations.kick",
                    serde_json::json!({"channel": channel, "user": user}),
                )
                .await?
            }

            "get_channel_info" => {
                let channel = get_str("channel")?;
                self.api(
                    "conversations.info",
                    serde_json::json!({"channel": channel}),
                )
                .await?
            }

            "get_user_info" => {
                let user = get_str("user")?;
                self.api("users.info", serde_json::json!({"user": user}))
                    .await?
            }

            "list_channels" => {
                self.api(
                    "conversations.list",
                    serde_json::json!({"types": "public_channel,private_channel", "limit": 200}),
                )
                .await?
            }

            "list_users" => {
                self.api("users.list", serde_json::json!({"limit": 200}))
                    .await?
            }

            "add_reaction" => {
                let channel = get_str("channel")?;
                let ts = get_str("ts")?;
                let text = get_str("text")?; // reaction name
                self.api(
                    "reactions.add",
                    serde_json::json!({
                        "channel": channel,
                        "timestamp": ts,
                        "name": text,
                    }),
                )
                .await?
            }

            "remove_reaction" => {
                let channel = get_str("channel")?;
                let ts = get_str("ts")?;
                let text = get_str("text")?; // reaction name
                self.api(
                    "reactions.remove",
                    serde_json::json!({
                        "channel": channel,
                        "timestamp": ts,
                        "name": text,
                    }),
                )
                .await?
            }

            other => {
                return Err(ToolError::InvalidParameters(format!(
                    "Unknown action: '{other}'"
                )));
            }
        };

        Ok(ToolOutput::success(
            serde_json::json!({"action": action, "result": result}),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, params: &serde_json::Value) -> ApprovalRequirement {
        let action = params.get("action").and_then(|v| v.as_str()).unwrap_or("");

        match action {
            "kick_from_channel" | "delete_message" => ApprovalRequirement::Always,
            "get_channel_info" | "get_user_info" | "list_channels" | "list_users" => {
                ApprovalRequirement::Never
            }
            _ => ApprovalRequirement::UnlessAutoApproved,
        }
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_name() {
        let tool = SlackActionsTool::new("xoxb-fake");
        assert_eq!(tool.name(), "slack_actions");
    }

    #[test]
    fn test_approval_destructive() {
        let tool = SlackActionsTool::new("xoxb-fake");
        assert!(matches!(
            tool.requires_approval(&serde_json::json!({"action": "kick_from_channel"})),
            ApprovalRequirement::Always
        ));
    }

    #[test]
    fn test_approval_read_only() {
        let tool = SlackActionsTool::new("xoxb-fake");
        assert!(matches!(
            tool.requires_approval(&serde_json::json!({"action": "list_channels"})),
            ApprovalRequirement::Never
        ));
    }

    #[test]
    fn test_approval_moderate() {
        let tool = SlackActionsTool::new("xoxb-fake");
        assert!(matches!(
            tool.requires_approval(&serde_json::json!({"action": "send_message"})),
            ApprovalRequirement::UnlessAutoApproved
        ));
    }
}
