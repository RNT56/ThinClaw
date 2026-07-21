//! Apple Mail channel (macOS only).
//!
//! Reads incoming emails by polling Apple Mail's Envelope Index SQLite
//! database and sends replies via `osascript` (AppleScript) controlling
//! Mail.app. This approach requires:
//! - macOS with Full Disk Access granted to the terminal/app
//! - Mail.app running and signed into an Apple ID / iCloud account
//!
//! We use the `sqlite3` CLI to avoid SQLite linkage conflicts with
//! libsql/sqlx that already exist in the project.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, Notify, mpsc};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::ReceiverStream;

use thinclaw_channels_core::{
    Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate,
};
use thinclaw_types::error::ChannelError;

use crate::util::{decode_sqlite_hex, floor_char_boundary, output_with_timeout};

/// Channel name constant.
const NAME: &str = "apple_mail";

/// Default polling interval in seconds.
const POLL_INTERVAL_SECS: u64 = 10;

/// Maximum email body length.
const MAX_BODY_LENGTH: usize = 100_000;

const MAX_OUTBOUND_ATTACHMENTS: usize = 10;
const MAX_OUTBOUND_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;
const MAX_OUTBOUND_ATTACHMENTS_TOTAL_BYTES: usize = 50 * 1024 * 1024;

const CHANNEL_TASK_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

const MARK_READ_APPLESCRIPT: &str = r#"on run argv
    set wantedSubject to item 1 of argv
    set wantedSender to item 2 of argv
    tell application "Mail"
        set foundMessages to (every message of inbox whose subject is my wantedSubject and sender contains my wantedSender)
        repeat with msg in foundMessages
            set read status of msg to true
        end repeat
    end tell
end run"#;

const SEND_EMAIL_APPLESCRIPT: &str = r#"on run argv
    set recipientAddress to item 1 of argv
    set messageSubject to item 2 of argv
    set messageBody to item 3 of argv
    tell application "Mail"
        set newMessage to make new outgoing message with properties {subject:my messageSubject, content:my messageBody, visible:false}
        tell newMessage
            make new to recipient at end of to recipients with properties {address:my recipientAddress}
        end tell
        if (count of argv) > 3 then
            repeat with attachmentIndex from 4 to count of argv
                set attachmentPath to item attachmentIndex of argv
                tell newMessage to make new attachment with properties {file name:POSIX file (my attachmentPath)} at after last paragraph
            end repeat
        end if
        send newMessage
    end tell
end run"#;

const SEND_REPLY_APPLESCRIPT: &str = r#"on run argv
    set recipientAddress to item 1 of argv
    set messageSubject to "Re: " & (item 2 of argv)
    set messageBody to item 3 of argv
    tell application "Mail"
        set newMessage to make new outgoing message with properties {subject:my messageSubject, content:my messageBody, visible:false}
        tell newMessage
            make new to recipient at end of to recipients with properties {address:my recipientAddress}
        end tell
        if (count of argv) > 3 then
            repeat with attachmentIndex from 4 to count of argv
                set attachmentPath to item attachmentIndex of argv
                tell newMessage to make new attachment with properties {file name:POSIX file (my attachmentPath)} at after last paragraph
            end repeat
        end if
        send newMessage
    end tell
end run"#;

// ── Configuration ───────────────────────────────────────────────────

/// Apple Mail channel configuration.
#[derive(Debug, Clone)]
pub struct AppleMailConfig {
    /// Path to Envelope Index (default: auto-detected from ~/Library/Mail/).
    pub db_path: Option<PathBuf>,
    /// Allowed sender email addresses (empty = allow all).
    pub allow_from: Vec<String>,
    /// Polling interval in seconds.
    pub poll_interval_secs: u64,
    /// Mailbox to monitor (default: INBOX).
    pub mailbox: String,
    /// Only process unread messages.
    pub unread_only: bool,
    /// Mark messages as read after processing.
    pub mark_as_read: bool,
    /// Maximum message age to process at startup (seconds).
    /// Messages older than this are skipped. Default: 300 (5 minutes).
    pub max_message_age_secs: u64,
}

