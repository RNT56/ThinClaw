//! Telegram moderation actions tool.
//!
//! Provides the agent with Telegram-specific moderation capabilities:
//! - Send/forward/pin/unpin messages
//! - Kick/ban/unban users from groups
//! - Delete messages
//! - Get chat/member info
//!
//! Requires the bot to have appropriate admin permissions in the group.

use std::time::Instant;

use async_trait::async_trait;
use reqwest::Client;

use crate::context::JobContext;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolDomain, ToolError, ToolOutput};

/// Telegram actions tool for group moderation.
pub struct TelegramActionsTool {
    client: Client,
    api_base: String,
}

impl std::fmt::Debug for TelegramActionsTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelegramActionsTool")
            .field("api_base", &"<redacted>")
            .finish()
    }
}

impl TelegramActionsTool {
    /// Create a new Telegram actions tool.
    ///
    /// The `bot_token` is used to construct the API base URL.
    pub fn new(bot_token: &str) -> Self {
        Self {
            client: Client::new(),
            api_base: format!("https://api.telegram.org/bot{bot_token}"),
        }
    }

    /// Call a Telegram Bot API method.
    async fn api_call(
        &self,
        method: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let resp = self
            .client
            .post(format!("{}/{method}", self.api_base))
            .json(&body)
            .send()
            .await
            .map_err(|e| ToolError::ExternalService(format!("Telegram API: {e}")))?;

        let status = resp.status();
        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ToolError::ExternalService(format!("Response parse: {e}")))?;

        if !status.is_success() || json.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            let desc = json
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            return Err(ToolError::ExternalService(format!(
                "Telegram API {method}: {desc}"
            )));
        }

        Ok(json
            .get("result")
            .cloned()
            .unwrap_or(serde_json::Value::Null))
    }
}

#[async_trait]
impl Tool for TelegramActionsTool {
    fn name(&self) -> &str {
        "telegram_actions"
    }

