//! Session management CLI commands.
//!
//! Subcommands:
//! - `sessions list`     — list all active sessions
//! - `sessions show`     — show session details (threads, owners)
//! - `sessions prune`    — force-prune stale sessions
//! - `sessions export`   — export a session transcript

use std::sync::Arc;

use clap::Subcommand;

use crate::agent::SessionManager;

#[derive(Subcommand, Debug, Clone)]
pub enum SessionCommand {
    /// List all active sessions
    List {
        /// Output format: table (default) or json
        #[arg(long, default_value = "table")]
        format: String,
    },

    /// Show details for a session
    Show {
        /// User ID to look up
        user_id: String,

        /// Channel to filter by
        #[arg(long)]
        channel: Option<String>,
    },

    /// Force-prune sessions idle longer than the given threshold
    Prune {
        /// Idle threshold in seconds (default: 3600 = 1 hour)
        #[arg(long, default_value = "3600")]
        idle_secs: u64,
    },

    /// Export a session transcript
    Export {
        /// User ID whose session to export
        user_id: String,

        /// Channel to export from
        #[arg(long, default_value = "cli")]
        channel: String,

        /// Output format: markdown (default) or json
        #[arg(long, default_value = "markdown")]
        format: String,

        /// Output file path (prints to stdout if not specified)
        #[arg(short, long)]
        output: Option<String>,
    },
}

/// Run a sessions CLI command against the given session manager.
pub async fn run_sessions_command(cmd: SessionCommand, session_mgr: &Arc<SessionManager>) {
    match cmd {
        SessionCommand::List { format } => list_sessions(session_mgr, &format).await,
        SessionCommand::Show { user_id, channel } => {
            show_session(session_mgr, &user_id, channel.as_deref()).await;
        }
        SessionCommand::Prune { idle_secs } => {
            let duration = std::time::Duration::from_secs(idle_secs);
            let pruned = session_mgr.prune_stale_sessions(duration).await;
            println!("Pruned {} stale session(s).", pruned);
        }
        SessionCommand::Export {
            user_id,
            channel,
            format,
            output,
        } => {
            export_session(session_mgr, &user_id, &channel, &format, output.as_deref()).await;
        }
    }
}

async fn list_sessions(session_mgr: &Arc<SessionManager>, format: &str) {
    let summary = session_mgr.list_sessions().await;

    if summary.is_empty() {
        println!("No active sessions.");
        return;
    }

    if format == "json" {
        println!(
            "{}",
            serde_json::to_string_pretty(&summary).unwrap_or_default()
        );
        return;
    }

    println!(
        "{:<20}  {:<14}  {:<10}  {:<20}  OWNER",
        "USER", "CHANNEL", "THREADS", "LAST ACTIVE"
    );
    println!("{}", "-".repeat(80));

    for session in &summary {
        let user = session["user_id"].as_str().unwrap_or("?");
        let channel = session["channel"].as_str().unwrap_or("?");
        let threads = session["thread_count"].as_u64().unwrap_or(0);
        let last_active = session["last_active"].as_str().unwrap_or("—");
        let owner = session["owner"].as_str().unwrap_or("—");

        println!(
            "{:<20}  {:<14}  {:<10}  {:<20}  {}",
            user, channel, threads, last_active, owner
        );
    }

    println!("\n{} session(s) active.", summary.len());
}

async fn show_session(session_mgr: &Arc<SessionManager>, user_id: &str, channel: Option<&str>) {
    let detail = session_mgr
        .describe_session(user_id, channel.unwrap_or("cli"))
        .await;

    match detail {
        Some(info) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&info).unwrap_or_default()
            );
        }
        None => {
            let ch = channel.unwrap_or("cli");
            eprintln!(
                "No session found for user '{}' on channel '{}'.",
                user_id, ch
            );
        }
    }
}

