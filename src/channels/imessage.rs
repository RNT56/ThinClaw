//! iMessage channel (macOS only).
//!
//! Reads incoming messages by polling Apple's `chat.db` SQLite database
//! via the system `sqlite3` CLI and sends replies via `osascript`
//! (AppleScript). This approach requires:
//! - macOS with Full Disk Access granted to the terminal/app
//! - Messages.app running (for sending)
//!
//! We use the `sqlite3` CLI instead of `rusqlite` to avoid SQLite
//! linkage conflicts with libsql/sqlx that already exist in the project.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use super::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use crate::error::ChannelError;

/// Channel name constant.
const NAME: &str = "imessage";

/// Default polling interval in seconds.
const POLL_INTERVAL_SECS: u64 = 3;

/// Maximum message length for a single iMessage bubble.
const MAX_MESSAGE_LENGTH: usize = 20_000;

// ── Configuration ───────────────────────────────────────────────────

/// iMessage channel configuration.
#[derive(Debug, Clone)]
pub struct IMessageConfig {
    /// Path to chat.db (default: ~/Library/Messages/chat.db).
    pub db_path: PathBuf,
    /// Allowed phone numbers / email addresses (empty = allow all).
    pub allow_from: Vec<String>,
    /// Polling interval in seconds.
    pub poll_interval_secs: u64,
}

impl Default for IMessageConfig {
    fn default() -> Self {
        Self {
            db_path: default_chat_db_path(),
            allow_from: Vec::new(),
            poll_interval_secs: POLL_INTERVAL_SECS,
        }
    }
}

/// Get the default chat.db path.
fn default_chat_db_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/Users/Shared"))
        .join("Library/Messages/chat.db")
}

// ── Raw message from chat.db ────────────────────────────────────────

/// A raw message parsed from sqlite3 output.
#[derive(Debug, Clone)]
struct ChatDbMessage {
    /// ROWID from the message table.
    rowid: i64,
    /// Message text.
    text: String,
    /// Sender identifier (phone number or email).
    sender: String,
    /// Chat identifier (e.g., "iMessage;-;+1234567890").
    chat_id: String,
    /// Whether the message is from me (outgoing).
    is_from_me: bool,
}

// ── Channel implementation ──────────────────────────────────────────

/// iMessage channel using chat.db polling + osascript sending.
pub struct IMessageChannel {
    config: IMessageConfig,
    /// Shutdown flag.
    shutdown: Arc<AtomicBool>,
    /// Last processed message ROWID.
    last_rowid: Arc<AtomicI64>,
}

impl IMessageChannel {
    /// Create a new iMessage channel.
    pub fn new(config: IMessageConfig) -> Result<Self, ChannelError> {
        // Verify chat.db exists and is readable
        if !config.db_path.exists() {
            return Err(ChannelError::Configuration(format!(
                "chat.db not found at {}. Grant Full Disk Access to your terminal.",
                config.db_path.display()
            )));
        }

        Ok(Self {
            config,
            shutdown: Arc::new(AtomicBool::new(false)),
            last_rowid: Arc::new(AtomicI64::new(0)),
        })
    }

