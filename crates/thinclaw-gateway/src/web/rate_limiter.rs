use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

const MAX_KEYED_BUCKETS: usize = 4096;
const MAX_RATE_LIMIT_KEY_BYTES: usize = 512;

#[derive(Debug, Clone, Copy)]
struct KeyedBucket {
    remaining: u64,
    window_start: u64,
}

/// Simple sliding-window rate limiter.
///
/// Tracks the number of requests in the current window. Resets when the window expires.
/// The unkeyed [`Self::check`] method protects truly global/public surfaces.
/// Authenticated multi-principal surfaces should use [`Self::check_for`] so
/// one client cannot exhaust every other client's allowance.
pub struct RateLimiter {
    /// Requests remaining in the current window.
    remaining: AtomicU64,
    /// Epoch second when the current window started.
    window_start: AtomicU64,
    /// Maximum requests per window.
    max_requests: u64,
    /// Window duration in seconds.
    window_secs: u64,
    /// Per-principal/device windows. Cardinality is bounded so an attacker
    /// cannot turn arbitrary identity strings into unbounded gateway memory.
    keyed_buckets: Mutex<HashMap<String, KeyedBucket>>,
}

impl RateLimiter {
    pub fn new(max_requests: u64, window_secs: u64) -> Self {
        // A zero-sized limiter used to underflow at the first window reset and
        // effectively become unlimited in release builds. Keep construction
        // total and fail toward the smallest meaningful allowance instead.
        let max_requests = max_requests.max(1);
        let window_secs = window_secs.max(1);
        Self {
            remaining: AtomicU64::new(max_requests),
            window_start: AtomicU64::new(now_epoch_seconds()),
            max_requests,
            window_secs,
            keyed_buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Try to consume one request. Returns `true` if allowed, `false` if rate limited.
    pub fn check(&self) -> bool {
        let now = now_epoch_seconds();

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

    /// Consume one request from a bounded, independent bucket for `key`.
    ///
    /// Invalid/oversized keys and poisoned state fail closed. Expired buckets
    /// are pruned before the cardinality cap is enforced, so normal identity
    /// churn does not permanently consume the table.
    pub fn check_for(&self, key: &str) -> bool {
        if key.is_empty()
            || key.len() > MAX_RATE_LIMIT_KEY_BYTES
            || key.chars().any(char::is_control)
        {
            return false;
        }

        let now = now_epoch_seconds();
        let Ok(mut buckets) = self.keyed_buckets.lock() else {
            return false;
        };

        if !buckets.contains_key(key) && buckets.len() >= MAX_KEYED_BUCKETS {
            let window_secs = self.window_secs;
            buckets.retain(|_, bucket| now.saturating_sub(bucket.window_start) < window_secs);
            if buckets.len() >= MAX_KEYED_BUCKETS {
                return false;
            }
        }

        let bucket = buckets.entry(key.to_owned()).or_insert(KeyedBucket {
            remaining: self.max_requests,
            window_start: now,
        });
        if now.saturating_sub(bucket.window_start) >= self.window_secs {
            bucket.window_start = now;
            bucket.remaining = self.max_requests;
        }
        if bucket.remaining == 0 {
            return false;
        }
        bucket.remaining -= 1;
        true
    }
}

fn now_epoch_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyed_buckets_are_independent() {
        let limiter = RateLimiter::new(1, 60);
        assert!(limiter.check_for("principal:a"));
        assert!(!limiter.check_for("principal:a"));
        assert!(limiter.check_for("principal:b"));
    }

    #[test]
    fn invalid_keys_fail_closed() {
        let limiter = RateLimiter::new(1, 60);
        assert!(!limiter.check_for(""));
        assert!(!limiter.check_for("bad\nkey"));
        assert!(!limiter.check_for(&"x".repeat(MAX_RATE_LIMIT_KEY_BYTES + 1)));
    }

    #[test]
    fn zero_constructor_values_are_safe_and_bounded() {
        let limiter = RateLimiter::new(0, 0);
        assert!(limiter.check());
        assert!(!limiter.check());
        assert!(limiter.check_for("a"));
        assert!(!limiter.check_for("a"));
    }
}
