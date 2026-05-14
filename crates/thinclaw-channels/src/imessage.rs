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

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use thinclaw_channels_core::{
    Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate,
};
use thinclaw_media::MediaContent;
use thinclaw_types::error::ChannelError;

use crate::util::floor_char_boundary;

/// Channel name constant.
const NAME: &str = "imessage";

/// Default polling interval in seconds.
const POLL_INTERVAL_SECS: u64 = 3;

/// Maximum message length for a single iMessage bubble.
const MAX_MESSAGE_LENGTH: usize = 20_000;

/// Maximum single attachment size we'll read from disk (20 MB).
const MAX_IMESSAGE_ATTACHMENT_SIZE: u64 = 20 * 1024 * 1024;

/// Maximum entries in the deduplication ring buffer.
const DEDUP_RING_CAPACITY: usize = 1000;

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
    /// Maximum message age to process (seconds). Messages older than this
    /// at startup time are skipped. Default: 300 (5 minutes).
    pub max_message_age_secs: u64,
}

impl Default for IMessageConfig {
    fn default() -> Self {
        Self {
            db_path: default_chat_db_path(),
            allow_from: Vec::new(),
            poll_interval_secs: POLL_INTERVAL_SECS,
            max_message_age_secs: 300,
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
    /// Number of attachments on this message.
    attachment_count: i64,
    /// Whether this is a group conversation.
    is_group: bool,
}

// ── Diagnostics ─────────────────────────────────────────────────────

/// Preflight diagnostic for iMessage channel.
#[derive(Debug, Clone, serde::Serialize)]
pub struct IMessageDiagnostic {
    /// Whether chat.db exists at the configured path.
    pub db_exists: bool,
    /// Whether sqlite3 is available.
    pub sqlite3_available: bool,
    /// Whether osascript is available.
    pub osascript_available: bool,
    /// Whether Messages.app is running.
    pub messages_running: bool,
    /// Total message count in chat.db.
    pub total_messages: Option<i64>,
    /// Errors found during diagnostic.
    pub errors: Vec<String>,
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

    /// Run a preflight diagnostic check.
    pub async fn diagnose(config: &IMessageConfig) -> IMessageDiagnostic {
        let mut errors = Vec::new();
        let db_exists = config.db_path.exists();
        if !db_exists {
            errors.push(format!(
                "chat.db not found at {}. Grant Full Disk Access.",
                config.db_path.display()
            ));
        }

        // Check sqlite3
        let sqlite3_available = tokio::process::Command::new("sqlite3")
            .arg("--version")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !sqlite3_available {
            errors.push("sqlite3 CLI not found".to_string());
        }

        // Check osascript
        let osascript_available = tokio::process::Command::new("osascript")
            .arg("-e")
            .arg("return \"ok\"")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !osascript_available {
            errors.push("osascript not available".to_string());
        }

        // Check Messages.app running
        let messages_running = tokio::process::Command::new("pgrep")
            .arg("-x")
            .arg("Messages")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !messages_running {
            errors.push("Messages.app is not running (required for sending)".to_string());
        }

        // Get total message count
        let total_messages = if db_exists && sqlite3_available {
            tokio::process::Command::new("sqlite3")
                .arg(&config.db_path)
                .arg("SELECT COUNT(*) FROM message;")
                .output()
                .await
                .ok()
                .and_then(|o| {
                    String::from_utf8_lossy(&o.stdout)
                        .trim()
                        .parse::<i64>()
                        .ok()
                })
        } else {
            None
        };

        IMessageDiagnostic {
            db_exists,
            sqlite3_available,
            osascript_available,
            messages_running,
            total_messages,
            errors,
        }
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

    /// Find the minimum ROWID that is newer than `max_age_secs` ago.
    ///
    /// chat.db stores `date` as nanoseconds since Apple's CoreData epoch
    /// (2001-01-01 00:00:00 UTC). We compute the cutoff and find the
    /// smallest ROWID above it, falling back to `latest_rowid` if no
    /// messages are within the window (meaning nothing recent to process).
    async fn get_age_floor_rowid(
        db_path: &std::path::Path,
        max_age_secs: u64,
        latest_rowid: i64,
    ) -> i64 {
        // Apple CoreData epoch offset from Unix epoch (seconds).
        const APPLE_EPOCH_OFFSET: i64 = 978_307_200;

        let now_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        // chat.db stores nanoseconds since Apple epoch
        let cutoff_ns = (now_unix - APPLE_EPOCH_OFFSET - max_age_secs as i64) * 1_000_000_000;

        let query = format!("SELECT MIN(ROWID) FROM message WHERE date > {cutoff_ns};");

        match tokio::process::Command::new("sqlite3")
            .arg(db_path)
            .arg(&query)
            .output()
            .await
        {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                match stdout.trim().parse::<i64>() {
                    Ok(floor) if floor > 0 => {
                        // Start just before the first message in the window
                        // so it gets picked up by the first poll.
                        let effective = (floor - 1).max(0);
                        tracing::debug!(
                            cutoff_ns,
                            floor_rowid = floor,
                            effective_rowid = effective,
                            "iMessage: age-based ROWID floor computed"
                        );
                        effective
                    }
                    _ => {
                        // No messages in window — skip everything
                        tracing::debug!(
                            max_age_secs,
                            "iMessage: no messages within age window, using latest ROWID"
                        );
                        latest_rowid
                    }
                }
            }
            _ => {
                tracing::debug!("iMessage: age floor query failed, using latest ROWID");
                latest_rowid
            }
        }
    }

    /// Poll for new messages since the given ROWID using sqlite3 CLI.
    ///
    /// Enhanced query: joins chat_message_join to detect group chats
    /// (display_name IS NOT NULL = group) and counts attachments via
    /// message_attachment_join.
    async fn poll_messages(
        db_path: &std::path::Path,
        since_rowid: i64,
    ) -> Result<Vec<ChatDbMessage>, ChannelError> {
        let query = format!(
            "SELECT m.ROWID, \
                    REPLACE(COALESCE(m.text,''), '|', ' '), \
                    m.is_from_me, \
                    COALESCE(h.id, 'unknown'), \
                    COALESCE(c.chat_identifier, 'unknown'), \
                    (SELECT COUNT(*) FROM message_attachment_join maj WHERE maj.message_id = m.ROWID), \
                    CASE WHEN c.display_name IS NOT NULL AND c.display_name != '' THEN 1 ELSE 0 END \
             FROM message m \
             LEFT JOIN handle h ON m.handle_id = h.ROWID \
             LEFT JOIN chat_message_join cmj ON m.ROWID = cmj.message_id \
             LEFT JOIN chat c ON cmj.chat_id = c.ROWID \
             WHERE m.ROWID > {since_rowid} \
               AND (m.text IS NOT NULL OR (SELECT COUNT(*) FROM message_attachment_join maj2 WHERE maj2.message_id = m.ROWID) > 0) \
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
            let parts: Vec<&str> = line.splitn(7, '|').collect();
            if parts.len() < 7 {
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
            let attachment_count: i64 = parts[5].parse().unwrap_or(0);
            let is_group = parts[6] == "1";

            // Allow empty text messages if they have attachments
            if text.is_empty() && attachment_count == 0 {
                continue;
            }

            messages.push(ChatDbMessage {
                rowid,
                text,
                sender,
                chat_id,
                is_from_me,
                attachment_count,
                is_group,
            });
        }

        Ok(messages)
    }

    /// Send a message via osascript (AppleScript).
    async fn send_via_osascript(recipient: &str, text: &str) -> Result<(), ChannelError> {
        // Escape text for AppleScript
        let escaped = escape_applescript(text);

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

    fn conversation_kind(is_group: bool) -> &'static str {
        if is_group { "group" } else { "direct" }
    }

    fn conversation_scope_id(chat_id: &str, is_group: bool) -> String {
        if is_group {
            format!("imessage:group:{chat_id}")
        } else {
            format!("imessage:direct:{chat_id}")
        }
    }

    fn external_conversation_key(chat_id: &str, is_group: bool) -> String {
        if is_group {
            format!("imessage://group/{chat_id}")
        } else {
            format!("imessage://direct/{chat_id}")
        }
    }

    /// Determine if a sender identifier looks like a phone number.
    fn is_phone_number(s: &str) -> bool {
        let cleaned: String = s
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == '+')
            .collect();
        cleaned.len() >= 10
            && (cleaned.starts_with('+') || cleaned.chars().all(|c| c.is_ascii_digit()))
    }

    /// Determine if a sender identifier looks like an email.
    fn is_email(s: &str) -> bool {
        s.contains('@') && s.contains('.')
    }

    /// Attempt to send a file attachment via AppleScript.
    ///
    /// This is best-effort — `send file` is unreliable on modern macOS (14+).
    /// Returns `Ok(true)` if sent, `Ok(false)` if the AppleScript command
    /// failed (expected on some macOS versions), `Err` on hard failures.
    async fn send_file_via_osascript(
        recipient: &str,
        file_path: &Path,
    ) -> Result<bool, ChannelError> {
        let posix_path = file_path.to_string_lossy();
        let escaped_recipient = escape_applescript(recipient);
        let script = format!(
            r#"tell application "Messages"
    set targetService to 1st account whose service type = iMessage
    set targetBuddy to participant "{escaped_recipient}" of targetService
    send file (POSIX file "{posix_path}") to targetBuddy
end tell"#
        );

        let output = tokio::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: format!("osascript file send failed: {e}"),
            })?;

        if output.status.success() {
            tracing::debug!(
                file = %redact(&posix_path),
                "iMessage: attachment sent via AppleScript"
            );
            Ok(true)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::debug!(
                error = %stderr,
                "iMessage: AppleScript file send not supported on this macOS version"
            );
            Ok(false)
        }
    }

