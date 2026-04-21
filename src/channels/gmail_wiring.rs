//! Gmail pub/sub wiring.
//!
//! Configuration, payload parsing, and sender filtering for Gmail
//! push notifications via Google Pub/Sub.

use serde::{Deserialize, Serialize};

/// Gmail channel configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GmailConfig {
    pub enabled: bool,
    pub project_id: String,
    pub subscription_id: String,
    pub topic_id: String,
    pub webhook_path: String,
    pub oauth_token: Option<String>,
    pub label_filters: Vec<String>,
    pub allowed_senders: Vec<String>,
    pub max_message_size_bytes: usize,
}

impl Default for GmailConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            project_id: String::new(),
            subscription_id: String::new(),
            topic_id: String::new(),
            webhook_path: "/webhooks/gmail".into(),
            oauth_token: None,
            label_filters: vec!["INBOX".into(), "UNREAD".into()],
            allowed_senders: Vec::new(),
            max_message_size_bytes: 10 * 1024 * 1024,
        }
    }
}

impl GmailConfig {
    /// Create from environment variables.
    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Ok(p) = std::env::var("GMAIL_PROJECT_ID") {
            config.project_id = p;
        }
        if let Ok(s) = std::env::var("GMAIL_SUBSCRIPTION_ID") {
            config.subscription_id = s;
        }
        if let Ok(t) = std::env::var("GMAIL_TOPIC_ID") {
            config.topic_id = t;
        }
        if let Ok(path) = std::env::var("GMAIL_WEBHOOK_PATH") {
            config.webhook_path = path;
        }
        if let Ok(senders) = std::env::var("GMAIL_ALLOWED_SENDERS") {
            config.allowed_senders = senders.split(',').map(|s| s.trim().to_string()).collect();
        }
        if std::env::var("GMAIL_ENABLED").is_ok() {
            config.enabled = true;
        }
        config
    }

    /// Check if Gmail is fully configured.
    pub fn is_configured(&self) -> bool {
        self.enabled && !self.project_id.is_empty() && !self.subscription_id.is_empty()
    }

    /// Validate config and return list of missing fields.
    pub fn validate(&self) -> Vec<String> {
        let mut missing = Vec::new();
        if self.project_id.is_empty() {
            missing.push("project_id".into());
        }
        if self.subscription_id.is_empty() {
            missing.push("subscription_id".into());
        }
        if self.topic_id.is_empty() {
            missing.push("topic_id".into());
        }
        missing
    }

    /// Get current status.
    pub fn status(&self) -> GmailStatus {
        if !self.enabled {
            return GmailStatus::Disabled;
        }
        let missing = self.validate();
        if !missing.is_empty() {
            return GmailStatus::MissingCredentials { fields: missing };
        }
        GmailStatus::Ready {
            subscription: self.subscription_id.clone(),
        }
    }
}

/// Gmail integration status.
#[derive(Debug, Clone, PartialEq)]
pub enum GmailStatus {
    Disabled,
    Ready { subscription: String },
    MissingCredentials { fields: Vec<String> },
    Error(String),
}

/// Parsed Gmail push payload.
#[derive(Debug, Clone)]
pub struct GmailPushPayload {
    pub message_id: String,
    pub history_id: Option<String>,
    pub subscription: String,
}

/// Startup diagnostic info.
pub struct GmailStartupInfo {
    pub status: GmailStatus,
    pub webhook_url: String,
    pub subscription_id: String,
    pub label_filter_count: usize,
}

impl GmailStartupInfo {
    pub fn from_config(config: &GmailConfig, base_url: &str) -> Self {
        Self {
            status: config.status(),
            webhook_url: format!("{}{}", base_url, config.webhook_path),
            subscription_id: config.subscription_id.clone(),
            label_filter_count: config.label_filters.len(),
        }
    }
}

/// Parse a Pub/Sub push notification body.
pub fn parse_pubsub_push(body: &[u8]) -> Result<GmailPushPayload, GmailError> {
    let text = std::str::from_utf8(body)
        .map_err(|_| GmailError::InvalidPayload("not valid UTF-8".into()))?;

    // Simple JSON parsing: look for message_id, historyId, subscription
    let message_id = extract_json_string(text, "message_id")
        .or_else(|| extract_json_string(text, "messageId"))
        .ok_or_else(|| GmailError::InvalidPayload("missing message_id".into()))?;

    let history_id = extract_json_string(text, "historyId");
    let subscription =
        extract_json_string(text, "subscription").unwrap_or_else(|| "unknown".into());

    Ok(GmailPushPayload {
        message_id,
        history_id,
        subscription,
    })
}

