//! Docker Chromium support.
//!
//! Configuration for running a headless Chrome/Chromium browser
//! inside a Docker container.
//!
//! This is used when no local Chrome/Chromium binary is available (e.g.
//! headless Linux servers without a desktop environment).  The container
//! exposes a CDP (Chrome DevTools Protocol) debugging port that the
//! [`BrowserTool`](crate::tools::builtin::browser::BrowserTool) connects
//! to via `chromiumoxide::Browser::connect()`.

use std::process::Command;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Public multi-arch headless Chrome image used by default.
///
/// The image supports linux/amd64 and linux/arm64, which keeps the Pi OS Lite
/// browser fallback aligned with the release artifact targets.
pub const DEFAULT_CHROMIUM_IMAGE: &str = "chromedp/headless-shell:latest";

/// Deterministic Docker container name so we can re-attach across restarts.
const CONTAINER_NAME: &str = "thinclaw-chromium";

const DOCKER_COMMAND_TIMEOUT: Duration = Duration::from_secs(20);

/// Configuration for Docker-based Chromium.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerChromiumConfig {
    /// Docker image containing Chromium + Xvfb.
    pub image: String,
    /// Whether to pull the image on startup.
    pub auto_pull: bool,
    /// Xvfb display number.
    pub display: String,
    /// Screen resolution for Xvfb.
    pub resolution: String,
    /// Color depth.
    pub color_depth: u8,
    /// Port to expose for remote debugging.
    pub debug_port: u16,
    /// Additional Chrome flags.
    pub chrome_flags: Vec<String>,
    /// Memory limit for the container.
    pub memory_limit: String,
    /// SHM size (shared memory for Chrome).
    pub shm_size: String,
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
                "--no-sandbox".to_string(),
                "--headless=new".to_string(),
                "--disable-gpu".to_string(),
                "--disable-dev-shm-usage".to_string(),
                "--disable-setuid-sandbox".to_string(),
            ],
            memory_limit: "2g".to_string(),
            shm_size: "2g".to_string(),
        }
    }
}

