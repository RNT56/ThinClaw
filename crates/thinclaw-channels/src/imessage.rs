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
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Mutex, Notify, mpsc};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::ReceiverStream;

use thinclaw_channels_core::{
    Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate,
};
use thinclaw_media::MediaContent;
use thinclaw_types::error::ChannelError;

use crate::util::{decode_sqlite_hex, floor_char_boundary, output_with_timeout};

/// Channel name constant.
const NAME: &str = "imessage";

/// Default polling interval in seconds.
const POLL_INTERVAL_SECS: u64 = 3;

const CHANNEL_TASK_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

/// Maximum message length for a single iMessage bubble.
const MAX_MESSAGE_LENGTH: usize = 20_000;

/// Maximum single attachment size we'll read from disk (20 MB).
const MAX_IMESSAGE_ATTACHMENT_SIZE: u64 = 20 * 1024 * 1024;

const MAX_OUTBOUND_ATTACHMENTS: usize = 10;
const MAX_OUTBOUND_ATTACHMENTS_TOTAL_BYTES: usize = 50 * 1024 * 1024;
const MAX_INBOUND_ATTACHMENTS: usize = 20;

/// Maximum entries in the deduplication ring buffer.
const DEDUP_RING_CAPACITY: usize = 1000;

const SEND_MESSAGE_APPLESCRIPT: &str = r#"on run argv
    set recipientAddress to item 1 of argv
    set messageText to item 2 of argv
    tell application "Messages"
        set targetService to 1st account whose service type = iMessage
        set targetBuddy to participant (my recipientAddress) of targetService
        send (my messageText) to targetBuddy
    end tell
end run"#;

const SEND_FILE_APPLESCRIPT: &str = r#"on run argv
    set recipientAddress to item 1 of argv
    set attachmentPath to item 2 of argv
    tell application "Messages"
        set targetService to 1st account whose service type = iMessage
        set targetBuddy to participant (my recipientAddress) of targetService
        send file (POSIX file (my attachmentPath)) to targetBuddy
    end tell
