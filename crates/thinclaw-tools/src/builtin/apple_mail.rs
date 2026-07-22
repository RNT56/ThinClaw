//! Apple Mail search and send tool (macOS only).
//!
//! Provides two operations for the Apple Mail Envelope Index:
//! - `search`: Query emails by sender, subject, or date range
//! - `send`: Compose and send an email via Mail.app (AppleScript)

use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;

use thinclaw_tools_core::{
    ApprovalRequirement, Tool, ToolDomain, ToolError, ToolOutput, ToolRateLimitConfig,
};
use thinclaw_types::JobContext;

use crate::execution::bounded_command_output;

const LOCAL_UTILITY_TIMEOUT: Duration = Duration::from_secs(20);
const SQLITE_STDOUT_LIMIT: usize = 2 * 1024 * 1024;
const LOCAL_STDERR_LIMIT: usize = 256 * 1024;
const MAX_SEARCH_FILTER_BYTES: usize = 2048;
const MAX_SEARCH_RESULTS: u32 = 100;
const MAX_RECIPIENT_BYTES: usize = 320;
const MAX_SUBJECT_BYTES: usize = 998;
const MAX_BODY_BYTES: usize = 1024 * 1024;
const MAX_MAIL_DIRECTORY_ENTRIES: usize = 128;

const SEND_EMAIL_APPLESCRIPT: &str = r#"on run argv
    set recipientAddress to item 1 of argv
    set messageSubject to item 2 of argv
    set messageBody to item 3 of argv
    tell application "Mail"
        set newMessage to make new outgoing message with properties {subject:my messageSubject, content:my messageBody, visible:false}
        tell newMessage
            make new to recipient at end of to recipients with properties {address:my recipientAddress}
        end tell
        send newMessage
    end tell
end run"#;

/// Built-in tool for searching Apple Mail's Envelope Index and sending mail.
pub struct AppleMailTool {
    db_path: PathBuf,
}

impl AppleMailTool {
    pub fn new(db_path: PathBuf) -> Self {
        Self { db_path }
    }

    /// Auto-detect the Envelope Index path.
    pub fn auto_detect() -> Option<Self> {
        let home = std::env::var_os("HOME").map(PathBuf::from)?;
        // Walk ~/Library/Mail/V*/MailData/Envelope Index
        let mail_dir = home.join("Library").join("Mail");
        if !is_real_directory(&mail_dir) {
            return None;
        }
        let canonical_mail_dir = mail_dir.canonicalize().ok()?;
        let entries = std::fs::read_dir(&mail_dir).ok()?;
        let mut candidates = Vec::new();
        for (index, entry) in entries.enumerate() {
            if index >= MAX_MAIL_DIRECTORY_ENTRIES {
                return None;
            }
            let entry = entry.ok()?;
            let Some(version) = mail_version(&entry.file_name()) else {
                continue;
            };
            if !is_real_directory(&entry.path()) {
                continue;
            }
            let mail_data = entry.path().join("MailData");
            if !is_real_directory(&mail_data) {
                continue;
            }
            let db = mail_data.join("Envelope Index");
            if !is_single_link_regular_file(&db) {
                continue;
            }
            let canonical_db = db.canonicalize().ok()?;
            if canonical_db.starts_with(&canonical_mail_dir) {
                candidates.push((version, canonical_db));
            }
        }
        candidates.sort_by(|left, right| right.0.cmp(&left.0));
        candidates
            .into_iter()
            .next()
            .map(|(_, db_path)| Self { db_path })
    }

