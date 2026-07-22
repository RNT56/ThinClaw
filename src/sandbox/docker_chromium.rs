//! Validated configuration for the Docker-backed Chromium fallback.
//!
//! Container lifecycle and network isolation are owned by the root browser
//! adapter. This module deliberately contains no `docker run -p 9222:9222`
//! shortcut: CDP is a privileged unauthenticated control plane and must only be
//! published on loopback, while page traffic must traverse ThinClaw's pinned,
//! authenticated proxy on an isolated bridge.

use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Stable multi-architecture Chromium image. A tag alone is mutable; retaining
/// both the human-readable version and OCI index digest makes startup
/// reproducible on amd64 and arm64.
pub const DEFAULT_CHROMIUM_IMAGE: &str = "chromedp/headless-shell:150.0.7871.125@sha256:7f8ec4782f1b138c30900e65ae53795d5966fbf52168b8fc062843db3e6d5be5";

const DOCKER_COMMAND_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_IMAGE_REFERENCE_BYTES: usize = 512;
const MAX_CHROME_FLAGS: usize = 64;
const MAX_CHROME_FLAG_BYTES: usize = 512;
const MIN_BROWSER_MEMORY_BYTES: i64 = 128 * 1024 * 1024;
const MAX_BROWSER_MEMORY_BYTES: i64 = 8 * 1024 * 1024 * 1024;
const MIN_BROWSER_SHM_BYTES: i64 = 64 * 1024 * 1024;

fn default_runtime_scope() -> String {
    crate::runtime_lease::runtime_scope_id_for_path(&crate::platform::resolve_data_dir(""))
}

fn default_relay_image() -> String {
    thinclaw_types::sandbox::SandboxConfig::default().image
}

/// Configuration for Docker-based Chromium.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerChromiumConfig {
    /// Digest-pinned Docker image containing headless-shell.
    pub image: String,
    /// Whether to pull the image on startup when it is absent.
    pub auto_pull: bool,
    /// Legacy Xvfb display number (the default image is truly headless).
    pub display: String,
    /// Browser viewport shape retained for compatibility/readiness output.
    pub resolution: String,
    /// Legacy Xvfb color depth.
    pub color_depth: u8,
    /// CDP port inside the container. Docker assigns a random loopback host port.
    pub debug_port: u16,
    /// Additional safe Chrome flags.
    pub chrome_flags: Vec<String>,
    /// Memory limit for the container.
    pub memory_limit: String,
    /// Shared-memory size for Chrome.
    pub shm_size: String,
    /// Stable owner label for cleanup and collision isolation.
    #[serde(default = "default_runtime_scope")]
    pub runtime_scope: String,
    /// ThinClaw worker image containing the narrow network-relay entrypoint.
    #[serde(default = "default_relay_image")]
    pub relay_image: String,
}

impl Default for DockerChromiumConfig {
    fn default() -> Self {
        Self {
            image: DEFAULT_CHROMIUM_IMAGE.to_string(),
            auto_pull: true,
            display: String::new(),
            resolution: "1920x1080".to_string(),
            color_depth: 24,
            debug_port: 9222,
            chrome_flags: vec![
                "--headless=new".to_string(),
                "--disable-gpu".to_string(),
                "--disable-background-networking".to_string(),
                "--disable-component-update".to_string(),
                "--disable-default-apps".to_string(),
                "--disable-sync".to_string(),
                "--metrics-recording-only".to_string(),
                "--no-first-run".to_string(),
                "--no-default-browser-check".to_string(),
            ],
            memory_limit: "2g".to_string(),
            shm_size: "512m".to_string(),
            runtime_scope: default_runtime_scope(),
            relay_image: default_relay_image(),
        }
    }
}