impl Default for AppleMailConfig {
    fn default() -> Self {
        Self {
            db_path: None,
            allow_from: Vec::new(),
            poll_interval_secs: POLL_INTERVAL_SECS,
            mailbox: "INBOX".to_string(),
            unread_only: true,
            mark_as_read: true,
            max_message_age_secs: 300,
        }
    }
}

// ── Raw message from Envelope Index ─────────────────────────────────

/// A raw email message parsed from sqlite3 output.
#[derive(Debug, Clone)]
struct MailMessage {
    /// ROWID from the messages table.
    rowid: i64,
    /// Message subject.
    subject: String,
    /// Sender email address.
    sender: String,
    /// Email body (plain text summary).
    body: String,
    /// Date received (Unix timestamp).
    date_received: i64,
    /// Whether the message has been read.
    is_read: bool,
    /// Message-ID header for threading.
    message_id: String,
}

// ── Diagnostics ─────────────────────────────────────────────────────

/// Preflight diagnostic for Apple Mail channel.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AppleMailDiagnostic {
    /// Whether the Envelope Index database exists.
    pub db_exists: bool,
    /// Resolved path to the database.
    pub db_path: String,
    /// Whether sqlite3 is available.
    pub sqlite3_available: bool,
    /// Whether osascript is available.
    pub osascript_available: bool,
    /// Whether Mail.app is running.
    pub mail_running: bool,
    /// Total message count in the database.
    pub total_messages: Option<i64>,
    /// Errors found during diagnostic.
    pub errors: Vec<String>,
}

// ── Channel implementation ──────────────────────────────────────────

/// Apple Mail channel using Envelope Index polling + AppleScript sending.
pub struct AppleMailChannel {
    config: AppleMailConfig,
    db_path: PathBuf,
    /// Shutdown flag.
    shutdown: Arc<AtomicBool>,
    shutdown_notify: Arc<Notify>,
    poll_task: Mutex<Option<JoinHandle<()>>>,
    /// Last processed message ROWID.
    last_rowid: Arc<AtomicI64>,
}

impl AppleMailChannel {
    /// Create a new Apple Mail channel.
    pub fn new(config: AppleMailConfig) -> Result<Self, ChannelError> {
        let db_path = match &config.db_path {
            Some(p) => p.clone(),
            None => find_envelope_index()?,
        };

        if !db_path.exists() {
            return Err(ChannelError::Configuration(format!(
                "Mail Envelope Index not found at {}. \
                 Grant Full Disk Access and ensure Mail.app is configured.",
                db_path.display()
            )));
        }

        Ok(Self {
            config,
            db_path,
            shutdown: Arc::new(AtomicBool::new(false)),
            shutdown_notify: Arc::new(Notify::new()),
            poll_task: Mutex::new(None),
            last_rowid: Arc::new(AtomicI64::new(0)),
        })
    }

