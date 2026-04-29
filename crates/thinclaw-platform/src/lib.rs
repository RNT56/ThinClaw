//! Platform abstractions for host-sensitive behavior.
//!
//! Windows support is added through this layer so macOS/Linux behavior can
//! remain stable while individual call sites move off ad-hoc platform checks.

pub mod paths;
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

fn executable_in_path(binary: &str) -> Option<std::path::PathBuf> {
    let path_env = std::env::var_os("PATH")?;
    std::env::split_paths(&path_env)
        .map(|dir| dir.join(binary))
        .find(|candidate| candidate.is_file())
}

pub fn executable_available(binary: &str) -> bool {
    let path = std::path::Path::new(binary);
    if path.is_absolute() || binary.contains(std::path::MAIN_SEPARATOR) {
        path.is_file()
    } else {
        executable_in_path(binary).is_some()
    }
}

pub fn env_flag_enabled(key: &str) -> bool {
    std::env::var(key)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserDockerMode {
    Auto,
    Always,
    Never,
}

impl BrowserDockerMode {
    pub fn parse(value: Option<&str>) -> Result<Self, String> {
        match value.map(str::trim).filter(|value| !value.is_empty()) {
            None => Ok(Self::Auto),
            Some(value) if value.eq_ignore_ascii_case("auto") => Ok(Self::Auto),
            Some(value) if value.eq_ignore_ascii_case("always") => Ok(Self::Always),
            Some(value) if value.eq_ignore_ascii_case("never") => Ok(Self::Never),
            Some(value) => Err(format!(
                "invalid BROWSER_DOCKER value '{value}' (expected auto, always, or never)"
            )),
        }
    }

    pub fn from_env() -> Result<Self, String> {
        Self::parse(std::env::var("BROWSER_DOCKER").ok().as_deref())
    }

    pub fn from_env_lossy() -> Self {
        match Self::from_env() {
            Ok(mode) => mode,
            Err(error) => {
                tracing::warn!(%error, "Falling back to BROWSER_DOCKER=auto");
                Self::Auto
            }
        }
    }

    pub fn allows_docker(self) -> bool {
        !matches!(self, Self::Never)
    }

    pub fn forces_docker(self) -> bool {
        matches!(self, Self::Always)
    }
}

fn browser_env_override() -> Option<std::path::PathBuf> {
    for key in ["BROWSER_EXECUTABLE", "CHROME_PATH"] {
        let Ok(raw) = std::env::var(key) else {
            continue;
        };
        let value = raw.trim();
        if value.is_empty() {
            continue;
        }
        let path = std::path::PathBuf::from(value);
        if path.is_file() {
            return Some(path);
        }
        if !path.is_absolute()
            && !value.contains(std::path::MAIN_SEPARATOR)
            && let Some(found) = executable_in_path(value)
        {
            return Some(found);
        }
    }
    None
}

pub fn browser_binary_names() -> &'static [&'static str] {
    if cfg!(target_os = "windows") {
        &["chrome.exe", "msedge.exe", "brave.exe"]
    } else if cfg!(target_os = "linux") {
        &[
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chromium-browser",
            "brave-browser",
            "brave",
            "microsoft-edge",
            "microsoft-edge-stable",
            "msedge",
        ]
    } else {
        &[
            "google-chrome",
            "chromium",
            "brave-browser",
            "microsoft-edge",
        ]
    }
}

pub fn browser_absolute_candidates() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &[
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
            "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
            "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        ]
    } else if cfg!(target_os = "linux") {
        &[
            "/usr/bin/google-chrome",
            "/usr/bin/google-chrome-stable",
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
            "/snap/bin/chromium",
            "/usr/bin/brave-browser",
            "/snap/bin/brave",
            "/usr/bin/microsoft-edge",
            "/usr/bin/microsoft-edge-stable",
            "/opt/microsoft/msedge/microsoft-edge",
        ]
    } else if cfg!(target_os = "windows") {
        &[
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe",
            r"C:\Program Files (x86)\BraveSoftware\Brave-Browser\Application\brave.exe",
        ]
    } else {
        &[]
    }
}

pub fn find_browser_executable() -> Option<std::path::PathBuf> {
    if let Some(path) = browser_env_override() {
        return Some(path);
    }

    for candidate in browser_absolute_candidates() {
        let path = std::path::PathBuf::from(candidate);
        if path.is_file() {
            return Some(path);
        }
    }

    browser_binary_names()
        .iter()
        .find_map(|binary| executable_in_path(binary))
}

pub fn linux_screen_capture_commands() -> &'static [&'static str] {
    &["gnome-screenshot", "scrot", "import"]
}

pub fn linux_screen_capture_available() -> bool {
    linux_screen_capture_commands()
        .iter()
        .any(|command| executable_available(command))
}

