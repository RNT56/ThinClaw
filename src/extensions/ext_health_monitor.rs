//! Extension health monitor.
//!
//! Tracks health of active extensions using a state machine:
//! Unknown → Healthy (on successes), Healthy → Degraded (on failure),
//! Degraded → Unhealthy (at threshold), Unhealthy → Healthy (recovery).

use std::collections::HashMap;

/// Health monitor configuration.
#[derive(Debug, Clone)]
pub struct ExtensionHealthConfig {
    /// Seconds between checks.
    pub check_interval_secs: u64,
    /// Consecutive failures to become Unhealthy.
    pub failure_threshold: u32,
    /// Consecutive successes to recover to Healthy.
    pub recovery_threshold: u32,
    /// Per-check timeout seconds.
    pub timeout_secs: u64,
}

impl Default for ExtensionHealthConfig {
    fn default() -> Self {
        Self {
            check_interval_secs: 60,
            failure_threshold: 3,
            recovery_threshold: 2,
            timeout_secs: 5,
        }
    }
}

/// Health status of an extension.
#[derive(Debug, Clone, PartialEq)]
pub enum HealthStatus {
    Healthy,
    Degraded { failures: u32 },
    Unhealthy { failures: u32, since: String },
    Unknown,
}

impl HealthStatus {
    pub fn label(&self) -> &str {
        match self {
            Self::Healthy => "healthy",
            Self::Degraded { .. } => "degraded",
            Self::Unhealthy { .. } => "unhealthy",
            Self::Unknown => "unknown",
        }
    }

    pub fn is_healthy(&self) -> bool {
        matches!(self, Self::Healthy)
    }
}

/// Health state for a single extension.
pub struct ExtensionHealth {
    pub name: String,
    pub status: HealthStatus,
    pub last_check: Option<String>,
    pub consecutive_failures: u32,
    pub consecutive_successes: u32,
    pub total_checks: u64,
}

/// Summary of all extension health.
pub struct HealthSummary {
    pub total: usize,
    pub healthy: usize,
    pub degraded: usize,
    pub unhealthy: usize,
    pub unknown: usize,
}

/// Extension health monitor.
pub struct ExtensionHealthMonitor {
    config: ExtensionHealthConfig,
    entries: HashMap<String, ExtensionHealth>,
}

impl ExtensionHealthMonitor {
    pub fn new(config: ExtensionHealthConfig) -> Self {
        Self {
            config,
            entries: HashMap::new(),
        }
    }

    /// Register an extension for monitoring.
    pub fn register(&mut self, name: &str) {
        self.entries.insert(
            name.to_string(),
            ExtensionHealth {
                name: name.to_string(),
                status: HealthStatus::Unknown,
                last_check: None,
                consecutive_failures: 0,
                consecutive_successes: 0,
                total_checks: 0,
            },
        );
    }

    /// Record a successful check.
    pub fn record_success(&mut self, name: &str, timestamp: &str) {
        if let Some(entry) = self.entries.get_mut(name) {
            entry.total_checks += 1;
            entry.last_check = Some(timestamp.to_string());
            entry.consecutive_successes += 1;
            entry.consecutive_failures = 0;

            if entry.consecutive_successes >= self.config.recovery_threshold {
                entry.status = HealthStatus::Healthy;
            }
        }
    }

    /// Record a failed check.
    pub fn record_failure(&mut self, name: &str, timestamp: &str) {
        if let Some(entry) = self.entries.get_mut(name) {
            entry.total_checks += 1;
            entry.last_check = Some(timestamp.to_string());
            entry.consecutive_failures += 1;
            entry.consecutive_successes = 0;

            if entry.consecutive_failures >= self.config.failure_threshold {
                entry.status = HealthStatus::Unhealthy {
                    failures: entry.consecutive_failures,
                    since: timestamp.to_string(),
                };
            } else if entry.consecutive_failures > 0 {
                entry.status = HealthStatus::Degraded {
                    failures: entry.consecutive_failures,
                };
            }
        }
    }

    /// Get health for a specific extension.
    pub fn get_status(&self, name: &str) -> Option<&ExtensionHealth> {
        self.entries.get(name)
    }

    /// Get all unhealthy extensions.
    pub fn unhealthy(&self) -> Vec<&ExtensionHealth> {
        self.entries
            .values()
            .filter(|e| matches!(e.status, HealthStatus::Unhealthy { .. }))
            .collect()
    }