    /// Send outbound media attachments to a recipient.
    ///
    /// Writes each [`MediaContent`] to a temporary file and attempts to send
    /// it via AppleScript. Failures are logged but do not abort the response
    /// (best-effort delivery since `send file` is unreliable on macOS 14+).
    async fn send_attachments(recipient: &str, attachments: &[thinclaw_media::MediaContent]) {
        for attachment in attachments {
            let filename = attachment.filename.as_deref().unwrap_or("attachment");

            // Write to a temp file so osascript can reference a POSIX path
            let tmp_dir = std::env::temp_dir().join("thinclaw_imessage");
            if std::fs::create_dir_all(&tmp_dir).is_err() {
                tracing::warn!("iMessage: failed to create temp dir for attachment");
                continue;
            }
            let tmp_path = tmp_dir.join(filename);
            if let Err(e) = std::fs::write(&tmp_path, &attachment.data) {
                tracing::warn!(
                    error = %e,
                    "iMessage: failed to write attachment to temp file"
                );
                continue;
            }

            match Self::send_file_via_osascript(recipient, &tmp_path).await {
                Ok(true) => {
                    tracing::info!(
                        filename = %redact(filename),
                        "iMessage: attachment sent successfully"
                    );
                }
                Ok(false) => {
                    tracing::debug!(
                        filename = %redact(filename),
                        "iMessage: AppleScript file send not supported — attachment skipped"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        filename = %redact(filename),
                        "iMessage: failed to send attachment"
                    );
                }
            }

            // Clean up temp file (best-effort)
            let _ = std::fs::remove_file(&tmp_path);
        }
    }
}

