use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinuxReadinessProfile {
    Server,
    Remote,
    DesktopGnome,
    PiOsLite64,
    AllFeatures,
}

impl LinuxReadinessProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Remote => "remote",
            Self::DesktopGnome => "desktop-gnome",
            Self::PiOsLite64 => "pi-os-lite-64",
            Self::AllFeatures => "all-features",
        }
    }

    fn needs_desktop(self) -> bool {
        matches!(self, Self::DesktopGnome | Self::AllFeatures)
    }

    fn needs_all_features(self) -> bool {
        matches!(self, Self::AllFeatures)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinuxProbeStatus {
    Pass,
    Fail,
    Skip,
}

impl LinuxProbeStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Skip => "skip",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxProbe {
    pub id: &'static str,
    pub label: &'static str,
    pub status: LinuxProbeStatus,
    pub detail: String,
    pub guidance: Option<String>,
}

impl LinuxProbe {
    fn pass(id: &'static str, label: &'static str, detail: impl Into<String>) -> Self {
        Self {
            id,
            label,
            status: LinuxProbeStatus::Pass,
            detail: detail.into(),
            guidance: None,
        }
    }

    fn fail(
        id: &'static str,
        label: &'static str,
        detail: impl Into<String>,
        guidance: impl Into<String>,
    ) -> Self {
        Self {
            id,
            label,
            status: LinuxProbeStatus::Fail,
            detail: detail.into(),
            guidance: Some(guidance.into()),
        }
    }

    fn skip(id: &'static str, label: &'static str, detail: impl Into<String>) -> Self {
        Self {
            id,
            label,
            status: LinuxProbeStatus::Skip,
            detail: detail.into(),
            guidance: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxReadinessReport {
    pub profile: LinuxReadinessProfile,
    pub probes: Vec<LinuxProbe>,
}

impl LinuxReadinessReport {
    pub fn passed(&self) -> usize {
        self.probes
            .iter()
            .filter(|probe| probe.status == LinuxProbeStatus::Pass)
            .count()
    }

    pub fn failed(&self) -> usize {
        self.probes
            .iter()
            .filter(|probe| probe.status == LinuxProbeStatus::Fail)
            .count()
    }

    pub fn skipped(&self) -> usize {
        self.probes
            .iter()
            .filter(|probe| probe.status == LinuxProbeStatus::Skip)
            .count()
    }

    pub fn is_ready(&self) -> bool {
        self.failed() == 0
    }
}

pub async fn linux_readiness_report(profile: LinuxReadinessProfile) -> LinuxReadinessReport {
    if !cfg!(target_os = "linux") {
        return LinuxReadinessReport {
            profile,
            probes: vec![LinuxProbe::skip(
                "linux_host",
                "Linux host",
                "Linux readiness probes are only applicable on Linux.",
            )],
        };
    }

    let mut probes = Vec::new();
    probes.extend(probe_pi_os_lite_host(profile));
    probes.push(probe_secrets().await);
    probes.push(probe_systemd_user(profile));
    if profile == LinuxReadinessProfile::Remote {
        probes.push(probe_remote_gateway());
        probes.push(probe_remote_cli());
        probes.push(probe_remote_oauth_callback());
        probes.push(probe_remote_gateway_health().await);
    }
    if profile == LinuxReadinessProfile::PiOsLite64 {
        probes.push(probe_pi_gateway());
        probes.push(probe_pi_database());
    }

    let docker = crate::sandbox::check_docker().await;
    probes.push(probe_docker(profile, docker.status));
    if profile == LinuxReadinessProfile::PiOsLite64 {
        probes.push(probe_pi_docker_compose(docker.status));
        probes.push(probe_pi_tailscale());
    }
    probes.push(probe_bubblewrap(profile));
    probes.push(probe_browser(profile, docker.status));
    probes.push(probe_screen_capture(profile));
    probes.push(probe_camera(profile));
    probes.push(probe_microphone(profile));
    probes.push(probe_geoclue(profile));
    probes.extend(probe_desktop_stack(profile));
    probes.push(probe_ocr(profile));
    probes.extend(probe_all_feature_build_prereqs(profile));

    LinuxReadinessReport { profile, probes }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PiOsLiteHostFacts {
    arch: String,
    os_release: String,
    rpi_issue: Option<String>,
}

impl PiOsLiteHostFacts {
    fn current() -> Self {
        Self {
            arch: command_output_trimmed("uname", &["-m"]).unwrap_or_default(),
            os_release: std::fs::read_to_string("/etc/os-release").unwrap_or_default(),
            rpi_issue: std::fs::read_to_string("/etc/rpi-issue").ok(),
        }
    }
}

fn probe_pi_os_lite_host(profile: LinuxReadinessProfile) -> Vec<LinuxProbe> {
    if profile != LinuxReadinessProfile::PiOsLite64 {
        return Vec::new();
    }

    let facts = PiOsLiteHostFacts::current();
    pi_os_lite_host_probes(&facts)
}

fn pi_os_lite_host_probes(facts: &PiOsLiteHostFacts) -> Vec<LinuxProbe> {
    let mut probes = Vec::new();

    if is_arm64_arch(&facts.arch) {
        probes.push(LinuxProbe::pass(
            "pi_arch",
            "ARM64 architecture",
            format!("Host architecture is {}.", facts.arch),
        ));
    } else {
        probes.push(LinuxProbe::fail(
            "pi_arch",
            "ARM64 architecture",
            format!(
                "Host architecture is '{}', not aarch64/arm64.",
                empty_as_unknown(&facts.arch)
            ),
            "Use Raspberry Pi OS Lite 64-bit on ARM64 hardware, or use the generic Linux server profile.",
        ));
    }

    let os = parse_os_release(&facts.os_release);
    if is_pi_os_lite_compatible(&os, facts.rpi_issue.as_deref()) {
        probes.push(LinuxProbe::pass(
            "pi_os_lite",
            "Raspberry Pi OS Lite",
            pi_os_detail(&os, facts.rpi_issue.as_deref()),
        ));
    } else {
        probes.push(LinuxProbe::fail(
            "pi_os_lite",
            "Raspberry Pi OS Lite",
            pi_os_detail(&os, facts.rpi_issue.as_deref()),
            "Use Raspberry Pi OS Lite 64-bit Bookworm, or choose `--profile server` for generic Debian/Ubuntu hosts.",
        ));
    }

    if command_success("systemctl", &["--version"]) {
        probes.push(LinuxProbe::pass(
            "pi_systemd",
            "systemd",
            "systemctl is available for the native system service.",
        ));
    } else {
        probes.push(LinuxProbe::fail(
            "pi_systemd",
            "systemd",
            "systemctl was not found.",
            "Use the standard Raspberry Pi OS Lite image with systemd enabled.",
        ));
    }

    probes
}

async fn probe_secrets() -> LinuxProbe {
    let probe = crate::platform::secure_store::probe_availability().await;
    if probe.available {
        LinuxProbe::pass("secrets", "Secrets", probe.detail)
    } else {
        LinuxProbe::fail("secrets", "Secrets", probe.detail, probe.guidance)
    }
}

fn probe_systemd_user(profile: LinuxReadinessProfile) -> LinuxProbe {
    let required = profile.needs_desktop()
        || profile == LinuxReadinessProfile::Remote
        || crate::platform::env_flag_enabled("SERVICE_ENABLED");
    if command_success("systemctl", &["--user", "show-environment"]) {
        return LinuxProbe::pass(
            "systemd_user",
            "systemd user",
            "systemd --user is reachable.",
        );
    }
    if required {
        LinuxProbe::fail(
            "systemd_user",
            "systemd user",
            "systemd --user is not reachable for this login session.",
            "Run from a normal user session with pam_systemd, then retry `systemctl --user status`.",
        )
    } else {
        LinuxProbe::skip(
            "systemd_user",
            "systemd user",
            "Not required unless installing a user service or running desktop autonomy.",
        )
    }
}

fn probe_pi_gateway() -> LinuxProbe {
    if std::env::var("GATEWAY_ENABLED")
        .map(|value| value.trim().eq_ignore_ascii_case("false") || value.trim() == "0")
        .unwrap_or(false)
    {
        return LinuxProbe::fail(
            "pi_gateway",
            "Pi gateway",
            "GATEWAY_ENABLED disables the remote gateway.",
            "Set GATEWAY_ENABLED=true and GATEWAY_AUTH_TOKEN for Pi OS Lite remote access.",
        );
    }

    if std::env::var_os("GATEWAY_AUTH_TOKEN").is_some() {
        LinuxProbe::pass(
            "pi_gateway",
            "Pi gateway",
            format!(
                "Gateway is enabled for port {}.",
                std::env::var("GATEWAY_PORT").unwrap_or_else(|_| "3000".to_string())
            ),
        )
    } else {
        LinuxProbe::fail(
            "pi_gateway",
            "Pi gateway",
            "GATEWAY_AUTH_TOKEN is not configured.",
            "Set a long random GATEWAY_AUTH_TOKEN before exposing the Pi gateway.",
        )
    }
}

fn probe_remote_gateway() -> LinuxProbe {
    let access = crate::platform::gateway_access::GatewayAccessInfo::from_env_and_settings(None);
    if !access.enabled {
        return LinuxProbe::fail(
            "remote_gateway",
            "Remote gateway",
            "GATEWAY_ENABLED disables the WebUI gateway.",
            "Run `thinclaw onboard --profile remote` or set GATEWAY_ENABLED=true.",
        );
    }
    if access.auth_token.is_none() {
        return LinuxProbe::fail(
            "remote_gateway",
            "Remote gateway",
            "GATEWAY_AUTH_TOKEN is not configured.",
            "Run `thinclaw onboard --profile remote`, or set a long random GATEWAY_AUTH_TOKEN before remote access.",
        );
    }
    LinuxProbe::pass(
        "remote_gateway",
        "Remote gateway",
        format!("Gateway is enabled on {}.", access.bind_display()),
    )
}

fn probe_remote_cli() -> LinuxProbe {
    let access = crate::platform::gateway_access::GatewayAccessInfo::from_env_and_settings(None);
    if access.cli_enabled {
        LinuxProbe::fail(
            "remote_cli",
            "Service-safe CLI",
            "CLI_ENABLED is true; a service/headless process can receive stdin EOF and shut down the REPL.",
            "Set CLI_ENABLED=false for remote/service hosts.",
        )
    } else {
        LinuxProbe::pass(
            "remote_cli",
            "Service-safe CLI",
            "CLI_ENABLED=false is configured for service/headless runtime.",
        )
    }
}

fn probe_remote_oauth_callback() -> LinuxProbe {
    let tunnel = crate::cli::oauth_defaults::ssh_callback_tunnel_command();
    LinuxProbe::pass(
        "remote_oauth_callback",
        "OAuth callback tunnel",
        format!("Use `{tunnel}` before starting OAuth from an SSH/headless host."),
    )
}

async fn probe_remote_gateway_health() -> LinuxProbe {
    let access = crate::platform::gateway_access::GatewayAccessInfo::from_env_and_settings(None);
    if !access.enabled {
        return LinuxProbe::skip(
            "remote_gateway_health",
            "Gateway health",
            "Gateway is disabled.",
        );
    }

    let Ok(client) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
    else {
        return LinuxProbe::fail(
            "remote_gateway_health",
            "Gateway health",
            "Could not create the HTTP client for the health check.",
            "Retry `thinclaw doctor --profile remote` after checking local networking.",
        );
    };

    match client.get(access.health_url()).send().await {
        Ok(response) if response.status().is_success() => LinuxProbe::pass(
            "remote_gateway_health",
            "Gateway health",
            "The WebUI health endpoint is reachable.",
        ),
        _ => LinuxProbe::fail(
            "remote_gateway_health",
            "Gateway health",
            format!("Could not reach {}.", access.health_url()),
            "Start the runtime with `thinclaw run --no-onboard` or `thinclaw service start`, then retry.",
        ),
    }
}

fn probe_pi_database() -> LinuxProbe {
    let backend = std::env::var("DATABASE_BACKEND").unwrap_or_else(|_| "libsql".to_string());
    if matches!(
        backend.trim().to_ascii_lowercase().as_str(),
        "libsql" | "sqlite" | "turso"
    ) {
        LinuxProbe::pass(
            "pi_database",
            "Pi database",
            "libSQL is selected for the Pi OS Lite local database.",
        )
    } else {
        LinuxProbe::skip(
            "pi_database",
            "Pi database",
            format!(
                "DATABASE_BACKEND={} is configured; first-class Pi OS Lite defaults use libSQL.",
                backend
            ),
        )
    }
}

fn probe_docker(
    profile: LinuxReadinessProfile,
    status: crate::sandbox::DockerStatus,
) -> LinuxProbe {
    let browser_mode = crate::platform::BrowserDockerMode::from_env_lossy();
    let required = profile.needs_all_features()
        || crate::platform::env_flag_enabled("SANDBOX_ENABLED")
        || browser_mode.forces_docker();
    match status {
        crate::sandbox::DockerStatus::Available => LinuxProbe::pass(
            "docker",
            "Docker daemon",
            "Docker is installed and running.",
        ),
        crate::sandbox::DockerStatus::NotInstalled if required => LinuxProbe::fail(
            "docker",
            "Docker daemon",
            "Docker is required for the selected profile but is not installed.",
            "Install Docker Engine and start the daemon: sudo apt install docker.io && sudo systemctl enable --now docker",
        ),
        crate::sandbox::DockerStatus::NotRunning if required => LinuxProbe::fail(
            "docker",
            "Docker daemon",
            "Docker is installed but the daemon is not reachable.",
            "Start Docker with `sudo systemctl start docker`, or configure rootless Docker/DOCKER_HOST.",
        ),
        _ => LinuxProbe::skip(
            "docker",
            "Docker daemon",
            "Docker-backed sandbox/browser fallback is not enabled for this profile.",
        ),
    }
}

fn probe_pi_docker_compose(status: crate::sandbox::DockerStatus) -> LinuxProbe {
    if status != crate::sandbox::DockerStatus::Available {
        return LinuxProbe::skip(
            "pi_docker_compose",
            "Pi Docker Compose",
            "Docker is optional on Pi OS Lite; install it for sandbox jobs, Docker deployment, or Docker Chromium fallback.",
        );
    }

    if command_success("docker", &["compose", "version"]) {
        LinuxProbe::pass(
            "pi_docker_compose",
            "Pi Docker Compose",
            "Docker Compose v2 is available.",
        )
    } else {
        LinuxProbe::fail(
            "pi_docker_compose",
            "Pi Docker Compose",
            "Docker is available but `docker compose` was not found.",
            "Install the docker-compose-plugin package for the Docker deployment path.",
        )
    }
}

fn probe_pi_tailscale() -> LinuxProbe {
    if crate::platform::executable_available("tailscale") {
        LinuxProbe::pass(
            "pi_tailscale",
            "Pi Tailscale",
            "tailscale is installed for private gateway access.",
        )
    } else {
        LinuxProbe::skip(
            "pi_tailscale",
            "Pi Tailscale",
            "Tailscale is optional but recommended for private Scrappy access to a Pi.",
        )
    }
}

fn probe_bubblewrap(profile: LinuxReadinessProfile) -> LinuxProbe {
    let required =
        profile.needs_all_features() || crate::platform::env_flag_enabled("THINCLAW_REQUIRE_BWRAP");
    if crate::platform::executable_available("bwrap") {
        return LinuxProbe::pass("bubblewrap", "bubblewrap", "bwrap is available.");
    }
    if required {
        LinuxProbe::fail(
            "bubblewrap",
            "bubblewrap",
            "bwrap is required for Linux host-local no-network execution but was not found.",
            "Install bubblewrap: sudo apt install bubblewrap",
        )
    } else {
        LinuxProbe::skip(
            "bubblewrap",
            "bubblewrap",
            "Only required for Linux host-local no-network execution.",
        )
    }
}

fn probe_browser(
    profile: LinuxReadinessProfile,
    docker_status: crate::sandbox::DockerStatus,
) -> LinuxProbe {
    let docker_mode = match crate::platform::BrowserDockerMode::from_env() {
        Ok(mode) => mode,
        Err(error) => {
            return LinuxProbe::fail(
                "browser",
                "Browser",
                error,
                "Set BROWSER_DOCKER to auto, always, or never.",
            );
        }
    };
    let required = profile.needs_all_features()
        || std::env::var_os("BROWSER_EXECUTABLE").is_some()
        || docker_mode.forces_docker();

    if !docker_mode.forces_docker()
        && let Some(path) = crate::platform::find_browser_executable()
    {
        return LinuxProbe::pass(
            "browser",
            "Browser",
            format!("Local browser detected at {}.", path.display()),
        );
    }

    if docker_mode.allows_docker() && docker_status == crate::sandbox::DockerStatus::Available {
        match docker_chromium_image_readiness_detail() {
            Ok(detail) => {
                return LinuxProbe::pass(
                    "browser",
                    "Browser",
                    format!("No local browser required. {detail}"),
                );
            }
            Err(detail) if required => {
                return LinuxProbe::fail(
                    "browser",
                    "Browser",
                    detail,
                    "Install chromium/google-chrome/brave/edge locally, or set CHROMIUM_IMAGE to a reachable CDP-capable multi-arch Chromium image.",
                );
            }
            Err(detail) => {
                return LinuxProbe::skip(
                    "browser",
                    "Browser",
                    format!(
                        "Browser automation is optional for this profile. Docker fallback is not ready: {detail}"
                    ),
                );
            }
        }
    }

    if required {
        LinuxProbe::fail(
            "browser",
            "Browser",
            "No Chrome, Chromium, Brave, Edge, or usable Docker Chromium fallback was found.",
            "Install chromium, google-chrome-stable, brave-browser, or microsoft-edge-stable; or set BROWSER_DOCKER=auto with Docker running.",
        )
    } else {
        LinuxProbe::skip(
            "browser",
            "Browser",
            "Browser automation is optional for this profile.",
        )
    }
}

#[cfg(feature = "browser")]
fn docker_chromium_image_readiness_detail() -> Result<String, String> {
    crate::sandbox::docker_chromium::DockerChromiumConfig::from_env()
        .image_readiness_detail()
        .map_err(|error| error.to_string())
}

#[cfg(not(feature = "browser"))]
fn docker_chromium_image_readiness_detail() -> Result<String, String> {
    Err("current build does not include the browser feature".to_string())
}

fn probe_screen_capture(profile: LinuxReadinessProfile) -> LinuxProbe {
    let required =
        profile.needs_desktop() || crate::platform::env_flag_enabled("SCREEN_CAPTURE_ENABLED");
    let capability = linux_screen_capture_capability();
    if capability.core_modes_available() {
        return LinuxProbe::pass("screen_capture", "Screen capture", capability.detail());
    }
    if required {
        LinuxProbe::fail(
            "screen_capture",
            "Screen capture",
            capability.missing_detail(),
            "Install a screenshot tool: sudo apt install gnome-screenshot scrot imagemagick",
        )
    } else if capability.any_mode_available() {
        LinuxProbe::pass("screen_capture", "Screen capture", capability.detail())
    } else {
        LinuxProbe::skip(
            "screen_capture",
            "Screen capture",
            "Only required when SCREEN_CAPTURE_ENABLED=true or desktop autonomy is enabled.",
        )
    }
}

fn probe_camera(profile: LinuxReadinessProfile) -> LinuxProbe {
    let required =
        profile.needs_all_features() || crate::platform::env_flag_enabled("CAMERA_CAPTURE_ENABLED");
    let capability = linux_camera_capability();
    if capability.ready() {
        return LinuxProbe::pass("camera", "Camera capture", capability.detail());
    }
    if required {
        LinuxProbe::fail(
            "camera",
            "Camera capture",
            capability.missing_detail(),
            "Install camera tooling: sudo apt install ffmpeg fswebcam",
        )
    } else {
        LinuxProbe::skip(
            "camera",
            "Camera capture",
            "Only required when CAMERA_CAPTURE_ENABLED=true.",
        )
    }
}

fn probe_microphone(profile: LinuxReadinessProfile) -> LinuxProbe {
    let required =
        profile.needs_all_features() || crate::platform::env_flag_enabled("TALK_MODE_ENABLED");
    let capability = match linux_microphone_capability() {
        Ok(capability) => capability,
        Err(error) => {
            return LinuxProbe::fail(
                "microphone",
                "Microphone capture",
                error,
                "Set THINCLAW_MICROPHONE_BACKEND=auto, pipewire, pulse, or alsa.",
            );
        }
    };
    if capability.ready() {
        return LinuxProbe::pass("microphone", "Microphone capture", capability.detail());
    }
    if required {
        LinuxProbe::fail(
            "microphone",
            "Microphone capture",
            capability.missing_detail(),
            "Install ffmpeg and configure an input source. For Ubuntu/Debian: sudo apt install ffmpeg pipewire-pulse alsa-utils",
        )
    } else {
        LinuxProbe::skip(
            "microphone",
            "Microphone capture",
            "Only required when TALK_MODE_ENABLED=true.",
        )
    }
}

fn probe_geoclue(profile: LinuxReadinessProfile) -> LinuxProbe {
    let required =
        profile.needs_all_features() || crate::platform::env_flag_enabled("LOCATION_ENABLED");
    geoclue_probe_from_facts(required, current_geoclue_facts())
}

fn probe_desktop_stack(profile: LinuxReadinessProfile) -> Vec<LinuxProbe> {
    if profile == LinuxReadinessProfile::PiOsLite64 {
        if crate::platform::env_flag_enabled("DESKTOP_AUTONOMY_ENABLED") {
            return vec![LinuxProbe::fail(
                "pi_desktop_autonomy",
                "Pi desktop autonomy",
                "Desktop autonomy is not supported on Raspberry Pi OS Lite.",
                "Disable DESKTOP_AUTONOMY_ENABLED on Pi OS Lite, or use `--profile desktop-gnome` on a supported GNOME/X11 Linux desktop host.",
            )];
        }

        return vec![LinuxProbe::skip(
            "pi_desktop_autonomy",
            "Pi desktop autonomy",
            "Pi OS Lite support is headless; desktop autonomy is intentionally out of scope.",
        )];
    }

    let required =
        profile.needs_desktop() || crate::platform::env_flag_enabled("DESKTOP_AUTONOMY_ENABLED");
    let mut probes = Vec::new();
    if !required {
        probes.push(LinuxProbe::skip(
            "gnome_x11",
            "GNOME/X11 desktop",
            "Only required for the desktop-gnome profile.",
        ));
        probes.push(LinuxProbe::skip(
            "desktop_apps",
            "Desktop apps",
            "Only required for the desktop-gnome profile.",
        ));
        probes.push(LinuxProbe::skip(
            "at_spi",
            "AT-SPI accessibility",
            "Only required for the desktop-gnome profile.",
        ));
        return probes;
    }

    let session = current_linux_desktop_session_facts();
    if linux_gnome_x11_ready(&session) {
        probes.push(LinuxProbe::pass(
            "gnome_x11",
            "GNOME/X11 desktop",
            linux_gnome_x11_detail(&session),
        ));
    } else {
        probes.push(LinuxProbe::fail(
            "gnome_x11",
            "GNOME/X11 desktop",
            linux_gnome_x11_detail(&session),
            "Log out and choose 'GNOME on Xorg' from the session gear; KDE and Wayland are intentionally unsupported for now.",
        ));
    }

    let required_commands = [
        "python3",
        "libreoffice",
        "evolution",
        "gdbus",
        "xdotool",
        "wmctrl",
    ];
    let missing = required_commands
        .iter()
        .filter(|command| !crate::platform::executable_available(command))
        .copied()
        .collect::<Vec<_>>();
    if missing.is_empty()
        && python_module_available("pyatspi")
        && python_module_available("gi")
        && python_module_available("uno")
    {
        probes.push(LinuxProbe::pass(
            "desktop_apps",
            "Desktop apps",
            "Linux desktop app commands and Python modules are available.",
        ));
    } else {
        let mut detail = if missing.is_empty() {
            "Required commands are present".to_string()
        } else {
            format!("Missing commands: {}.", missing.join(", "))
        };
        let missing_modules = ["pyatspi", "gi", "uno"]
            .iter()
            .filter(|module| !python_module_available(module))
            .copied()
            .collect::<Vec<_>>();
        if !missing_modules.is_empty() {
            detail.push_str(&format!(
                " Missing Python modules: {}.",
                missing_modules.join(", ")
            ));
        }
        probes.push(LinuxProbe::fail(
            "desktop_apps",
            "Desktop apps",
            detail,
            ubuntu_debian_desktop_install_block(),
        ));
    }

    let at_spi_ok = std::env::var_os("AT_SPI_BUS_ADDRESS").is_some()
        || std::env::var_os("GTK_MODULES")
            .is_some_and(|value| value.to_string_lossy().contains("gail"));
    if at_spi_ok {
        probes.push(LinuxProbe::pass(
            "at_spi",
            "AT-SPI accessibility",
            "AT-SPI accessibility bus appears active.",
        ));
    } else {
        probes.push(LinuxProbe::fail(
            "at_spi",
            "AT-SPI accessibility",
            "AT-SPI accessibility bus is not active.",
            "Enable accessibility in GNOME and install at-spi2-core, then start a fresh GNOME/X11 session.",
        ));
    }
    probes
}

fn probe_ocr(profile: LinuxReadinessProfile) -> LinuxProbe {
    let required =
        profile.needs_desktop() || crate::platform::env_flag_enabled("DESKTOP_AUTONOMY_ENABLED");
    if crate::platform::executable_available("tesseract") {
        return LinuxProbe::pass(
            "ocr",
            "OCR",
            "tesseract is available for desktop screenshot text recognition.",
        );
    }
    if required {
        LinuxProbe::fail(
            "ocr",
            "OCR",
            "tesseract was not found.",
            "Install OCR support: sudo apt install tesseract-ocr",
        )
    } else {
        LinuxProbe::skip("ocr", "OCR", "Only required for desktop autonomy.")
    }
}

fn probe_all_feature_build_prereqs(profile: LinuxReadinessProfile) -> Vec<LinuxProbe> {
    if !profile.needs_all_features() {
        return vec![LinuxProbe::skip(
            "all_features_build",
            "all-features build prereqs",
            "Only checked by the all-features profile.",
        )];
    }

    vec![
        probe_command_required(
            "alsa_dev",
            "ALSA development headers",
            command_success("pkg-config", &["--exists", "alsa"]),
            "pkg-config can resolve alsa.",
            "libasound2-dev/pkg-config was not found.",
            "Install build headers: sudo apt install pkg-config libasound2-dev",
        ),
        probe_command_required(
            "wasm32_wasip2",
            "wasm32-wasip2 target",
            rustup_has_wasm32_wasip2(),
            "rustup has the wasm32-wasip2 target installed.",
            "rustup target wasm32-wasip2 is missing.",
            "Install it with: rustup target add wasm32-wasip2",
        ),
        probe_command_required(
            "wasm_tools",
            "wasm-tools",
            command_success("wasm-tools", &["--version"]),
            "wasm-tools is available.",
            "wasm-tools was not found.",
            "Install it with: cargo install wasm-tools --locked",
        ),
        probe_command_required(
            "aws_credentials",
            "AWS Bedrock credentials",
            aws_credentials_present(),
            "AWS credential environment/profile is present.",
            "No AWS credentials were detected for the bedrock feature.",
            "Set AWS_PROFILE or AWS_ACCESS_KEY_ID/AWS_SECRET_ACCESS_KEY before using --features bedrock or --all-features.",
        ),
    ]
}

fn probe_command_required(
    id: &'static str,
    label: &'static str,
    ok: bool,
    pass_detail: &'static str,
    fail_detail: &'static str,
    guidance: &'static str,
) -> LinuxProbe {
    if ok {
        LinuxProbe::pass(id, label, pass_detail)
    } else {
        LinuxProbe::fail(id, label, fail_detail, guidance)
    }
}

fn command_success(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn command_output_trimmed(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct LinuxScreenCaptureCapability {
    fullscreen: Vec<&'static str>,
    window: Vec<&'static str>,
    interactive: Vec<&'static str>,
    delay: Vec<&'static str>,
}

impl LinuxScreenCaptureCapability {
    fn core_modes_available(&self) -> bool {
        !self.fullscreen.is_empty()
            && !self.window.is_empty()
            && !self.interactive.is_empty()
            && !self.delay.is_empty()
    }

    fn any_mode_available(&self) -> bool {
        !self.fullscreen.is_empty()
            || !self.window.is_empty()
            || !self.interactive.is_empty()
            || !self.delay.is_empty()
    }

    fn detail(&self) -> String {
        format!(
            "Supported modes: fullscreen via {}; window via {}; interactive via {}; delay via {}.",
            tools_or_unavailable(&self.fullscreen),
            tools_or_unavailable(&self.window),
            tools_or_unavailable(&self.interactive),
            tools_or_unavailable(&self.delay),
        )
    }

    fn missing_detail(&self) -> String {
        if !self.any_mode_available() {
            return "No supported Linux screenshot tool was found.".to_string();
        }

        let mut missing = Vec::new();
        if self.fullscreen.is_empty() {
            missing.push("fullscreen");
        }
        if self.window.is_empty() {
            missing.push("window");
        }
        if self.interactive.is_empty() {
            missing.push("interactive");
        }
        if self.delay.is_empty() {
            missing.push("delay");
        }
        format!(
            "Linux screenshot tooling is incomplete: missing {} support. {}",
            missing.join(", "),
            self.detail()
        )
    }
}

fn linux_screen_capture_capability() -> LinuxScreenCaptureCapability {
    linux_screen_capture_capability_with(crate::platform::executable_available)
}

fn linux_screen_capture_capability_with(
    mut available: impl FnMut(&str) -> bool,
) -> LinuxScreenCaptureCapability {
    let mut capability = LinuxScreenCaptureCapability::default();
    if available("gnome-screenshot") {
        capability.fullscreen.push("gnome-screenshot");
        capability.window.push("gnome-screenshot");
        capability.interactive.push("gnome-screenshot");
        capability.delay.push("gnome-screenshot");
    }
    if available("scrot") {
        capability.fullscreen.push("scrot");
        capability.window.push("scrot");
        capability.interactive.push("scrot");
        capability.delay.push("scrot");
    }
    if available("import") {
        capability.fullscreen.push("ImageMagick import");
        capability.window.push("ImageMagick import");
        capability.interactive.push("ImageMagick import");
    }
    capability
}

fn tools_or_unavailable(tools: &[&'static str]) -> String {
    if tools.is_empty() {
        "unavailable".to_string()
    } else {
        tools.join(", ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LinuxCameraCapability {
    fswebcam: bool,
    ffmpeg: bool,
    configured_device: Option<String>,
    detected_devices: Vec<String>,
}

impl LinuxCameraCapability {
    fn ready(&self) -> bool {
        self.tool_available() && self.device_available()
    }

    fn tool_available(&self) -> bool {
        self.fswebcam || self.ffmpeg
    }

    fn device_available(&self) -> bool {
        !self.detected_devices.is_empty()
    }

    fn detail(&self) -> String {
        format!(
            "{} available and {}.",
            self.tool_detail(),
            self.device_detail()
        )
    }

    fn missing_detail(&self) -> String {
        let mut missing = Vec::new();
        if !self.tool_available() {
            missing.push("fswebcam or ffmpeg");
        }
        if !self.device_available() {
            missing.push(match self.configured_device.as_deref() {
                Some(_) => "configured V4L2 device",
                None => "/dev/video*",
            });
        }
        format!("Missing camera prerequisites: {}.", missing.join(", "))
    }

    fn tool_detail(&self) -> &'static str {
        if self.fswebcam && self.ffmpeg {
            "fswebcam and ffmpeg are"
        } else if self.fswebcam {
            "fswebcam is"
        } else {
            "ffmpeg is"
        }
    }

    fn device_detail(&self) -> String {
        if let Some(device) = self.configured_device.as_deref() {
            format!("configured V4L2 device {device} exists")
        } else {
            format!(
                "V4L2 devices detected: {}",
                self.detected_devices.join(", ")
            )
        }
    }
}

fn linux_camera_capability() -> LinuxCameraCapability {
    linux_camera_capability_from(
        crate::platform::executable_available("fswebcam"),
        crate::platform::executable_available("ffmpeg"),
        configured_env_value("THINCLAW_CAMERA_DEVICE"),
        linux_video_devices_from_dev(),
    )
}

fn linux_camera_capability_from(
    fswebcam: bool,
    ffmpeg: bool,
    configured_device: Option<String>,
    detected_devices: Vec<String>,
) -> LinuxCameraCapability {
    let detected_devices = if let Some(device) = configured_device.as_deref() {
        if Path::new(device).exists() {
            vec![device.to_string()]
        } else {
            Vec::new()
        }
    } else {
        detected_devices
    };

    LinuxCameraCapability {
        fswebcam,
        ffmpeg,
        configured_device,
        detected_devices,
    }
}

fn linux_video_devices_from_dev() -> Vec<String> {
    let mut devices = std::fs::read_dir("/dev")
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("video") {
                Some(entry.path().display().to_string())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    devices.sort();
    devices
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinuxMicrophoneBackendProbe {
    Auto,
    Pipewire,
    Pulse,
    Alsa,
}

impl LinuxMicrophoneBackendProbe {
    fn from_env() -> Result<Self, String> {
        match std::env::var("THINCLAW_MICROPHONE_BACKEND")
            .ok()
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            None => Ok(Self::Auto),
            Some(value) if value.eq_ignore_ascii_case("auto") => Ok(Self::Auto),
            Some(value) if value.eq_ignore_ascii_case("pipewire") => Ok(Self::Pipewire),
            Some(value)
                if value.eq_ignore_ascii_case("pulse")
                    || value.eq_ignore_ascii_case("pulseaudio") =>
            {
                Ok(Self::Pulse)
            }
            Some(value) if value.eq_ignore_ascii_case("alsa") => Ok(Self::Alsa),
            Some(value) => Err(format!(
                "invalid THINCLAW_MICROPHONE_BACKEND value '{value}' (expected auto, pipewire, pulse, or alsa)"
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Pipewire => "pipewire",
            Self::Pulse => "pulse",
            Self::Alsa => "alsa",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct LinuxAudioSourceHints {
    configured_device: Option<String>,
    pipewire: bool,
    pulse: bool,
    alsa: bool,
}

impl LinuxAudioSourceHints {
    fn any(&self) -> bool {
        self.configured_device.is_some() || self.pipewire || self.pulse || self.alsa
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LinuxMicrophoneCapability {
    backend: LinuxMicrophoneBackendProbe,
    ffmpeg: bool,
    sources: LinuxAudioSourceHints,
}

impl LinuxMicrophoneCapability {
    fn ready(&self) -> bool {
        self.ffmpeg && self.backend_available()
    }

    fn detail(&self) -> String {
        format!(
            "ffmpeg is available and THINCLAW_MICROPHONE_BACKEND={} can use {}.",
            self.backend.as_str(),
            self.ready_backends().join(", ")
        )
    }

    fn missing_detail(&self) -> String {
        let mut missing = Vec::new();
        if !self.ffmpeg {
            missing.push("ffmpeg");
        }
        if !self.backend_available() {
            missing.push("a configured audio backend");
        }
        format!("Missing microphone prerequisites: {}.", missing.join(", "))
    }

    fn backend_available(&self) -> bool {
        if self.sources.configured_device.is_some() {
            return true;
        }

        match self.backend {
            LinuxMicrophoneBackendProbe::Auto => self.sources.any(),
            LinuxMicrophoneBackendProbe::Pipewire => self.sources.pipewire,
            LinuxMicrophoneBackendProbe::Pulse => self.sources.pulse || self.sources.pipewire,
            LinuxMicrophoneBackendProbe::Alsa => self.sources.alsa,
        }
    }

    fn ready_backends(&self) -> Vec<&'static str> {
        let mut backends = Vec::new();
        if self.sources.configured_device.is_some() {
            backends.push("explicit source override");
        }
        if self.sources.pipewire {
            backends.push("PipeWire/Pulse server");
        }
        if self.sources.pulse {
            backends.push("PulseAudio");
        }
        if self.sources.alsa {
            backends.push("ALSA");
        }
        backends
    }
}

fn linux_microphone_capability() -> Result<LinuxMicrophoneCapability, String> {
    Ok(linux_microphone_capability_from(
        LinuxMicrophoneBackendProbe::from_env()?,
        crate::platform::executable_available("ffmpeg"),
        current_linux_audio_source_hints(),
    ))
}

fn linux_microphone_capability_from(
    backend: LinuxMicrophoneBackendProbe,
    ffmpeg: bool,
    sources: LinuxAudioSourceHints,
) -> LinuxMicrophoneCapability {
    LinuxMicrophoneCapability {
        backend,
        ffmpeg,
        sources,
    }
}

fn current_linux_audio_source_hints() -> LinuxAudioSourceHints {
    let xdg_runtime_dir = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from);
    let pipewire_runtime_dir =
        std::env::var_os("PIPEWIRE_RUNTIME_DIR").map(std::path::PathBuf::from);
    let pipewire_socket = pipewire_runtime_dir
        .as_ref()
        .map(|dir| dir.join("pipewire-0").exists())
        .unwrap_or(false)
        || xdg_runtime_dir
            .as_ref()
            .map(|dir| dir.join("pipewire-0").exists())
            .unwrap_or(false);
    let pulse_socket = xdg_runtime_dir
        .as_ref()
        .map(|dir| dir.join("pulse/native").exists())
        .unwrap_or(false);

    LinuxAudioSourceHints {
        configured_device: configured_env_value("THINCLAW_MICROPHONE_DEVICE"),
        pipewire: pipewire_socket,
        pulse: std::env::var_os("PULSE_SERVER").is_some()
            || pulse_socket
            || crate::platform::executable_available("pactl"),
        alsa: Path::new("/proc/asound/cards").exists()
            || Path::new("/dev/snd").exists()
            || crate::platform::executable_available("arecord"),
    }
}

fn configured_env_value(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GeoclueFacts {
    has_gdbus: bool,
    system_bus_reachable: bool,
    manager_reachable: bool,
    ip_fallback_allowed: bool,
}

fn current_geoclue_facts() -> GeoclueFacts {
    let has_gdbus = crate::platform::executable_available("gdbus");
    let system_bus_reachable = has_gdbus
        && command_success(
            "gdbus",
            &[
                "introspect",
                "--system",
                "--dest",
                "org.freedesktop.DBus",
                "--object-path",
                "/org/freedesktop/DBus",
            ],
        );
    let manager_reachable = system_bus_reachable
        && command_success(
            "gdbus",
            &[
                "introspect",
                "--system",
                "--dest",
                "org.freedesktop.GeoClue2",
                "--object-path",
                "/org/freedesktop/GeoClue2/Manager",
            ],
        );

    GeoclueFacts {
        has_gdbus,
        system_bus_reachable,
        manager_reachable,
        ip_fallback_allowed: crate::platform::env_flag_enabled("LOCATION_ALLOW_IP_FALLBACK"),
    }
}

fn geoclue_probe_from_facts(required: bool, facts: GeoclueFacts) -> LinuxProbe {
    if facts.manager_reachable {
        return LinuxProbe::pass(
            "geoclue",
            "GeoClue",
            "GeoClue manager is reachable on the system D-Bus.",
        );
    }

    let detail = geoclue_missing_detail(facts);
    if required {
        LinuxProbe::fail(
            "geoclue",
            "GeoClue",
            detail,
            "Install geoclue-2.0 and libglib2.0-bin, then run from a logged-in user session. IP fallback is approximate and requires LOCATION_ALLOW_IP_FALLBACK=true.",
        )
    } else {
        LinuxProbe::skip(
            "geoclue",
            "GeoClue",
            format!("{detail} Only required when LOCATION_ENABLED=true."),
        )
    }
}

fn geoclue_missing_detail(facts: GeoclueFacts) -> String {
    let mut detail = if !facts.has_gdbus {
        "gdbus was not found, so GeoClue cannot be probed.".to_string()
    } else if !facts.system_bus_reachable {
        "gdbus is installed but the system D-Bus is not reachable.".to_string()
    } else {
        "system D-Bus is reachable but org.freedesktop.GeoClue2.Manager is not available."
            .to_string()
    };

    if facts.ip_fallback_allowed {
        detail.push_str(
            " LOCATION_ALLOW_IP_FALLBACK is enabled, but IP lookup is approximate and is not native device location.",
        );
    }
    detail
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct LinuxDesktopSessionFacts {
    display: Option<String>,
    wayland_display: Option<String>,
    session_type: Option<String>,
    current_desktop: Option<String>,
}

fn current_linux_desktop_session_facts() -> LinuxDesktopSessionFacts {
    LinuxDesktopSessionFacts {
        display: configured_env_value("DISPLAY"),
        wayland_display: configured_env_value("WAYLAND_DISPLAY"),
        session_type: configured_env_value("XDG_SESSION_TYPE"),
        current_desktop: configured_env_value("XDG_CURRENT_DESKTOP"),
    }
}

fn linux_gnome_x11_ready(facts: &LinuxDesktopSessionFacts) -> bool {
    let session_is_x11 = facts
        .session_type
        .as_deref()
        .map(|value| value.eq_ignore_ascii_case("x11"))
        .unwrap_or(false);
    let desktop_is_gnome = facts
        .current_desktop
        .as_deref()
        .unwrap_or_default()
        .split(':')
        .any(|value| value.eq_ignore_ascii_case("gnome"));

    facts.display.is_some() && facts.wayland_display.is_none() && session_is_x11 && desktop_is_gnome
}

fn linux_gnome_x11_detail(facts: &LinuxDesktopSessionFacts) -> String {
    if linux_gnome_x11_ready(facts) {
        return "DISPLAY is set and XDG_CURRENT_DESKTOP reports GNOME on X11.".to_string();
    }

    format!(
        "Linux desktop autonomy requires GNOME on X11 with DISPLAY set; detected DISPLAY={}, WAYLAND_DISPLAY={}, XDG_SESSION_TYPE={}, XDG_CURRENT_DESKTOP={}.",
        facts.display.as_deref().unwrap_or("unset"),
        facts.wayland_display.as_deref().unwrap_or("unset"),
        facts.session_type.as_deref().unwrap_or("unset"),
        facts.current_desktop.as_deref().unwrap_or("unset"),
    )
}

fn is_arm64_arch(arch: &str) -> bool {
    matches!(
        arch.trim().to_ascii_lowercase().as_str(),
        "aarch64" | "arm64"
    )
}

fn parse_os_release(contents: &str) -> HashMap<String, String> {
    contents
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once('=')?;
            Some((key.to_string(), unquote_os_release_value(value)))
        })
        .collect()
}

fn unquote_os_release_value(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
    }
}

fn is_pi_os_lite_compatible(os: &HashMap<String, String>, rpi_issue: Option<&str>) -> bool {
    let codename = os
        .get("VERSION_CODENAME")
        .or_else(|| os.get("DEBIAN_CODENAME"))
        .map(|value| value.eq_ignore_ascii_case("bookworm"))
        .unwrap_or(false);
    let id_like = format!(
        "{} {} {}",
        os.get("ID").map(String::as_str).unwrap_or_default(),
        os.get("ID_LIKE").map(String::as_str).unwrap_or_default(),
        os.get("PRETTY_NAME")
            .map(String::as_str)
            .unwrap_or_default()
    )
    .to_ascii_lowercase();
    let is_debian_family = id_like.contains("debian");
    let is_raspberry_pi = rpi_issue
        .map(|issue| issue.to_ascii_lowercase().contains("raspberry pi"))
        .unwrap_or(false)
        || id_like.contains("raspberry pi os")
        || id_like.contains("raspbian");

    codename && is_debian_family && is_raspberry_pi
}

fn pi_os_detail(os: &HashMap<String, String>, rpi_issue: Option<&str>) -> String {
    let pretty = os
        .get("PRETTY_NAME")
        .map(String::as_str)
        .unwrap_or("unknown Linux");
    let codename = os
        .get("VERSION_CODENAME")
        .or_else(|| os.get("DEBIAN_CODENAME"))
        .map(String::as_str)
        .unwrap_or("unknown");
    let rpi = if rpi_issue.is_some() {
        "rpi-issue present"
    } else {
        "rpi-issue missing"
    };
    format!("{pretty}; codename={codename}; {rpi}.")
}

fn empty_as_unknown(value: &str) -> &str {
    if value.trim().is_empty() {
        "unknown"
    } else {
        value
    }
}

fn python_module_available(module: &str) -> bool {
    command_success("python3", &["-c", &format!("import {module}")])
}

fn rustup_has_wasm32_wasip2() -> bool {
    let Ok(output) = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        return false;
    };
    output.status.success()
        && String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line.trim() == "wasm32-wasip2")
}

fn aws_credentials_present() -> bool {
    std::env::var_os("AWS_PROFILE").is_some()
        || (std::env::var_os("AWS_ACCESS_KEY_ID").is_some()
            && std::env::var_os("AWS_SECRET_ACCESS_KEY").is_some())
}

fn ubuntu_debian_desktop_install_block() -> &'static str {
    "Install the GNOME/X11 desktop prerequisites: sudo apt install python3 python3-gi python3-pyatspi libreoffice libreoffice-script-provider-python evolution evolution-data-server-bin xdotool wmctrl tesseract-ocr gnome-screenshot scrot imagemagick at-spi2-core libglib2.0-bin geoclue-2.0 ffmpeg fswebcam"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_names_are_stable() {
        assert_eq!(LinuxReadinessProfile::Server.as_str(), "server");
        assert_eq!(LinuxReadinessProfile::Remote.as_str(), "remote");
        assert_eq!(
            LinuxReadinessProfile::DesktopGnome.as_str(),
            "desktop-gnome"
        );
        assert_eq!(LinuxReadinessProfile::PiOsLite64.as_str(), "pi-os-lite-64");
        assert_eq!(LinuxReadinessProfile::AllFeatures.as_str(), "all-features");
    }

    #[test]
    fn arm64_arch_detection_accepts_pi_64_bit_names() {
        assert!(is_arm64_arch("aarch64"));
        assert!(is_arm64_arch("arm64"));
        assert!(!is_arm64_arch("armv7l"));
        assert!(!is_arm64_arch("x86_64"));
    }

    #[test]
    fn pi_os_lite_detection_accepts_bookworm_raspberry_pi_os() {
        let os = parse_os_release(
            r#"
PRETTY_NAME="Raspberry Pi OS GNU/Linux 12 (bookworm)"
NAME="Raspberry Pi OS GNU/Linux"
VERSION_ID="12"
VERSION_CODENAME=bookworm
ID=debian
"#,
        );
        assert!(is_pi_os_lite_compatible(
            &os,
            Some("Raspberry Pi reference 2024-03-15")
        ));
    }

    #[test]
    fn pi_os_lite_detection_rejects_non_pi_debian() {
        let os = parse_os_release(
            r#"
PRETTY_NAME="Debian GNU/Linux 12 (bookworm)"
VERSION_CODENAME=bookworm
ID=debian
"#,
        );
        assert!(!is_pi_os_lite_compatible(&os, None));
    }

    #[test]
    fn pi_os_lite_host_probes_capture_expected_failures() {
        let probes = pi_os_lite_host_probes(&PiOsLiteHostFacts {
            arch: "armv7l".to_string(),
            os_release: "ID=debian\nVERSION_CODENAME=bullseye\n".to_string(),
            rpi_issue: Some("Raspberry Pi reference".to_string()),
        });

        assert!(
            probes
                .iter()
                .any(|probe| probe.id == "pi_arch" && probe.status == LinuxProbeStatus::Fail)
        );
        assert!(
            probes
                .iter()
                .any(|probe| probe.id == "pi_os_lite" && probe.status == LinuxProbeStatus::Fail)
        );
    }

    #[test]
    fn screen_capture_capability_tracks_modes_and_delay() {
        let import_only = linux_screen_capture_capability_with(|command| command == "import");
        assert!(import_only.any_mode_available());
        assert!(!import_only.core_modes_available());
        assert!(import_only.delay.is_empty());
        assert!(import_only.missing_detail().contains("delay"));

        let scrot = linux_screen_capture_capability_with(|command| command == "scrot");
        assert!(scrot.core_modes_available());
        assert!(scrot.window.contains(&"scrot"));
        assert!(scrot.interactive.contains(&"scrot"));
    }

    #[test]
    fn camera_capability_requires_tool_and_device() {
        let no_device = linux_camera_capability_from(true, false, None, Vec::new());
        assert!(!no_device.ready());
        assert!(no_device.missing_detail().contains("/dev/video*"));

        let detected =
            linux_camera_capability_from(false, true, None, vec!["/dev/video2".to_string()]);
        assert!(detected.ready());

        let configured_missing = linux_camera_capability_from(
            false,
            true,
            Some("/definitely/missing/thinclaw-camera".to_string()),
            vec!["/dev/video0".to_string()],
        );
        assert!(!configured_missing.ready());
        assert!(
            configured_missing
                .missing_detail()
                .contains("configured V4L2 device")
        );

        let temp = tempfile::NamedTempFile::new().expect("temp camera device placeholder");
        let configured_present = linux_camera_capability_from(
            false,
            true,
            Some(temp.path().display().to_string()),
            Vec::new(),
        );
        assert!(configured_present.ready());
    }

    #[test]
    fn microphone_capability_validates_backend_and_source() {
        let missing_source = linux_microphone_capability_from(
            LinuxMicrophoneBackendProbe::Auto,
            true,
            LinuxAudioSourceHints::default(),
        );
        assert!(!missing_source.ready());

        let pulse_from_pipewire = linux_microphone_capability_from(
            LinuxMicrophoneBackendProbe::Pulse,
            true,
            LinuxAudioSourceHints {
                pipewire: true,
                ..Default::default()
            },
        );
        assert!(pulse_from_pipewire.ready());

        let explicit_override = linux_microphone_capability_from(
            LinuxMicrophoneBackendProbe::Alsa,
            true,
            LinuxAudioSourceHints {
                configured_device: Some("hw:1,0".to_string()),
                ..Default::default()
            },
        );
        assert!(explicit_override.ready());

        let missing_ffmpeg = linux_microphone_capability_from(
            LinuxMicrophoneBackendProbe::Pipewire,
            false,
            LinuxAudioSourceHints {
                pipewire: true,
                ..Default::default()
            },
        );
        assert!(!missing_ffmpeg.ready());
    }

    #[test]
    fn geoclue_probe_does_not_treat_ip_fallback_as_native_location() {
        let probe = geoclue_probe_from_facts(
            true,
            GeoclueFacts {
                has_gdbus: true,
                system_bus_reachable: true,
                manager_reachable: false,
                ip_fallback_allowed: true,
            },
        );
        assert_eq!(probe.status, LinuxProbeStatus::Fail);
        assert!(probe.detail.contains("not native device location"));

        let native = geoclue_probe_from_facts(
            true,
            GeoclueFacts {
                has_gdbus: true,
                system_bus_reachable: true,
                manager_reachable: true,
                ip_fallback_allowed: false,
            },
        );
        assert_eq!(native.status, LinuxProbeStatus::Pass);
    }

    #[test]
    fn gnome_x11_detection_rejects_wayland_and_kde() {
        let gnome_x11 = LinuxDesktopSessionFacts {
            display: Some(":0".to_string()),
            wayland_display: None,
            session_type: Some("x11".to_string()),
            current_desktop: Some("GNOME".to_string()),
        };
        assert!(linux_gnome_x11_ready(&gnome_x11));

        let wayland = LinuxDesktopSessionFacts {
            display: Some(":0".to_string()),
            wayland_display: Some("wayland-0".to_string()),
            session_type: Some("wayland".to_string()),
            current_desktop: Some("GNOME".to_string()),
        };
        assert!(!linux_gnome_x11_ready(&wayland));

        let kde = LinuxDesktopSessionFacts {
            display: Some(":0".to_string()),
            wayland_display: None,
            session_type: Some("x11".to_string()),
            current_desktop: Some("KDE".to_string()),
        };
        assert!(!linux_gnome_x11_ready(&kde));
    }

    #[test]
    fn report_counts_statuses() {
        let report = LinuxReadinessReport {
            profile: LinuxReadinessProfile::Server,
            probes: vec![
                LinuxProbe::pass("a", "a", "ok"),
                LinuxProbe::fail("b", "b", "bad", "fix it"),
                LinuxProbe::skip("c", "c", "unused"),
            ],
        };
        assert_eq!(report.passed(), 1);
        assert_eq!(report.failed(), 1);
        assert_eq!(report.skipped(), 1);
        assert!(!report.is_ready());
    }
}