    async fn search(
        &self,
        sender: Option<&str>,
        subject: Option<&str>,
        limit: u32,
        unread_only: bool,
    ) -> Result<String, ToolError> {
        for (label, value) in [("sender", sender), ("subject", subject)] {
            if value.is_some_and(|value| {
                value.len() > MAX_SEARCH_FILTER_BYTES || value.chars().any(char::is_control)
            }) {
                return Err(ToolError::InvalidParameters(format!(
                    "{label} filter is malformed or exceeds {MAX_SEARCH_FILTER_BYTES} bytes"
                )));
            }
        }
        let limit = limit.clamp(1, MAX_SEARCH_RESULTS);
        let mut conditions = vec!["m.deleted = 0".to_string()];

        if let Some(s) = sender {
            let escaped = s.replace('\'', "''");
            conditions.push(format!("a.address LIKE '%{escaped}%'"));
        }
        if let Some(s) = subject {
            let escaped = s.replace('\'', "''");
            conditions.push(format!("sub.subject LIKE '%{escaped}%'"));
        }
        if unread_only {
            conditions.push("m.read = 0".to_string());
        }

        let where_clause = conditions.join(" AND ");
        let query = format!(
            "SELECT m.ROWID, \
                    hex(CAST(substr(COALESCE(sub.subject, '(no subject)'), 1, 998) AS TEXT)), \
                    hex(CAST(substr(COALESCE(a.address, 'unknown'), 1, 2048) AS TEXT)), \
                    hex(CAST(substr(COALESCE(summ.summary, ''), 1, 4096) AS TEXT)), \
                    COALESCE(m.date_received, 0), \
                    m.read \
             FROM messages m \
             LEFT JOIN subjects sub ON m.subject = sub.ROWID \
             LEFT JOIN summaries summ ON m.summary = summ.ROWID \
             LEFT JOIN addresses a ON m.sender = a.ROWID \
             WHERE {where_clause} \
             ORDER BY m.date_received DESC \
             LIMIT {limit};"
        );

        let mut command = tokio::process::Command::new("sqlite3");
        command
            .arg("-separator")
            .arg("|")
            .arg(&self.db_path)
            .arg(&query);
        let output = bounded_command_output(
            &mut command,
            LOCAL_UTILITY_TIMEOUT,
            SQLITE_STDOUT_LIMIT,
            LOCAL_STDERR_LIMIT,
            "Apple Mail sqlite query",
        )
        .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ToolError::ExecutionFailed(format!(
                "sqlite3 error: {stderr}"
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.trim().is_empty() {
            return Ok("No emails found matching the criteria.".to_string());
        }

        let mut results = Vec::new();
        for line in stdout.lines() {
            let parts: Vec<&str> = line.splitn(6, '|').collect();
            if parts.len() < 6 {
                continue;
            }
            let (Some(subject), Some(sender), Some(summary)) = (
                decode_sqlite_hex(parts[1]),
                decode_sqlite_hex(parts[2]),
                decode_sqlite_hex(parts[3]),
            ) else {
                continue;
            };
            let read_status = if parts[5] == "1" { "read" } else { "UNREAD" };
            let date_ts: i64 = parts[4].parse().unwrap_or(0);
            // Apple Mail's Envelope Index stores date_received as Unix
            // timestamps (seconds since 1970-01-01), NOT Core Data timestamps.
            let date_str = if date_ts > 0 {
                chrono::DateTime::from_timestamp(date_ts, 0)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| format!("ts:{date_ts}"))
            } else {
                "unknown date".to_string()
            };

            results.push(format!(
                "• [{read_status}] {date_str} — From: {} — Subject: {}\n  Preview: {}",
                sender,
                subject,
                if summary.is_empty() {
                    "(no preview)"
                } else {
                    &summary
                }
            ));
        }

        Ok(format!(
            "Found {} email(s):\n\n{}",
            results.len(),
            results.join("\n\n")
        ))
    }

    async fn send_email(&self, to: &str, subject: &str, body: &str) -> Result<String, ToolError> {
        if to.is_empty()
            || to.len() > MAX_RECIPIENT_BYTES
            || !to.contains('@')
            || to.chars().any(char::is_control)
        {
            return Err(ToolError::InvalidParameters(
                "recipient is not a valid bounded email address".to_string(),
            ));
        }
        if subject.len() > MAX_SUBJECT_BYTES || subject.chars().any(|char| char == '\0') {
            return Err(ToolError::InvalidParameters(format!(
                "subject exceeds {MAX_SUBJECT_BYTES} bytes or contains NUL"
            )));
        }
        if body.len() > MAX_BODY_BYTES || body.chars().any(|char| char == '\0') {
            return Err(ToolError::InvalidParameters(format!(
                "body exceeds {MAX_BODY_BYTES} bytes or contains NUL"
            )));
        }

        let mut command = tokio::process::Command::new("osascript");
        command
            .arg("-e")
            .arg(SEND_EMAIL_APPLESCRIPT)
            .arg("--")
            .arg(to)
            .arg(subject)
            .arg(body);
        let output = bounded_command_output(
            &mut command,
            LOCAL_UTILITY_TIMEOUT,
            64 * 1024,
            LOCAL_STDERR_LIMIT,
            "Apple Mail send",
        )
        .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ToolError::ExecutionFailed(format!(
                "Failed to send email: {stderr}"
            )));
        }

