//! iMessage channel wiring — connects IMessageChannel to the main startup.
//!
//! The IMessageChannel implementation exists but wasn't wired into the
//! main channel startup flow. This module provides the wiring layer.

use serde::{Deserialize, Serialize};

/// iMessage channel configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IMessageConfig {
    /// Whether iMessage integration is enabled.
    pub enabled: bool,
    /// Path to the chat.db database.
    pub chat_db_path: String,
    /// Poll interval in seconds.
    pub poll_interval_secs: u64,
    /// Whether to auto-reply to unknown contacts.
    pub auto_reply: bool,
    /// Contacts to monitor (empty = all).
    pub monitored_contacts: Vec<String>,
    /// Maximum message age to process (seconds).
    pub max_message_age_secs: u64,
}

impl Default for IMessageConfig {
    fn default() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/Users".to_string());
        Self {
            enabled: std::env::var("IMESSAGE_ENABLED")
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(false),
            chat_db_path: format!("{}/Library/Messages/chat.db", home),
            poll_interval_secs: 5,
            auto_reply: false,
            monitored_contacts: Vec::new(),
            max_message_age_secs: 300, // 5 minutes
        }
    }
}

impl IMessageConfig {
    /// Create from environment variables.
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(path) = std::env::var("IMESSAGE_CHAT_DB") {
            config.chat_db_path = path;
        }
        if let Ok(interval) = std::env::var("IMESSAGE_POLL_INTERVAL") {
            if let Ok(secs) = interval.parse() {
                config.poll_interval_secs = secs;
            }
        }
        if let Ok(contacts) = std::env::var("IMESSAGE_CONTACTS") {
            config.monitored_contacts = contacts.split(',').map(|s| s.trim().to_string()).collect();
        }
        if let Ok(max_age) = std::env::var("IMESSAGE_MAX_AGE") {
            if let Ok(secs) = max_age.parse() {
                config.max_message_age_secs = secs;
            }
        }

        config
    }

    /// Validate the configuration.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();

        if !cfg!(target_os = "macos") {
            errors.push("iMessage integration is only available on macOS.".to_string());
        }

        if self.poll_interval_secs == 0 {
            errors.push("Poll interval must be > 0.".to_string());
        }

        if !std::path::Path::new(&self.chat_db_path).exists() {
            errors.push(format!("chat.db not found at: {}", self.chat_db_path));
        }

        errors
    }

    /// Whether this config is ready to use.
    pub fn is_ready(&self) -> bool {
        self.enabled && self.validate().is_empty()
    }
}

/// iMessage channel startup descriptor.
#[derive(Debug)]
pub struct IMessageStartupInfo {
    pub config: IMessageConfig,
    pub status: IMessageStatus,
}

/// Current status of the iMessage channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum IMessageStatus {
    /// Not configured.
    Disabled,
    /// Configuration found but not validated.
    Configured,
    /// Ready to start.
    Ready,
    /// Running and polling.
    Running,
    /// Error state.
    Error(String),
}

impl IMessageStartupInfo {
    /// Create startup info from config.
    pub fn from_config(config: IMessageConfig) -> Self {
        let status = if !config.enabled {
            IMessageStatus::Disabled
        } else {
            let errors = config.validate();
            if errors.is_empty() {
                IMessageStatus::Ready
            } else {
                IMessageStatus::Error(errors.join("; "))
            }
        };

        Self { config, status }
    }

    /// Whether the channel should be started.
    pub fn should_start(&self) -> bool {
        self.status == IMessageStatus::Ready
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = IMessageConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.poll_interval_secs, 5);
        assert!(config.chat_db_path.contains("chat.db"));
    }

    #[test]
    fn test_disabled_not_ready() {
        let config = IMessageConfig::default();
        assert!(!config.is_ready());
    }

    #[test]
    fn test_validate_zero_interval() {
        let config = IMessageConfig {
            poll_interval_secs: 0,
            ..Default::default()
        };
        let errors = config.validate();
        assert!(errors.iter().any(|e| e.contains("interval")));
    }

    #[test]
    fn test_startup_disabled() {
        let config = IMessageConfig::default();
        let info = IMessageStartupInfo::from_config(config);
        assert_eq!(info.status, IMessageStatus::Disabled);
        assert!(!info.should_start());
    }

    #[test]
    fn test_startup_enabled_missing_db() {
        let config = IMessageConfig {
            enabled: true,
            chat_db_path: "/nonexistent/chat.db".to_string(),
            ..Default::default()
        };
        let info = IMessageStartupInfo::from_config(config);
        assert!(matches!(info.status, IMessageStatus::Error(_)));
    }

    #[test]
    fn test_status_equality() {
        assert_eq!(IMessageStatus::Disabled, IMessageStatus::Disabled);
        assert_ne!(IMessageStatus::Ready, IMessageStatus::Disabled);
    }

    #[test]
    fn test_monitored_contacts() {
        let config = IMessageConfig {
            monitored_contacts: vec!["Alice".to_string(), "Bob".to_string()],
            ..Default::default()
        };
        assert_eq!(config.monitored_contacts.len(), 2);
    }
}