impl DockerChromiumConfig {
    /// Create from environment. Every override is validated before use.
    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Ok(image) = std::env::var("CHROMIUM_IMAGE") {
            config.image = image;
        }
        if let Ok(resolution) = std::env::var("CHROMIUM_RESOLUTION") {
            config.resolution = resolution;
        }
        if let Ok(port) = std::env::var("CHROMIUM_DEBUG_PORT")
            && let Ok(port) = port.parse()
        {
            config.debug_port = port;
        }
        if let Ok(image) = std::env::var("BROWSER_RELAY_IMAGE") {
            config.relay_image = image;
        }
        config
    }

    pub fn with_runtime_scope(mut self, runtime_scope: impl Into<String>) -> Self {
        self.runtime_scope = runtime_scope.into();
        self
    }

    pub fn with_relay_image(mut self, relay_image: impl Into<String>) -> Self {
        self.relay_image = relay_image.into();
        self
    }

    pub fn validate(&self) -> Result<(), DockerError> {
        validate_image_reference(&self.image, true)?;
        validate_image_reference(&self.relay_image, false)?;
        if self.runtime_scope.is_empty()
            || self.runtime_scope.len() > 128
            || !self.runtime_scope.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            })
        {
            return Err(DockerError::InvalidConfig(
                "Docker Chromium runtime scope is invalid".to_string(),
            ));
        }
        if self.debug_port == 0 {
            return Err(DockerError::InvalidConfig(
                "Docker Chromium CDP port cannot be zero".to_string(),
            ));
        }
        validate_resolution(&self.resolution)?;
        if !(16..=32).contains(&self.color_depth) {
            return Err(DockerError::InvalidConfig(
                "Docker Chromium color depth must be between 16 and 32".to_string(),
            ));
        }
        if self.display.len() > 32 || self.display.chars().any(char::is_control) {
            return Err(DockerError::InvalidConfig(
                "Docker Chromium display value is invalid".to_string(),
            ));
        }
        if self.chrome_flags.len() > MAX_CHROME_FLAGS {
            return Err(DockerError::InvalidConfig(format!(
                "Docker Chromium has too many flags (maximum {MAX_CHROME_FLAGS})"
            )));
        }
        for flag in &self.chrome_flags {
            validate_chrome_flag(flag)?;
        }
        let memory = self.memory_bytes()?;
        let shm = self.shm_bytes()?;
        if !(MIN_BROWSER_MEMORY_BYTES..=MAX_BROWSER_MEMORY_BYTES).contains(&memory) {
            return Err(DockerError::InvalidConfig(
                "Docker Chromium memory limit must be between 128 MiB and 8 GiB".to_string(),
            ));
        }
        if shm < MIN_BROWSER_SHM_BYTES || shm > memory {
            return Err(DockerError::InvalidConfig(
                "Docker Chromium shared memory must be at least 64 MiB and no larger than its memory limit"
                    .to_string(),
            ));
        }
        Ok(())
    }

    pub fn memory_bytes(&self) -> Result<i64, DockerError> {
        parse_byte_size(&self.memory_limit)
    }

    pub fn shm_bytes(&self) -> Result<i64, DockerError> {
        parse_byte_size(&self.shm_size)
    }

    /// Stable per-runtime name. Runtime leases prevent two live processes from
    /// sharing the same scope, while labels protect against unrelated collisions.
    pub fn container_name(&self) -> String {
        let scope = self.runtime_scope.chars().take(32).collect::<String>();
        format!("thinclaw-chromium-{scope}")
    }

    /// Chrome arguments installed behind the isolated relay. The caller passes
    /// the relay's network alias; no direct host or LAN route is available.
    pub fn chrome_args(
        &self,
        proxy_host: &str,
        proxy_port: u16,
    ) -> Result<Vec<String>, DockerError> {
        self.validate()?;
        if proxy_host.is_empty()
            || proxy_host.len() > 128
            || !proxy_host.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '.')
            })
            || proxy_port == 0
        {
            return Err(DockerError::InvalidConfig(
                "Docker Chromium proxy endpoint is invalid".to_string(),
            ));
        }
        let mut args = vec![
            "--remote-debugging-address=0.0.0.0".to_string(),
            format!("--remote-debugging-port={}", self.debug_port),
            format!("--proxy-server=http://{proxy_host}:{proxy_port}"),
            "--proxy-bypass-list=<-loopback>".to_string(),
            "--disable-quic".to_string(),
            "--force-webrtc-ip-handling-policy=disable_non_proxied_udp".to_string(),
            "--webrtc-ip-handling-policy=disable_non_proxied_udp".to_string(),
            "--user-data-dir=/tmp/thinclaw-browser-profile".to_string(),
        ];
        args.extend(self.chrome_flags.clone());
        args.push("about:blank".to_string());
        Ok(args)
    }

    /// Build a legacy Xvfb command string for diagnostics.
    pub fn xvfb_command(&self) -> String {
        let display = if self.display.trim().is_empty() {
            ":99"
        } else {
            self.display.as_str()
        };
        format!(
            "Xvfb {} -screen 0 {}x{} -ac +extension GLX +render -noreset",
            display, self.resolution, self.color_depth
        )
    }

    /// Compatibility diagnostic: the executable followed by validated flags.
    pub fn chrome_command(&self, url: Option<&str>) -> Vec<String> {
        let mut command = vec![
            "headless-shell".to_string(),
            "--remote-debugging-address=0.0.0.0".to_string(),
            format!("--remote-debugging-port={}", self.debug_port),
        ];
        command.extend(self.chrome_flags.clone());
        if let Some(url) = url {
            command.push(url.to_string());
        }
        command
    }

    pub fn debugger_url_for_host_port(host_port: u16) -> String {
        format!("ws://127.0.0.1:{host_port}")
    }

    pub fn http_endpoint_for_host_port(host_port: u16) -> String {
        format!("http://127.0.0.1:{host_port}")
    }

    pub fn is_docker_available() -> bool {
        command_success_with_timeout(
            Command::new("docker").arg("version"),
            Duration::from_secs(5),
        )
    }

    pub fn image_available_locally(&self) -> bool {
        self.validate().is_ok() && image_reference_available_locally(&self.image)
    }

    pub fn relay_image_available_locally(&self) -> bool {
        self.validate().is_ok() && image_reference_available_locally(&self.relay_image)
    }

    pub fn image_manifest_available(&self) -> bool {
        self.validate().is_ok() && image_manifest_available(&self.image)
    }

    pub fn image_readiness_detail(&self) -> Result<String, DockerError> {
        self.validate()?;
        let browser_ready =
            self.image_available_locally() || (self.auto_pull && self.image_manifest_available());
        if !browser_ready {
            return Err(DockerError::ImageUnavailable(format!(
                "Docker Chromium image `{}` is unavailable. Set CHROMIUM_IMAGE to a reachable digest-pinned CDP image, or install a local browser.",
                self.image
            )));
        }
        let relay_ready = self.relay_image_available_locally()
            || (self.auto_pull
                && self.relay_image.contains("@sha256:")
                && image_manifest_available(&self.relay_image));
        if !relay_ready {
            return Err(DockerError::ImageUnavailable(format!(
                "Browser relay image `{}` is not available locally. Build Dockerfile.worker with that tag, or set BROWSER_RELAY_IMAGE to a local or digest-pinned ThinClaw image.",
                self.relay_image
            )));
        }
        Ok(format!(
            "Docker Chromium image `{}` and relay image `{}` are ready.",
            self.image, self.relay_image
        ))
    }
}

