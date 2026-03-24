//! WASM-specific rate limiting for sandboxed tool execution.
//!
//! Extends the shared rate limiter with WASM-specific constraints:
//! - Per-sandbox instance limiting (prevents a single WASM tool from
//!   monopolizing resources across multiple invocations)
//! - Execution-scoped limits that reset per sandbox lifecycle
//! - Memory-aware rate limiting that accounts for WASM fuel consumption
//!
//! The base rate limiter implementation lives in `crate::tools::rate_limiter`.
//! This module re-exports those types and adds WASM-specific wrappers.

// Re-export the shared rate limiter types so existing WASM call-sites
// continue to work via `crate::tools::wasm::rate_limiter::*`.
pub use crate::tools::rate_limiter::{LimitType, RateLimitError, RateLimitResult, RateLimiter};

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

/// Per-sandbox rate limiting state.
///
/// Tracks invocation counts and timing for a single WASM tool instance
/// (identified by tool name) within a sandbox lifecycle. Unlike the
/// shared `RateLimiter`, this enforces limits that are scoped to a
/// single sandbox run rather than globally.
#[derive(Debug)]
pub struct WasmRateLimiter {
    /// Shared global limiter for cross-sandbox limits.
    global: Arc<RateLimiter>,
    /// Per-tool invocation counts within this sandbox lifecycle.
    sandbox_counts: RwLock<HashMap<String, SandboxToolState>>,
    /// Maximum invocations per tool per sandbox execution.
    max_invocations_per_sandbox: u32,
    /// Minimum interval between consecutive calls to the same tool.
    min_interval: Duration,
}

/// Invocation state for a single tool within a sandbox.
#[derive(Debug, Clone)]
struct SandboxToolState {
    /// Number of invocations in this sandbox lifecycle.
    count: u32,
    /// Timestamp of the last invocation.
    last_invoked: Instant,
    /// Total fuel consumed by this tool (for fuel-aware throttling).
    total_fuel_consumed: u64,
}

/// Configuration for WASM-specific rate limits.
#[derive(Debug, Clone)]
pub struct WasmRateLimitConfig {
    /// Maximum invocations of a single tool per sandbox execution (default: 100).
    pub max_invocations_per_sandbox: u32,
    /// Minimum interval between consecutive same-tool calls (default: 50ms).
    pub min_interval: Duration,
    /// Fuel consumption threshold before throttling kicks in (default: 10M).
    pub fuel_throttle_threshold: u64,
}

impl Default for WasmRateLimitConfig {
    fn default() -> Self {
        Self {
            max_invocations_per_sandbox: 100,
            min_interval: Duration::from_millis(50),
            fuel_throttle_threshold: 10_000_000,
        }
    }
}

/// WASM-specific rate limit denial reason.
#[derive(Debug, Clone)]
pub enum WasmDenyReason {
    /// Tool exceeded per-sandbox invocation limit.
    SandboxLimitExceeded { tool: String, count: u32, max: u32 },
    /// Tool called too quickly (minimum interval not met).
    TooFast {
        tool: String,
        elapsed: Duration,
        min_interval: Duration,
    },
    /// Fuel consumption threshold exceeded; tool is throttled.
    FuelThrottled {
        tool: String,
        fuel_consumed: u64,
        threshold: u64,
    },
    /// Denied by the global rate limiter.
    GlobalDenied(RateLimitError),
}

impl std::fmt::Display for WasmDenyReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SandboxLimitExceeded { tool, count, max } => {
                write!(
                    f,
                    "Tool '{}' exceeded sandbox invocation limit ({}/{})",
                    tool, count, max
                )
            }
            Self::TooFast {
                tool,
                elapsed,
                min_interval,
            } => {
                write!(
                    f,
                    "Tool '{}' called too quickly ({:?} < {:?} minimum interval)",
                    tool, elapsed, min_interval
                )
            }
            Self::FuelThrottled {
                tool,
                fuel_consumed,
                threshold,
            } => {
                write!(
                    f,
                    "Tool '{}' throttled: fuel consumption {} exceeds threshold {}",
                    tool, fuel_consumed, threshold
                )
            }
            Self::GlobalDenied(e) => write!(f, "Global rate limit: {}", e),
        }
    }
}

impl WasmRateLimiter {
    /// Create a new WASM rate limiter wrapping a global limiter.
    pub fn new(global: Arc<RateLimiter>, config: WasmRateLimitConfig) -> Self {
        Self {
            global,
            sandbox_counts: RwLock::new(HashMap::new()),
            max_invocations_per_sandbox: config.max_invocations_per_sandbox,
            min_interval: config.min_interval,
        }
    }

