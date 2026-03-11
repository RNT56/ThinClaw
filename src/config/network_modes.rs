//! Network mode support: loopback, LAN, and remote.
//!
//! Controls how the gateway binds and what transport modes are available.
//! Promotes the existing HTTP-only mode to support all three network modes.

use serde::{Deserialize, Serialize};

/// Network mode for the gateway.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkMode {
    /// Loopback only (127.0.0.1) — most secure, same-machine only.
    Loopback,
    /// LAN mode (0.0.0.0) — accessible from local network.
    Lan,
    /// Remote mode — accessible from the internet (requires auth).
    Remote,
}

impl Default for NetworkMode {
    fn default() -> Self {
        Self::Loopback
    }
}

impl NetworkMode {
    /// Create from environment variable.
    pub fn from_env() -> Self {
        match std::env::var("NETWORK_MODE")
            .unwrap_or_default()
            .to_lowercase()
            .as_str()
        {
            "lan" | "local" => Self::Lan,
            "remote" | "public" => Self::Remote,
            _ => Self::Loopback,
        }
    }

    /// Get the bind address for this mode.
    pub fn bind_address(&self) -> &str {
        match self {
            Self::Loopback => "127.0.0.1",
            Self::Lan | Self::Remote => "0.0.0.0",
        }
    }

    /// Whether auth is required for this mode.
    pub fn requires_auth(&self) -> bool {
        matches!(self, Self::Remote)
    }

    /// Whether mDNS/Bonjour discovery should be active.
    pub fn discovery_enabled(&self) -> bool {
        matches!(self, Self::Lan | Self::Remote)
    }

    /// Security recommendations for this mode.
    pub fn security_recommendations(&self) -> Vec<&str> {
        match self {
            Self::Loopback => vec!["Default safe mode — no external access"],
            Self::Lan => vec![
                "Enable gateway token auth",
                "Use firewall rules to restrict access",
                "Consider TLS for sensitive data",
            ],
            Self::Remote => vec![
                "REQUIRED: Enable TLS",
                "REQUIRED: Enable gateway token auth",
                "Recommended: Use reverse proxy",
                "Recommended: Rate limit connections",
                "Recommended: Enable IP allowlist",
            ],
        }
    }
}

/// Network configuration aggregating mode + port + discovery.
#[derive(Debug, Clone)]
pub struct NetworkConfig {
    pub mode: NetworkMode,
    pub port: u16,
    pub discovery_port: Option<u16>,
    pub tls_enabled: bool,
    pub auth_required: bool,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            mode: NetworkMode::default(),
            port: 3000,
            discovery_port: None,
            tls_enabled: false,
            auth_required: false,
        }
    }
}

impl NetworkConfig {
    /// Create from environment.
    pub fn from_env() -> Self {
        let mode = NetworkMode::from_env();
        let auth_required = mode.requires_auth()
            || std::env::var("GATEWAY_AUTH_REQUIRED")
                .map(|v| v != "0")
                .unwrap_or(false);

        Self {
            port: std::env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(3000),
            discovery_port: if mode.discovery_enabled() {
                Some(5353)
            } else {
                None
            },
            tls_enabled: std::env::var("TLS_ENABLED")
                .map(|v| v != "0")
                .unwrap_or(false),
            auth_required,
            mode,
        }
    }

    /// Full bind address with port.
    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.mode.bind_address(), self.port)
    }

    /// Validate the config (check for security issues).
    pub fn validate(&self) -> Vec<String> {
        let mut warnings = Vec::new();

        if self.mode == NetworkMode::Remote && !self.tls_enabled {
            warnings.push("Remote mode without TLS is insecure!".to_string());
        }

        if self.mode == NetworkMode::Remote && !self.auth_required {
            warnings.push("Remote mode without auth is dangerous!".to_string());
        }

        if self.mode == NetworkMode::Lan && !self.auth_required {
            warnings.push("LAN mode without auth — anyone on the network can access.".to_string());
        }

        warnings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_mode() {
        assert_eq!(NetworkMode::default(), NetworkMode::Loopback);
    }

    #[test]
    fn test_bind_address() {
        assert_eq!(NetworkMode::Loopback.bind_address(), "127.0.0.1");
        assert_eq!(NetworkMode::Lan.bind_address(), "0.0.0.0");
        assert_eq!(NetworkMode::Remote.bind_address(), "0.0.0.0");
    }

    #[test]
    fn test_requires_auth() {
        assert!(!NetworkMode::Loopback.requires_auth());
        assert!(!NetworkMode::Lan.requires_auth());
        assert!(NetworkMode::Remote.requires_auth());
    }

    #[test]
    fn test_discovery_enabled() {
        assert!(!NetworkMode::Loopback.discovery_enabled());
        assert!(NetworkMode::Lan.discovery_enabled());
        assert!(NetworkMode::Remote.discovery_enabled());
    }

    #[test]
    fn test_security_recommendations() {
        let recs = NetworkMode::Remote.security_recommendations();
        assert!(recs.len() >= 3);
        assert!(recs.iter().any(|r| r.contains("TLS")));
    }

    #[test]
    fn test_default_config() {
        let config = NetworkConfig::default();
        assert_eq!(config.port, 3000);
        assert_eq!(config.mode, NetworkMode::Loopback);
    }

    #[test]
    fn test_bind_addr() {
        let config = NetworkConfig {
            port: 8080,
            mode: NetworkMode::Lan,
            ..Default::default()
        };
        assert_eq!(config.bind_addr(), "0.0.0.0:8080");
    }

    #[test]
    fn test_validate_remote_no_tls() {
        let config = NetworkConfig {
            mode: NetworkMode::Remote,
            tls_enabled: false,
            auth_required: true,
            ..Default::default()
        };
        let warnings = config.validate();
        assert!(warnings.iter().any(|w| w.contains("insecure")));
    }

    #[test]
    fn test_validate_remote_no_auth() {
        let config = NetworkConfig {
            mode: NetworkMode::Remote,
            tls_enabled: true,
            auth_required: false,
            ..Default::default()
        };
        let warnings = config.validate();
        assert!(warnings.iter().any(|w| w.contains("dangerous")));
    }

    #[test]
    fn test_validate_loopback_clean() {
        let config = NetworkConfig::default();
        assert!(config.validate().is_empty());
    }
}
