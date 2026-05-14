use std::sync::atomic::{AtomicU64, Ordering};

/// Simple sliding-window rate limiter.
///
/// Tracks the number of requests in the current window. Resets when the window expires.
/// Not per-IP (since this is a single-user gateway with auth), but prevents flooding.
pub struct RateLimiter {
    /// Requests remaining in the current window.
    remaining: AtomicU64,
    /// Epoch second when the current window started.
    window_start: AtomicU64,
    /// Maximum requests per window.
    max_requests: u64,
    /// Window duration in seconds.
    window_secs: u64,
}

impl RateLimiter {
    pub fn new(max_requests: u64, window_secs: u64) -> Self {
        Self {
            remaining: AtomicU64::new(max_requests),
            window_start: AtomicU64::new(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            ),
            max_requests,
            window_secs,
        }
    }

    /// Try to consume one request. Returns `true` if allowed, `false` if rate limited.
    pub fn check(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let window = self.window_start.load(Ordering::Acquire);
        if now.saturating_sub(window) >= self.window_secs {
            // Window expired - try to reset. Only one thread wins the CAS.
            if self
                .window_start
                .compare_exchange(window, now, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                self.remaining
                    .store(self.max_requests - 1, Ordering::Release);
                return true;
            }
            // Lost the race - another thread already reset. Fall through
            // to the normal decrement path.
        }

        // Try to decrement remaining.
        loop {
            let current = self.remaining.load(Ordering::Acquire);
            if current == 0 {
                return false;
            }
            if self
                .remaining
                .compare_exchange_weak(current, current - 1, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                return true;
            }
        }
    }
}