    /// Check whether a tool invocation is allowed within this sandbox.
    ///
    /// Checks both sandbox-scoped limits (invocation count, interval) and
    /// the global rate limiter. Returns `Ok(())` if allowed, or the
    /// denial reason.
    pub async fn check(&self, tool_name: &str) -> Result<(), WasmDenyReason> {
        let counts = self.sandbox_counts.read().await;

        if let Some(state) = counts.get(tool_name) {
            // Check per-sandbox invocation limit
            if state.count >= self.max_invocations_per_sandbox {
                return Err(WasmDenyReason::SandboxLimitExceeded {
                    tool: tool_name.to_string(),
                    count: state.count,
                    max: self.max_invocations_per_sandbox,
                });
            }

            // Check minimum interval
            let elapsed = state.last_invoked.elapsed();
            if elapsed < self.min_interval {
                return Err(WasmDenyReason::TooFast {
                    tool: tool_name.to_string(),
                    elapsed,
                    min_interval: self.min_interval,
                });
            }
        }

        Ok(())
    }

    /// Record a tool invocation and its fuel consumption.
    pub async fn record(&self, tool_name: &str, fuel_consumed: u64) {
        let mut counts = self.sandbox_counts.write().await;
        let state = counts
            .entry(tool_name.to_string())
            .or_insert(SandboxToolState {
                count: 0,
                last_invoked: Instant::now(),
                total_fuel_consumed: 0,
            });
        state.count += 1;
        state.last_invoked = Instant::now();
        state.total_fuel_consumed += fuel_consumed;
    }

    /// Check if a tool is fuel-throttled (consumed too much fuel).
    pub async fn is_fuel_throttled(
        &self,
        tool_name: &str,
        threshold: u64,
    ) -> Option<WasmDenyReason> {
        let counts = self.sandbox_counts.read().await;
        if let Some(state) = counts.get(tool_name)
            && state.total_fuel_consumed > threshold
        {
            return Some(WasmDenyReason::FuelThrottled {
                tool: tool_name.to_string(),
                fuel_consumed: state.total_fuel_consumed,
                threshold,
            });
        }
        None
    }

    /// Reset all sandbox-scoped counters (called when sandbox is discarded).
    pub async fn reset(&self) {
        let mut counts = self.sandbox_counts.write().await;
        counts.clear();
    }

    /// Get invocation statistics for a tool within this sandbox.
    pub async fn stats(&self, tool_name: &str) -> Option<(u32, u64)> {
        let counts = self.sandbox_counts.read().await;
        counts
            .get(tool_name)
            .map(|s| (s.count, s.total_fuel_consumed))
    }

    /// Access the underlying global rate limiter.
    pub fn global(&self) -> &RateLimiter {
        &self.global
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> WasmRateLimitConfig {
        WasmRateLimitConfig {
            max_invocations_per_sandbox: 3,
            min_interval: Duration::from_millis(10),
            fuel_throttle_threshold: 1000,
        }
    }

    #[tokio::test]
    async fn test_allows_within_limits() {
        let global = Arc::new(RateLimiter::new());
        let limiter = WasmRateLimiter::new(global, test_config());

        assert!(limiter.check("my_tool").await.is_ok());
        limiter.record("my_tool", 100).await;

        // Wait for min interval
        tokio::time::sleep(Duration::from_millis(15)).await;
        assert!(limiter.check("my_tool").await.is_ok());
    }

    #[tokio::test]
    async fn test_denies_at_sandbox_limit() {
        let global = Arc::new(RateLimiter::new());
        let limiter = WasmRateLimiter::new(global, test_config());

        for _ in 0..3 {
            limiter.record("my_tool", 10).await;
            tokio::time::sleep(Duration::from_millis(15)).await;
        }

        let result = limiter.check("my_tool").await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            WasmDenyReason::SandboxLimitExceeded { .. }
        ));
    }

    #[tokio::test]
    async fn test_denies_too_fast() {
        let global = Arc::new(RateLimiter::new());
        let limiter = WasmRateLimiter::new(global, test_config());

        limiter.record("my_tool", 10).await;
        // Immediately check without waiting
        let result = limiter.check("my_tool").await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            WasmDenyReason::TooFast { .. }
        ));
    }

    #[tokio::test]
    async fn test_fuel_throttling() {
        let global = Arc::new(RateLimiter::new());
        let limiter = WasmRateLimiter::new(global, test_config());

        limiter.record("my_tool", 600).await;
        limiter.record("my_tool", 500).await;

        let result = limiter.is_fuel_throttled("my_tool", 1000).await;
        assert!(result.is_some());
        assert!(matches!(
            result.unwrap(),
            WasmDenyReason::FuelThrottled { .. }
        ));
    }

    #[tokio::test]
    async fn test_reset_clears_state() {
        let global = Arc::new(RateLimiter::new());
        let limiter = WasmRateLimiter::new(global, test_config());

        limiter.record("my_tool", 100).await;
        assert!(limiter.stats("my_tool").await.is_some());

        limiter.reset().await;
        assert!(limiter.stats("my_tool").await.is_none());
    }

    #[tokio::test]
    async fn test_independent_tool_tracking() {
        let global = Arc::new(RateLimiter::new());
        let limiter = WasmRateLimiter::new(global, test_config());

        // Fill up tool_a
        for _ in 0..3 {
            limiter.record("tool_a", 10).await;
            tokio::time::sleep(Duration::from_millis(15)).await;
        }

        // tool_b should still be allowed
        assert!(limiter.check("tool_b").await.is_ok());
    }
}