/// Check if a sender is allowed.
pub fn is_sender_allowed(sender: &str, allowed: &[String]) -> bool {
    if allowed.is_empty() {
        return true; // Empty = allow all
    }

    let sender_norm = canonical_sender(sender);
    allowed.iter().any(|entry| {
        let entry = entry.trim();
        if entry.is_empty() {
            return false;
        }
        if entry == "*" {
            return true;
        }
        let entry_norm = canonical_sender(entry);
        if entry_norm.starts_with('@') {
            // Allow domain-level rules like "@example.com".
            return sender_norm.ends_with(&entry_norm);
        }
        sender_norm == entry_norm
    })
}

fn canonical_sender(value: &str) -> String {
    let trimmed = value.trim();
    let core = if let (Some(start), Some(end)) = (trimmed.rfind('<'), trimmed.rfind('>')) {
        if start < end {
            &trimmed[start + 1..end]
        } else {
            trimmed
        }
    } else {
        trimmed
    };
    core.trim().to_lowercase()
}

/// Simple JSON string extractor (avoids serde_json dependency for wiring layer).
fn extract_json_string(json: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{}\"", key);
    let pos = json.find(&pattern)?;
    let after_key = &json[pos + pattern.len()..];
    // Skip :, whitespace, and opening "
    let value_start = after_key.find('"')? + 1;
    let rest = &after_key[value_start..];
    let value_end = rest.find('"')?;
    Some(rest[..value_end].to_string())
}

/// Gmail errors.
#[derive(Debug, thiserror::Error)]
pub enum GmailError {
    #[error("Invalid push payload: {0}")]
    InvalidPayload(String),
    #[error("Missing credentials")]
    MissingCredentials,
    #[error("Unauthorized sender: {0}")]
    UnauthorizedSender(String),
    #[error("Message too large: {0} bytes")]
    TooLarge(usize),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = GmailConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.webhook_path, "/webhooks/gmail");
    }

    #[test]
    fn test_is_configured_false_when_missing() {
        let config = GmailConfig::default();
        assert!(!config.is_configured());
    }

    #[test]
    fn test_validate_config_errors() {
        let config = GmailConfig::default();
        let errors = config.validate();
        assert!(errors.contains(&"project_id".to_string()));
        assert!(errors.contains(&"subscription_id".to_string()));
    }

    #[test]
    fn test_parse_valid_push_payload() {
        let body = br#"{"message":{"message_id":"abc123","data":"eA=="},"subscription":"projects/p/subscriptions/s","historyId":"99"}"#;
        let result = parse_pubsub_push(body).unwrap();
        assert_eq!(result.message_id, "abc123");
        assert_eq!(result.history_id, Some("99".into()));
    }

    #[test]
    fn test_parse_invalid_payload() {
        let body = b"not json at all";
        assert!(parse_pubsub_push(body).is_err());
    }

    #[test]
    fn test_status_ready() {
        let config = GmailConfig {
            enabled: true,
            project_id: "proj".into(),
            subscription_id: "sub".into(),
            topic_id: "topic".into(),
            ..Default::default()
        };
        assert!(matches!(config.status(), GmailStatus::Ready { .. }));
    }

    #[test]
    fn test_status_missing_creds() {
        let config = GmailConfig {
            enabled: true,
            ..Default::default()
        };
        assert!(matches!(
            config.status(),
            GmailStatus::MissingCredentials { .. }
        ));
    }

    #[test]
    fn test_is_sender_allowed() {
        assert!(is_sender_allowed("anyone@example.com", &[]));
        assert!(is_sender_allowed(
            "boss@example.com",
            &["boss@example.com".into()]
        ));
        assert!(!is_sender_allowed(
            "stranger@evil.com",
            &["boss@example.com".into()]
        ));
    }

    #[test]
    fn test_startup_info() {
        let config = GmailConfig {
            enabled: true,
            project_id: "p".into(),
            subscription_id: "s".into(),
            topic_id: "t".into(),
            ..Default::default()
        };
        let info = GmailStartupInfo::from_config(&config, "http://localhost:8080");
        assert_eq!(info.webhook_url, "http://localhost:8080/webhooks/gmail");
        assert_eq!(info.label_filter_count, 2);
    }
}
