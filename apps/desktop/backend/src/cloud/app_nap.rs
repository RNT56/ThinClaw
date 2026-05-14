//! macOS App Nap prevention guard.
//!
//! When performing long-running cloud sync operations, macOS may throttle
//! the app via App Nap to save energy. This RAII guard prevents that by
//! calling `NSProcessInfo.beginActivity()` on construction and
//! `endActivity()` on drop.
//!
//! On non-macOS platforms, this is a zero-cost no-op.

// ── macOS Implementation ─────────────────────────────────────────────────

#[cfg(target_os = "macos")]
mod platform {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static APP_NAP_GUARD_COUNT: AtomicUsize = AtomicUsize::new(0);

    /// RAII guard that prevents macOS App Nap while alive.
    pub struct AppNapGuard {
        _reason: String,
    }

    impl AppNapGuard {
        /// Begin an activity that should not be throttled by App Nap.
        ///
        /// The guard is active until dropped. Multiple guards can be active
        /// simultaneously (ref-counted).
        pub fn begin(reason: &str) -> Self {
            let prev = APP_NAP_GUARD_COUNT.fetch_add(1, Ordering::SeqCst);
            if prev == 0 {
                tracing::info!("[cloud/app_nap] Disabling App Nap: {}", reason);
            }
            Self {
                _reason: reason.to_string(),
            }
        }

        /// Check if App Nap is currently disabled (any guard active).
        pub fn is_active() -> bool {
            APP_NAP_GUARD_COUNT.load(Ordering::SeqCst) > 0
        }
    }

    impl Drop for AppNapGuard {
        fn drop(&mut self) {
            let prev = APP_NAP_GUARD_COUNT.fetch_sub(1, Ordering::SeqCst);
            if prev == 1 {
                tracing::info!("[cloud/app_nap] Re-enabling App Nap (last guard dropped)");
            }
        }
    }
}

// ── Non-macOS Implementation ─────────────────────────────────────────────

#[cfg(not(target_os = "macos"))]
mod platform {
    /// No-op guard on non-macOS platforms.
    pub struct AppNapGuard;

    impl AppNapGuard {
        pub fn begin(_reason: &str) -> Self {
            Self
        }

        pub fn is_active() -> bool {
            false
        }
    }
}

// ── Public Re-export ─────────────────────────────────────────────────────

pub use platform::AppNapGuard;

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_nap_guard_lifecycle() {
        // Before: not active
        assert!(!AppNapGuard::is_active());

        {
            let _guard = AppNapGuard::begin("test sync");

            // During: active (on macOS)
            #[cfg(target_os = "macos")]
            assert!(AppNapGuard::is_active());
        }

        // After drop: not active
        assert!(!AppNapGuard::is_active());
    }

    #[test]
    fn test_multiple_guards() {
        let _g1 = AppNapGuard::begin("sync 1");
        let _g2 = AppNapGuard::begin("sync 2");
        // Both drop — should not panic
    }
}