    /// Run a preflight diagnostic check.
    pub async fn diagnose(config: &AppleMailConfig) -> AppleMailDiagnostic {
        let mut errors = Vec::new();

        let db_path = match &config.db_path {
            Some(p) => p.clone(),
            None => find_envelope_index().unwrap_or_else(|_| PathBuf::from("(not found)")),
        };

        let db_exists = db_path.exists();
        if !db_exists {
            errors.push(format!(
                "Envelope Index not found at {}. Grant Full Disk Access.",
                db_path.display()
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

        // Check Mail.app running
        let mut pgrep = tokio::process::Command::new("pgrep");
        pgrep.arg("-x").arg("Mail");
        let mail_running = output_with_timeout(&mut pgrep, "pgrep Mail diagnostic")
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !mail_running {
            errors.push("Mail.app is not running (required for sending)".to_string());
        }

        // Get total message count
        let total_messages = if db_exists && sqlite3_available {
            let mut count = tokio::process::Command::new("sqlite3");
            count.arg(&db_path).arg("SELECT COUNT(*) FROM messages;");
            output_with_timeout(&mut count, "sqlite3 Mail count diagnostic")
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

        AppleMailDiagnostic {
            db_exists,
            db_path: db_path.to_string_lossy().to_string(),
            sqlite3_available,
            osascript_available,
            mail_running,
            total_messages,
            errors,
        }
    }

    /// Get the latest ROWID from the messages table.
    async fn get_latest_rowid(db_path: &std::path::Path) -> Result<i64, ChannelError> {
        let mut cmd = tokio::process::Command::new("sqlite3");
        cmd.arg(db_path).arg("SELECT MAX(ROWID) FROM messages;");
        let output = output_with_timeout(&mut cmd, "sqlite3 max-rowid")
            .await
            .map_err(|reason| ChannelError::StartupFailed {
                name: NAME.to_string(),
                reason,
            })?;

        // A failed query must NOT silently become ROWID 0 — that resets the
        // cursor and replays the account's entire unread backlog (each of which
        // the agent would auto-reply to). Fail instead so start() can retry.
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ChannelError::StartupFailed {
                name: NAME.to_string(),
                reason: format!("sqlite3 max-rowid failed: {stderr}"),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let trimmed = stdout.trim();
        // An empty result means MAX(ROWID) was NULL — a genuinely empty mailbox,
        // where starting from 0 is correct (there is nothing to replay).
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

    /// Poll for new messages since the given ROWID.
    ///
    /// Queries the Envelope Index database which stores all email metadata.
    /// The schema uses `messages` for envelope data and `addresses` for
    /// sender/recipient info.
    async fn poll_messages(
        db_path: &std::path::Path,
        since_rowid: i64,
        unread_only: bool,
    ) -> Result<Vec<MailMessage>, ChannelError> {
        let read_filter = if unread_only { "AND m.read = 0" } else { "" };

        let query = format!(
            "SELECT m.ROWID, \
                    hex(CAST(COALESCE(sub.subject, '(no subject)') AS TEXT)), \
                    hex(CAST(COALESCE(a.address, 'unknown') AS TEXT)), \
                    hex(CAST(COALESCE(summ.summary, '') AS TEXT)), \
                    COALESCE(m.date_received, 0), \
                    COALESCE(m.read, 0), \
                    hex(CAST(COALESCE(m.message_id, '') AS TEXT)) \
             FROM messages m \
             LEFT JOIN subjects sub ON m.subject = sub.ROWID \
             LEFT JOIN summaries summ ON m.summary = summ.ROWID \
             LEFT JOIN addresses a ON m.sender = a.ROWID \
             WHERE m.ROWID > {since_rowid} \
               {} \
             ORDER BY m.ROWID ASC \
             LIMIT 50;",
            read_filter
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
            let (Some(subject), Some(sender), Some(decoded_body), Some(message_id)) = (
                decode_sqlite_hex(parts[1]),
                decode_sqlite_hex(parts[2]),
                decode_sqlite_hex(parts[3]),
                decode_sqlite_hex(parts[6]),
            ) else {
                tracing::warn!(rowid, "Apple Mail: skipping row with malformed hex text");
                continue;
            };
            let body = {
                let b = decoded_body.as_str();
                if b.len() > MAX_BODY_LENGTH {
                    // Round down to a char boundary; slicing mid-codepoint panics.
                    let end = floor_char_boundary(b, MAX_BODY_LENGTH);
                    format!("{}... (truncated)", &b[..end])
                } else {
                    b.to_string()
                }
            };
            let date_received: i64 = parts[4].parse().unwrap_or(0);
            let is_read = parts[5] == "1";

            messages.push(MailMessage {
                rowid,
                subject,
                sender,
                body,
                date_received,
                is_read,
                message_id,
            });
        }

        Ok(messages)
    }

    /// Mark a message as read via AppleScript.
    async fn mark_as_read_applescript(subject: &str, sender: &str) -> Result<(), ChannelError> {
        let mut cmd = tokio::process::Command::new("osascript");
        cmd.arg("-e")
            .arg(MARK_READ_APPLESCRIPT)
            .arg("--")
            .arg(subject)
            .arg(sender);
        let output = output_with_timeout(&mut cmd, "osascript mark-as-read")
            .await
            .map_err(|reason| ChannelError::SendFailed {
                name: NAME.to_string(),
                reason,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!("Mark-as-read AppleScript error: {stderr}");
        }

        Ok(())
    }

    /// Send a reply email via AppleScript.
    async fn send_reply(
        to: &str,
        subject: &str,
        body: &str,
        attachments: &[thinclaw_media::MediaContent],
    ) -> Result<(), ChannelError> {
        let temp_files = write_temp_attachments(attachments).await?;

        let mut cmd = tokio::process::Command::new("osascript");
        cmd.arg("-e")
            .arg(SEND_REPLY_APPLESCRIPT)
            .arg("--")
            .arg(to)
            .arg(subject)
            .arg(body)
            .args(&temp_files);
        let send_result = output_with_timeout(&mut cmd, "osascript send")
            .await
            .map_err(|reason| ChannelError::SendFailed {
                name: NAME.to_string(),
                reason,
            });

        // Clean up temp attachments regardless of send success/failure/timeout.
        cleanup_temp_attachments(&temp_files).await;
        let output = send_result?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: format!("osascript error: {stderr}"),
            });
        }

        Ok(())
    }

    /// Send a new (non-reply) email via AppleScript.
    async fn send_new_email(
        to: &str,
        subject: &str,
        body: &str,
        attachments: &[thinclaw_media::MediaContent],
    ) -> Result<(), ChannelError> {
        let temp_files = write_temp_attachments(attachments).await?;

        let mut cmd = tokio::process::Command::new("osascript");
        cmd.arg("-e")
            .arg(SEND_EMAIL_APPLESCRIPT)
            .arg("--")
            .arg(to)
            .arg(subject)
            .arg(body)
            .args(&temp_files);
        let send_result = output_with_timeout(&mut cmd, "osascript send")
            .await
            .map_err(|reason| ChannelError::SendFailed {
                name: NAME.to_string(),
                reason,
            });

        // Clean up temp attachments regardless of send success/failure/timeout.
        cleanup_temp_attachments(&temp_files).await;
        let output = send_result?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: format!("osascript error: {stderr}"),
            });
        }

        Ok(())
    }

    /// Extract sender email from a "Name <email>" or bare "email" format.
    fn extract_email(sender: &str) -> &str {
        // Use the last angle brackets and guard the range: a display name may
        // contain '<'/'>' and an unguarded `start > end` slice panics.
        if let Some(start) = sender.rfind('<')
            && let Some(end) = sender.rfind('>')
            && start < end
        {
            return &sender[start + 1..end];
        }
        sender
    }
}

#[async_trait]
impl Channel for AppleMailChannel {
    fn name(&self) -> &str {
        NAME
    }

