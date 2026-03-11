//! Dangerous tool re-enable warning.
//!
//! When a previously disabled dangerous tool is re-enabled, the agent
//! should display a prominent warning to the user. This module manages
//! tracking of tool states and generating appropriate warnings.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

/// Known dangerous tools that warrant warnings on re-enable.
const DANGEROUS_TOOLS: &[&str] = &[
    "shell",
    "exec",
    "run_command",
    "write_file",
    "file_write",
    "delete_file",
    "http_fetch",
    "fetch",
    "sudo",
    "docker_exec",
    "eval",
];

/// Tool state tracking for dangerous tool warnings.
pub struct DangerousToolTracker {
    /// Tools that have been explicitly disabled.
    disabled_tools: HashSet<String>,
    /// History of tool state changes.
    state_history: Vec<ToolStateChange>,
}

/// A recorded tool state change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStateChange {
    pub tool_name: String,
    pub action: ToolAction,
    pub reason: Option<String>,
    pub timestamp: String,
}

/// The action taken on a tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ToolAction {
    Disabled,
    ReEnabled,
}

impl DangerousToolTracker {
    pub fn new() -> Self {
        Self {
            disabled_tools: HashSet::new(),
            state_history: Vec::new(),
        }
    }

    /// Check if a tool is considered dangerous.
    pub fn is_dangerous(tool_name: &str) -> bool {
        DANGEROUS_TOOLS.contains(&tool_name)
    }

    /// Disable a tool.
    pub fn disable(&mut self, tool_name: &str, reason: Option<&str>) {
        self.disabled_tools.insert(tool_name.to_string());
        self.state_history.push(ToolStateChange {
            tool_name: tool_name.to_string(),
            action: ToolAction::Disabled,
            reason: reason.map(String::from),
            timestamp: chrono::Utc::now().to_rfc3339(),
        });
    }

    /// Re-enable a tool. Returns a warning if it's dangerous.
    pub fn re_enable(&mut self, tool_name: &str) -> Option<String> {
        let was_disabled = self.disabled_tools.remove(tool_name);

        self.state_history.push(ToolStateChange {
            tool_name: tool_name.to_string(),
            action: ToolAction::ReEnabled,
            reason: None,
            timestamp: chrono::Utc::now().to_rfc3339(),
        });

        if was_disabled && Self::is_dangerous(tool_name) {
            Some(format!(
                "⚠️  WARNING: Dangerous tool `{}` has been re-enabled. \
                 This tool can perform potentially destructive actions. \
                 Exercise caution when reviewing its outputs.",
                tool_name
            ))
        } else {
            None
        }
    }

    /// Check if a tool is currently disabled.
    pub fn is_disabled(&self, tool_name: &str) -> bool {
        self.disabled_tools.contains(tool_name)
    }

    /// Get all disabled tools.
    pub fn disabled_tools(&self) -> Vec<&str> {
        self.disabled_tools.iter().map(|s| s.as_str()).collect()
    }

    /// Get the state history.
    pub fn history(&self) -> &[ToolStateChange] {
        &self.state_history
    }

    /// Generate a summary of current dangerous tool states.
    pub fn status_summary(&self) -> String {
        let mut lines = Vec::new();
        let dangerous_disabled: Vec<&str> = self
            .disabled_tools
            .iter()
            .filter(|t| Self::is_dangerous(t))
            .map(|s| s.as_str())
            .collect();

        if dangerous_disabled.is_empty() {
            lines.push("All dangerous tools are enabled.".to_string());
        } else {
            lines.push(format!(
                "{} dangerous tool(s) disabled: {}",
                dangerous_disabled.len(),
                dangerous_disabled.join(", ")
            ));
        }

        lines.join("\n")
    }
}

impl Default for DangerousToolTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_dangerous() {
        assert!(DangerousToolTracker::is_dangerous("shell"));
        assert!(DangerousToolTracker::is_dangerous("exec"));
        assert!(!DangerousToolTracker::is_dangerous("calculator"));
    }

    #[test]
    fn test_disable_and_reenable_warning() {
        let mut tracker = DangerousToolTracker::new();
        tracker.disable("shell", Some("testing"));

        assert!(tracker.is_disabled("shell"));

        let warning = tracker.re_enable("shell");
        assert!(warning.is_some());
        assert!(warning.unwrap().contains("WARNING"));
        assert!(!tracker.is_disabled("shell"));
    }

    #[test]
    fn test_reenable_non_dangerous_no_warning() {
        let mut tracker = DangerousToolTracker::new();
        tracker.disable("calculator", None);

        let warning = tracker.re_enable("calculator");
        assert!(warning.is_none());
    }

    #[test]
    fn test_reenable_never_disabled_no_warning() {
        let mut tracker = DangerousToolTracker::new();
        let warning = tracker.re_enable("shell");
        assert!(warning.is_none()); // Was never disabled
    }

    #[test]
    fn test_disabled_tools_list() {
        let mut tracker = DangerousToolTracker::new();
        tracker.disable("shell", None);
        tracker.disable("exec", None);

        let disabled = tracker.disabled_tools();
        assert_eq!(disabled.len(), 2);
    }

    #[test]
    fn test_history_tracking() {
        let mut tracker = DangerousToolTracker::new();
        tracker.disable("shell", Some("user request"));
        tracker.re_enable("shell");

        assert_eq!(tracker.history().len(), 2);
        assert_eq!(tracker.history()[0].action, ToolAction::Disabled);
        assert_eq!(tracker.history()[1].action, ToolAction::ReEnabled);
    }

    #[test]
    fn test_status_summary_all_enabled() {
        let tracker = DangerousToolTracker::new();
        let summary = tracker.status_summary();
        assert!(summary.contains("All dangerous tools are enabled"));
    }

    #[test]
    fn test_status_summary_some_disabled() {
        let mut tracker = DangerousToolTracker::new();
        tracker.disable("shell", None);
        let summary = tracker.status_summary();
        assert!(summary.contains("1 dangerous tool(s) disabled"));
    }
}