fn image_reference_available_locally(reference: &str) -> bool {
    command_success_with_timeout(
        Command::new("docker").args(["image", "inspect", reference]),
        Duration::from_secs(5),
    )
}

fn image_manifest_available(reference: &str) -> bool {
    command_success_with_timeout(
        Command::new("docker").args(["manifest", "inspect", reference]),
        DOCKER_COMMAND_TIMEOUT,
    )
}

#[derive(Debug, thiserror::Error)]
pub enum DockerError {
    #[error("Invalid Docker Chromium configuration: {0}")]
    InvalidConfig(String),
    #[error("Docker Chromium image unavailable: {0}")]
    ImageUnavailable(String),
    #[error("Docker command failed: {0}")]
    CommandFailed(String),
    #[error("Docker Chromium operation timed out: {0}")]
    Timeout(String),
}

fn validate_image_reference(reference: &str, require_digest: bool) -> Result<(), DockerError> {
    if reference.is_empty()
        || reference.len() > MAX_IMAGE_REFERENCE_BYTES
        || !reference.is_ascii()
        || reference
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(DockerError::InvalidConfig(
            "Docker image reference is empty, oversized, or malformed".to_string(),
        ));
    }
    if require_digest {
        let Some((name, digest)) = reference.rsplit_once("@sha256:") else {
            return Err(DockerError::InvalidConfig(
                "CHROMIUM_IMAGE must be pinned with an @sha256 digest".to_string(),
            ));
        };
        if name.is_empty()
            || digest.len() != 64
            || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(DockerError::InvalidConfig(
                "CHROMIUM_IMAGE has an invalid sha256 digest".to_string(),
            ));
        }
    }
    Ok(())
}

fn validate_resolution(resolution: &str) -> Result<(), DockerError> {
    let Some((width, height)) = resolution.split_once('x') else {
        return Err(DockerError::InvalidConfig(
            "Docker Chromium resolution must use WIDTHxHEIGHT".to_string(),
        ));
    };
    let width = width.parse::<u32>().unwrap_or(0);
    let height = height.parse::<u32>().unwrap_or(0);
    if !(320..=7_680).contains(&width) || !(200..=4_320).contains(&height) {
        return Err(DockerError::InvalidConfig(
            "Docker Chromium resolution is outside the supported range".to_string(),
        ));
    }
    Ok(())
}

