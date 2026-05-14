use serde::Serialize;

#[derive(Debug, Serialize, specta::Type)]
pub struct PermissionStatus {
    pub accessibility: bool,
    pub screen_recording: bool,
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::c_void;
    use std::process::Command;

    // Core Foundation types (opaque pointers)
    type CFDictionaryRef = *const c_void;
    type CFStringRef = *const c_void;
    type CFBooleanRef = *const c_void;
    type CFTypeRef = *const c_void;

    extern "C" {
        // Accessibility
        fn AXIsProcessTrusted() -> bool;
        fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;

        // Screen recording
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn CGRequestScreenCaptureAccess() -> bool;

        // Core Foundation helpers (always available on macOS)
        fn CFStringCreateWithCString(
            allocator: *const c_void,
            string: *const u8,
            encoding: u32,
        ) -> CFStringRef;
        fn CFDictionaryCreate(
            allocator: *const c_void,
            keys: *const CFTypeRef,
            values: *const CFTypeRef,
            count: isize,
            key_callbacks: *const c_void,
            value_callbacks: *const c_void,
        ) -> CFDictionaryRef;
        fn CFRelease(cf: *const c_void);

        // kCFBooleanTrue is a global constant
        static kCFBooleanTrue: CFBooleanRef;
        static kCFTypeDictionaryKeyCallBacks: c_void;
        static kCFTypeDictionaryValueCallBacks: c_void;
    }

    const K_CF_STRING_ENCODING_UTF8: u32 = 0x08000100;

    pub fn check_accessibility() -> bool {
        unsafe { AXIsProcessTrusted() }
    }

    /// Request accessibility permission using AXIsProcessTrustedWithOptions.
    ///
    /// When called with kAXTrustedCheckOptionPrompt = true, macOS will show
    /// the native system dialog prompting the user to grant Accessibility
    /// access in System Settings > Privacy & Security > Accessibility.
    ///
    /// Returns whether the app currently has accessibility permission.
    pub fn request_accessibility() -> bool {
        unsafe {
            // Create the key: "AXTrustedCheckOptionPrompt"
            let key_str = b"AXTrustedCheckOptionPrompt\0";
            let key = CFStringCreateWithCString(
                std::ptr::null(),
                key_str.as_ptr(),
                K_CF_STRING_ENCODING_UTF8,
            );

            if key.is_null() {
                return AXIsProcessTrusted();
            }

            // Create dictionary: { kAXTrustedCheckOptionPrompt: true }
            let keys = [key as CFTypeRef];
            let values = [kCFBooleanTrue as CFTypeRef];
            let dict = CFDictionaryCreate(
                std::ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
                &kCFTypeDictionaryKeyCallBacks as *const c_void,
                &kCFTypeDictionaryValueCallBacks as *const c_void,
            );

            let result = AXIsProcessTrustedWithOptions(dict);

            // Clean up
            if !dict.is_null() {
                CFRelease(dict);
            }
            CFRelease(key);

            result
        }
    }

    pub fn check_screen_recording() -> bool {
        unsafe { CGPreflightScreenCaptureAccess() }
    }

    pub fn request_screen_recording() -> bool {
        unsafe { CGRequestScreenCaptureAccess() }
    }

    /// Open System Settings to the specific privacy pane.
    ///
    /// macOS only shows each permission dialog once per app lifecycle.
    /// After the first prompt, subsequent calls are silent — so we fall
    /// back to opening the relevant System Settings pane directly.
    ///
    /// Also used for revoking: macOS doesn't support programmatic revocation,
    /// so we send the user to System Settings to toggle it off manually.
    pub fn open_privacy_settings(permission: &str) {
        let url = match permission {
            "accessibility" => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
            }
            "screen_recording" => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
            }
            _ => return,
        };

        tracing::info!("[permissions] Opening System Settings: {}", url);
        let _ = Command::new("open").arg(url).spawn();
    }
}

#[tauri::command]
#[specta::specta]
pub fn get_permission_status() -> PermissionStatus {
    #[cfg(target_os = "macos")]
    {
        PermissionStatus {
            accessibility: macos::check_accessibility(),
            screen_recording: macos::check_screen_recording(),
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        PermissionStatus {
            accessibility: true,
            screen_recording: true,
        }
    }
}

/// Request a specific OS permission and return the updated status.
///
/// Flow:
/// 1. Call the native macOS API to trigger the permission dialog
/// 2. If the permission was granted immediately, return the updated status
/// 3. If not granted (dialog was dismissed or already shown once before),
///    open System Settings to the relevant pane as a fallback
///
/// Returns the current PermissionStatus so the frontend can update the UI.
#[tauri::command]
#[specta::specta]
pub fn request_permission(permission: String) -> PermissionStatus {
    #[cfg(target_os = "macos")]
    {
        let already_granted = match permission.as_str() {
            "accessibility" => macos::check_accessibility(),
            "screen_recording" => macos::check_screen_recording(),
            _ => false,
        };

        if already_granted {
            tracing::info!("[permissions] {} already granted", permission);
            return PermissionStatus {
                accessibility: macos::check_accessibility(),
                screen_recording: macos::check_screen_recording(),
            };
        }

        // Try native dialog first
        let granted_after_request = match permission.as_str() {
            "accessibility" => {
                tracing::info!(
                    "[permissions] Requesting accessibility via AXIsProcessTrustedWithOptions"
                );
                macos::request_accessibility()
            }
            "screen_recording" => {
                tracing::info!(
                    "[permissions] Requesting screen recording via CGRequestScreenCaptureAccess"
                );
                macos::request_screen_recording()
            }
            _ => {
                tracing::warn!("[permissions] Unknown permission requested: {}", permission);
                false
            }
        };

        // If not granted after native request, the dialog was either dismissed
        // or already shown once before. Open System Settings as fallback.
        if !granted_after_request {
            tracing::info!(
                "[permissions] {} not granted after native request, opening System Settings",
                permission
            );
            macos::open_privacy_settings(&permission);
        }

        PermissionStatus {
            accessibility: macos::check_accessibility(),
            screen_recording: macos::check_screen_recording(),
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = permission;
        PermissionStatus {
            accessibility: true,
            screen_recording: true,
        }
    }
}

/// Open System Settings to the relevant privacy pane for a permission.
///
/// Used when the user wants to revoke a previously granted permission
/// (macOS doesn't support programmatic revocation).
#[tauri::command]
#[specta::specta]
pub fn open_permission_settings(permission: String) {
    #[cfg(target_os = "macos")]
    {
        macos::open_privacy_settings(&permission);
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = permission;
        tracing::info!("[permissions] open_permission_settings not supported on this OS");
    }
}