    fn formatting_hints(&self) -> Option<String> {
        Some("Email supports rich formatting. Use HTML-compatible structure with headings. Keep emails scannable.".to_string())
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let (tx, rx) = mpsc::channel(64);
        if let Some(handle) = self.poll_task.lock().await.take() {
            self.shutdown.store(true, Ordering::Relaxed);
            self.shutdown_notify.notify_waiters();
            drain_channel_task(handle, NAME).await;
        }
        self.shutdown.store(false, Ordering::Relaxed);

        // Initialize to current latest ROWID so we don't replay history
        let initial_rowid = Self::get_latest_rowid(&self.db_path).await?;
        self.last_rowid.store(initial_rowid, Ordering::Relaxed);
        tracing::info!(
            "Apple Mail channel started, polling from ROWID {}",
            initial_rowid
        );

        let db_path = self.db_path.clone();
        let allow_from = self.config.allow_from.clone();
        let poll_interval = Duration::from_secs(self.config.poll_interval_secs);
        let unread_only = self.config.unread_only;
        let mark_as_read = self.config.mark_as_read;
        let shutdown = self.shutdown.clone();
        let shutdown_notify = Arc::clone(&self.shutdown_notify);
        let last_rowid = self.last_rowid.clone();

        // Spawn polling task
        let handle = tokio::spawn(async move {
            let mut seen: std::collections::HashSet<i64> = std::collections::HashSet::new();

            loop {
                if shutdown.load(Ordering::Relaxed) {
                    break;
                }

                let current_rowid = last_rowid.load(Ordering::Relaxed);

                match Self::poll_messages(&db_path, current_rowid, unread_only).await {
                    Ok(messages) => {
                        for msg in messages {
                            // Deduplication
                            if seen.contains(&msg.rowid) {
                                last_rowid.store(msg.rowid, Ordering::Relaxed);
                                continue;
                            }
                            seen.insert(msg.rowid);

                            // Evict old entries (keep last 500)
                            if seen.len() > 500 {
                                let min_rowid = *seen.iter().min().unwrap_or(&0);
                                seen.remove(&min_rowid);
                            }

                            // Check allow-list
                            let sender_email = Self::extract_email(&msg.sender);
                            if !allow_from.is_empty()
                                && !allow_from
                                    .iter()
                                    .any(|a| a == "*" || a.eq_ignore_ascii_case(sender_email))
                            {
                                last_rowid.store(msg.rowid, Ordering::Relaxed);
                                continue;
                            }

                            // Format the incoming message content
                            let content = format!(
                                "📧 Email from: {}\nSubject: {}\n\n{}",
                                msg.sender, msg.subject, msg.body
                            );

                            let incoming = IncomingMessage::new(NAME, sender_email, &content)
                                .with_metadata(serde_json::json!({
                                    "rowid": msg.rowid,
                                    "subject": msg.subject,
                                    "sender": msg.sender,
                                    "sender_email": sender_email,
                                    "date_received": msg.date_received,
                                    "message_id": msg.message_id,
                                    "is_read": msg.is_read,
                                    "channel_type": "email",
                                }));

                            last_rowid.store(msg.rowid, Ordering::Relaxed);

                            // Mark as read before sending to agent
                            if mark_as_read {
                                let _ = Self::mark_as_read_applescript(&msg.subject, sender_email)
                                    .await;
                            }

                            if tx.send(incoming).await.is_err() {
                                tracing::warn!("Apple Mail channel receiver dropped");
                                return;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("Apple Mail polling error: {e}");
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
        let sender_email = msg
            .metadata
            .get("sender_email")
            .and_then(|v| v.as_str())
            .unwrap_or(&msg.user_id);

        let subject = msg
            .metadata
            .get("subject")
            .and_then(|v| v.as_str())
            .unwrap_or("(no subject)");

        Self::send_reply(
            sender_email,
            subject,
            &response.content,
            &response.attachments,
        )
        .await
    }

    async fn send_status(
        &self,
        _status: StatusUpdate,
        _metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        // Email doesn't support typing indicators
        Ok(())
    }

    async fn broadcast(
        &self,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        // Only send to valid email addresses
        if !user_id.contains('@') || !user_id.contains('.') {
            tracing::debug!(
                recipient = user_id,
                "Apple Mail: skipping broadcast — recipient is not an email address"
            );
            return Ok(());
        }
        Self::send_new_email(
            user_id,
            "ThinClaw Agent Notification",
            &response.content,
            &response.attachments,
        )
        .await
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        if self.db_path.exists() {
            Ok(())
        } else {
            Err(ChannelError::HealthCheckFailed {
                name: NAME.to_string(),
            })
        }
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

/// Find the Envelope Index database in ~/Library/Mail/.
///
/// Mail.app stores its database in a versioned directory,
/// e.g. ~/Library/Mail/V10/MailData/Envelope Index
fn find_envelope_index() -> Result<PathBuf, ChannelError> {
    let home = dirs::home_dir().ok_or(ChannelError::Configuration(
        "Cannot determine home directory".to_string(),
    ))?;
    let mail_dir = home.join("Library/Mail");

    // Try known versions in reverse order (V10, V9, V8, ...)
    for version in (4..=12).rev() {
        let candidate = mail_dir.join(format!("V{version}/MailData/Envelope Index"));
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    // Fallback: try glob for any version
    let fallback = mail_dir.join("V10/MailData/Envelope Index");
    Err(ChannelError::Configuration(format!(
        "Mail Envelope Index not found. Tried ~/Library/Mail/V{{4-12}}/MailData/Envelope Index. \
         Expected at: {}. Ensure Mail.app is configured and Full Disk Access is granted.",
        fallback.display()
    )))
}

async fn write_temp_attachments(
    attachments: &[thinclaw_media::MediaContent],
) -> Result<Vec<std::path::PathBuf>, ChannelError> {
    if attachments.len() > MAX_OUTBOUND_ATTACHMENTS {
        return Err(ChannelError::SendFailed {
            name: NAME.to_string(),
            reason: format!("at most {MAX_OUTBOUND_ATTACHMENTS} attachments may be sent at once"),
        });
    }

    let mut paths = Vec::new();
    let mut total_bytes = 0_usize;
    for attachment in attachments {
        if attachment.data.len() > MAX_OUTBOUND_ATTACHMENT_BYTES {
            cleanup_temp_attachments(&paths).await;
            return Err(ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: format!(
                    "attachment exceeds the {MAX_OUTBOUND_ATTACHMENT_BYTES}-byte limit"
                ),
            });
        }
        total_bytes = total_bytes
            .checked_add(attachment.data.len())
            .ok_or_else(|| ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: "attachment byte count overflowed".to_string(),
            })?;
        if total_bytes > MAX_OUTBOUND_ATTACHMENTS_TOTAL_BYTES {
            cleanup_temp_attachments(&paths).await;
            return Err(ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: format!(
                    "attachments exceed the {MAX_OUTBOUND_ATTACHMENTS_TOTAL_BYTES}-byte total limit"
                ),
            });
        }

        let filename = attachment.filename.as_deref().unwrap_or("attachment");
        let safe_name = safe_attachment_filename(filename);
        let path =
            std::env::temp_dir().join(format!("thinclaw-{}-{safe_name}", uuid::Uuid::new_v4()));

        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        let write_result = async {
            let mut file = options.open(&path).await?;
            file.write_all(&attachment.data).await?;
            file.sync_all().await
        }
        .await;
        if let Err(error) = write_result {
            let _ = tokio::fs::remove_file(&path).await;
            cleanup_temp_attachments(&paths).await;
            return Err(ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: format!(
                    "failed to write temp attachment {}: {error}",
                    path.display()
                ),
            });
        }
        paths.push(path);
    }
    Ok(paths)
}

fn safe_attachment_filename(filename: &str) -> String {
    let base = std::path::Path::new(filename)
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

async fn cleanup_temp_attachments(paths: &[std::path::PathBuf]) {
    for path in paths {
        let _ = tokio::fs::remove_file(path).await;
    }
}

/// Ensure a macOS application is running. If it's not running, launch it
/// in the background. This is used to auto-start Messages.app and Mail.app
/// when their channels are active.
pub async fn ensure_app_running(app_name: &str) -> bool {
    // Check if already running
    let mut pgrep = tokio::process::Command::new("pgrep");
    pgrep.arg("-x").arg(app_name);
    let running = output_with_timeout(&mut pgrep, "pgrep")
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);

    if running {
        return true;
    }

    tracing::info!("{app_name}.app is not running — launching it...");

    // `open` receives the application name as an argv value, so even a future
    // non-constant caller cannot turn it into executable AppleScript source.
    let mut launch = tokio::process::Command::new("open");
    launch.args(["-g", "-j", "-a"]).arg(app_name);
    let result = output_with_timeout(&mut launch, "open application").await;

    match result {
        Ok(output) if output.status.success() => {
            tracing::info!("{app_name}.app launched successfully");
            // Give it a few seconds to initialize
            tokio::time::sleep(Duration::from_secs(3)).await;
            true
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!("Failed to launch {app_name}.app: {stderr}");
            false
        }
        Err(e) => {
            tracing::warn!("Failed to launch {app_name}.app: {e}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Mutex;
    use tempfile::tempdir;

    static APPLE_MAIL_HOME_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_extract_email_angle_brackets() {
        assert_eq!(
            AppleMailChannel::extract_email("John Doe <john@example.com>"),
            "john@example.com"
        );
    }

    #[test]
    fn test_extract_email_bare() {
        assert_eq!(
            AppleMailChannel::extract_email("john@example.com"),
            "john@example.com"
        );
    }

    #[test]
    fn test_extract_email_missing_brackets() {
        assert_eq!(
            AppleMailChannel::extract_email("john <john@example.com"),
            "john <john@example.com"
        );
    }

    #[test]
    fn test_extract_email_empty() {
        assert_eq!(AppleMailChannel::extract_email("unknown"), "unknown");
    }

    #[test]
    fn test_extract_email_gt_before_lt_does_not_panic() {
        // Display name containing '>' before the real '<' must not panic.
        assert_eq!(
            AppleMailChannel::extract_email("\"a > b\" <x@y.com>"),
            "x@y.com"
        );
    }

    #[test]
    fn test_find_envelope_index_found() {
        let _guard = APPLE_MAIL_HOME_LOCK.lock().unwrap();

        let home = tempdir().unwrap();
        let mail_dir = home
            .path()
            .join("Library")
            .join("Mail")
            .join("V12")
            .join("MailData");
        fs::create_dir_all(&mail_dir).unwrap();
        let expected = mail_dir.join("Envelope Index");
        fs::write(&expected, b"").unwrap();

        let original_home = std::env::var_os("HOME");
        // SAFETY: Tests are sequential within this locked section and restore HOME immediately after use.
        unsafe {
            std::env::set_var("HOME", home.path());
        }
        let found = find_envelope_index().unwrap();
        assert_eq!(found, expected);

        match original_home {
            Some(home) => unsafe {
                std::env::set_var("HOME", home);
            },
            None => unsafe {
                std::env::remove_var("HOME");
            },
        }
    }

    #[test]
    fn test_find_envelope_index_missing() {
        let _guard = APPLE_MAIL_HOME_LOCK.lock().unwrap();

        let home = tempdir().unwrap();
        let original_home = std::env::var_os("HOME");
        unsafe {
            std::env::set_var("HOME", home.path());
        }
        let result = find_envelope_index();
        assert!(result.is_err());

        match original_home {
            Some(home) => unsafe {
                std::env::set_var("HOME", home);
            },
            None => unsafe {
                std::env::remove_var("HOME");
            },
        }
    }

    #[test]
    fn test_default_config() {
        let config = AppleMailConfig::default();
        assert!(config.db_path.is_none());
        assert!(config.allow_from.is_empty());
        assert_eq!(config.poll_interval_secs, POLL_INTERVAL_SECS);
        assert_eq!(config.mailbox, "INBOX");
        assert!(config.unread_only);
        assert!(config.mark_as_read);
    }

    #[test]
    fn test_diagnostic_serializable() {
        let diag = AppleMailDiagnostic {
            db_exists: true,
            db_path: "/Users/test/Library/Mail/V10/MailData/Envelope Index".to_string(),
            sqlite3_available: true,
            osascript_available: true,
            mail_running: false,
            total_messages: Some(5432),
            errors: vec!["Mail.app is not running".into()],
        };
        let json = serde_json::to_string(&diag).unwrap();
        assert!(json.contains("\"db_exists\":true"));
        assert!(json.contains("5432"));
    }

    #[test]
    fn formatting_hints_describe_rich_email_output() {
        let config = AppleMailConfig::default();
        let channel = AppleMailChannel {
            config,
            db_path: std::path::PathBuf::from("/tmp/Envelope Index"),
            shutdown: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            shutdown_notify: std::sync::Arc::new(tokio::sync::Notify::new()),
            poll_task: tokio::sync::Mutex::new(None),
            last_rowid: std::sync::Arc::new(std::sync::atomic::AtomicI64::new(0)),
        };
        assert_eq!(
            channel.formatting_hints().as_deref(),
            Some(
                "Email supports rich formatting. Use HTML-compatible structure with headings. Keep emails scannable."
            )
        );
    }
}
