//! Channel health monitoring with auto-restart.
//!
//! Periodically checks channel health and automatically restarts
//! channels that report failures, with configurable retry limits.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use crate::channels::ChannelManager;

/// Configuration for the channel health monitor.
#[derive(Debug, Clone)]
pub struct HealthMonitorConfig {
    /// How often to check channel health (default: 60s).
    pub check_interval: Duration,
    /// Maximum restart attempts per channel before giving up (default: 3).
    pub max_restart_attempts: u32,
    /// Cooldown after a restart before checking again (default: 30s).
    pub restart_cooldown: Duration,
    /// Whether to automatically restart failed channels.
    pub auto_restart: bool,
}

impl Default for HealthMonitorConfig {
    fn default() -> Self {
        Self {
            check_interval: Duration::from_secs(60),
            max_restart_attempts: 3,
            restart_cooldown: Duration::from_secs(30),
            auto_restart: true,
        }
    }
}

/// Tracks per-channel restart state.
#[derive(Debug, Default)]
struct ChannelState {
    /// Number of consecutive failures.
    consecutive_failures: u32,
    /// Number of restart attempts so far.
    restart_attempts: u32,
    /// Next instant when the channel is eligible for health checks again.
    cooldown_until: Option<std::time::Instant>,
}

/// Monitors channel health and restarts failed channels.
pub struct ChannelHealthMonitor {
    config: HealthMonitorConfig,
    channel_manager: Arc<ChannelManager>,
    states: Arc<RwLock<HashMap<String, ChannelState>>>,
    task_handle: RwLock<Option<JoinHandle<()>>>,
}

impl ChannelHealthMonitor {
    /// Create a new health monitor.
    pub fn new(channel_manager: Arc<ChannelManager>, config: HealthMonitorConfig) -> Self {
        Self {
            config,
            channel_manager,
            states: Arc::new(RwLock::new(HashMap::new())),
            task_handle: RwLock::new(None),
        }
    }

    /// Create with default configuration.
    pub fn with_defaults(channel_manager: Arc<ChannelManager>) -> Self {
        Self::new(channel_manager, HealthMonitorConfig::default())
    }