fn validate_chrome_flag(flag: &str) -> Result<(), DockerError> {
    const RESERVED: &[&str] = &[
        "--proxy-server",
        "--proxy-bypass-list",
        "--no-proxy-server",
        "--remote-debugging",
        "--host-resolver-rules",
        "--user-data-dir",
        "--disable-web-security",
        "--allow-running-insecure-content",
    ];
    if !flag.starts_with("--")
        || flag.len() > MAX_CHROME_FLAG_BYTES
        || !flag.is_ascii()
        || flag.chars().any(char::is_control)
        || RESERVED.iter().any(|reserved| flag.starts_with(reserved))
    {
        return Err(DockerError::InvalidConfig(format!(
            "Docker Chromium flag is invalid or reserved: {}",
            flag.chars().take(64).collect::<String>()
        )));
    }
    Ok(())
}

fn parse_byte_size(value: &str) -> Result<i64, DockerError> {
    let value = value.trim().to_ascii_lowercase();
    let split = value
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(value.len());
    let (number, suffix) = value.split_at(split);
    if number.is_empty() {
        return Err(DockerError::InvalidConfig(
            "Docker Chromium byte size is invalid".to_string(),
        ));
    }
    let number = number.parse::<i64>().map_err(|_| {
        DockerError::InvalidConfig("Docker Chromium byte size is invalid".to_string())
    })?;
    let multiplier = match suffix {
        "" | "b" => 1_i64,
        "k" | "kb" | "kib" => 1024,
        "m" | "mb" | "mib" => 1024 * 1024,
        "g" | "gb" | "gib" => 1024 * 1024 * 1024,
        _ => {
            return Err(DockerError::InvalidConfig(
                "Docker Chromium byte size suffix is invalid".to_string(),
            ));
        }
    };
    number.checked_mul(multiplier).ok_or_else(|| {
        DockerError::InvalidConfig("Docker Chromium byte size overflowed".to_string())
    })
}

fn command_success_with_timeout(command: &mut Command, timeout: Duration) -> bool {
    thinclaw_platform::bounded_std_command_output(command, timeout, 1024, 1024)
        .is_ok_and(|output| output.status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_pinned_and_valid() {
        let config = DockerChromiumConfig::default();
        assert_eq!(config.image, DEFAULT_CHROMIUM_IMAGE);
        assert!(config.image.contains("@sha256:"));
        assert_eq!(config.debug_port, 9222);
        config.validate().unwrap();
    }

    #[test]
    fn chrome_arguments_force_proxy_and_reserve_security_flags() {
        let config = DockerChromiumConfig::default();
        let args = config.chrome_args("relay-browser", 18_080).unwrap();
        assert!(
            args.iter()
                .any(|arg| arg == "--proxy-server=http://relay-browser:18080")
        );
        assert!(args.iter().any(|arg| arg == "--disable-quic"));
        assert!(!args.iter().any(|arg| arg == "--no-sandbox"));

        let mut invalid = config;
        invalid
            .chrome_flags
            .push("--proxy-server=http://evil:1".to_string());
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn legacy_diagnostics_remain_well_formed() {
        let config = DockerChromiumConfig::default();
        let xvfb = config.xvfb_command();
        assert!(xvfb.contains(":99"));
        assert!(xvfb.contains("1920x1080"));
        let command = config.chrome_command(Some("https://example.com"));
        assert!(command.iter().any(|arg| arg.contains("9222")));
        assert!(command.contains(&"https://example.com".to_string()));
    }

    #[test]
    fn endpoint_helpers_only_publish_loopback() {
        assert_eq!(
            DockerChromiumConfig::http_endpoint_for_host_port(39_123),
            "http://127.0.0.1:39123"
        );
        assert_eq!(
            DockerChromiumConfig::debugger_url_for_host_port(39_123),
            "ws://127.0.0.1:39123"
        );
    }

    #[test]
    fn names_are_scoped_and_resource_sizes_are_bounded() {
        let config = DockerChromiumConfig::default().with_runtime_scope("abc123");
        assert_eq!(config.container_name(), "thinclaw-chromium-abc123");
        assert_eq!(parse_byte_size("512m").unwrap(), 512 * 1024 * 1024);
        assert!(parse_byte_size("2tb").is_err());
    }

    #[test]
    fn mutable_or_malformed_images_fail_closed() {
        let mut config = DockerChromiumConfig::default();
        config.image = "chromedp/headless-shell:latest".to_string();
        assert!(config.validate().is_err());
        config.image = "repo@sha256:not-a-digest".to_string();
        assert!(config.validate().is_err());
    }
}