async fn export_session(
    session_mgr: &Arc<SessionManager>,
    user_id: &str,
    channel: &str,
    format: &str,
    output: Option<&str>,
) {
    let detail = session_mgr.describe_session(user_id, channel).await;

    let info = match detail {
        Some(info) => info,
        Option::None => {
            eprintln!(
                "No session found for user '{}' on channel '{}'.",
                user_id, channel
            );
            return;
        }
    };

    let transcript = match format {
        "json" => serde_json::to_string_pretty(&info).unwrap_or_default(),
        _ => format_as_markdown(user_id, channel, &info),
    };

    match output {
        Some(path) => {
            if let Err(e) = std::fs::write(path, &transcript) {
                eprintln!("Failed to write to '{}': {}", path, e);
            } else {
                println!("Session exported to '{}'.", path);
            }
        }
        Option::None => println!("{}", transcript),
    }
}

/// Format session data as a markdown transcript.
fn format_as_markdown(user_id: &str, channel: &str, info: &serde_json::Value) -> String {
    let mut md = String::new();
    md.push_str("# Session Transcript\n\n");
    md.push_str(&format!("- **User:** {}\n", user_id));
    md.push_str(&format!("- **Channel:** {}\n", channel));

    if let Some(exported) = chrono::Utc::now().to_rfc3339().into() {
        md.push_str(&format!("- **Exported:** {}\n", exported));
    }

    md.push_str("\n---\n\n");

    // If the session info contains messages, format them
    if let Some(messages) = info.get("messages").and_then(|m| m.as_array()) {
        for msg in messages {
            let role = msg
                .get("role")
                .and_then(|r| r.as_str())
                .unwrap_or("unknown");
            let content = msg.get("content").and_then(|c| c.as_str()).unwrap_or("");
            let timestamp = msg.get("timestamp").and_then(|t| t.as_str()).unwrap_or("");

            let role_label = match role {
                "user" => "🧑 User",
                "assistant" => "🤖 Assistant",
                "system" => "⚙️ System",
                "tool" => "🔧 Tool",
                _ => role,
            };

            if !timestamp.is_empty() {
                md.push_str(&format!("### {} ({})\n\n", role_label, timestamp));
            } else {
                md.push_str(&format!("### {}\n\n", role_label));
            }

            md.push_str(content);
            md.push_str("\n\n");
        }
    } else {
        // Fallback: dump the entire session info as JSON
        md.push_str("```json\n");
        md.push_str(&serde_json::to_string_pretty(info).unwrap_or_default());
        md.push_str("\n```\n");
    }

    md
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_list_empty_sessions() {
        let mgr = Arc::new(SessionManager::new());
        // Just verify it doesn't panic
        run_sessions_command(
            SessionCommand::List {
                format: "table".to_string(),
            },
            &mgr,
        )
        .await;
    }

    #[tokio::test]
    async fn test_prune_sessions() {
        let mgr = Arc::new(SessionManager::new());
        run_sessions_command(SessionCommand::Prune { idle_secs: 60 }, &mgr).await;
    }

    #[test]
    fn test_format_as_markdown_fallback() {
        let info = serde_json::json!({
            "user_id": "test-user",
            "channel": "cli",
            "thread_count": 0
        });
        let md = format_as_markdown("test-user", "cli", &info);
        assert!(md.contains("# Session Transcript"));
        assert!(md.contains("**User:** test-user"));
        assert!(md.contains("**Channel:** cli"));
        // No messages → should contain JSON fallback
        assert!(md.contains("```json"));
    }

    #[test]
    fn test_format_as_markdown_with_messages() {
        let info = serde_json::json!({
            "messages": [
                {"role": "user", "content": "Hello!", "timestamp": "2026-03-03T12:00:00Z"},
                {"role": "assistant", "content": "Hi there!", "timestamp": "2026-03-03T12:00:01Z"},
            ]
        });
        let md = format_as_markdown("user-1", "cli", &info);
        assert!(md.contains("🧑 User"));
        assert!(md.contains("🤖 Assistant"));
        assert!(md.contains("Hello!"));
        assert!(md.contains("Hi there!"));
    }
}
