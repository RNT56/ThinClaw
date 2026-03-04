//! Podman container support.
//!
//! Alternative to Docker for running sandboxed tasks,
//! using Podman's rootless container runtime.

use serde::{Deserialize, Serialize};

/// Podman configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodmanConfig {
    /// Path to the podman binary.
    pub binary_path: String,
    /// Default image for tasks.
    pub default_image: String,
    /// Whether rootless mode is enforced.
    pub rootless: bool,
    /// Resource limits.
    pub limits: PodmanLimits,
    /// Network mode.
    pub network: PodmanNetwork,
    /// Additional labels.
    pub labels: Vec<(String, String)>,
}

/// Resource limits for Podman containers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodmanLimits {
    /// Memory limit (e.g., "1g").
    pub memory: String,
    /// CPU limit (e.g., "2").
    pub cpus: String,
    /// Disk size limit (e.g., "10g").
    pub disk: Option<String>,
    /// No new privileges flag.
    pub no_new_privileges: bool,
    /// Read-only root filesystem.
    pub read_only_rootfs: bool,
}

impl Default for PodmanLimits {
    fn default() -> Self {
        Self {
            memory: "1g".to_string(),
            cpus: "2".to_string(),
            disk: None,
            no_new_privileges: true,
            read_only_rootfs: false,
        }
    }
}

/// Podman network modes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PodmanNetwork {
    /// No network access.
    None,
    /// Host network (not recommended).
    Host,
    /// Bridge (default isolated).
    Bridge,
    /// Custom network name.
    Custom(String),
}

impl Default for PodmanNetwork {
    fn default() -> Self {
        Self::Bridge
    }
}

impl Default for PodmanConfig {
    fn default() -> Self {
        Self {
            binary_path: "podman".to_string(),
            default_image: "docker.io/library/ubuntu:22.04".to_string(),
            rootless: true,
            limits: PodmanLimits::default(),
            network: PodmanNetwork::default(),
            labels: vec![("managed-by".to_string(), "ironclaw".to_string())],
        }
    }
}

impl PodmanConfig {
    /// Create from environment.
    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Ok(path) = std::env::var("PODMAN_BINARY") {
            config.binary_path = path;
        }
        if let Ok(image) = std::env::var("PODMAN_IMAGE") {
            config.default_image = image;
        }
        if let Ok(mem) = std::env::var("PODMAN_MEMORY") {
            config.limits.memory = mem;
        }
        config
    }

    /// Build the command-line arguments for `podman run`.
    pub fn run_args(&self, image: Option<&str>, cmd: &[&str]) -> Vec<String> {
        let mut args = vec!["run".to_string(), "--rm".to_string()];

        // Resource limits
        args.push(format!("--memory={}", self.limits.memory));
        args.push(format!("--cpus={}", self.limits.cpus));

        if self.limits.no_new_privileges {
            args.push("--security-opt=no-new-privileges".to_string());
        }
        if self.limits.read_only_rootfs {
            args.push("--read-only".to_string());
        }

        // Network
        match &self.network {
            PodmanNetwork::None => args.push("--network=none".to_string()),
            PodmanNetwork::Host => args.push("--network=host".to_string()),
            PodmanNetwork::Bridge => {} // default
            PodmanNetwork::Custom(name) => args.push(format!("--network={}", name)),
        }

        // Labels
        for (k, v) in &self.labels {
            args.push(format!("--label={}={}", k, v));
        }

        // Image
        args.push(image.unwrap_or(&self.default_image).to_string());

        // Command
        for c in cmd {
            args.push(c.to_string());
        }

        args
    }

    /// Detect if podman is available.
    pub fn is_available(&self) -> bool {
        std::process::Command::new(&self.binary_path)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = PodmanConfig::default();
        assert!(config.rootless);
        assert_eq!(config.limits.memory, "1g");
    }

    #[test]
    fn test_run_args_basic() {
        let config = PodmanConfig::default();
        let args = config.run_args(None, &["bash", "-c", "echo hello"]);
        assert!(args.contains(&"run".to_string()));
        assert!(args.contains(&"--rm".to_string()));
        assert!(args.iter().any(|a| a.contains("--memory")));
        assert!(args.contains(&config.default_image));
    }

    #[test]
    fn test_run_args_custom_image() {
        let config = PodmanConfig::default();
        let args = config.run_args(Some("my-image:latest"), &["ls"]);
        assert!(args.contains(&"my-image:latest".to_string()));
    }

    #[test]
    fn test_run_args_no_network() {
        let config = PodmanConfig {
            network: PodmanNetwork::None,
            ..Default::default()
        };
        let args = config.run_args(None, &["echo"]);
        assert!(args.contains(&"--network=none".to_string()));
    }

    #[test]
    fn test_no_new_privileges() {
        let config = PodmanConfig::default();
        let args = config.run_args(None, &["echo"]);
        assert!(args.iter().any(|a| a.contains("no-new-privileges")));
    }

    #[test]
    fn test_labels() {
        let config = PodmanConfig::default();
        let args = config.run_args(None, &["echo"]);
        assert!(args.iter().any(|a| a.contains("managed-by=ironclaw")));
    }

    #[test]
    fn test_network_variants() {
        assert_eq!(PodmanNetwork::default(), PodmanNetwork::Bridge);
        assert_ne!(PodmanNetwork::None, PodmanNetwork::Host);
    }
}