fn linux_video_device_available() -> bool {
    std::fs::read_dir("/dev")
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .any(|entry| entry.file_name().to_string_lossy().starts_with("video"))
}

fn linux_audio_source_hint_available() -> bool {
    let xdg_runtime_dir = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from);
    let pipewire_runtime_dir =
        std::env::var_os("PIPEWIRE_RUNTIME_DIR").map(std::path::PathBuf::from);

    std::env::var_os("THINCLAW_MICROPHONE_DEVICE").is_some()
        || std::env::var_os("PULSE_SERVER").is_some()
        || pipewire_runtime_dir
            .as_ref()
            .is_some_and(|dir| dir.join("pipewire-0").exists())
        || xdg_runtime_dir
            .as_ref()
            .is_some_and(|dir| dir.join("pipewire-0").exists() || dir.join("pulse/native").exists())
        || std::path::Path::new("/proc/asound/cards").exists()
        || std::path::Path::new("/dev/snd").exists()
        || executable_available("arecord")
        || executable_available("pactl")
}

fn linux_geoclue_service_hint_available() -> bool {
    [
        "/usr/share/dbus-1/system-services/org.freedesktop.GeoClue2.service",
        "/usr/local/share/dbus-1/system-services/org.freedesktop.GeoClue2.service",
    ]
    .iter()
    .any(|path| std::path::Path::new(path).exists())
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
    Unavailable,
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
        #[cfg(target_os = "macos")]
        let secure_store = SecureStoreKind::OsSecureStore;
        #[cfg(target_os = "linux")]
        let secure_store = if std::env::var_os("SECRETS_MASTER_KEY").is_some()
            && env_flag_enabled("THINCLAW_ALLOW_ENV_MASTER_KEY")
        {
            SecureStoreKind::EnvOnly
        } else if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some()
            || std::env::var_os("XDG_RUNTIME_DIR").is_some()
        {
            SecureStoreKind::OsSecureStore
        } else {
            SecureStoreKind::Unavailable
        };
        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        let secure_store = SecureStoreKind::EnvOnly;

        #[cfg(target_os = "macos")]
        let service_manager = ServiceManagerKind::Launchd;
        #[cfg(target_os = "linux")]
        let service_manager = ServiceManagerKind::SystemdUser;
        #[cfg(target_os = "windows")]
        let service_manager = ServiceManagerKind::WindowsScm;
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        let service_manager = ServiceManagerKind::None;
        let docker_available = any_command_available(&["docker"]);
        let browser_available = find_browser_executable().is_some();
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
        let linux_screen_capture_available =
            cfg!(target_os = "linux") && linux_screen_capture_available();
        let linux_camera_capture_available = cfg!(target_os = "linux")
            && any_command_available(&["ffmpeg", "fswebcam"])
            && linux_video_device_available();
        let linux_microphone_capture_available = cfg!(target_os = "linux")
            && executable_available("ffmpeg")
            && linux_audio_source_hint_available();
        let linux_native_location_available = cfg!(target_os = "linux")
            && executable_available("gdbus")
            && linux_geoclue_service_hint_available();

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
                    || linux_screen_capture_available
                    || cfg!(target_os = "windows"),
                interactive_screen_capture: cfg!(target_os = "macos")
                    || linux_screen_capture_available,
                window_screen_capture: cfg!(target_os = "macos") || linux_screen_capture_available,
                camera_capture: if cfg!(target_os = "linux") {
                    linux_camera_capture_available
                } else {
                    any_command_available(&["ffmpeg", "fswebcam"])
                },
                microphone_capture: if cfg!(target_os = "linux") {
                    linux_microphone_capture_available
                } else {
                    any_command_available(&["ffmpeg"])
                },
                native_location: cfg!(target_os = "macos") || linux_native_location_available,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_browser_docker_modes() {
        assert_eq!(
            BrowserDockerMode::parse(None).unwrap(),
            BrowserDockerMode::Auto
        );
        assert_eq!(
            BrowserDockerMode::parse(Some("always")).unwrap(),
            BrowserDockerMode::Always
        );
        assert_eq!(
            BrowserDockerMode::parse(Some("never")).unwrap(),
            BrowserDockerMode::Never
        );
        assert!(BrowserDockerMode::parse(Some("sometimes")).is_err());
    }

    #[test]
    fn browser_candidate_list_mentions_linux_brave_and_edge() {
        if cfg!(target_os = "linux") {
            let names = browser_binary_names();
            assert!(names.contains(&"brave-browser"));
            assert!(names.contains(&"microsoft-edge"));
        }
    }
}

pub fn platform_capabilities() -> PlatformCapabilities {
    PlatformCapabilities::current()
}