#[async_trait]
impl Channel for IMessageChannel {
    fn name(&self) -> &str {
        NAME
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let (tx, rx) = mpsc::channel(64);

        // Initialize to current latest ROWID so we don't replay history.
        // Then apply max_message_age_secs: find the minimum ROWID that falls
        // within the age window so we skip truly stale messages on startup.
        let latest_rowid = Self::get_latest_rowid(&self.config.db_path).await?;
        let initial_rowid = if self.config.max_message_age_secs > 0 {
            Self::get_age_floor_rowid(
                &self.config.db_path,
                self.config.max_message_age_secs,
                latest_rowid,
            )
            .await
        } else {
            latest_rowid
        };
        self.last_rowid.store(initial_rowid, Ordering::Relaxed);
        tracing::info!(
            latest_rowid,
            effective_rowid = initial_rowid,
            max_age_secs = self.config.max_message_age_secs,
            "iMessage channel started, polling from ROWID"
        );

        let db_path = self.config.db_path.clone();
        let allow_from = self.config.allow_from.clone();
        let poll_interval = Duration::from_secs(self.config.poll_interval_secs);
        let shutdown = self.shutdown.clone();
        let last_rowid = self.last_rowid.clone();

        // Spawn polling task
        tokio::spawn(async move {
            // Bounded dedup ring: tracks ROWIDs seen in this session to
            // handle sqlite3 returning the same message if ROWID
            // boundaries shift (e.g., deleted messages).
            // Uses a VecDeque + HashSet combo capped at DEDUP_RING_CAPACITY.
            let mut seen_set: HashSet<i64> = HashSet::new();
            let mut seen_ring: VecDeque<i64> = VecDeque::new();

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

                            // Deduplication via bounded ring buffer
                            if seen_set.contains(&msg.rowid) {
                                last_rowid.store(msg.rowid, Ordering::Relaxed);
                                continue;
                            }
                            seen_set.insert(msg.rowid);
                            seen_ring.push_back(msg.rowid);

                            // Evict oldest entry when ring is full
                            while seen_ring.len() > DEDUP_RING_CAPACITY {
                                if let Some(old) = seen_ring.pop_front() {
                                    seen_set.remove(&old);
                                }
                            }

                            // Check allow-list
                            if !allow_from.is_empty()
                                && !allow_from.iter().any(|a| a == "*" || a == &msg.sender)
                            {
                                last_rowid.store(msg.rowid, Ordering::Relaxed);
                                continue;
                            }

                            let recipient = Self::extract_recipient(&msg.chat_id);

                            // Fetch and download media attachments from chat.db
                            let attachments = if msg.attachment_count > 0 {
                                fetch_imessage_attachments(&db_path, msg.rowid).await
                            } else {
                                Vec::new()
                            };

                            let content = if msg.text.is_empty() && !attachments.is_empty() {
                                "[Media received — please analyze the attached content]".to_string()
                            } else {
                                msg.text.clone()
                            };

                            let conversation_kind = Self::conversation_kind(msg.is_group);
                            let conversation_scope_id =
                                Self::conversation_scope_id(&msg.chat_id, msg.is_group);
                            let external_conversation_key =
                                Self::external_conversation_key(&msg.chat_id, msg.is_group);

                            let incoming = IncomingMessage::new(NAME, &msg.sender, &content)
                                .with_metadata(serde_json::json!({
                                    "chat_id": msg.chat_id,
                                    "rowid": msg.rowid,
                                    "recipient": recipient,
                                    "is_group": msg.is_group,
                                    "attachment_count": msg.attachment_count,
                                    "conversation_kind": conversation_kind,
                                    "conversation_scope_id": conversation_scope_id,
                                    "external_conversation_key": external_conversation_key,
                                    "raw_sender_id": msg.sender.clone(),
                                    "stable_sender_id": msg.sender.clone(),
                                    "sender_type": if Self::is_phone_number(&msg.sender) {
                                        "phone"
                                    } else if Self::is_email(&msg.sender) {
                                        "email"
                                    } else {
                                        "unknown"
                                    },
                                }))
                                .with_attachments(attachments);

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

        // Send outbound media attachments (best-effort)
        if !response.attachments.is_empty() {
            Self::send_attachments(recipient, &response.attachments).await;
        }

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

    fn formatting_hints(&self) -> Option<String> {
        Some(
            "- iMessage renders plain text best. Avoid markdown-heavy formatting.\n\
- Keep replies compact and conversational, with short paragraphs or simple bullets."
                .to_string(),
        )
    }

    async fn broadcast(
        &self,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        // Only send to valid phone numbers or email addresses.
        // Proactive notifications use broadcast() with user_id set to
        // something like "default" — that's not a real iMessage contact.
        if !Self::is_phone_number(user_id) && !Self::is_email(user_id) {
            tracing::debug!(
                recipient = user_id,
                "iMessage: skipping broadcast — recipient is not a phone number or email"
            );
            return Ok(());
        }

        // Send outbound media attachments (best-effort)
        if !response.attachments.is_empty() {
            Self::send_attachments(user_id, &response.attachments).await;
        }

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

    async fn diagnostics(&self) -> Option<serde_json::Value> {
        let diag = Self::diagnose(&self.config).await;
        serde_json::to_value(&diag).ok()
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        self.shutdown.store(true, Ordering::Relaxed);
        Ok(())
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Fetch media attachments for an iMessage by querying chat.db.
///
/// Reads the `attachment` and `message_attachment_join` tables to find
/// file paths, then reads the binary data from disk.
async fn fetch_imessage_attachments(
    db_path: &std::path::Path,
    message_rowid: i64,
) -> Vec<MediaContent> {
    // Query: get attachment filename and MIME type for this message
    let query = format!(
        "SELECT a.filename, COALESCE(a.mime_type, 'application/octet-stream'), a.total_bytes \
         FROM attachment a \
         INNER JOIN message_attachment_join maj ON a.ROWID = maj.attachment_id \
         WHERE maj.message_id = {} \
         ORDER BY a.ROWID ASC;",
        message_rowid
    );

    let output = match tokio::process::Command::new("sqlite3")
        .arg("-separator")
        .arg("|")
        .arg(db_path)
        .arg(&query)
        .output()
        .await
    {
        Ok(o) if o.status.success() => o,
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            tracing::warn!(
                rowid = message_rowid,
                error = %stderr,
                "iMessage: failed to query attachments"
            );
            return Vec::new();
        }
        Err(e) => {
            tracing::warn!(
                rowid = message_rowid,
                error = %e,
                "iMessage: sqlite3 attachment query failed"
            );
            return Vec::new();
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut result = Vec::new();

    for line in stdout.lines() {
        let parts: Vec<&str> = line.splitn(3, '|').collect();
        if parts.len() < 3 {
            continue;
        }

        let raw_path = parts[0];
        let mime = parts[1];
        let total_bytes: u64 = parts[2].parse().unwrap_or(0);

        // Skip oversized files
        if total_bytes > MAX_IMESSAGE_ATTACHMENT_SIZE {
            tracing::warn!(
                path = %raw_path,
                size = total_bytes,
                "iMessage: skipping oversized attachment"
            );
            continue;
        }

        // chat.db stores paths with ~ prefix → expand to real path
        let file_path = if let Some(rest) = raw_path.strip_prefix("~/") {
            let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/"));
            home.join(rest)
        } else {
            std::path::PathBuf::from(raw_path)
        };

        if !file_path.exists() {
            tracing::debug!(
                path = %file_path.display(),
                "iMessage: attachment file not found on disk"
            );
            continue;
        }

        match std::fs::read(&file_path) {
            Ok(data) => {
                let filename = file_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("attachment")
                    .to_string();
                let mc = MediaContent::new(data, mime).with_filename(filename.clone());
                tracing::debug!(
                    filename = %filename,
                    mime = %mime,
                    size = mc.size(),
                    "iMessage: loaded attachment from disk"
                );
                result.push(mc);
            }
            Err(e) => {
                tracing::warn!(
                    path = %file_path.display(),
                    error = %e,
                    "iMessage: failed to read attachment file"
                );
            }
        }
    }

    result
}

/// Split a long message into chunks, preferring line-break boundaries.
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

        // Safe for multi-byte UTF-8: round down to a valid char boundary
        let safe_end = floor_char_boundary(remaining, MAX_MESSAGE_LENGTH);
        let split_at = remaining[..safe_end].rfind('\n').unwrap_or(safe_end);

        chunks.push(remaining[..split_at].to_string());
        remaining = remaining[split_at..].trim_start_matches('\n');
    }

    chunks
}

/// Redact phone numbers and email addresses from log output.
fn redact(text: &str) -> String {
    use std::sync::OnceLock;
    static PHONE_RE: OnceLock<regex::Regex> = OnceLock::new();
    static EMAIL_RE: OnceLock<regex::Regex> = OnceLock::new();

    let phone_re = PHONE_RE
        .get_or_init(|| regex::Regex::new(r"\+?\d{7,15}").expect("static phone regex is valid"));
    let email_re = EMAIL_RE.get_or_init(|| {
        regex::Regex::new(r"[\w.+-]+@[\w-]+\.[\w.]+").expect("static email regex is valid")
    });
    let s = phone_re.replace_all(text, "[REDACTED]");
    email_re.replace_all(&s, "[REDACTED]").to_string()
}

/// Escape text for safe inclusion in AppleScript strings.
fn escape_applescript(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── extract_recipient tests ────────────────────────────────────

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
    fn test_extract_recipient_sms() {
        assert_eq!(
            IMessageChannel::extract_recipient("SMS;-;+4917612345678"),
            "+4917612345678"
        );
    }

    #[test]
    fn test_extract_recipient_empty() {
        assert_eq!(IMessageChannel::extract_recipient(""), "");
    }

    // ── sender type detection ──────────────────────────────────────

    #[test]
    fn test_is_phone_number() {
        assert!(IMessageChannel::is_phone_number("+1234567890"));
        assert!(IMessageChannel::is_phone_number("+4917612345678"));
        assert!(!IMessageChannel::is_phone_number("user@icloud.com"));
        assert!(!IMessageChannel::is_phone_number("short"));
    }

    #[test]
    fn test_is_email() {
        assert!(IMessageChannel::is_email("user@icloud.com"));
        assert!(IMessageChannel::is_email("test@gmail.com"));
        assert!(!IMessageChannel::is_email("+1234567890"));
        assert!(!IMessageChannel::is_email("noidea"));
    }

    // ── split_message tests ────────────────────────────────────────

    #[test]
    fn test_split_message_short() {
        let chunks = split_message("Hello!");
        assert_eq!(chunks, vec!["Hello!"]);
    }

    #[test]
    fn test_split_message_exact_boundary() {
        let text = "x".repeat(MAX_MESSAGE_LENGTH);
        let chunks = split_message(&text);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].len(), MAX_MESSAGE_LENGTH);
    }

    #[test]
    fn test_split_message_empty() {
        let chunks = split_message("");
        assert_eq!(chunks, vec![""]);
    }

    #[test]
    fn test_split_message_newline_boundary() {
        let mut text = "a".repeat(MAX_MESSAGE_LENGTH - 5);
        text.push('\n');
        text.push_str(&"b".repeat(100));
        let chunks = split_message(&text);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), MAX_MESSAGE_LENGTH - 5);
    }

    #[test]
    fn test_split_message_unicode_boundary() {
        let mut text = "a".repeat(MAX_MESSAGE_LENGTH - 1);
        text.push('🙂');
        let chunks = split_message(&text);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], "a".repeat(MAX_MESSAGE_LENGTH - 1));
        assert_eq!(chunks[1], "🙂");
    }

    #[test]
    fn test_conversation_key_helpers() {
        assert_eq!(IMessageChannel::conversation_kind(false), "direct");
        assert_eq!(IMessageChannel::conversation_kind(true), "group");
        assert_eq!(
            IMessageChannel::conversation_scope_id("chat-1", false),
            "imessage:direct:chat-1"
        );
        assert_eq!(
            IMessageChannel::conversation_scope_id("chat-2", true),
            "imessage:group:chat-2"
        );
        assert_eq!(
            IMessageChannel::external_conversation_key("chat-3", false),
            "imessage://direct/chat-3"
        );
        assert_eq!(
            IMessageChannel::external_conversation_key("chat-4", true),
            "imessage://group/chat-4"
        );
    }

    // ── escape tests ───────────────────────────────────────────────

    #[test]
    fn test_escape_applescript() {
        assert_eq!(escape_applescript(r#"say "hello""#), r#"say \"hello\""#);
        assert_eq!(escape_applescript("back\\slash"), "back\\\\slash");
        assert_eq!(escape_applescript("normal"), "normal");
    }

    // ── config tests ───────────────────────────────────────────────

    #[test]
    fn test_default_config() {
        let config = IMessageConfig::default();
        assert!(config.db_path.to_string_lossy().contains("chat.db"));
        assert!(config.allow_from.is_empty());
        assert_eq!(config.poll_interval_secs, POLL_INTERVAL_SECS);
        assert_eq!(config.max_message_age_secs, 300);
    }

    #[test]
    fn test_default_chat_db_path() {
        let path = default_chat_db_path();
        assert!(path.to_string_lossy().contains("Library/Messages/chat.db"));
    }

    #[test]
    fn test_config_with_allow_from() {
        let config = IMessageConfig {
            allow_from: vec!["+1234567890".into(), "user@icloud.com".into()],
            ..Default::default()
        };
        assert_eq!(config.allow_from.len(), 2);
        assert!(config.allow_from.contains(&"+1234567890".to_string()));
    }

    // ── channel creation tests ─────────────────────────────────────

    #[test]
    fn test_new_channel_missing_db() {
        let config = IMessageConfig {
            db_path: PathBuf::from("/nonexistent/chat.db"),
            ..Default::default()
        };
        let result = IMessageChannel::new(config);
        assert!(result.is_err());
    }

    // ── diagnostic struct tests ────────────────────────────────────

    #[test]
    fn test_diagnostic_serializable() {
        let diag = IMessageDiagnostic {
            db_exists: true,
            sqlite3_available: true,
            osascript_available: true,
            messages_running: false,
            total_messages: Some(12345),
            errors: vec!["Messages.app is not running".into()],
        };
        let json = serde_json::to_string(&diag).unwrap();
        assert!(json.contains("\"db_exists\":true"));
        assert!(json.contains("12345"));
    }

    #[test]
    fn test_redact_replaces_phone_and_email() {
        assert_eq!(
            redact("Reach me at +1234567890 or user@example.com"),
            "Reach me at [REDACTED] or [REDACTED]"
        );
        assert_eq!(redact("No sensitive data here"), "No sensitive data here");
    }
}