    /// Start the health monitoring background task.
    pub async fn start(&self) {
        let config = self.config.clone();
        let manager = Arc::clone(&self.channel_manager);
        let states = Arc::clone(&self.states);

        let handle = tokio::spawn(async move {
            tracing::info!(
                interval_secs = config.check_interval.as_secs(),
                auto_restart = config.auto_restart,
                "Channel health monitor started"
            );

            loop {
                tokio::time::sleep(config.check_interval).await;

                let results = manager.health_check_all().await;
                let channel_names = manager.channel_names().await;

                let mut states_lock = states.write().await;

                for name in &channel_names {
                    let state = states_lock.entry(name.clone()).or_default();

                    // Skip channels in cooldown.
                    if let Some(until) = state.cooldown_until {
                        if std::time::Instant::now() < until {
                            tracing::debug!(
                                channel = %name,
                                "Channel in cooldown, skipping health check"
                            );
                            continue;
                        }
                        state.cooldown_until = None;
                    }

                    if let Some(result) = results.get(name) {
                        match result {
                            Ok(()) => {
                                if state.consecutive_failures > 0 {
                                    tracing::info!(
                                        channel = %name,
                                        prev_failures = state.consecutive_failures,
                                        "Channel recovered"
                                    );
                                }
                                state.consecutive_failures = 0;
                                // Don't reset restart_attempts — keep the history
                            }
                            Err(e) => {
                                state.consecutive_failures += 1;
                                let consecutive_failures = state.consecutive_failures;
                                let restart_attempts = state.restart_attempts;
                                tracing::warn!(
                                    channel = %name,
                                    error = %e,
                                    consecutive = consecutive_failures,
                                    restarts = restart_attempts,
                                    max_restarts = config.max_restart_attempts,
                                    "Channel health check failed"
                                );

                                let mut restart_threshold = 2;
                                if name == "telegram" {
                                    drop(states_lock);
                                    if let Some(diagnostics) =
                                        manager.channel_diagnostics(name).await
                                    {
                                        let transport_mode = diagnostics
                                            .get("transport_mode")
                                            .and_then(|value| value.as_str());
                                        let transport_override = diagnostics
                                            .get("transport_override")
                                            .and_then(|value| value.as_str());
                                        if transport_mode == Some("webhook")
                                            && transport_override == Some("polling")
                                        {
                                            restart_threshold = 1;
                                        }
                                    }
                                    states_lock = states.write().await;
                                }

                                let state = states_lock.entry(name.clone()).or_default();

                                // Auto-restart if enabled and within limits
                                if config.auto_restart
                                    && state.restart_attempts < config.max_restart_attempts
                                    && state.consecutive_failures >= restart_threshold
                                {
                                    state.restart_attempts += 1;
                                    state.cooldown_until =
                                        Some(std::time::Instant::now() + config.restart_cooldown);

                                    tracing::info!(
                                        channel = %name,
                                        attempt = state.restart_attempts,
                                        max = config.max_restart_attempts,
                                        "Attempting channel restart"
                                    );

                                    // Drop the states lock before the async restart call
                                    // to avoid holding it across an await point.
                                    let restart_name = name.clone();
                                    drop(states_lock);

                                    match manager.restart_channel(&restart_name).await {
                                        Ok(()) => {
                                            tracing::info!(
                                                channel = %restart_name,
                                                "Channel restarted successfully"
                                            );
                                            // Reset failure counter on success.
                                            let mut reacquired = states.write().await;
                                            if let Some(s) = reacquired.get_mut(&restart_name) {
                                                s.consecutive_failures = 0;
                                            }
                                        }
                                        Err(e) => {
                                            tracing::error!(
                                                channel = %restart_name,
                                                error = %e,
                                                "Channel restart failed"
                                            );
                                        }
                                    }

                                    // Re-enter the loop — states_lock was dropped.
                                    states_lock = states.write().await;
                                    continue;
                                } else if state.restart_attempts >= config.max_restart_attempts {
                                    tracing::error!(
                                        channel = %name,
                                        "Channel has exceeded max restart attempts, giving up"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        });

        *self.task_handle.write().await = Some(handle);
    }

    /// Stop the health monitor.
    pub async fn stop(&self) {
        if let Some(handle) = self.task_handle.write().await.take() {
            handle.abort();
            tracing::info!("Channel health monitor stopped");
        }
    }

    /// Get current health status of all channels.
    pub async fn status(&self) -> HashMap<String, ChannelHealthStatus> {
        let results = self.channel_manager.health_check_all().await;
        let states = self.states.read().await;

        results
            .into_iter()
            .map(|(name, result)| {
                let state = states.get(&name);
                let status = ChannelHealthStatus {
                    healthy: result.is_ok(),
                    error: result.err().map(|e| e.to_string()),
                    consecutive_failures: state.map_or(0, |s| s.consecutive_failures),
                    restart_attempts: state.map_or(0, |s| s.restart_attempts),
                };
                (name, status)
            })
            .collect()
    }
}

/// Health status for a single channel.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ChannelHealthStatus {
    pub healthy: bool,
    pub error: Option<String>,
    pub consecutive_failures: u32,
    pub restart_attempts: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = HealthMonitorConfig::default();
        assert_eq!(config.check_interval, Duration::from_secs(60));
        assert_eq!(config.max_restart_attempts, 3);
        assert_eq!(config.restart_cooldown, Duration::from_secs(30));
        assert!(config.auto_restart);
    }

    #[test]
    fn test_channel_state_default() {
        let state = ChannelState::default();
        assert_eq!(state.consecutive_failures, 0);
        assert_eq!(state.restart_attempts, 0);
        assert!(state.cooldown_until.is_none());
    }

    #[tokio::test]
    async fn test_monitor_creation() {
        let manager = Arc::new(ChannelManager::new());
        let monitor = ChannelHealthMonitor::with_defaults(manager);
        let status = monitor.status().await;
        assert!(status.is_empty());
    }

    #[tokio::test]
    async fn test_monitor_stop_without_start() {
        let manager = Arc::new(ChannelManager::new());
        let monitor = ChannelHealthMonitor::with_defaults(manager);
        monitor.stop().await; // Should not panic
    }
}
