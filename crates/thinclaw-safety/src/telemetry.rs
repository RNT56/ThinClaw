//! Bounded, metadata-only safety telemetry for operator-facing diagnostics.

use std::collections::VecDeque;
use std::sync::Mutex;

use chrono::Utc;
use serde::{Deserialize, Serialize};

const MAX_RECENT_EVENTS: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafetyTelemetryAction {
    Sanitized,
    Redacted,
    Blocked,
    Warned,
}

impl SafetyTelemetryAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sanitized => "sanitized",
            Self::Redacted => "redacted",
            Self::Blocked => "blocked",
            Self::Warned => "warned",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafetyTelemetryEvent {
    pub occurred_at_ms: i64,
    pub action: SafetyTelemetryAction,
    pub source: String,
    pub reason: String,
    pub severity: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafetyTelemetrySnapshot {
    pub sanitized: u64,
    pub redacted: u64,
    pub blocked: u64,
    pub warned: u64,
    pub recent_events: Vec<SafetyTelemetryEvent>,
}

#[derive(Default)]
struct SafetyTelemetryState {
    snapshot: SafetyTelemetrySnapshot,
    recent_events: VecDeque<SafetyTelemetryEvent>,
}

/// Thread-safe counters and a bounded event ring. Events contain rule metadata
/// only; input/output content is deliberately never accepted by this API.
#[derive(Default)]
pub struct SafetyTelemetry {
    state: Mutex<SafetyTelemetryState>,
}

impl SafetyTelemetry {
    pub fn record(
        &self,
        action: SafetyTelemetryAction,
        source: impl Into<String>,
        reason: impl Into<String>,
        severity: impl Into<String>,
    ) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        match action {
            SafetyTelemetryAction::Sanitized => state.snapshot.sanitized += 1,
            SafetyTelemetryAction::Redacted => state.snapshot.redacted += 1,
            SafetyTelemetryAction::Blocked => state.snapshot.blocked += 1,
            SafetyTelemetryAction::Warned => state.snapshot.warned += 1,
        }
        if state.recent_events.len() == MAX_RECENT_EVENTS {
            state.recent_events.pop_front();
        }
        state.recent_events.push_back(SafetyTelemetryEvent {
            occurred_at_ms: Utc::now().timestamp_millis(),
            action,
            source: source.into(),
            reason: reason.into(),
            severity: severity.into(),
        });
    }

    pub fn snapshot(&self) -> SafetyTelemetrySnapshot {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let mut snapshot = state.snapshot.clone();
        snapshot.recent_events = state.recent_events.iter().cloned().rev().collect();
        snapshot
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telemetry_is_bounded_and_newest_first() {
        let telemetry = SafetyTelemetry::default();
        for index in 0..55 {
            telemetry.record(
                SafetyTelemetryAction::Sanitized,
                "tool:test",
                format!("rule-{index}"),
                "medium",
            );
        }

        let snapshot = telemetry.snapshot();
        assert_eq!(snapshot.sanitized, 55);
        assert_eq!(snapshot.recent_events.len(), MAX_RECENT_EVENTS);
        assert_eq!(snapshot.recent_events[0].reason, "rule-54");
        assert_eq!(snapshot.recent_events[49].reason, "rule-5");
    }

    #[test]
    fn action_labels_are_stable_wire_values() {
        assert_eq!(SafetyTelemetryAction::Sanitized.as_str(), "sanitized");
        assert_eq!(SafetyTelemetryAction::Redacted.as_str(), "redacted");
        assert_eq!(SafetyTelemetryAction::Blocked.as_str(), "blocked");
        assert_eq!(SafetyTelemetryAction::Warned.as_str(), "warned");
    }
}
