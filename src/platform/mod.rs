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

fn command_available(command: &str) -> bool {
    std::process::Command::new(command)
        .arg("--version")
        .output()
        .is_ok()
}

fn any_command_available(commands: &[&str]) -> bool {
    commands.iter().any(|command| command_available(command))
}

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
        let docker_available = any_command_available(&["docker"]);
        let browser_available = if cfg!(target_os = "macos") {
            any_command_available(&["open"])
        } else if cfg!(target_os = "windows") {
            any_command_available(&["cmd"])
        } else {
            any_command_available(&["xdg-open", "gio"])
        };
        let brave_available = if cfg!(target_os = "macos") {
            std::path::Path::new("/Applications/Brave Browser.app").exists()
        } else {
            any_command_available(&["brave-browser", "brave"])
        };
        let edge_available = if cfg!(target_os = "windows") {
            any_command_available(&["msedge", "microsoft-edge"])
        } else if cfg!(target_os = "macos") {
            std::path::Path::new("/Applications/Microsoft Edge.app").exists()
        } else {
            any_command_available(&["microsoft-edge", "microsoft-edge-stable", "msedge"])
        };

        Self {
            shell,
            secure_store,
            service_manager,
            local_browser_supported: browser_available,
            docker_browser_fallback_supported: docker_available,
            edge_browser_supported: edge_available,
            brave_browser_supported: brave_available,
            imessage_supported: cfg!(target_os = "macos"),
            apple_mail_supported: cfg!(target_os = "macos"),
            devices: DeviceCapabilities {
                full_screen_capture: cfg!(target_os = "macos")
                    || cfg!(target_os = "linux")
                    || cfg!(target_os = "windows"),
                interactive_screen_capture: cfg!(target_os = "macos") || cfg!(target_os = "linux"),
                window_screen_capture: cfg!(target_os = "macos") || cfg!(target_os = "linux"),
                camera_capture: any_command_available(&["ffmpeg"]),
                microphone_capture: any_command_available(&["ffmpeg"]),
                native_location: cfg!(target_os = "macos"),
            },
        }
    }
}

pub fn platform_capabilities() -> PlatformCapabilities {
    PlatformCapabilities::current()
}