        Ok(format!("Email sent to {to} with subject \"{subject}\""))
    }
}

fn mail_version(name: &std::ffi::OsStr) -> Option<u32> {
    let value = name.to_str()?.strip_prefix('V')?;
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

fn is_real_directory(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
}

fn is_single_link_regular_file(path: &Path) -> bool {
    thinclaw_platform::fs::regular_file_has_single_link(path).is_ok_and(|single| single)
}

#[async_trait]
impl Tool for AppleMailTool {
    fn name(&self) -> &str {
        "apple_mail"
    }

    fn description(&self) -> &str {
        "Search emails in Apple Mail and send new emails via Mail.app. \
         Operations: 'search' (query inbox by sender/subject/unread status) \
         and 'send' (compose and send an email). No API key required — \
         uses the local Mail.app on macOS."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["search", "send"],
                    "description": "Operation to perform: 'search' to query emails, 'send' to send an email"
                },
                "sender": {
                    "type": "string",
                    "description": "(search) Filter by sender email address (partial match)"
                },
                "subject": {
                    "type": "string",
                    "description": "(search/send) Filter by subject (partial match for search, full subject for send)"
                },
                "unread_only": {
                    "type": "boolean",
                    "description": "(search) Only return unread emails. Default: false"
                },
                "limit": {
                    "type": "integer",
                    "description": "(search) Maximum number of results. Default: 20"
                },
                "to": {
                    "type": "string",
                    "description": "(send) Recipient email address"
                },
                "body": {
                    "type": "string",
                    "description": "(send) Email body content"
                }
            },
            "required": ["operation"]
        })
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }

    fn requires_approval(&self, params: &serde_json::Value) -> ApprovalRequirement {
        if params.get("operation").and_then(|value| value.as_str()) == Some("send") {
            ApprovalRequirement::UnlessAutoApproved
        } else {
            ApprovalRequirement::Never
        }
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        Some(ToolRateLimitConfig::new(10, 60))
    }

    fn execution_timeout(&self) -> Duration {
        Duration::from_secs(30)
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let operation = params
            .get("operation")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidParameters("Missing required parameter: operation".to_string())
            })?;

        match operation {
            "search" => {
                let sender = params.get("sender").and_then(|v| v.as_str());
                let subject = params.get("subject").and_then(|v| v.as_str());
                let unread_only = params
                    .get("unread_only")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let limit = params
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(20)
                    .min(u32::MAX as u64) as u32;

                let result = self.search(sender, subject, limit, unread_only).await?;
                Ok(ToolOutput::text(result, start.elapsed()))
            }
            "send" => {
                let to = params.get("to").and_then(|v| v.as_str()).ok_or_else(|| {
                    ToolError::InvalidParameters(
                        "Missing required parameter for send: 'to'".to_string(),
                    )
                })?;
                let subject = params
                    .get("subject")
                    .and_then(|v| v.as_str())
                    .unwrap_or("(no subject)");
                let body = params.get("body").and_then(|v| v.as_str()).unwrap_or("");

                let result = self.send_email(to, subject, body).await?;
                Ok(ToolOutput::text(result, start.elapsed()))
            }
            other => Err(ToolError::InvalidParameters(format!(
                "Unknown operation: '{other}'. Use 'search' or 'send'."
            ))),
        }
    }
}

fn decode_sqlite_hex(value: &str) -> Option<String> {
    String::from_utf8(hex::decode(value).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_hex_round_trips_row_delimiters() {
        let value = "subject|with\nnewlines\0🙂";
        assert_eq!(
            decode_sqlite_hex(&hex::encode(value)),
            Some(value.to_string())
        );
    }

    #[test]
    fn sending_mail_requires_approval() {
        let tool = AppleMailTool::new(PathBuf::from("Envelope Index"));
        assert_eq!(
            tool.requires_approval(&serde_json::json!({"operation": "send"})),
            ApprovalRequirement::UnlessAutoApproved
        );
        assert_eq!(
            tool.requires_approval(&serde_json::json!({"operation": "search"})),
            ApprovalRequirement::Never
        );
    }

    #[test]
    fn mail_version_names_are_strict() {
        assert_eq!(mail_version(std::ffi::OsStr::new("V10")), Some(10));
        assert_eq!(mail_version(std::ffi::OsStr::new("V2")), Some(2));
        assert_eq!(mail_version(std::ffi::OsStr::new("V10-old")), None);
        assert_eq!(mail_version(std::ffi::OsStr::new("MailData")), None);
    }
}
