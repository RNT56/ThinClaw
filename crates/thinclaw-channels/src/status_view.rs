//! Full channel status view.
//!
//! Per-channel state, message counters, error info, formatted table/JSON output.
//!
//! The [`ChannelStatusEvent`] struct provides the SSE event shape
//! for real-time status updates via `openclaw-event` (see §17.4).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Channel lifecycle state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ChannelViewState {
    Running { uptime_secs: u64 },
    Connecting { attempt: u32 },
    Reconnecting { attempt: u32, next_retry_secs: u64 },
    Failed { error: String, failed_at: String },
    Disabled,
    Draining,
}

impl ChannelViewState {
    /// Human-readable label.
    pub fn label(&self) -> &str {
        match self {
            Self::Running { .. } => "running",
            Self::Connecting { .. } => "connecting",
            Self::Reconnecting { .. } => "reconnecting",
            Self::Failed { .. } => "failed",
            Self::Disabled => "disabled",
            Self::Draining => "draining",
        }
    }

    /// Whether this state is considered healthy.
    pub fn is_healthy(&self) -> bool {
        matches!(self, Self::Running { .. })
    }
}

/// Status entry for a single channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelStatusEntry {
    pub name: String,
    pub channel_type: String,
    pub state: ChannelViewState,
    pub last_message_at: Option<String>,
    pub last_error: Option<String>,
    pub messages_received: u64,
    pub messages_sent: u64,
    pub errors: u32,
}

/// Aggregated status summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusSummary {
    pub total: usize,
    pub running: usize,
    pub failed: usize,
    pub connecting: usize,
    pub disabled: usize,
}

/// Channel status view.
pub struct ChannelStatusView {
    entries: HashMap<String, ChannelStatusEntry>,
}

impl ChannelStatusView {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Insert or update an entry.
    pub fn upsert(&mut self, entry: ChannelStatusEntry) {
        self.entries.insert(entry.name.clone(), entry);
    }

    /// Get entry by name.
    pub fn get(&self, name: &str) -> Option<&ChannelStatusEntry> {
        self.entries.get(name)
    }

    /// All entries.
    pub fn all(&self) -> Vec<&ChannelStatusEntry> {
        self.entries.values().collect()
    }

    /// Only running channels.
    pub fn running(&self) -> Vec<&ChannelStatusEntry> {
        self.entries
            .values()
            .filter(|e| matches!(e.state, ChannelViewState::Running { .. }))
            .collect()
    }

    /// Only failed channels.
    pub fn failed(&self) -> Vec<&ChannelStatusEntry> {
        self.entries
            .values()
            .filter(|e| matches!(e.state, ChannelViewState::Failed { .. }))
            .collect()
    }

    /// Format as table.
    pub fn format_table(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!(
            "{:<20} {:<12} {:<12} {:>8} {:>8}",
            "Channel", "Type", "Status", "Recv", "Sent"
        ));
        lines.push("-".repeat(64));
        let mut sorted: Vec<_> = self.entries.values().collect();
        sorted.sort_by_key(|e| &e.name);
        for entry in sorted {
            lines.push(format!(
                "{:<20} {:<12} {:<12} {:>8} {:>8}",
                entry.name,
                entry.channel_type,
                entry.state.label(),
                entry.messages_received,
                entry.messages_sent,
            ));
        }
        lines.join("\n")
    }

    /// Format as JSON.
    pub fn format_json(&self) -> String {
        let mut entries = Vec::new();
        let mut sorted: Vec<_> = self.entries.values().collect();
        sorted.sort_by_key(|e| &e.name);
        for entry in sorted {
            entries.push(format!(
                r#"{{"name":"{}","type":"{}","status":"{}","received":{},"sent":{},"errors":{}}}"#,
                entry.name,
                entry.channel_type,
                entry.state.label(),
                entry.messages_received,
                entry.messages_sent,
                entry.errors,
            ));
        }
        format!("[{}]", entries.join(","))
    }

    /// Summary counts.
    pub fn summary(&self) -> StatusSummary {
        let mut s = StatusSummary {
            total: self.entries.len(),
            running: 0,
            failed: 0,
            connecting: 0,
            disabled: 0,
        };
        for entry in self.entries.values() {
            match entry.state {
                ChannelViewState::Running { .. } => s.running += 1,
                ChannelViewState::Failed { .. } => s.failed += 1,
                ChannelViewState::Connecting { .. } | ChannelViewState::Reconnecting { .. } => {
                    s.connecting += 1
                }
                ChannelViewState::Disabled => s.disabled += 1,
                ChannelViewState::Draining => {}
            }
        }
        s
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for ChannelStatusView {
    fn default() -> Self {
        Self::new()
    }
}

/// SSE event payload for channel status changes.
///
/// Emitted via `AppHandle::emit("openclaw-event", ...)` with `kind: "ChannelStatus"`.
/// Scrappy subscribes to these for real-time status updates.
/// See §17.4 integration contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelStatusEvent {
    /// Always `"ChannelStatus"` for event routing.
    pub kind: String,
    /// Channel name (e.g., "telegram", "discord").
    pub channel: String,
    /// New state label (e.g., "running", "reconnecting", "failed").
    pub state: String,
    /// ISO 8601 timestamp.
    pub timestamp: String,
}

