//! Platform abstractions for host-sensitive behavior.
//!
//! Windows support is added through this layer so macOS/Linux behavior can
//! remain stable while individual call sites move off ad-hoc platform checks.

pub mod paths;
pub mod secure_store;
pub mod shell;

pub use paths::{
    StatePaths, expand_home_dir, resolve_data_dir, resolve_temp_path, resolve_thinclaw_home,
    state_paths,
};
pub use shell::{ShellFlavor, ShellLauncher, shell_launcher};

/// Supported service manager kinds for the current host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceManagerKind {
    Launchd,
    SystemdUser,
    WindowsScm,
    None,
}

/// Secure-store availability for the current host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecureStoreKind {
    OsSecureStore,
    EnvOnly,
}

/// Device/runtime capabilities exposed by the local host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceCapabilities {
    pub full_screen_capture: bool,
    pub interactive_screen_capture: bool,
    pub window_screen_capture: bool,
    pub camera_capture: bool,
    pub microphone_capture: bool,
    pub native_location: bool,
}

/// Centralized platform capabilities for host-specific UX and runtime paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformCapabilities {
    pub shell: ShellFlavor,
    pub secure_store: SecureStoreKind,
    pub service_manager: ServiceManagerKind,
    pub local_browser_supported: bool,
    pub docker_browser_fallback_supported: bool,
    pub edge_browser_supported: bool,
    pub brave_browser_supported: bool,
    pub imessage_supported: bool,
    pub apple_mail_supported: bool,
    pub devices: DeviceCapabilities,
}

impl PlatformCapabilities {
    pub fn current() -> Self {
        let shell = shell_launcher().flavor();
        #[cfg(target_os = "windows")]
        let secure_store = SecureStoreKind::OsSecureStore;
        #[cfg(not(target_os = "windows"))]
        let secure_store = SecureStoreKind::OsSecureStore;

        #[cfg(target_os = "macos")]
        let service_manager = ServiceManagerKind::Launchd;
        #[cfg(target_os = "linux")]
        let service_manager = ServiceManagerKind::SystemdUser;
        #[cfg(target_os = "windows")]
        let service_manager = ServiceManagerKind::WindowsScm;
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        let service_manager = ServiceManagerKind::None;

        Self {
            shell,
            secure_store,
            service_manager,
            local_browser_supported: true,
            docker_browser_fallback_supported: true,
            edge_browser_supported: cfg!(target_os = "windows"),
            brave_browser_supported: true,
            imessage_supported: cfg!(target_os = "macos"),
            apple_mail_supported: cfg!(target_os = "macos"),
            devices: DeviceCapabilities {
                full_screen_capture: true,
                interactive_screen_capture: !cfg!(target_os = "windows"),
                window_screen_capture: !cfg!(target_os = "windows"),
                camera_capture: true,
                microphone_capture: true,
                native_location: cfg!(target_os = "macos"),
            },
        }
    }
}

pub fn platform_capabilities() -> PlatformCapabilities {
    PlatformCapabilities::current()
}