    fn description(&self) -> &str {
        "Perform Telegram moderation actions: send messages, kick/ban users, \
         pin messages, delete messages, and get chat information. \
         Requires the bot to have admin permissions in the target group."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "send_message", "forward_message", "delete_message",
                        "pin_message", "unpin_message",
                        "kick_member", "ban_member", "unban_member",
                        "get_chat", "get_member",
                        "set_chat_title", "set_chat_description"
                    ],
                    "description": "The moderation action to perform"
                },
                "chat_id": {
                    "type": ["integer", "string"],
                    "description": "Target chat/group ID"
                },
                "user_id": {
                    "type": "integer",
                    "description": "Target user ID (for kick/ban/unban/get_member)"
                },
                "message_id": {
                    "type": "integer",
                    "description": "Message ID (for delete/pin/unpin/forward)"
                },
                "text": {
                    "type": "string",
                    "description": "Message text (for send_message) or title/description"
                },
                "from_chat_id": {
                    "type": ["integer", "string"],
                    "description": "Source chat ID (for forward_message)"
                }
            },
            "required": ["action", "chat_id"]
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
            .ok_or_else(|| ToolError::InvalidParameters("Missing 'action'".to_string()))?;

        let chat_id = params
            .get("chat_id")
            .ok_or_else(|| ToolError::InvalidParameters("Missing 'chat_id'".to_string()))?;

        let result = match action {
            "send_message" => {
                let text = params.get("text").and_then(|v| v.as_str()).ok_or_else(|| {
                    ToolError::InvalidParameters("Missing 'text' for send_message".to_string())
                })?;
                self.api_call(
                    "sendMessage",
                    serde_json::json!({
                        "chat_id": chat_id,
                        "text": text,
                    }),
                )
                .await?
            }

            "forward_message" => {
                let from_chat_id = params.get("from_chat_id").ok_or_else(|| {
                    ToolError::InvalidParameters("Missing 'from_chat_id'".to_string())
                })?;
                let message_id = params
                    .get("message_id")
                    .and_then(|v| v.as_i64())
                    .ok_or_else(|| {
                        ToolError::InvalidParameters("Missing 'message_id'".to_string())
                    })?;
                self.api_call(
                    "forwardMessage",
                    serde_json::json!({
                        "chat_id": chat_id,
                        "from_chat_id": from_chat_id,
                        "message_id": message_id,
                    }),
                )
                .await?
            }

            "delete_message" => {
                let message_id = params
                    .get("message_id")
                    .and_then(|v| v.as_i64())
                    .ok_or_else(|| {
                        ToolError::InvalidParameters("Missing 'message_id'".to_string())
                    })?;
                self.api_call(
                    "deleteMessage",
                    serde_json::json!({
                        "chat_id": chat_id,
                        "message_id": message_id,
                    }),
                )
                .await?
            }

            "pin_message" => {
                let message_id = params
                    .get("message_id")
                    .and_then(|v| v.as_i64())
                    .ok_or_else(|| {
                        ToolError::InvalidParameters("Missing 'message_id'".to_string())
                    })?;
                self.api_call(
                    "pinChatMessage",
                    serde_json::json!({
                        "chat_id": chat_id,
                        "message_id": message_id,
                    }),
                )
                .await?
            }

            "unpin_message" => {
                let message_id = params
                    .get("message_id")
                    .and_then(|v| v.as_i64())
                    .ok_or_else(|| {
                        ToolError::InvalidParameters("Missing 'message_id'".to_string())
                    })?;
                self.api_call(
                    "unpinChatMessage",
                    serde_json::json!({
                        "chat_id": chat_id,
                        "message_id": message_id,
                    }),
                )
                .await?
            }

            "kick_member" => {
                let user_id = params
                    .get("user_id")
                    .and_then(|v| v.as_i64())
                    .ok_or_else(|| ToolError::InvalidParameters("Missing 'user_id'".to_string()))?;
                // Kick = ban then immediately unban
                self.api_call(
                    "banChatMember",
                    serde_json::json!({
                        "chat_id": chat_id,
                        "user_id": user_id,
                    }),
                )
                .await?;
                // Unban so they can rejoin
                self.api_call(
                    "unbanChatMember",
                    serde_json::json!({
                        "chat_id": chat_id,
                        "user_id": user_id,
                        "only_if_banned": true,
                    }),
                )
                .await?
            }

            "ban_member" => {
                let user_id = params
                    .get("user_id")
                    .and_then(|v| v.as_i64())
                    .ok_or_else(|| ToolError::InvalidParameters("Missing 'user_id'".to_string()))?;
                self.api_call(
                    "banChatMember",
                    serde_json::json!({
                        "chat_id": chat_id,
                        "user_id": user_id,
                    }),
                )
                .await?
            }

            "unban_member" => {
                let user_id = params
                    .get("user_id")
                    .and_then(|v| v.as_i64())
                    .ok_or_else(|| ToolError::InvalidParameters("Missing 'user_id'".to_string()))?;
                self.api_call(
                    "unbanChatMember",
                    serde_json::json!({
                        "chat_id": chat_id,
                        "user_id": user_id,
                        "only_if_banned": true,
                    }),
                )
                .await?
            }

            "get_chat" => {
                self.api_call("getChat", serde_json::json!({ "chat_id": chat_id }))
                    .await?
            }

            "get_member" => {
                let user_id = params
                    .get("user_id")
                    .and_then(|v| v.as_i64())
                    .ok_or_else(|| ToolError::InvalidParameters("Missing 'user_id'".to_string()))?;
                self.api_call(
                    "getChatMember",
                    serde_json::json!({
                        "chat_id": chat_id,
                        "user_id": user_id,
                    }),
                )
                .await?
            }

            "set_chat_title" => {
                let text = params.get("text").and_then(|v| v.as_str()).ok_or_else(|| {
                    ToolError::InvalidParameters("Missing 'text' for title".to_string())
                })?;
                self.api_call(
                    "setChatTitle",
                    serde_json::json!({
                        "chat_id": chat_id,
                        "title": text,
                    }),
                )
                .await?
            }

            "set_chat_description" => {
                let text = params.get("text").and_then(|v| v.as_str()).ok_or_else(|| {
                    ToolError::InvalidParameters("Missing 'text' for description".to_string())
                })?;
                self.api_call(
                    "setChatDescription",
                    serde_json::json!({
                        "chat_id": chat_id,
                        "description": text,
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
            serde_json::json!({
                "action": action,
                "result": result,
            }),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, params: &serde_json::Value) -> ApprovalRequirement {
        let action = params.get("action").and_then(|v| v.as_str()).unwrap_or("");

        match action {
            // Destructive actions always need approval
            "kick_member" | "ban_member" | "delete_message" => ApprovalRequirement::Always,
            // Read-only actions are safe
            "get_chat" | "get_member" => ApprovalRequirement::Never,
            // Others need approval unless auto-approved
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
        let tool = TelegramActionsTool::new("fake_token");
        assert_eq!(tool.name(), "telegram_actions");
    }

    #[test]
    fn test_approval_destructive() {
        let tool = TelegramActionsTool::new("fake_token");
        assert!(matches!(
            tool.requires_approval(&serde_json::json!({"action": "kick_member"})),
            ApprovalRequirement::Always
        ));
        assert!(matches!(
            tool.requires_approval(&serde_json::json!({"action": "ban_member"})),
            ApprovalRequirement::Always
        ));
    }

    #[test]
    fn test_approval_read_only() {
        let tool = TelegramActionsTool::new("fake_token");
        assert!(matches!(
            tool.requires_approval(&serde_json::json!({"action": "get_chat"})),
            ApprovalRequirement::Never
        ));
    }

    #[test]
    fn test_approval_moderate() {
        let tool = TelegramActionsTool::new("fake_token");
        assert!(matches!(
            tool.requires_approval(&serde_json::json!({"action": "send_message"})),
            ApprovalRequirement::UnlessAutoApproved
        ));
    }
}