impl ChannelStatusEvent {
    /// Create a status change event from an entry.
    pub fn from_entry(entry: &ChannelStatusEntry) -> Self {
        Self {
            kind: "ChannelStatus".to_string(),
            channel: entry.name.clone(),
            state: entry.state.label().to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(name: &str, state: ChannelViewState) -> ChannelStatusEntry {
        ChannelStatusEntry {
            name: name.into(),
            channel_type: "test".into(),
            state,
            last_message_at: None,
            last_error: None,
            messages_received: 10,
            messages_sent: 5,
            errors: 0,
        }
    }

    #[test]
    fn test_upsert_and_get() {
        let mut view = ChannelStatusView::new();
        view.upsert(make_entry(
            "slack",
            ChannelViewState::Running { uptime_secs: 100 },
        ));
        assert!(view.get("slack").is_some());
    }

    #[test]
    fn test_all_returns_all() {
        let mut view = ChannelStatusView::new();
        view.upsert(make_entry(
            "a",
            ChannelViewState::Running { uptime_secs: 1 },
        ));
        view.upsert(make_entry("b", ChannelViewState::Disabled));
        assert_eq!(view.all().len(), 2);
    }

    #[test]
    fn test_running_filter() {
        let mut view = ChannelStatusView::new();
        view.upsert(make_entry(
            "a",
            ChannelViewState::Running { uptime_secs: 1 },
        ));
        view.upsert(make_entry("b", ChannelViewState::Disabled));
        assert_eq!(view.running().len(), 1);
    }

    #[test]
    fn test_failed_filter() {
        let mut view = ChannelStatusView::new();
        view.upsert(make_entry(
            "bad",
            ChannelViewState::Failed {
                error: "timeout".into(),
                failed_at: "now".into(),
            },
        ));
        view.upsert(make_entry(
            "good",
            ChannelViewState::Running { uptime_secs: 1 },
        ));
        assert_eq!(view.failed().len(), 1);
    }

    #[test]
    fn test_format_table() {
        let mut view = ChannelStatusView::new();
        view.upsert(make_entry(
            "slack",
            ChannelViewState::Running { uptime_secs: 100 },
        ));
        let table = view.format_table();
        assert!(table.contains("Channel"));
        assert!(table.contains("slack"));
    }

    #[test]
    fn test_format_json() {
        let mut view = ChannelStatusView::new();
        view.upsert(make_entry(
            "tg",
            ChannelViewState::Running { uptime_secs: 50 },
        ));
        let json = view.format_json();
        assert!(json.starts_with('['));
        assert!(json.contains("\"name\":\"tg\""));
    }

    #[test]
    fn test_summary_counts() {
        let mut view = ChannelStatusView::new();
        view.upsert(make_entry(
            "a",
            ChannelViewState::Running { uptime_secs: 1 },
        ));
        view.upsert(make_entry("b", ChannelViewState::Disabled));
        view.upsert(make_entry(
            "c",
            ChannelViewState::Failed {
                error: "e".into(),
                failed_at: "t".into(),
            },
        ));
        let s = view.summary();
        assert_eq!(s.total, 3);
        assert_eq!(s.running, 1);
        assert_eq!(s.disabled, 1);
        assert_eq!(s.failed, 1);
    }

    #[test]
    fn test_state_labels() {
        assert_eq!(
            ChannelViewState::Running { uptime_secs: 0 }.label(),
            "running"
        );
        assert_eq!(
            ChannelViewState::Connecting { attempt: 1 }.label(),
            "connecting"
        );
        assert_eq!(ChannelViewState::Disabled.label(), "disabled");
        assert_eq!(ChannelViewState::Draining.label(), "draining");
    }

    #[test]
    fn test_state_is_healthy() {
        assert!(ChannelViewState::Running { uptime_secs: 0 }.is_healthy());
        assert!(!ChannelViewState::Disabled.is_healthy());
    }

    #[test]
    fn test_default_empty() {
        let view = ChannelStatusView::default();
        assert!(view.is_empty());
    }

    #[test]
    fn test_channel_status_event_from_entry() {
        let entry = make_entry("telegram", ChannelViewState::Running { uptime_secs: 100 });
        let event = ChannelStatusEvent::from_entry(&entry);
        assert_eq!(event.kind, "ChannelStatus");
        assert_eq!(event.channel, "telegram");
        assert_eq!(event.state, "running");
        assert!(!event.timestamp.is_empty());
    }

    #[test]
    fn test_channel_status_event_serializable() {
        let entry = make_entry(
            "discord",
            ChannelViewState::Failed {
                error: "timeout".into(),
                failed_at: "now".into(),
            },
        );
        let event = ChannelStatusEvent::from_entry(&entry);
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"kind\":\"ChannelStatus\""));
        assert!(json.contains("\"channel\":\"discord\""));
        assert!(json.contains("\"state\":\"failed\""));
    }

    #[test]
    fn test_channel_status_entry_serializable() {
        let entry = make_entry("slack", ChannelViewState::Running { uptime_secs: 50 });
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"name\":\"slack\""));
    }
}
