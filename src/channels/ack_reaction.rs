//! Per-channel acknowledgement reaction configuration.
//!
//! Different channels may use different emoji to acknowledge receipt
//! of a message (e.g., 👍 on Telegram, ✅ on Discord, 👀 on Slack).
//!
//! Configuration via env vars:
//! - `ACK_REACTION` — default acknowledgement reaction (default: "👍")
//! - `ACK_REACTION_TELEGRAM` — Telegram-specific reaction
//! - `ACK_REACTION_DISCORD` — Discord-specific reaction
//! - `ACK_REACTION_SIGNAL` — Signal-specific reaction
//! - `ACK_REACTION_SLACK` — Slack-specific reaction
//! - `ACK_REACTION_ENABLED` — whether to send ack reactions (default: true)

use std::collections::HashMap;

/// Configuration for per-channel acknowledgement reactions.
#[derive(Debug, Clone)]
pub struct AckReactionConfig {
    /// Default reaction emoji.
    pub default_reaction: String,
    /// Per-channel overrides.
    pub channel_reactions: HashMap<String, String>,
    /// Whether ack reactions are enabled.
    pub enabled: bool,
    /// Reaction to add when processing is complete.
    pub done_reaction: Option<String>,
}

impl Default for AckReactionConfig {
    fn default() -> Self {
        Self {
            default_reaction: "👍".to_string(),
            channel_reactions: HashMap::new(),
            enabled: true,
            done_reaction: None,
        }
    }
}

impl AckReactionConfig {
    /// Create from environment variables.
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(r) = std::env::var("ACK_REACTION") {
            config.default_reaction = r;
        }

        if let Ok(val) = std::env::var("ACK_REACTION_ENABLED") {
            config.enabled = val != "0" && !val.eq_ignore_ascii_case("false");
        }

        if let Ok(r) = std::env::var("ACK_REACTION_DONE") {
            config.done_reaction = Some(r);
        }

        // Per-channel overrides
        let channels = [
            ("ACK_REACTION_TELEGRAM", "telegram"),
            ("ACK_REACTION_DISCORD", "discord"),
            ("ACK_REACTION_SIGNAL", "signal"),
            ("ACK_REACTION_SLACK", "slack"),
            ("ACK_REACTION_IMESSAGE", "imessage"),
            ("ACK_REACTION_WEB", "web"),
        ];

        for (env_var, channel) in &channels {
            if let Ok(r) = std::env::var(env_var) {
                config.channel_reactions.insert(channel.to_string(), r);
            }
        }

        config
    }

    /// Get the ack reaction for a specific channel.
    pub fn reaction_for(&self, channel: &str) -> &str {
        self.channel_reactions
            .get(channel)
            .map(|s| s.as_str())
            .unwrap_or(&self.default_reaction)
    }

    /// Get the done reaction (if configured).
    pub fn done_reaction_for(&self, _channel: &str) -> Option<&str> {
        self.done_reaction.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_reaction() {
        let config = AckReactionConfig::default();
        assert_eq!(config.reaction_for("telegram"), "👍");
        assert_eq!(config.reaction_for("discord"), "👍");
    }

    #[test]
    fn test_channel_override() {
        let mut config = AckReactionConfig::default();
        config
            .channel_reactions
            .insert("discord".to_string(), "✅".to_string());

        assert_eq!(config.reaction_for("discord"), "✅");
        assert_eq!(config.reaction_for("telegram"), "👍"); // still default
    }

    #[test]
    fn test_done_reaction() {
        let config = AckReactionConfig {
            done_reaction: Some("✅".to_string()),
            ..Default::default()
        };
        assert_eq!(config.done_reaction_for("telegram"), Some("✅"));
    }

    #[test]
    fn test_disabled() {
        let config = AckReactionConfig {
            enabled: false,
            ..Default::default()
        };
        assert!(!config.enabled);
    }
}
