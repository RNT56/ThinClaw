use serde::Serialize;

#[derive(Debug, Serialize, specta::Type)]
pub struct PermissionStatus {
    pub accessibility: bool,
    pub screen_recording: bool,
}

#[cfg(target_os = "macos")]
mod macos {
    extern "C" {
        fn AXIsProcessTrusted() -> bool;
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn CGRequestScreenCaptureAccess() -> bool;
    }

    pub fn check_accessibility() -> bool {
        unsafe { AXIsProcessTrusted() }
    }

    pub fn check_screen_recording() -> bool {
        unsafe { CGPreflightScreenCaptureAccess() }
    }

    pub fn request_screen_recording() -> bool {
        unsafe { CGRequestScreenCaptureAccess() }
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

#[tauri::command]
#[specta::specta]
pub fn request_permission(permission: String) {
    #[cfg(target_os = "macos")]
    {
        if permission == "screen_recording" {
            macos::request_screen_recording();
        }
        // Accessibility request is usually triggered by AXIsProcessTrustedWithOptions but we can just use the open URL trick for now if needed,
        // or rely on the system to prompt when we try to use it.
    }
}
