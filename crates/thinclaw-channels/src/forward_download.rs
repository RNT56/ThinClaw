//! Forwarded attachment download support.
//!
//! When a user forwards a message to the bot, attached media may need
//! to be re-downloaded from the platform's CDN. This module handles
//! detecting forwarded messages and fetching their attachments.
//!
//! Configuration:
//! - `FORWARD_DOWNLOAD_ENABLED` — enable forwarded attachment downloads (default: true)
//! - `FORWARD_DOWNLOAD_MAX_MB` — max file size for forwarded downloads (default: 25)

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Configuration for forwarded attachment handling.
#[derive(Debug, Clone)]
pub struct ForwardDownloadConfig {
    /// Whether to download attachments from forwarded messages.
    pub enabled: bool,
    /// Maximum file size in bytes.
    pub max_bytes: u64,
    /// Download timeout.
    pub timeout: Duration,
}

impl Default for ForwardDownloadConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_bytes: 25 * 1024 * 1024, // 25 MB
            timeout: Duration::from_secs(30),
        }
    }
}

impl ForwardDownloadConfig {
    /// Create from environment variables.
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(val) = std::env::var("FORWARD_DOWNLOAD_ENABLED") {
            config.enabled = val != "0" && !val.eq_ignore_ascii_case("false");
        }

        if let Ok(mb) = std::env::var("FORWARD_DOWNLOAD_MAX_MB")
            && let Ok(m) = mb.parse::<u64>()
        {
            config.max_bytes = m * 1024 * 1024;
        }

        config
    }
}

/// Information about a forwarded attachment that needs downloading.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForwardedAttachment {
    /// Platform-specific file ID.
    pub file_id: String,
    /// Original filename if known.
    pub filename: Option<String>,
    /// MIME type if known.
    pub mime_type: Option<String>,
    /// File size in bytes if known.
    pub file_size: Option<u64>,
    /// URL to download from (platform CDN).
    pub download_url: Option<String>,
}

impl ForwardedAttachment {
    /// Check if the attachment exceeds the size limit.
    pub fn exceeds_limit(&self, config: &ForwardDownloadConfig) -> bool {
        self.file_size
            .map(|size| size > config.max_bytes)
            .unwrap_or(false)
    }
}

/// Detect forwarded message metadata from incoming message.
///
/// Checks platform-specific metadata fields:
/// - Telegram: `forward_from`, `forward_from_chat`
/// - Discord: `message_reference`
/// - Signal: `quote`
pub fn is_forwarded(metadata: &serde_json::Value) -> bool {
    // Telegram
    if metadata.get("forward_from").is_some() || metadata.get("forward_from_chat").is_some() {
        return true;
    }

    // Discord — message_reference with type "FORWARD" or "DEFAULT" (reply)
    if let Some(ref_data) = metadata.get("message_reference")
        && ref_data.get("message_id").is_some()
    {
        return true;
    }

    // Signal — quote field indicates forwarded/quoted message
    if metadata.get("quote").is_some() {
        return true;
    }

    false
}

