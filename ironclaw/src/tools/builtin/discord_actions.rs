//! Discord moderation actions tool.
//!
//! Provides the agent with Discord-specific moderation capabilities:
//! - Send/delete/pin/unpin messages
//! - Kick/ban/unban users from guilds
//! - Manage roles (add/remove)
//! - Get guild/channel/member info
//! - Create/delete channels
//!
//! Requires the bot to have appropriate permissions in the guild.

use std::time::Instant;

use async_trait::async_trait;
use reqwest::Client;

use crate::context::JobContext;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolDomain, ToolError, ToolOutput};

const API_BASE: &str = "https://discord.com/api/v10";

/// Discord actions tool for guild moderation.
pub struct DiscordActionsTool {
    client: Client,
    bot_token: String,
}

impl std::fmt::Debug for DiscordActionsTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscordActionsTool")
            .field("bot_token", &"<redacted>")
            .finish()
    }
}

impl DiscordActionsTool {
    /// Create a new Discord actions tool.
    pub fn new(bot_token: &str) -> Self {
        Self {
            client: Client::new(),
            bot_token: bot_token.to_string(),
        }
    }

    /// Call a Discord REST API endpoint.
    async fn api(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, ToolError> {
        let mut req = self
            .client
            .request(method, format!("{API_BASE}{path}"))
            .header("Authorization", format!("Bot {}", self.bot_token));

        if let Some(b) = body {
            req = req.json(&b);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| ToolError::ExternalService(format!("Discord API: {e}")))?;

        let status = resp.status();
        if status.as_u16() == 204 {
            return Ok(serde_json::json!({"success": true}));
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .unwrap_or(serde_json::json!({"raw_status": status.as_u16()}));

        if !status.is_success() {
            let msg = json
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            return Err(ToolError::ExternalService(format!(
                "Discord {status}: {msg}"
            )));
        }

        Ok(json)
    }
}

#[async_trait]
impl Tool for DiscordActionsTool {
    fn name(&self) -> &str {
        "discord_actions"
    }

    fn description(&self) -> &str {
        "Perform Discord moderation actions: send/delete/pin messages, \
         kick/ban/unban users, manage roles, get guild/channel info. \
         Requires the bot to have appropriate permissions."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "send_message", "delete_message",
                        "pin_message", "unpin_message",
                        "kick_member", "ban_member", "unban_member",
                        "add_role", "remove_role",
                        "get_guild", "get_channel", "get_member",
                        "create_channel", "delete_channel",
                        "set_nickname"
                    ],
                    "description": "The moderation action to perform"
                },
                "guild_id": {
                    "type": "string",
                    "description": "Guild (server) ID"
                },
                "channel_id": {
                    "type": "string",
                    "description": "Channel ID"
                },
                "user_id": {
                    "type": "string",
                    "description": "Target user ID"
                },
                "message_id": {
                    "type": "string",
                    "description": "Message ID (for delete/pin/unpin)"
                },
                "role_id": {
                    "type": "string",
                    "description": "Role ID (for add_role/remove_role)"
                },
                "text": {
                    "type": "string",
                    "description": "Message content or name/reason"
                },
                "reason": {
                    "type": "string",
                    "description": "Audit log reason"
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
                let channel_id = get_str("channel_id")?;
                let text = get_str("text")?;
                self.api(
                    reqwest::Method::POST,
                    &format!("/channels/{channel_id}/messages"),
                    Some(serde_json::json!({"content": text})),
                )
                .await?
            }

            "delete_message" => {
                let channel_id = get_str("channel_id")?;
                let message_id = get_str("message_id")?;
                self.api(
                    reqwest::Method::DELETE,
                    &format!("/channels/{channel_id}/messages/{message_id}"),
                    None,
                )
                .await?
            }

            "pin_message" => {
                let channel_id = get_str("channel_id")?;
                let message_id = get_str("message_id")?;
                self.api(
                    reqwest::Method::PUT,
                    &format!("/channels/{channel_id}/pins/{message_id}"),
                    None,
                )
                .await?
            }

            "unpin_message" => {
                let channel_id = get_str("channel_id")?;
                let message_id = get_str("message_id")?;
                self.api(
                    reqwest::Method::DELETE,
                    &format!("/channels/{channel_id}/pins/{message_id}"),
                    None,
                )
                .await?
            }

            "kick_member" => {
                let guild_id = get_str("guild_id")?;
                let user_id = get_str("user_id")?;
                self.api(
                    reqwest::Method::DELETE,
                    &format!("/guilds/{guild_id}/members/{user_id}"),
                    None,
                )
                .await?
            }

            "ban_member" => {
                let guild_id = get_str("guild_id")?;
                let user_id = get_str("user_id")?;
                let reason = params.get("reason").and_then(|v| v.as_str());
                let mut body = serde_json::json!({});
                if let Some(r) = reason {
                    body["reason"] = serde_json::Value::String(r.to_string());
                }
                self.api(
                    reqwest::Method::PUT,
                    &format!("/guilds/{guild_id}/bans/{user_id}"),
                    Some(body),
                )
                .await?
            }

            "unban_member" => {
                let guild_id = get_str("guild_id")?;
                let user_id = get_str("user_id")?;
                self.api(
                    reqwest::Method::DELETE,
                    &format!("/guilds/{guild_id}/bans/{user_id}"),
                    None,
                )
                .await?
            }

            "add_role" => {
                let guild_id = get_str("guild_id")?;
                let user_id = get_str("user_id")?;
                let role_id = get_str("role_id")?;
                self.api(
                    reqwest::Method::PUT,
                    &format!("/guilds/{guild_id}/members/{user_id}/roles/{role_id}"),
                    None,
                )
                .await?
            }

            "remove_role" => {
                let guild_id = get_str("guild_id")?;
                let user_id = get_str("user_id")?;
                let role_id = get_str("role_id")?;
                self.api(
                    reqwest::Method::DELETE,
                    &format!("/guilds/{guild_id}/members/{user_id}/roles/{role_id}"),
                    None,
                )
                .await?
            }

            "get_guild" => {
                let guild_id = get_str("guild_id")?;
                self.api(reqwest::Method::GET, &format!("/guilds/{guild_id}"), None)
                    .await?
            }

            "get_channel" => {
                let channel_id = get_str("channel_id")?;
                self.api(
                    reqwest::Method::GET,
                    &format!("/channels/{channel_id}"),
                    None,
                )
                .await?
            }

            "get_member" => {
                let guild_id = get_str("guild_id")?;
                let user_id = get_str("user_id")?;
                self.api(
                    reqwest::Method::GET,
                    &format!("/guilds/{guild_id}/members/{user_id}"),
                    None,
                )
                .await?
            }

            "create_channel" => {
                let guild_id = get_str("guild_id")?;
                let text = get_str("text")?; // channel name
                self.api(
                    reqwest::Method::POST,
                    &format!("/guilds/{guild_id}/channels"),
                    Some(serde_json::json!({"name": text, "type": 0})),
                )
                .await?
            }

            "delete_channel" => {
                let channel_id = get_str("channel_id")?;
                self.api(
                    reqwest::Method::DELETE,
                    &format!("/channels/{channel_id}"),
                    None,
                )
                .await?
            }

            "set_nickname" => {
                let guild_id = get_str("guild_id")?;
                let user_id = get_str("user_id")?;
                let text = get_str("text")?;
                self.api(
                    reqwest::Method::PATCH,
                    &format!("/guilds/{guild_id}/members/{user_id}"),
                    Some(serde_json::json!({"nick": text})),
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
            "kick_member" | "ban_member" | "delete_message" | "delete_channel" => {
                ApprovalRequirement::Always
            }
            "get_guild" | "get_channel" | "get_member" => ApprovalRequirement::Never,
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
        let tool = DiscordActionsTool::new("fake_token");
        assert_eq!(tool.name(), "discord_actions");
    }

    #[test]
    fn test_approval_destructive() {
        let tool = DiscordActionsTool::new("fake_token");
        assert!(matches!(
            tool.requires_approval(&serde_json::json!({"action": "kick_member"})),
            ApprovalRequirement::Always
        ));
        assert!(matches!(
            tool.requires_approval(&serde_json::json!({"action": "ban_member"})),
            ApprovalRequirement::Always
        ));
        assert!(matches!(
            tool.requires_approval(&serde_json::json!({"action": "delete_channel"})),
            ApprovalRequirement::Always
        ));
    }

    #[test]
    fn test_approval_read_only() {
        let tool = DiscordActionsTool::new("fake_token");
        assert!(matches!(
            tool.requires_approval(&serde_json::json!({"action": "get_guild"})),
            ApprovalRequirement::Never
        ));
        assert!(matches!(
            tool.requires_approval(&serde_json::json!({"action": "get_member"})),
            ApprovalRequirement::Never
        ));
    }
}
