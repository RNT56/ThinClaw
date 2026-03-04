//! Per-channel media size limits.
//!
//! Configurable maximum attachment sizes per channel. When a media file
//! exceeds the limit, it is either rejected or auto-downscaled (for images).
//!
//! Configuration via env vars:
//! - `MEDIA_MAX_SIZE_MB` — global default (default: 25 MB)
//! - `TELEGRAM_MAX_MEDIA_MB` — override for Telegram
//! - `DISCORD_MAX_MEDIA_MB` — override for Discord
//! - `SLACK_MAX_MEDIA_MB` — override for Slack
//! etc.

use std::collections::HashMap;

/// Per-channel media size limits (in bytes).
#[derive(Debug, Clone)]
pub struct MediaLimits {
    /// Global default limit in bytes.
    pub default_max_bytes: u64,
    /// Per-channel overrides (channel name → max bytes).
    pub channel_overrides: HashMap<String, u64>,
}

impl Default for MediaLimits {
    fn default() -> Self {
        Self {
            default_max_bytes: 25 * 1024 * 1024, // 25 MB
            channel_overrides: HashMap::new(),
        }
    }
}

impl MediaLimits {
    /// Create from environment variables.
    ///
    /// Reads `MEDIA_MAX_SIZE_MB` for global default, then
    /// `{CHANNEL}_MAX_MEDIA_MB` for per-channel overrides.
    pub fn from_env() -> Self {
        let default_mb: u64 = std::env::var("MEDIA_MAX_SIZE_MB")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(25);

        let mut overrides = HashMap::new();

        let channels = [
            "telegram", "discord", "slack", "signal", "imessage", "http", "nostr", "matrix",
            "webchat",
        ];

        for channel in &channels {
            let env_key = format!("{}_MAX_MEDIA_MB", channel.to_uppercase());
            if let Ok(val) = std::env::var(&env_key) {
                if let Ok(mb) = val.parse::<u64>() {
                    overrides.insert(channel.to_string(), mb * 1024 * 1024);
                }
            }
        }

        Self {
            default_max_bytes: default_mb * 1024 * 1024,
            channel_overrides: overrides,
        }
    }

    /// Get the effective limit for a channel.
    pub fn limit_for(&self, channel: &str) -> u64 {
        self.channel_overrides
            .get(channel)
            .copied()
            .unwrap_or(self.default_max_bytes)
    }

    /// Set an override for a specific channel.
    pub fn set_limit(&mut self, channel: impl Into<String>, max_bytes: u64) {
        self.channel_overrides.insert(channel.into(), max_bytes);
    }

    /// Check if a file size exceeds the limit for a channel.
    pub fn exceeds_limit(&self, channel: &str, file_size: u64) -> bool {
        file_size > self.limit_for(channel)
    }

    /// Filter attachments, returning only those within the size limit.
    /// Returns `(accepted, rejected_count)`.
    pub fn filter_attachments<T, F>(
        &self,
        channel: &str,
        items: Vec<T>,
        size_fn: F,
    ) -> (Vec<T>, usize)
    where
        F: Fn(&T) -> u64,
    {
        let limit = self.limit_for(channel);
        let total = items.len();
        let accepted: Vec<T> = items
            .into_iter()
            .filter(|item| size_fn(item) <= limit)
            .collect();
        let rejected = total - accepted.len();
        (accepted, rejected)
    }
}

/// Serialize-friendly representation for config display.
impl MediaLimits {
    pub fn to_config_map(&self) -> HashMap<String, String> {
        let mut map = HashMap::new();
        map.insert(
            "default".to_string(),
            format!("{} MB", self.default_max_bytes / (1024 * 1024)),
        );
        for (ch, bytes) in &self.channel_overrides {
            map.insert(ch.clone(), format!("{} MB", bytes / (1024 * 1024)));
        }
        map
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_limits() {
        let limits = MediaLimits::default();
        assert_eq!(limits.default_max_bytes, 25 * 1024 * 1024);
        assert_eq!(limits.limit_for("telegram"), 25 * 1024 * 1024);
    }

    #[test]
    fn test_channel_override() {
        let mut limits = MediaLimits::default();
        limits.set_limit("telegram", 50 * 1024 * 1024);
        assert_eq!(limits.limit_for("telegram"), 50 * 1024 * 1024);
        assert_eq!(limits.limit_for("discord"), 25 * 1024 * 1024); // default
    }

    #[test]
    fn test_exceeds_limit() {
        let limits = MediaLimits::default();
        assert!(!limits.exceeds_limit("telegram", 10 * 1024 * 1024));
        assert!(limits.exceeds_limit("telegram", 30 * 1024 * 1024));
    }

    #[test]
    fn test_filter_attachments() {
        let mut limits = MediaLimits::default();
        limits.set_limit("test", 100);

        let items: Vec<(String, u64)> = vec![
            ("small.jpg".into(), 50),
            ("big.mp4".into(), 200),
            ("medium.pdf".into(), 100),
        ];

        let (accepted, rejected) = limits.filter_attachments("test", items, |item| item.1);
        assert_eq!(accepted.len(), 2);
        assert_eq!(rejected, 1);
    }

    #[test]
    fn test_config_map() {
        let mut limits = MediaLimits::default();
        limits.set_limit("telegram", 50 * 1024 * 1024);
        let map = limits.to_config_map();
        assert_eq!(map["default"], "25 MB");
        assert_eq!(map["telegram"], "50 MB");
    }
}