    /// Overall health summary.
    pub fn summary(&self) -> HealthSummary {
        let mut summary = HealthSummary {
            total: self.entries.len(),
            healthy: 0,
            degraded: 0,
            unhealthy: 0,
            unknown: 0,
        };
        for entry in self.entries.values() {
            match entry.status {
                HealthStatus::Healthy => summary.healthy += 1,
                HealthStatus::Degraded { .. } => summary.degraded += 1,
                HealthStatus::Unhealthy { .. } => summary.unhealthy += 1,
                HealthStatus::Unknown => summary.unknown += 1,
            }
        }
        summary
    }

    /// Total extensions tracked.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_unknown() {
        let mut monitor = ExtensionHealthMonitor::new(ExtensionHealthConfig::default());
        monitor.register("notion");
        let health = monitor.get_status("notion").unwrap();
        assert_eq!(health.status, HealthStatus::Unknown);
    }

    #[test]
    fn test_success_transitions_to_healthy() {
        let mut monitor = ExtensionHealthMonitor::new(ExtensionHealthConfig::default());
        monitor.register("slack");
        monitor.record_success("slack", "2026-01-01T00:00:00Z");
        monitor.record_success("slack", "2026-01-01T00:01:00Z");
        let h = monitor.get_status("slack").unwrap();
        assert_eq!(h.status, HealthStatus::Healthy);
    }

    #[test]
    fn test_failures_transition_to_degraded() {
        let mut monitor = ExtensionHealthMonitor::new(ExtensionHealthConfig::default());
        monitor.register("mcp");
        monitor.record_failure("mcp", "2026-01-01T00:00:00Z");
        let h = monitor.get_status("mcp").unwrap();
        assert!(matches!(h.status, HealthStatus::Degraded { failures: 1 }));
    }

    #[test]
    fn test_failures_transition_to_unhealthy() {
        let mut monitor = ExtensionHealthMonitor::new(ExtensionHealthConfig::default());
        monitor.register("bad");
        monitor.record_failure("bad", "t0");
        monitor.record_failure("bad", "t1");
        monitor.record_failure("bad", "t2");
        let h = monitor.get_status("bad").unwrap();
        assert!(matches!(h.status, HealthStatus::Unhealthy { .. }));
    }

    #[test]
    fn test_recovery_from_unhealthy() {
        let mut monitor = ExtensionHealthMonitor::new(ExtensionHealthConfig::default());
        monitor.register("x");
        monitor.record_failure("x", "t0");
        monitor.record_failure("x", "t1");
        monitor.record_failure("x", "t2");
        assert!(matches!(
            monitor.get_status("x").unwrap().status,
            HealthStatus::Unhealthy { .. }
        ));
        monitor.record_success("x", "t3");
        monitor.record_success("x", "t4");
        assert_eq!(
            monitor.get_status("x").unwrap().status,
            HealthStatus::Healthy
        );
    }

    #[test]
    fn test_unhealthy_list() {
        let mut monitor = ExtensionHealthMonitor::new(ExtensionHealthConfig::default());
        monitor.register("a");
        monitor.register("b");
        monitor.record_failure("a", "t0");
        monitor.record_failure("a", "t1");
        monitor.record_failure("a", "t2");
        assert_eq!(monitor.unhealthy().len(), 1);
    }

    #[test]
    fn test_summary() {
        let mut monitor = ExtensionHealthMonitor::new(ExtensionHealthConfig::default());
        monitor.register("a");
        monitor.register("b");
        monitor.record_success("a", "t0");
        monitor.record_success("a", "t1");
        let summary = monitor.summary();
        assert_eq!(summary.total, 2);
        assert_eq!(summary.healthy, 1);
        assert_eq!(summary.unknown, 1);
    }

    #[test]
    fn test_config_defaults() {
        let config = ExtensionHealthConfig::default();
        assert_eq!(config.failure_threshold, 3);
        assert_eq!(config.recovery_threshold, 2);
        assert_eq!(config.check_interval_secs, 60);
    }

    #[test]
    fn test_multiple_extensions() {
        let mut monitor = ExtensionHealthMonitor::new(ExtensionHealthConfig::default());
        monitor.register("a");
        monitor.register("b");
        monitor.record_success("a", "t0");
        monitor.record_success("a", "t1");
        monitor.record_failure("b", "t0");
        assert_eq!(
            monitor.get_status("a").unwrap().status,
            HealthStatus::Healthy
        );
        assert!(matches!(
            monitor.get_status("b").unwrap().status,
            HealthStatus::Degraded { .. }
        ));
    }
}
