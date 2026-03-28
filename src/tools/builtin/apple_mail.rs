//! Apple Mail search and send tool (macOS only).
//!
//! Provides two operations for the Apple Mail Envelope Index:
//! - `search`: Query emails by sender, subject, or date range
//! - `send`: Compose and send an email via Mail.app (AppleScript)

use std::path::PathBuf;

use async_trait::async_trait;

use crate::context::JobContext;
use crate::tools::tool::{Tool, ToolDomain, ToolError, ToolOutput};

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
        let home = dirs::home_dir()?;
        // Walk ~/Library/Mail/V*/MailData/Envelope Index
        let mail_dir = home.join("Library").join("Mail");
        if let Ok(entries) = std::fs::read_dir(&mail_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                if name.to_string_lossy().starts_with('V') {
                    let db = entry
                        .path()
                        .join("MailData")
                        .join("Envelope Index");
                    if db.exists() {
                        return Some(Self { db_path: db });
                    }
                }
            }
        }
        None
    }

    async fn search(
        &self,
        sender: Option<&str>,
        subject: Option<&str>,
        limit: u32,
        unread_only: bool,
    ) -> Result<String, ToolError> {
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
                    COALESCE(sub.subject, '(no subject)'), \
                    COALESCE(a.address, 'unknown'), \
                    COALESCE(summ.summary, ''), \
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

        let output = tokio::process::Command::new("sqlite3")
            .arg("-separator")
            .arg("|")
            .arg(&self.db_path)
            .arg(&query)
            .output()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("sqlite3 failed: {e}")))?;

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
                parts[2],
                parts[1],
                if parts[3].is_empty() {
                    "(no preview)"
                } else {
                    parts[3]
                }
            ));
        }

        Ok(format!(
            "Found {} email(s):\n\n{}",
            results.len(),
            results.join("\n\n")
        ))
    }

    async fn send_email(
        &self,
        to: &str,
        subject: &str,
        body: &str,
    ) -> Result<String, ToolError> {
        let escaped_to = to.replace('"', "\\\"");
        let escaped_subject = subject.replace('"', "\\\"");
        let escaped_body = body.replace('"', "\\\"").replace('\n', "\\n");

        let script = format!(
            r#"tell application "Mail"
    set newMessage to make new outgoing message with properties {{subject:"{escaped_subject}", content:"{escaped_body}", visible:true}}
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
            .map_err(|e| ToolError::ExecutionFailed(format!("osascript failed: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ToolError::ExecutionFailed(format!(
                "Failed to send email: {stderr}"
            )));
        }

        Ok(format!("Email sent to {to} with subject \"{subject}\""))
    }
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
                    .unwrap_or(20) as u32;

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
                let body = params
                    .get("body")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                let result = self.send_email(to, subject, body).await?;
                Ok(ToolOutput::text(result, start.elapsed()))
            }
            other => Err(ToolError::InvalidParameters(format!(
                "Unknown operation: '{other}'. Use 'search' or 'send'."
            ))),
        }
    }
}
