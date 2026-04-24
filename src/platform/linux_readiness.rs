use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinuxReadinessProfile {
    Server,
    DesktopGnome,
    AllFeatures,
}

impl LinuxReadinessProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::DesktopGnome => "desktop-gnome",
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
    probes.push(probe_secrets().await);
    probes.push(probe_systemd_user(profile));

    let docker = crate::sandbox::check_docker().await;
    probes.push(probe_docker(profile, docker.status));
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

async fn probe_secrets() -> LinuxProbe {
    let probe = crate::platform::secure_store::probe_availability().await;
    if probe.available {
        LinuxProbe::pass("secrets", "Secrets", probe.detail)
    } else {
        LinuxProbe::fail("secrets", "Secrets", probe.detail, probe.guidance)
    }
}

fn probe_systemd_user(profile: LinuxReadinessProfile) -> LinuxProbe {
    let required = profile.needs_desktop() || crate::platform::env_flag_enabled("SERVICE_ENABLED");
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
        return LinuxProbe::pass(
            "browser",
            "Browser",
            "No local browser required because Docker Chromium fallback is available.",
        );
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

fn probe_screen_capture(profile: LinuxReadinessProfile) -> LinuxProbe {
    let required =
        profile.needs_desktop() || crate::platform::env_flag_enabled("SCREEN_CAPTURE_ENABLED");
    if crate::platform::linux_screen_capture_available() {
        return LinuxProbe::pass(
            "screen_capture",
            "Screen capture",
            "At least one of gnome-screenshot, scrot, or ImageMagick import is available.",
        );
    }
    if required {
        LinuxProbe::fail(
            "screen_capture",
            "Screen capture",
            "No supported Linux screenshot tool was found.",
            "Install a screenshot tool: sudo apt install gnome-screenshot scrot imagemagick",
        )
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
    if crate::platform::executable_available("fswebcam")
        || crate::platform::executable_available("ffmpeg")
    {
        return LinuxProbe::pass(
            "camera",
            "Camera capture",
            "fswebcam or ffmpeg is available for webcam capture.",
        );
    }
    if required {
        LinuxProbe::fail(
            "camera",
            "Camera capture",
            "No supported webcam capture command was found.",
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
    if crate::platform::executable_available("ffmpeg") {
        return LinuxProbe::pass(
            "microphone",
            "Microphone capture",
            "ffmpeg is available for microphone capture.",
        );
    }
    if required {
        LinuxProbe::fail(
            "microphone",
            "Microphone capture",
            "ffmpeg is required for Linux microphone capture but was not found.",
            "Install ffmpeg: sudo apt install ffmpeg",
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
    let has_dbus = std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some()
        || std::env::var_os("XDG_RUNTIME_DIR").is_some();
    if has_dbus && crate::platform::executable_available("gdbus") {
        return LinuxProbe::pass(
            "geoclue",
            "GeoClue",
            "D-Bus session and gdbus are available for GeoClue location lookup.",
        );
    }
    if required {
        LinuxProbe::fail(
            "geoclue",
            "GeoClue",
            "GeoClue requires a user D-Bus session and gdbus, but one or both are missing.",
            "Install geoclue-2.0 and libglib2.0-bin, then run from a logged-in user session. IP fallback requires LOCATION_ALLOW_IP_FALLBACK=true.",
        )
    } else {
        LinuxProbe::skip(
            "geoclue",
            "GeoClue",
            "Only required when LOCATION_ENABLED=true.",
        )
    }
}

fn probe_desktop_stack(profile: LinuxReadinessProfile) -> Vec<LinuxProbe> {
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

    let session_type = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let has_gnome = desktop
        .split(':')
        .any(|value| value.eq_ignore_ascii_case("gnome"));
    let display_ok = std::env::var_os("DISPLAY").is_some()
        && !session_type.eq_ignore_ascii_case("wayland")
        && has_gnome;
    if display_ok {
        probes.push(LinuxProbe::pass(
            "gnome_x11",
            "GNOME/X11 desktop",
            "DISPLAY is set and XDG_CURRENT_DESKTOP reports GNOME/X11.",
        ));
    } else {
        probes.push(LinuxProbe::fail(
            "gnome_x11",
            "GNOME/X11 desktop",
            "Linux desktop autonomy only supports a logged-in GNOME on X11 session for this release.",
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
    "Install the GNOME/X11 desktop prerequisites: sudo apt install python3 python3-gi python3-pyatspi libreoffice libreoffice-script-provider-python evolution evolution-data-server-bin xdotool wmctrl tesseract-ocr gnome-screenshot scrot imagemagick at-spi2-core libglib2.0-bin"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_names_are_stable() {
        assert_eq!(LinuxReadinessProfile::Server.as_str(), "server");
        assert_eq!(
            LinuxReadinessProfile::DesktopGnome.as_str(),
            "desktop-gnome"
        );
        assert_eq!(LinuxReadinessProfile::AllFeatures.as_str(), "all-features");
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
