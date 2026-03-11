//! Cron stagger controls — jitter and concurrency limits for scheduled jobs.
//!
//! Prevents multiple cron jobs from firing simultaneously and overwhelming
//! the LLM backend. Also provides a finished-run webhook notification.
//!
//! Configuration:
//! - `CRON_STAGGER_SECS` — maximum random jitter in seconds (default: 30)
//! - `CRON_MAX_CONCURRENT` — max concurrent routine executions (default: 3)
//! - `CRON_FINISHED_WEBHOOK` — URL to POST when a routine run completes

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Configuration for cron stagger behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaggerConfig {
    /// Maximum random jitter in seconds added before each run.
    /// Each job gets a random delay in `[0, max_jitter_secs)`.
    pub max_jitter_secs: u64,
    /// Maximum number of cron routines executing concurrently.
    pub max_concurrent: usize,
    /// Optional webhook URL to POST when a routine run finishes.
    pub finished_webhook_url: Option<String>,
}

impl Default for StaggerConfig {
    fn default() -> Self {
        Self {
            max_jitter_secs: 30,
            max_concurrent: 3,
            finished_webhook_url: None,
        }
    }
}

impl StaggerConfig {
    /// Create from environment variables.
    pub fn from_env() -> Self {
        let max_jitter_secs = std::env::var("CRON_STAGGER_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);

        let max_concurrent = std::env::var("CRON_MAX_CONCURRENT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3);

        let finished_webhook_url = std::env::var("CRON_FINISHED_WEBHOOK").ok();

        Self {
            max_jitter_secs,
            max_concurrent,
            finished_webhook_url,
        }
    }

    /// Compute a random jitter delay.
    pub fn jitter_delay(&self) -> Duration {
        if self.max_jitter_secs == 0 {
            return Duration::ZERO;
        }
        let jitter_ms = rand::random::<u64>() % (self.max_jitter_secs * 1000);
        Duration::from_millis(jitter_ms)
    }
}

/// Concurrency gate for limiting simultaneous cron runs.
#[derive(Debug, Clone)]
pub struct CronGate {
    active: Arc<AtomicUsize>,
    max: usize,
}

impl CronGate {
    /// Create a new gate with a maximum concurrency.
    pub fn new(max: usize) -> Self {
        Self {
            active: Arc::new(AtomicUsize::new(0)),
            max: max.max(1),
        }
    }

    /// Try to acquire a slot. Returns a guard that releases on drop.
    pub fn try_acquire(&self) -> Option<CronGuard> {
        loop {
            let current = self.active.load(Ordering::Acquire);
            if current >= self.max {
                return None;
            }
            if self
                .active
                .compare_exchange(current, current + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Some(CronGuard {
                    active: self.active.clone(),
                });
            }
        }
    }

    /// Get the number of currently active runs.
    pub fn active_count(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }
}

/// RAII guard that releases a cron slot when dropped.
pub struct CronGuard {
    active: Arc<AtomicUsize>,
}

impl Drop for CronGuard {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::Release);
    }
}

/// Payload sent to the finished-run webhook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinishedRunPayload {
    /// Routine ID.
    pub routine_id: String,
    /// Routine name.
    pub routine_name: String,
    /// Whether the run succeeded.
    pub success: bool,
    /// Duration of the run in milliseconds.
    pub duration_ms: u64,
    /// Error message if failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// ISO 8601 timestamp when the run completed.
    pub completed_at: String,
}

/// Send a finished-run notification to the webhook URL.
pub async fn notify_finished_run(url: &str, payload: &FinishedRunPayload) {
    let client = reqwest::Client::new();
    match client.post(url).json(payload).send().await {
        Ok(resp) if resp.status().is_success() => {
            tracing::debug!(
                routine = %payload.routine_name,
                "Cron finished-run webhook delivered"
            );
        }
        Ok(resp) => {
            tracing::warn!(
                routine = %payload.routine_name,
                status = %resp.status(),
                "Cron finished-run webhook returned non-success"
            );
        }
        Err(e) => {
            tracing::warn!(
                routine = %payload.routine_name,
                error = %e,
                "Failed to send cron finished-run webhook"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = StaggerConfig::default();
        assert_eq!(config.max_jitter_secs, 30);
        assert_eq!(config.max_concurrent, 3);
        assert!(config.finished_webhook_url.is_none());
    }

    #[test]
    fn test_jitter_delay_zero() {
        let config = StaggerConfig {
            max_jitter_secs: 0,
            ..Default::default()
        };
        assert_eq!(config.jitter_delay(), Duration::ZERO);
    }

    #[test]
    fn test_jitter_delay_bounded() {
        let config = StaggerConfig {
            max_jitter_secs: 5,
            ..Default::default()
        };
        for _ in 0..100 {
            let delay = config.jitter_delay();
            assert!(delay < Duration::from_secs(5));
        }
    }

    #[test]
    fn test_cron_gate_acquire_release() {
        let gate = CronGate::new(2);

        let g1 = gate.try_acquire();
        assert!(g1.is_some());
        assert_eq!(gate.active_count(), 1);

        let g2 = gate.try_acquire();
        assert!(g2.is_some());
        assert_eq!(gate.active_count(), 2);

        // Third should fail
        assert!(gate.try_acquire().is_none());

        // Drop one
        drop(g1);
        assert_eq!(gate.active_count(), 1);

        // Now we can acquire again
        let g3 = gate.try_acquire();
        assert!(g3.is_some());
        assert_eq!(gate.active_count(), 2);
    }

    #[test]
    fn test_finished_run_payload_serialization() {
        let payload = FinishedRunPayload {
            routine_id: "abc-123".to_string(),
            routine_name: "daily-summary".to_string(),
            success: true,
            duration_ms: 1500,
            error: None,
            completed_at: "2026-03-03T18:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("daily-summary"));
        assert!(!json.contains("error")); // skipped when None
    }
}