impl DockerChromiumConfig {
    /// Create from environment.
    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Ok(image) = std::env::var("CHROMIUM_IMAGE") {
            config.image = image;
        }
        if let Ok(res) = std::env::var("CHROMIUM_RESOLUTION") {
            config.resolution = res;
        }
        if let Ok(port) = std::env::var("CHROMIUM_DEBUG_PORT")
            && let Ok(p) = port.parse()
        {
            config.debug_port = p;
        }
        config
    }

    /// Build a legacy Xvfb command string.
    ///
    /// The current default image is truly headless and does not need Xvfb. This
    /// method is kept for compatibility with callers/tests that inspect the
    /// configured display shape.
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

    /// Build Chrome launch command.
    pub fn chrome_command(&self, url: Option<&str>) -> Vec<String> {
        let mut args = vec![
            "headless-shell".to_string(),
            "--remote-debugging-address=0.0.0.0".to_string(),
            format!("--remote-debugging-port={}", self.debug_port),
        ];
        args.extend(self.chrome_flags.clone());

        if let Some(url) = url {
            args.push(url.to_string());
        }
        args
    }

    /// Docker run arguments.
    pub fn docker_args(&self) -> Vec<String> {
        let mut args = vec![
            "run".to_string(),
            "--rm".to_string(),
            "-d".to_string(),
            format!("--name={}", CONTAINER_NAME),
            format!("--memory={}", self.memory_limit),
            format!("--shm-size={}", self.shm_size),
            format!("-p={}:{}", self.debug_port, self.debug_port),
            self.image.clone(),
        ];
        args.extend(self.chrome_command(Some("about:blank")).into_iter().skip(1));
        args
    }

    /// WebSocket debugger URL.
    pub fn debugger_url(&self) -> String {
        format!("ws://127.0.0.1:{}", self.debug_port)
    }

    /// HTTP endpoint for Chrome's `/json/version` (used by `chromiumoxide` to
    /// discover the WebSocket URL automatically).
    pub fn http_endpoint(&self) -> String {
        format!("http://127.0.0.1:{}", self.debug_port)
    }

    // ── Container lifecycle ─────────────────────────────────────────────

    /// Check if Docker is available on this system.
    pub fn is_docker_available() -> bool {
        command_success_with_timeout(
            Command::new("docker").arg("version"),
            Duration::from_secs(5),
        )
    }

    /// Check whether the configured image is already present locally.
    pub fn image_available_locally(&self) -> bool {
        command_success_with_timeout(
            Command::new("docker").args(["image", "inspect", &self.image]),
            Duration::from_secs(5),
        )
    }

    /// Check whether Docker can resolve the configured image manifest.
    pub fn image_manifest_available(&self) -> bool {
        command_success_with_timeout(
            Command::new("docker").args(["manifest", "inspect", &self.image]),
            DOCKER_COMMAND_TIMEOUT,
        )
    }

    /// Human-readable readiness detail for diagnostics.
    pub fn image_readiness_detail(&self) -> Result<String, DockerError> {
        if self.image_available_locally() {
            return Ok(format!(
                "Docker Chromium image `{}` is present locally.",
                self.image
            ));
        }

        if self.auto_pull && self.image_manifest_available() {
            return Ok(format!(
                "Docker Chromium image `{}` is pullable for this host.",
                self.image
            ));
        }

        Err(DockerError::ImageUnavailable(format!(
            "Docker Chromium image `{}` is not present locally and its manifest could not be resolved. Set CHROMIUM_IMAGE to a reachable CDP-capable Chromium image, or install a local browser.",
            self.image
        )))
    }

    /// Check if our container is already running.
    pub fn is_container_running(&self) -> bool {
        command_output_with_timeout(
            Command::new("docker").args(["inspect", "-f", "{{.State.Running}}", CONTAINER_NAME]),
            Duration::from_secs(5),
        )
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "true")
        .unwrap_or(false)
    }

    /// Start the Chromium Docker container.
    ///
    /// If a container with the same name is already running, it is left as-is.
    /// Returns the container ID on success.
    pub fn start_container(&self) -> Result<String, DockerError> {
        // If container is already running, return its ID.
        if self.is_container_running() {
            let id = command_output_with_timeout(
                Command::new("docker").args(["inspect", "-f", "{{.Id}}", CONTAINER_NAME]),
                Duration::from_secs(5),
            )
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
            tracing::debug!(id = %id, "Chromium container already running");
            return Ok(id);
        }

        // Pull image if configured.
        if self.auto_pull && !self.image_available_locally() {
            tracing::info!(image = %self.image, "Pulling Chromium Docker image");
            let pull = command_output_with_timeout(
                Command::new("docker").args(["pull", &self.image]),
                DOCKER_COMMAND_TIMEOUT,
            )
            .map_err(DockerError::CommandFailed)?;
            if !pull.status.success() {
                return Err(DockerError::ImageUnavailable(format!(
                    "failed to pull `{}`: {}",
                    self.image,
                    command_error_text(&pull)
                )));
            }
        }

        // Remove any stopped container with the same name.
        let _ = command_output_with_timeout(
            Command::new("docker").args(["rm", "-f", CONTAINER_NAME]),
            Duration::from_secs(10),
        );

        // Build full docker run command.
        let output = command_output_with_timeout(
            Command::new("docker").args(self.docker_args()),
            DOCKER_COMMAND_TIMEOUT,
        )
        .map_err(|e| DockerError::CommandFailed(format!("Failed to run docker: {e}")))?;

        if !output.status.success() {
            return Err(DockerError::ContainerStart(format!(
                "docker run failed: {}",
                command_error_text(&output)
            )));
        }

        let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        tracing::info!(id = %container_id, "Started Chromium Docker container");
        Ok(container_id)
    }

    /// Wait for Chrome inside the container to accept CDP connections.
    ///
    /// Polls the debug port via TCP until a connection succeeds or the
    /// timeout expires.
    pub async fn wait_for_ready(&self, timeout: Duration) -> Result<(), DockerError> {
        let start = Instant::now();
        let addr = format!("127.0.0.1:{}", self.debug_port);

        loop {
            if start.elapsed() > timeout {
                return Err(DockerError::Timeout(format!(
                    "Chrome in Docker not ready after {timeout:?}"
                )));
            }

            // Try a TCP connection to the debug port.
            match std::net::TcpStream::connect_timeout(
                &addr.parse().expect("valid socket addr"),
                Duration::from_secs(1),
            ) {
                Ok(_) => {
                    tracing::debug!(
                        elapsed = ?start.elapsed(),
                        "Chrome in Docker is ready (port {} open)",
                        self.debug_port
                    );
                    return Ok(());
                }
                Err(_) => {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }
    }

    /// Stop and remove the Chromium Docker container.
    pub fn stop_container(&self) -> Result<(), DockerError> {
        let output = command_output_with_timeout(
            Command::new("docker").args(["rm", "-f", CONTAINER_NAME]),
            Duration::from_secs(10),
        )
        .map_err(|e| DockerError::CommandFailed(format!("Failed to run docker rm: {e}")))?;

        if output.status.success() {
            tracing::info!("Stopped Chromium Docker container");
        } else {
            return Err(DockerError::CommandFailed(format!(
                "docker rm failed: {}",
                command_error_text(&output)
            )));
        }
        Ok(())
    }
}

/// Errors related to Docker Chromium container management.
#[derive(Debug, thiserror::Error)]
pub enum DockerError {
    #[error("Docker command failed: {0}")]
    CommandFailed(String),

    #[error("Container start failed: {0}")]
    ContainerStart(String),

    #[error("Docker Chromium image unavailable: {0}")]
    ImageUnavailable(String),

    #[error("Timeout waiting for Chrome: {0}")]
    Timeout(String),
}

fn command_success_with_timeout(command: &mut Command, timeout: Duration) -> bool {
    command_output_with_timeout(command, timeout)
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn command_output_with_timeout(
    command: &mut Command,
    timeout: Duration,
) -> Result<std::process::Output, String> {
    let mut child = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to spawn command: {error}"))?;

    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                return child
                    .wait_with_output()
                    .map_err(|error| format!("failed to collect command output: {error}"));
            }
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("command timed out after {timeout:?}"));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("failed to poll command: {error}"));
            }
        }
    }
}

