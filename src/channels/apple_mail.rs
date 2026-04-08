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
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use super::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use crate::error::ChannelError;

/// Channel name constant.
const NAME: &str = "apple_mail";

/// Default polling interval in seconds.
const POLL_INTERVAL_SECS: u64 = 10;

/// Maximum email body length.
const MAX_BODY_LENGTH: usize = 100_000;

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

        // Check Mail.app running
        let mail_running = tokio::process::Command::new("pgrep")
            .arg("-x")
            .arg("Mail")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !mail_running {
            errors.push("Mail.app is not running (required for sending)".to_string());
        }

        // Get total message count
        let total_messages = if db_exists && sqlite3_available {
            tokio::process::Command::new("sqlite3")
                .arg(&db_path)
                .arg("SELECT COUNT(*) FROM messages;")
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
        let output = tokio::process::Command::new("sqlite3")
            .arg(db_path)
            .arg("SELECT MAX(ROWID) FROM messages;")
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
                    REPLACE(COALESCE(sub.subject, '(no subject)'), '|', ' '), \
                    COALESCE(a.address, 'unknown'), \
                    COALESCE(summ.summary, ''), \
                    COALESCE(m.date_received, 0), \
                    COALESCE(m.read, 0), \
                    COALESCE(m.message_id, 0) \
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
            let subject = parts[1].to_string();
            let sender = parts[2].to_string();
            let body = {
                let b = parts[3];
                if b.len() > MAX_BODY_LENGTH {
                    format!("{}... (truncated)", &b[..MAX_BODY_LENGTH])
                } else {
                    b.to_string()
                }
            };
            let date_received: i64 = parts[4].parse().unwrap_or(0);
            let is_read = parts[5] == "1";
            let message_id = parts[6].to_string();

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
        let escaped_subject = escape_applescript(subject);
        let escaped_sender = escape_applescript(sender);

        let script = format!(
            r#"tell application "Mail"
    set foundMessages to (every message of inbox whose subject is "{escaped_subject}" and sender contains "{escaped_sender}")
    repeat with msg in foundMessages
        set read status of msg to true
    end repeat
end tell"#
        );

        let output = tokio::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: format!("osascript mark-as-read failed: {e}"),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!("Mark-as-read AppleScript error: {stderr}");
        }

        Ok(())
    }

    /// Send a reply email via AppleScript.
    async fn send_reply(to: &str, subject: &str, body: &str) -> Result<(), ChannelError> {
        let escaped_to = escape_applescript(to);
        let escaped_subject = escape_applescript(subject);
        let escaped_body = escape_applescript(body);

        let script = format!(
            r#"tell application "Mail"
    set newMessage to make new outgoing message with properties {{subject:"Re: {escaped_subject}", content:"{escaped_body}", visible:false}}
    tell newMessage
        make new to recipient at end of to recipients with properties {{address:"{escaped_to}"}}
    end tell
    send newMessage
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

        Ok(())
    }

    /// Send a new (non-reply) email via AppleScript.
    async fn send_new_email(to: &str, subject: &str, body: &str) -> Result<(), ChannelError> {
        let escaped_to = escape_applescript(to);
        let escaped_subject = escape_applescript(subject);
        let escaped_body = escape_applescript(body);

        let script = format!(
            r#"tell application "Mail"
    set newMessage to make new outgoing message with properties {{subject:"{escaped_subject}", content:"{escaped_body}", visible:false}}
    tell newMessage
        make new to recipient at end of to recipients with properties {{address:"{escaped_to}"}}
    end tell
    send newMessage
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

        Ok(())
    }

    /// Extract sender email from a "Name <email>" or bare "email" format.
    fn extract_email(sender: &str) -> &str {
        if let Some(start) = sender.find('<') {
            if let Some(end) = sender.find('>') {
                return &sender[start + 1..end];
            }
        }
        sender
    }
}

#[async_trait]
impl Channel for AppleMailChannel {
    fn name(&self) -> &str {
        NAME
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let (tx, rx) = mpsc::channel(64);

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
        let last_rowid = self.last_rowid.clone();

        // Spawn polling task
        tokio::spawn(async move {
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

        Self::send_reply(sender_email, subject, &response.content).await
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
        Self::send_new_email(user_id, "ThinClaw Agent Notification", &response.content).await
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
        Ok(())
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

/// Escape text for safe inclusion in AppleScript strings.
fn escape_applescript(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Ensure a macOS application is running. If it's not running, launch it
/// in the background. This is used to auto-start Messages.app and Mail.app
/// when their channels are active.
pub async fn ensure_app_running(app_name: &str) -> bool {
    // Check if already running
    let running = tokio::process::Command::new("pgrep")
        .arg("-x")
        .arg(app_name)
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);

    if running {
        return true;
    }

    tracing::info!("{app_name}.app is not running — launching it...");

    // Launch the app minimized (no window activation on headless)
    let result = tokio::process::Command::new("osascript")
        .arg("-e")
        .arg(format!(r#"tell application "{app_name}" to launch"#))
        .output()
        .await;

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
    fn test_extract_email_empty() {
        assert_eq!(AppleMailChannel::extract_email("unknown"), "unknown");
    }

    #[test]
    fn test_escape_applescript() {
        assert_eq!(escape_applescript(r#"say "hello""#), r#"say \"hello\""#);
        assert_eq!(escape_applescript("back\\slash"), "back\\\\slash");
        assert_eq!(escape_applescript("normal"), "normal");
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
}