    /// Get the latest ROWID from chat.db using sqlite3 CLI.
    async fn get_latest_rowid(db_path: &std::path::Path) -> Result<i64, ChannelError> {
        let output = tokio::process::Command::new("sqlite3")
            .arg(db_path)
            .arg("SELECT MAX(ROWID) FROM message;")
            .output()
            .await
            .map_err(|e| ChannelError::StartupFailed {
                name: NAME.to_string(),
                reason: format!("sqlite3 failed: {e}"),
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let rowid: i64 = stdout.trim().parse().unwrap_or(0);
        Ok(rowid)
    }

    /// Poll for new messages since the given ROWID using sqlite3 CLI.
    async fn poll_messages(
        db_path: &std::path::Path,
        since_rowid: i64,
    ) -> Result<Vec<ChatDbMessage>, ChannelError> {
        let query = format!(
            "SELECT m.ROWID, \
                    REPLACE(COALESCE(m.text,''), '|', ' '), \
                    m.is_from_me, \
                    COALESCE(h.id, 'unknown'), \
                    COALESCE(c.chat_identifier, 'unknown') \
             FROM message m \
             LEFT JOIN handle h ON m.handle_id = h.ROWID \
             LEFT JOIN chat_message_join cmj ON m.ROWID = cmj.message_id \
             LEFT JOIN chat c ON cmj.chat_id = c.ROWID \
             WHERE m.ROWID > {since_rowid} \
               AND m.text IS NOT NULL \
               AND m.text != '' \
             ORDER BY m.ROWID ASC \
             LIMIT 50;"
        );

        let output = tokio::process::Command::new("sqlite3")
            .arg("-separator")
            .arg("|")
            .arg(db_path)
            .arg(&query)
            .output()
            .await
            .map_err(|e| ChannelError::Disconnected {
                name: NAME.to_string(),
                reason: format!("sqlite3 poll failed: {e}"),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ChannelError::Disconnected {
                name: NAME.to_string(),
                reason: format!("sqlite3 error: {stderr}"),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut messages = Vec::new();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.splitn(5, '|').collect();
            if parts.len() < 5 {
                continue;
            }

            let rowid: i64 = match parts[0].parse() {
                Ok(r) => r,
                Err(_) => continue,
            };
            let text = parts[1].to_string();
            let is_from_me = parts[2] == "1";
            let sender = parts[3].to_string();
            let chat_id = parts[4].to_string();

            if text.is_empty() {
                continue;
            }

            messages.push(ChatDbMessage {
                rowid,
                text,
                sender,
                chat_id,
                is_from_me,
            });
        }

        Ok(messages)
    }

    /// Send a message via osascript (AppleScript).
    async fn send_via_osascript(recipient: &str, text: &str) -> Result<(), ChannelError> {
        // Escape text for AppleScript
        let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");

        // Split long messages
        let chunks = split_message(&escaped);

        for chunk in chunks {
            let script = format!(
                r#"tell application "Messages"
    set targetService to 1st account whose service type = iMessage
    set targetBuddy to participant "{recipient}" of targetService
    send "{chunk}" to targetBuddy
end tell"#
            );

            let output = tokio::process::Command::new("osascript")
                .arg("-e")
                .arg(&script)
                .output()
                .await
                .map_err(|e| ChannelError::SendFailed {
                    name: NAME.to_string(),
                    reason: format!("osascript failed: {e}"),
                })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(ChannelError::SendFailed {
                    name: NAME.to_string(),
                    reason: format!("osascript error: {stderr}"),
                });
            }
        }

        Ok(())
    }

    /// Extract the recipient identifier from a chat_id.
    ///
    /// chat.db stores identifiers like "iMessage;-;+1234567890" or
    /// "iMessage;-;user@icloud.com". We extract the last component.
    fn extract_recipient(chat_id: &str) -> &str {
        chat_id.rsplit(';').next().unwrap_or(chat_id)
    }
}

#[async_trait]
impl Channel for IMessageChannel {
    fn name(&self) -> &str {
        NAME
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let (tx, rx) = mpsc::channel(64);

        // Initialize to current latest ROWID so we don't replay history
        let initial_rowid = Self::get_latest_rowid(&self.config.db_path).await?;
        self.last_rowid.store(initial_rowid, Ordering::Relaxed);
        tracing::info!(
            "iMessage channel started, polling from ROWID {}",
            initial_rowid
        );

        let db_path = self.config.db_path.clone();
        let allow_from = self.config.allow_from.clone();
        let poll_interval = Duration::from_secs(self.config.poll_interval_secs);
        let shutdown = self.shutdown.clone();
        let last_rowid = self.last_rowid.clone();

        // Spawn polling task
        tokio::spawn(async move {
            loop {
                if shutdown.load(Ordering::Relaxed) {
                    break;
                }

                let current_rowid = last_rowid.load(Ordering::Relaxed);

                match Self::poll_messages(&db_path, current_rowid).await {
                    Ok(messages) => {
                        for msg in messages {
                            // Skip outgoing messages
                            if msg.is_from_me {
                                last_rowid.store(msg.rowid, Ordering::Relaxed);
                                continue;
                            }

                            // Check allow-list
                            if !allow_from.is_empty()
                                && !allow_from.iter().any(|a| a == "*" || a == &msg.sender)
                            {
                                last_rowid.store(msg.rowid, Ordering::Relaxed);
                                continue;
                            }

                            let recipient = Self::extract_recipient(&msg.chat_id);

                            let incoming = IncomingMessage::new(NAME, &msg.sender, &msg.text)
                                .with_metadata(serde_json::json!({
                                    "chat_id": msg.chat_id,
                                    "rowid": msg.rowid,
                                    "recipient": recipient,
                                }));

                            last_rowid.store(msg.rowid, Ordering::Relaxed);

                            if tx.send(incoming).await.is_err() {
                                tracing::warn!("iMessage channel receiver dropped");
                                return;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("iMessage polling error: {e}");
                    }
                }

                tokio::time::sleep(poll_interval).await;
            }
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let recipient = msg
            .metadata
            .get("recipient")
            .and_then(|v| v.as_str())
            .unwrap_or(&msg.user_id);

        Self::send_via_osascript(recipient, &response.content).await
    }

    async fn send_status(
        &self,
        _status: StatusUpdate,
        _metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        // iMessage doesn't support typing indicators
        Ok(())
    }

    async fn broadcast(
        &self,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        Self::send_via_osascript(user_id, &response.content).await
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        if self.config.db_path.exists() {
            Ok(())
        } else {
            Err(ChannelError::HealthCheckFailed {
                name: NAME.to_string(),
            })
        }
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        self.shutdown.store(true, Ordering::Relaxed);
        Ok(())
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Split a long message into chunks.
fn split_message(text: &str) -> Vec<String> {
    if text.len() <= MAX_MESSAGE_LENGTH {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= MAX_MESSAGE_LENGTH {
            chunks.push(remaining.to_string());
            break;
        }

        let split_at = remaining[..MAX_MESSAGE_LENGTH]
            .rfind('\n')
            .unwrap_or(MAX_MESSAGE_LENGTH);

        chunks.push(remaining[..split_at].to_string());
        remaining = remaining[split_at..].trim_start_matches('\n');
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_recipient_phone() {
        assert_eq!(
            IMessageChannel::extract_recipient("iMessage;-;+1234567890"),
            "+1234567890"
        );
    }

    #[test]
    fn test_extract_recipient_email() {
        assert_eq!(
            IMessageChannel::extract_recipient("iMessage;-;user@icloud.com"),
            "user@icloud.com"
        );
    }

    #[test]
    fn test_extract_recipient_bare() {
        assert_eq!(
            IMessageChannel::extract_recipient("+1234567890"),
            "+1234567890"
        );
    }

    #[test]
    fn test_split_message_short() {
        let chunks = split_message("Hello!");
        assert_eq!(chunks, vec!["Hello!"]);
    }

    #[test]
    fn test_default_config() {
        let config = IMessageConfig::default();
        assert!(config.db_path.to_string_lossy().contains("chat.db"));
        assert!(config.allow_from.is_empty());
        assert_eq!(config.poll_interval_secs, POLL_INTERVAL_SECS);
    }
}
