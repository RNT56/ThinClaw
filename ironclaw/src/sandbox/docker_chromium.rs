//! Docker Chromium + Xvfb support.
//!
//! Configuration for running a headless Chrome/Chromium browser
//! inside a Docker container with Xvfb for rendering.

use serde::{Deserialize, Serialize};

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
            image: "ghcr.io/thinclaw/chromium-xvfb:latest".to_string(),
            auto_pull: true,
            display: ":99".to_string(),
            resolution: "1920x1080".to_string(),
            color_depth: 24,
            debug_port: 9222,
            chrome_flags: vec![
                "--no-sandbox".to_string(),
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

    /// Build Xvfb start command.
    pub fn xvfb_command(&self) -> String {
        format!(
            "Xvfb {} -screen 0 {}x{} -ac +extension GLX +render -noreset",
            self.display, self.resolution, self.color_depth
        )
    }

    /// Build Chrome launch command.
    pub fn chrome_command(&self, url: Option<&str>) -> Vec<String> {
        let mut args = vec![
            "chromium-browser".to_string(),
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
        vec![
            "run".to_string(),
            "--rm".to_string(),
            "-d".to_string(),
            format!("--memory={}", self.memory_limit),
            format!("--shm-size={}", self.shm_size),
            format!("-p={}:{}", self.debug_port, self.debug_port),
            self.image.clone(),
        ]
    }

    /// WebSocket debugger URL.
    pub fn debugger_url(&self) -> String {
        format!("ws://127.0.0.1:{}", self.debug_port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = DockerChromiumConfig::default();
        assert!(config.image.contains("chromium"));
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
    }

    #[test]
    fn test_debugger_url() {
        let config = DockerChromiumConfig::default();
        assert_eq!(config.debugger_url(), "ws://127.0.0.1:9222");
    }
}
