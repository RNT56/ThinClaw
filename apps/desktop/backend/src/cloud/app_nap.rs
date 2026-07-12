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

    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2_foundation::{NSActivityOptions, NSObjectProtocol, NSProcessInfo, NSString};

    static APP_NAP_GUARD_COUNT: AtomicUsize = AtomicUsize::new(0);

    /// RAII guard that prevents macOS App Nap while alive.
    pub struct AppNapGuard {
        process_info: Retained<NSProcessInfo>,
        activity: Retained<ProtocolObject<dyn NSObjectProtocol>>,
    }

    // SAFETY: NSProcessInfo is explicitly Send + Sync in objc2-foundation.
    // The opaque activity token is owned exclusively by this guard, is never
    // exposed or shared, and is only passed back to NSProcessInfo once from
    // Drop. This permits a Tokio task holding the guard to migrate threads.
    unsafe impl Send for AppNapGuard {}

    impl AppNapGuard {
        /// Begin an activity that should not be throttled by App Nap.
        ///
        /// The guard is active until dropped. Multiple guards can be active
        /// simultaneously (ref-counted).
        pub fn begin(reason: &str) -> Self {
            let process_info = NSProcessInfo::processInfo();
            let ns_reason = NSString::from_str(reason);
            let activity = process_info.beginActivityWithOptions_reason(
                NSActivityOptions::UserInitiatedAllowingIdleSystemSleep,
                &ns_reason,
            );
            let prev = APP_NAP_GUARD_COUNT.fetch_add(1, Ordering::SeqCst);
            if prev == 0 {
                tracing::info!("[cloud/app_nap] Disabling App Nap: {}", reason);
            }
            Self {
                process_info,
                activity,
            }
        }

        /// Check if App Nap is currently disabled (any guard active).
        pub fn is_active() -> bool {
            APP_NAP_GUARD_COUNT.load(Ordering::SeqCst) > 0
        }
    }

    impl Drop for AppNapGuard {
        fn drop(&mut self) {
            // SAFETY: `activity` is the live token returned by this exact
            // NSProcessInfo instance and is ended exactly once from Drop.
            unsafe {
                self.process_info.endActivity(&self.activity);
            }
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
    use std::sync::Mutex;

    static APP_NAP_TEST_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn test_app_nap_guard_lifecycle() {
        let _guard_lock = APP_NAP_TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
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
        let _guard_lock = APP_NAP_TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _g1 = AppNapGuard::begin("sync 1");
        let _g2 = AppNapGuard::begin("sync 2");
        // Both drop — should not panic
    }
}
