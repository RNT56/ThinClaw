//! Agent runtime behavior configuration.
//!
//! Controls various runtime behaviors such as:
//! - Whether tool errors are surfaced to users
//! - Skill path compaction for prompt token savings
//! - Transcript/session size reporting

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Runtime behavior configuration.
#[derive(Debug, Clone)]
pub struct RuntimeBehavior {
    /// Whether to suppress tool error details from user-facing messages.
    /// When enabled, tool errors are logged but the user only sees a
    /// generic "tool encountered an issue" message.
    pub suppress_tool_errors: bool,

    /// Whether to compact skill paths in prompts using `~` prefix.
    /// E.g., `/Users/alice/.thinclaw/skills/web-search` → `~skills/web-search`
    pub skill_path_compaction: bool,

    /// Base path for skill compaction (default: `~/.thinclaw`).
    pub thinclaw_home: PathBuf,
}

impl Default for RuntimeBehavior {
    fn default() -> Self {
        let thinclaw_home = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".thinclaw");

        Self {
            suppress_tool_errors: false,
            skill_path_compaction: true,
            thinclaw_home,
        }
    }
}

impl RuntimeBehavior {
    /// Create from environment variables.
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(val) = std::env::var("SUPPRESS_TOOL_ERRORS") {
            config.suppress_tool_errors = val == "1" || val.eq_ignore_ascii_case("true");
        }

        if let Ok(val) = std::env::var("SKILL_PATH_COMPACTION") {
            config.skill_path_compaction = val != "0" && !val.eq_ignore_ascii_case("false");
        }

        if let Ok(home) = std::env::var("THINCLAW_HOME") {
            config.thinclaw_home = PathBuf::from(home);
        }

        config
    }

    /// Format a tool error message for user display.
    ///
    /// If `suppress_tool_errors` is enabled, returns a generic message.
    /// Otherwise, returns the full error details.
    pub fn format_tool_error(&self, tool_name: &str, error: &str) -> String {
        if self.suppress_tool_errors {
            format!(
                "Tool `{}` encountered an issue. The agent will try an alternative approach.",
                tool_name
            )
        } else {
            format!("Tool `{}` error: {}", tool_name, error)
        }
    }

    /// Compact a file path for prompt display.
    ///
    /// Replaces the IronClaw home directory with `~` to save tokens.
    /// E.g., `/Users/alice/.thinclaw/skills/web` → `~skills/web`
    pub fn compact_path(&self, path: &Path) -> String {
        if !self.skill_path_compaction {
            return path.display().to_string();
        }

        let path_str = path.display().to_string();
        let home_str = self.thinclaw_home.display().to_string();

        if path_str.starts_with(&home_str) {
            format!("~{}", &path_str[home_str.len()..])
        } else {
            // Also try $HOME
            if let Some(home) = dirs::home_dir() {
                let home_display = home.display().to_string();
                if path_str.starts_with(&home_display) {
                    return format!("~{}", &path_str[home_display.len()..]);
                }
            }
            path_str
        }
    }
}

/// Transcript/session size statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptStats {
    /// Number of messages in the session.
    pub message_count: usize,
    /// Total character count across all messages.
    pub total_chars: usize,
    /// Estimated token count (chars / 4 approximation).
    pub estimated_tokens: usize,
    /// Number of tool calls in the session.
    pub tool_call_count: usize,
    /// Number of media attachments.
    pub attachment_count: usize,
}

impl TranscriptStats {
    /// Create empty stats.
    pub fn empty() -> Self {
        Self {
            message_count: 0,
            total_chars: 0,
            estimated_tokens: 0,
            tool_call_count: 0,
            attachment_count: 0,
        }
    }

    /// Update stats from a message.
    pub fn add_message(&mut self, content: &str, is_tool_call: bool, attachments: usize) {
        self.message_count += 1;
        self.total_chars += content.len();
        self.estimated_tokens = self.total_chars / 4;
        if is_tool_call {
            self.tool_call_count += 1;
        }
        self.attachment_count += attachments;
    }

    /// Format for display in `thinclaw status`.
    pub fn display_summary(&self) -> String {
        let size_str = if self.total_chars < 1024 {
            format!("{} B", self.total_chars)
        } else if self.total_chars < 1024 * 1024 {
            format!("{:.1} KB", self.total_chars as f64 / 1024.0)
        } else {
            format!("{:.1} MB", self.total_chars as f64 / (1024.0 * 1024.0))
        };

        format!(
            "{} messages, {} (~{} tokens), {} tool calls, {} attachments",
            self.message_count,
            size_str,
            self.estimated_tokens,
            self.tool_call_count,
            self.attachment_count,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_suppress_tool_errors_disabled() {
        let config = RuntimeBehavior::default();
        let msg = config.format_tool_error("shell", "command not found");
        assert!(msg.contains("command not found"));
    }

    #[test]
    fn test_suppress_tool_errors_enabled() {
        let config = RuntimeBehavior {
            suppress_tool_errors: true,
            ..Default::default()
        };
        let msg = config.format_tool_error("shell", "command not found");
        assert!(!msg.contains("command not found"));
        assert!(msg.contains("encountered an issue"));
    }

    #[test]
    fn test_compact_path_within_home() {
        let config = RuntimeBehavior {
            thinclaw_home: PathBuf::from("/home/user/.thinclaw"),
            skill_path_compaction: true,
            ..Default::default()
        };

        let compacted = config.compact_path(Path::new("/home/user/.thinclaw/skills/web-search"));
        assert_eq!(compacted, "~/skills/web-search");
    }

    #[test]
    fn test_compact_path_outside_home() {
        let config = RuntimeBehavior {
            thinclaw_home: PathBuf::from("/home/user/.thinclaw"),
            skill_path_compaction: true,
            ..Default::default()
        };

        let compacted = config.compact_path(Path::new("/opt/tools/something"));
        assert_eq!(compacted, "/opt/tools/something");
    }

    #[test]
    fn test_compact_path_disabled() {
        let config = RuntimeBehavior {
            thinclaw_home: PathBuf::from("/home/user/.thinclaw"),
            skill_path_compaction: false,
            ..Default::default()
        };

        let compacted = config.compact_path(Path::new("/home/user/.thinclaw/skills/web"));
        assert_eq!(compacted, "/home/user/.thinclaw/skills/web");
    }

    #[test]
    fn test_transcript_stats_empty() {
        let stats = TranscriptStats::empty();
        assert_eq!(stats.message_count, 0);
        let summary = stats.display_summary();
        assert!(summary.contains("0 messages"));
    }

    #[test]
    fn test_transcript_stats_accumulation() {
        let mut stats = TranscriptStats::empty();
        stats.add_message("Hello, how are you?", false, 0);
        stats.add_message("Let me search for that.", true, 0);
        stats.add_message("Here's the result with an image.", false, 1);

        assert_eq!(stats.message_count, 3);
        assert_eq!(stats.tool_call_count, 1);
        assert_eq!(stats.attachment_count, 1);
        assert!(stats.total_chars > 0);
    }

    #[test]
    fn test_transcript_size_display() {
        let stats = TranscriptStats {
            message_count: 42,
            total_chars: 5120,
            estimated_tokens: 1280,
            tool_call_count: 5,
            attachment_count: 2,
        };

        let summary = stats.display_summary();
        assert!(summary.contains("42 messages"));
        assert!(summary.contains("KB"));
        assert!(summary.contains("1280 tokens"));
    }
}