end run"#;

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
    shutdown_notify: Arc<Notify>,
    poll_task: Mutex<Option<JoinHandle<()>>>,
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
            shutdown_notify: Arc::new(Notify::new()),
            poll_task: Mutex::new(None),
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
        let mut sqlite3_check = tokio::process::Command::new("sqlite3");
        sqlite3_check.arg("--version");
        let sqlite3_available = output_with_timeout(&mut sqlite3_check, "sqlite3 diagnostic")
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !sqlite3_available {
            errors.push("sqlite3 CLI not found".to_string());
        }

        // Check osascript
        let mut osascript_check = tokio::process::Command::new("osascript");
        osascript_check.arg("-e").arg("return \"ok\"");
        let osascript_available = output_with_timeout(&mut osascript_check, "osascript diagnostic")
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !osascript_available {
            errors.push("osascript not available".to_string());
        }

        // Check Messages.app running
        let mut pgrep = tokio::process::Command::new("pgrep");
        pgrep.arg("-x").arg("Messages");
        let messages_running = output_with_timeout(&mut pgrep, "pgrep Messages diagnostic")
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !messages_running {
            errors.push("Messages.app is not running (required for sending)".to_string());
        }

        // Get total message count
        let total_messages = if db_exists && sqlite3_available {
            let mut count = tokio::process::Command::new("sqlite3");
            count
                .arg(&config.db_path)
                .arg("SELECT COUNT(*) FROM message;");
            output_with_timeout(&mut count, "sqlite3 Messages count diagnostic")
                .await
                .ok()
                .filter(|output| output.status.success())
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
        let mut cmd = tokio::process::Command::new("sqlite3");
        cmd.arg(db_path).arg("SELECT MAX(ROWID) FROM message;");
        let output = output_with_timeout(&mut cmd, "sqlite3 max-rowid")
            .await
            .map_err(|reason| ChannelError::StartupFailed {
                name: NAME.to_string(),
                reason,
            })?;

        // Don't let a query failure silently become ROWID 0 (which would drop
        // the cursor to the start of history). An empty result is a genuinely
        // empty table where 0 is correct.
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ChannelError::StartupFailed {
                name: NAME.to_string(),
                reason: format!("sqlite3 max-rowid failed: {stderr}"),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let trimmed = stdout.trim();
        if trimmed.is_empty() {
            return Ok(0);
        }
        trimmed
            .parse::<i64>()
            .map_err(|e| ChannelError::StartupFailed {
                name: NAME.to_string(),
                reason: format!("sqlite3 max-rowid returned unparseable output: {e}"),
            })
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

        let mut cmd = tokio::process::Command::new("sqlite3");
        cmd.arg(db_path).arg(&query);
        match output_with_timeout(&mut cmd, "sqlite3 age-floor").await {
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
                    hex(CAST(COALESCE(m.text,'') AS TEXT)), \
                    m.is_from_me, \
                    hex(CAST(COALESCE(h.id, 'unknown') AS TEXT)), \
                    hex(CAST(COALESCE(c.chat_identifier, 'unknown') AS TEXT)), \
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

        let mut cmd = tokio::process::Command::new("sqlite3");
        cmd.arg("-separator").arg("|").arg(db_path).arg(&query);
        let output = output_with_timeout(&mut cmd, "sqlite3 poll")
            .await
            .map_err(|reason| ChannelError::Disconnected {
                name: NAME.to_string(),
                reason,
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
            let (Some(text), Some(sender), Some(chat_id)) = (
                decode_sqlite_hex(parts[1]),
                decode_sqlite_hex(parts[3]),
                decode_sqlite_hex(parts[4]),
            ) else {
                tracing::warn!(rowid, "iMessage: skipping row with malformed hex text");
                continue;
            };
            let is_from_me = parts[2] == "1";
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
        // Split the raw text, then pass each value through argv. No untrusted
        // value is interpolated into executable AppleScript source.
        let chunks = split_message(text);

        for chunk in chunks {
            let mut cmd = tokio::process::Command::new("osascript");
            cmd.arg("-e")
                .arg(SEND_MESSAGE_APPLESCRIPT)
                .arg("--")
                .arg(recipient)
                .arg(chunk);
            let output = output_with_timeout(&mut cmd, "osascript send")
                .await
                .map_err(|reason| ChannelError::SendFailed {
                    name: NAME.to_string(),
                    reason,
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

        let mut cmd = tokio::process::Command::new("osascript");
        cmd.arg("-e")
            .arg(SEND_FILE_APPLESCRIPT)
            .arg("--")
            .arg(recipient)
            .arg(posix_path.as_ref());
        let output = output_with_timeout(&mut cmd, "osascript file send")
            .await
            .map_err(|reason| ChannelError::SendFailed {
                name: NAME.to_string(),
                reason,
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
        if attachments.len() > MAX_OUTBOUND_ATTACHMENTS {
            tracing::warn!(
                supplied = attachments.len(),
                retained = MAX_OUTBOUND_ATTACHMENTS,
                "iMessage: outbound attachment count exceeds limit"
            );
        }
        let mut total_bytes = 0_usize;
        for attachment in attachments.iter().take(MAX_OUTBOUND_ATTACHMENTS) {
            if attachment.data.len() as u64 > MAX_IMESSAGE_ATTACHMENT_SIZE {
                tracing::warn!(
                    size = attachment.data.len(),
                    "iMessage: skipping oversized outbound attachment"
                );
                continue;
            }
            let Some(next_total) = total_bytes.checked_add(attachment.data.len()) else {
                tracing::warn!("iMessage: outbound attachment byte count overflowed");
                break;
            };
            if next_total > MAX_OUTBOUND_ATTACHMENTS_TOTAL_BYTES {
                tracing::warn!(
                    limit = MAX_OUTBOUND_ATTACHMENTS_TOTAL_BYTES,
                    "iMessage: outbound attachments exceed aggregate size limit"
                );
                break;
            }
            total_bytes = next_total;

            let filename = attachment.filename.as_deref().unwrap_or("attachment");
            let safe_name = safe_attachment_filename(filename);
            let tmp_path = std::env::temp_dir().join(format!(
                "thinclaw-imessage-{}-{safe_name}",
                uuid::Uuid::new_v4().simple()
            ));
            let mut options = tokio::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
            let write_result = async {
                let mut file = options.open(&tmp_path).await?;
                file.write_all(&attachment.data).await?;
                file.sync_all().await
            }
            .await;
            if let Err(e) = write_result {
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
            let _ = tokio::fs::remove_file(&tmp_path).await;
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
        if let Some(handle) = self.poll_task.lock().await.take() {
            self.shutdown.store(true, Ordering::Relaxed);
            self.shutdown_notify.notify_waiters();
            drain_channel_task(handle, NAME).await;
        }
        self.shutdown.store(false, Ordering::Relaxed);

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
        let shutdown_notify = Arc::clone(&self.shutdown_notify);
        let last_rowid = self.last_rowid.clone();

        // Spawn polling task
        let handle = tokio::spawn(async move {
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

                tokio::select! {
                    _ = tokio::time::sleep(poll_interval) => {}
                    _ = shutdown_notify.notified() => {
                        if shutdown.load(Ordering::Relaxed) {
                            break;
                        }
                    }
                }
            }
        });
        *self.poll_task.lock().await = Some(handle);

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
        self.shutdown_notify.notify_waiters();
        if let Some(handle) = self.poll_task.lock().await.take() {
            drain_channel_task(handle, NAME).await;
        }
        Ok(())
    }
}

async fn drain_channel_task(mut handle: JoinHandle<()>, name: &'static str) {
    tokio::select! {
        result = &mut handle => {
            if let Err(error) = result {
                tracing::warn!(channel = name, error = %error, "channel polling task exited with error");
            }
        }
        _ = tokio::time::sleep(CHANNEL_TASK_SHUTDOWN_TIMEOUT) => {
            handle.abort();
            let _ = handle.await;
            tracing::warn!(channel = name, "channel polling task did not drain before timeout; aborted");
        }
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
        "SELECT hex(CAST(a.filename AS TEXT)), \
                hex(CAST(COALESCE(a.mime_type, 'application/octet-stream') AS TEXT)), \
                a.total_bytes \
         FROM attachment a \
         INNER JOIN message_attachment_join maj ON a.ROWID = maj.attachment_id \
         WHERE maj.message_id = {} \
         ORDER BY a.ROWID ASC \
         LIMIT {};",
        message_rowid, MAX_INBOUND_ATTACHMENTS
    );

    let mut cmd = tokio::process::Command::new("sqlite3");
    cmd.arg("-separator").arg("|").arg(db_path).arg(&query);
    let output = match output_with_timeout(&mut cmd, "sqlite3 attachments").await {
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
    let canonical_attachments_root = match dirs::home_dir() {
        Some(home) => tokio::fs::canonicalize(home.join("Library/Messages/Attachments"))
            .await
            .ok(),
        None => None,
    };

    for line in stdout.lines() {
        let parts: Vec<&str> = line.splitn(3, '|').collect();
        if parts.len() < 3 {
            continue;
        }

        let (Some(raw_path), Some(mime)) =
            (decode_sqlite_hex(parts[0]), decode_sqlite_hex(parts[1]))
        else {
            tracing::warn!(rowid = message_rowid, "iMessage: malformed attachment row");
            continue;
        };
        let total_bytes: u64 = parts[2].parse().unwrap_or(0);

        // Skip oversized files
        if total_bytes > MAX_IMESSAGE_ATTACHMENT_SIZE {
            tracing::warn!(
                path = %redact(&raw_path),
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
            std::path::PathBuf::from(&raw_path)
        };

        let canonical_path = match tokio::fs::canonicalize(&file_path).await {
            Ok(path)
                if canonical_attachments_root
                    .as_ref()
                    .is_some_and(|root| path.starts_with(root)) =>
            {
                path
            }
            _ => {
                tracing::warn!(
                    path = %redact(&file_path.to_string_lossy()),
                    "iMessage: refusing attachment path outside Messages storage"
                );
                continue;
            }
        };
        let mut options = tokio::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW);
        let file = match options.open(&canonical_path).await {
            Ok(file) => file,
            Err(error) => {
                tracing::warn!(
                    path = %redact(&canonical_path.to_string_lossy()),
                    error = %error,
                    "iMessage: refusing attachment that cannot be opened safely"
                );
                continue;
            }
        };
        let metadata = match file.metadata().await {
            Ok(metadata)
                if metadata.is_file() && metadata.len() <= MAX_IMESSAGE_ATTACHMENT_SIZE =>
            {
                metadata
            }
            _ => {
                tracing::warn!("iMessage: refusing missing, non-file, or oversized attachment");
                continue;
            }
        };
        let mut data = Vec::with_capacity(metadata.len() as usize);
        let read_result = file
            .take(MAX_IMESSAGE_ATTACHMENT_SIZE + 1)
            .read_to_end(&mut data)
            .await;
        match read_result {
            Ok(_) if data.len() as u64 <= MAX_IMESSAGE_ATTACHMENT_SIZE => {
                let filename = file_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("attachment")
                    .to_string();
                let safe_mime = if mime.len() <= 255
                    && mime
                        .bytes()
                        .all(|byte| byte.is_ascii_graphic() && byte != b';')
                {
                    mime.as_str()
                } else {
                    "application/octet-stream"
                };
                let mc = MediaContent::new(data, safe_mime).with_filename(filename.clone());
                tracing::debug!(
                    filename = %filename,
                    mime = %mime,
                    size = mc.size(),
                    "iMessage: loaded attachment from disk"
                );
                result.push(mc);
            }
            Ok(_) => {
                tracing::warn!("iMessage: attachment grew beyond the size limit while reading");
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

fn safe_attachment_filename(filename: &str) -> String {
    let base = Path::new(filename)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("attachment");
    let sanitized = base
        .chars()
        .take(128)
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let sanitized = sanitized.trim_matches('.');
    if sanitized.is_empty() {
        "attachment".to_string()
    } else {
        sanitized.to_string()
    }
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

    #[test]
    fn attachment_filename_is_confined_to_one_safe_segment() {
        assert_eq!(safe_attachment_filename("../../secret.txt"), "secret.txt");
        assert_eq!(safe_attachment_filename("a/b\\c:name"), "b_c_name");
        assert_eq!(safe_attachment_filename(".."), "attachment");
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