fn command_error_text(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stdout.is_empty() {
        return stdout;
    }
    format!("exit status {}", output.status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = DockerChromiumConfig::default();
        assert_eq!(config.image, DEFAULT_CHROMIUM_IMAGE);
        assert_eq!(config.debug_port, 9222);
    }

    #[test]
    fn test_xvfb_command() {
        let config = DockerChromiumConfig::default();
        let cmd = config.xvfb_command();
        assert!(cmd.contains(":99"));
        assert!(cmd.contains("1920x1080"));
    }

    #[test]
    fn test_chrome_command() {
        let config = DockerChromiumConfig::default();
        let cmd = config.chrome_command(Some("https://example.com"));
        assert!(cmd.iter().any(|a| a.contains("9222")));
        assert!(cmd.contains(&"https://example.com".to_string()));
    }

    #[test]
    fn test_chrome_command_no_url() {
        let config = DockerChromiumConfig::default();
        let cmd = config.chrome_command(None);
        assert!(!cmd.iter().any(|a| a.starts_with("http")));
    }

    #[test]
    fn test_docker_args() {
        let config = DockerChromiumConfig::default();
        let args = config.docker_args();
        assert!(args.contains(&"run".to_string()));
        assert!(args.iter().any(|a| a.contains("shm-size")));
        assert!(args.contains(&"--remote-debugging-address=0.0.0.0".to_string()));
    }

    #[test]
    fn test_debugger_url() {
        let config = DockerChromiumConfig::default();
        assert_eq!(config.debugger_url(), "ws://127.0.0.1:9222");
    }

    #[test]
    fn test_http_endpoint() {
        let config = DockerChromiumConfig::default();
        assert_eq!(config.http_endpoint(), "http://127.0.0.1:9222");
    }

    #[test]
    fn test_container_name_is_deterministic() {
        assert_eq!(CONTAINER_NAME, "thinclaw-chromium");
    }

    #[test]
    fn test_custom_debug_port_in_endpoints() {
        let config = DockerChromiumConfig {
            debug_port: 9333,
            ..Default::default()
        };
        assert_eq!(config.http_endpoint(), "http://127.0.0.1:9333");
        assert_eq!(config.debugger_url(), "ws://127.0.0.1:9333");
    }
}
