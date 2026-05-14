//! Self-message bypass and message filtering.
//!
//! Prevents the agent from processing its own messages, which could
//! create infinite loops — especially on channels where the bot's
//! sent messages are echoed back.
//!
//! Configuration:
//! - `BOT_USER_ID` — the bot's user ID, used to detect self-messages
//! - `SELF_MESSAGE_BYPASS` — "true" (default) to enable bypass

use thinclaw_channels_core::IncomingMessage;

/// Configuration for self-message detection.
#[derive(Debug, Clone)]
pub struct SelfMessageConfig {
    /// The bot's user IDs (may have multiple across channels).
    pub bot_user_ids: Vec<String>,
    /// Whether self-message bypass is enabled.
    pub enabled: bool,
}

impl Default for SelfMessageConfig {
    fn default() -> Self {
        Self {
            bot_user_ids: Vec::new(),
            enabled: true,
        }
    }
}

impl SelfMessageConfig {
    /// Create from environment variables.
    pub fn from_env() -> Self {
        let mut config = Self::default();

        // Single bot ID
        if let Ok(id) = std::env::var("BOT_USER_ID") {
            config.bot_user_ids.push(id);
        }

        // Multiple bot IDs (comma-separated)
        if let Ok(ids) = std::env::var("BOT_USER_IDS") {
            for id in ids.split(',') {
                let trimmed = id.trim().to_string();
                if !trimmed.is_empty() && !config.bot_user_ids.contains(&trimmed) {
                    config.bot_user_ids.push(trimmed);
                }
            }
        }

        if let Ok(val) = std::env::var("SELF_MESSAGE_BYPASS") {
            config.enabled = val != "0" && !val.eq_ignore_ascii_case("false");
        }

        config
    }

    /// Register a bot user ID (for channels that report the bot's ID at connect time).
    pub fn register_bot_id(&mut self, id: impl Into<String>) {
        let id = id.into();
        if !self.bot_user_ids.contains(&id) {
            self.bot_user_ids.push(id);
        }
    }

    /// Check if a message is from the bot itself.
    pub fn is_self_message(&self, msg: &IncomingMessage) -> bool {
        if !self.enabled {
            return false;
        }
        self.bot_user_ids
            .iter()
            .any(|bot_id| bot_id == &msg.user_id)
    }

    /// Filter a batch of messages, removing self-messages.
    pub fn filter_messages(&self, messages: Vec<IncomingMessage>) -> Vec<IncomingMessage> {
        if !self.enabled || self.bot_user_ids.is_empty() {
            return messages;
        }
        messages
            .into_iter()
            .filter(|msg| !self.is_self_message(msg))
            .collect()
    }
}

/// Trusted metadata that is injected into the system context for hooks and tools.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TrustedMetadata {
    /// Sender user ID (platform-specific).
    pub sender_id: String,
    /// Channel name/type.
    pub channel: String,
    /// Thread/conversation ID.
    pub thread_id: Option<String>,
    /// Platform-specific user display name.
    pub sender_name: Option<String>,
    /// Whether the sender is the bot itself.
    pub is_self: bool,
    /// Whether the message is from a group chat.
    pub is_group: bool,
    /// Timestamp of the message.
    pub timestamp: String,
}

impl TrustedMetadata {
    /// Build trusted metadata from an incoming message.
    pub fn from_message(msg: &IncomingMessage, self_config: &SelfMessageConfig) -> Self {
        let is_group = msg
            .metadata
            .get("is_group")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        Self {
            sender_id: msg.user_id.clone(),
            channel: msg.channel.clone(),
            thread_id: msg.thread_id.clone(),
            sender_name: msg.user_name.clone(),
            is_self: self_config.is_self_message(msg),
            is_group,
            timestamp: msg.received_at.to_rfc3339(),
        }
    }

    /// Serialize to JSON for injection into system context.
    pub fn to_system_context(&self) -> String {
        format!(
            "[Trusted Metadata] sender_id={} channel={} thread={} is_group={} timestamp={}",
            self.sender_id,
            self.channel,
            self.thread_id.as_deref().unwrap_or("none"),
            self.is_group,
            self.timestamp,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_msg(user_id: &str, channel: &str) -> IncomingMessage {
        IncomingMessage::new(channel, user_id, "hello")
    }

    #[test]
    fn test_self_message_detected() {
        let mut config = SelfMessageConfig::default();
        config.register_bot_id("bot-123");

        let msg = make_msg("bot-123", "telegram");
        assert!(config.is_self_message(&msg));
    }

    #[test]
    fn test_non_self_message_passes() {
        let mut config = SelfMessageConfig::default();
        config.register_bot_id("bot-123");

        let msg = make_msg("user-456", "telegram");
        assert!(!config.is_self_message(&msg));
    }

    #[test]
    fn test_bypass_disabled() {
        let config = SelfMessageConfig {
            bot_user_ids: vec!["bot-123".to_string()],
            enabled: false,
        };

        let msg = make_msg("bot-123", "telegram");
        assert!(!config.is_self_message(&msg));
    }

    #[test]
    fn test_filter_messages() {
        let mut config = SelfMessageConfig::default();
        config.register_bot_id("bot-123");

        let messages = vec![
            make_msg("user-1", "tg"),
            make_msg("bot-123", "tg"),
            make_msg("user-2", "tg"),
        ];

        let filtered = config.filter_messages(messages);
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].user_id, "user-1");
        assert_eq!(filtered[1].user_id, "user-2");
    }

    #[test]
    fn test_multiple_bot_ids() {
        let config = SelfMessageConfig {
            bot_user_ids: vec!["bot-tg".to_string(), "bot-discord".to_string()],
            enabled: true,
        };

        assert!(config.is_self_message(&make_msg("bot-tg", "telegram")));
        assert!(config.is_self_message(&make_msg("bot-discord", "discord")));
        assert!(!config.is_self_message(&make_msg("user-1", "telegram")));
    }

    #[test]
    fn test_trusted_metadata_from_message() {
        let config = SelfMessageConfig {
            bot_user_ids: vec!["bot-123".to_string()],
            enabled: true,
        };

        let msg = make_msg("user-456", "telegram");
        let meta = TrustedMetadata::from_message(&msg, &config);

        assert_eq!(meta.sender_id, "user-456");
        assert_eq!(meta.channel, "telegram");
        assert!(!meta.is_self);
        assert!(!meta.is_group);
    }

    #[test]
    fn test_trusted_metadata_self_message() {
        let config = SelfMessageConfig {
            bot_user_ids: vec!["bot-123".to_string()],
            enabled: true,
        };

        let msg = make_msg("bot-123", "telegram");
        let meta = TrustedMetadata::from_message(&msg, &config);
        assert!(meta.is_self);
    }

    #[test]
    fn test_system_context_format() {
        let meta = TrustedMetadata {
            sender_id: "user-1".to_string(),
            channel: "telegram".to_string(),
            thread_id: Some("thread-42".to_string()),
            sender_name: Some("Alice".to_string()),
            is_self: false,
            is_group: true,
            timestamp: "2026-03-04T00:00:00Z".to_string(),
        };

        let ctx = meta.to_system_context();
        assert!(ctx.contains("sender_id=user-1"));
        assert!(ctx.contains("is_group=true"));
    }
}