/// Extract forwarded attachment info from message metadata.
pub fn extract_forwarded_attachments(metadata: &serde_json::Value) -> Vec<ForwardedAttachment> {
    let mut attachments = Vec::new();

    // Telegram — forward_from + photo/document/audio/video arrays
    if is_forwarded(metadata) {
        if let Some(photos) = metadata.get("photo").and_then(|v| v.as_array()) {
            // Telegram sends multiple sizes; take the largest
            if let Some(largest) = photos.last() {
                attachments.push(ForwardedAttachment {
                    file_id: largest
                        .get("file_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    filename: None,
                    mime_type: Some("image/jpeg".to_string()),
                    file_size: largest.get("file_size").and_then(|v| v.as_u64()),
                    download_url: None,
                });
            }
        }

        if let Some(doc) = metadata.get("document") {
            attachments.push(ForwardedAttachment {
                file_id: doc
                    .get("file_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                filename: doc
                    .get("file_name")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                mime_type: doc
                    .get("mime_type")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                file_size: doc.get("file_size").and_then(|v| v.as_u64()),
                download_url: None,
            });
        }
    }

    attachments
}

/// Download a forwarded attachment using the provided HTTP client.
pub async fn download_forwarded(
    client: &reqwest::Client,
    attachment: &ForwardedAttachment,
    config: &ForwardDownloadConfig,
) -> Result<Vec<u8>, ForwardDownloadError> {
    if !config.enabled {
        return Err(ForwardDownloadError::Disabled);
    }

    if attachment.exceeds_limit(config) {
        return Err(ForwardDownloadError::TooLarge {
            size: attachment.file_size.unwrap_or(0),
            limit: config.max_bytes,
        });
    }

    let url = attachment
        .download_url
        .as_deref()
        .ok_or(ForwardDownloadError::NoUrl)?;

    let response = tokio::time::timeout(config.timeout, client.get(url).send())
        .await
        .map_err(|_| ForwardDownloadError::Timeout)?
        .map_err(|e| ForwardDownloadError::Network(e.to_string()))?;

    if !response.status().is_success() {
        return Err(ForwardDownloadError::HttpError(response.status().as_u16()));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| ForwardDownloadError::Network(e.to_string()))?;

    Ok(bytes.to_vec())
}

/// Errors during forwarded attachment download.
#[derive(Debug, Clone)]
pub enum ForwardDownloadError {
    Disabled,
    TooLarge { size: u64, limit: u64 },
    NoUrl,
    Timeout,
    Network(String),
    HttpError(u16),
}

impl std::fmt::Display for ForwardDownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => write!(f, "Forward downloads disabled"),
            Self::TooLarge { size, limit } => {
                write!(
                    f,
                    "Attachment too large: {} bytes (limit: {} bytes)",
                    size, limit
                )
            }
            Self::NoUrl => write!(f, "No download URL available"),
            Self::Timeout => write!(f, "Download timed out"),
            Self::Network(e) => write!(f, "Network error: {}", e),
            Self::HttpError(code) => write!(f, "HTTP error: {}", code),
        }
    }
}

impl std::error::Error for ForwardDownloadError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ForwardDownloadConfig::default();
        assert!(config.enabled);
        assert_eq!(config.max_bytes, 25 * 1024 * 1024);
    }

    #[test]
    fn test_is_forwarded_telegram() {
        let meta = serde_json::json!({
            "forward_from": {"id": 12345}
        });
        assert!(is_forwarded(&meta));
    }

    #[test]
    fn test_is_forwarded_discord() {
        let meta = serde_json::json!({
            "message_reference": {"message_id": "123456"}
        });
        assert!(is_forwarded(&meta));
    }

    #[test]
    fn test_not_forwarded() {
        let meta = serde_json::json!({"text": "hello"});
        assert!(!is_forwarded(&meta));
    }

    #[test]
    fn test_exceeds_limit() {
        let config = ForwardDownloadConfig {
            max_bytes: 1024,
            ..Default::default()
        };

        let small = ForwardedAttachment {
            file_id: "f1".to_string(),
            filename: None,
            mime_type: None,
            file_size: Some(512),
            download_url: None,
        };
        assert!(!small.exceeds_limit(&config));

        let large = ForwardedAttachment {
            file_id: "f2".to_string(),
            filename: None,
            mime_type: None,
            file_size: Some(2048),
            download_url: None,
        };
        assert!(large.exceeds_limit(&config));
    }

    #[test]
    fn test_extract_forwarded_photo() {
        let meta = serde_json::json!({
            "forward_from": {"id": 1},
            "photo": [
                {"file_id": "small", "file_size": 100},
                {"file_id": "large", "file_size": 5000}
            ]
        });
        let attachments = extract_forwarded_attachments(&meta);
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].file_id, "large");
    }

    #[test]
    fn test_extract_forwarded_document() {
        let meta = serde_json::json!({
            "forward_from_chat": {"id": -100},
            "document": {
                "file_id": "doc-1",
                "file_name": "report.pdf",
                "mime_type": "application/pdf",
                "file_size": 12345
            }
        });
        let attachments = extract_forwarded_attachments(&meta);
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].filename, Some("report.pdf".to_string()));
    }

    #[test]
    fn test_error_display() {
        let err = ForwardDownloadError::TooLarge {
            size: 100,
            limit: 50,
        };
        let msg = format!("{}", err);
        assert!(msg.contains("too large"));
    }
}
