//! Bonjour/mDNS device discovery.
//!
//! Enables automatic discovery of IronClaw instances on the local network
//! using mDNS (Bonjour/Avahi) service advertisement and browsing.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// mDNS service type for IronClaw.
pub const SERVICE_TYPE: &str = "_ironclaw._tcp";

/// Configuration for mDNS discovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MdnsConfig {
    /// Whether discovery is enabled.
    pub enabled: bool,
    /// Service name to advertise.
    pub service_name: String,
    /// Port to advertise.
    pub port: u16,
    /// TTL for advertised records (seconds).
    pub ttl_secs: u32,
    /// Additional TXT record properties.
    pub txt_properties: HashMap<String, String>,
    /// Browse timeout in milliseconds.
    pub browse_timeout_ms: u64,
}

impl Default for MdnsConfig {
    fn default() -> Self {
        let hostname = std::process::Command::new("hostname")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "ironclaw".to_string());

        Self {
            enabled: false,
            service_name: format!("IronClaw on {}", hostname),
            port: 3000,
            ttl_secs: 120,
            txt_properties: HashMap::new(),
            browse_timeout_ms: 5000,
        }
    }
}

impl MdnsConfig {
    /// Create from environment.
    pub fn from_env() -> Self {
        let enabled = std::env::var("MDNS_ENABLED")
            .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
            .unwrap_or(false);

        let mut config = Self {
            enabled,
            ..Self::default()
        };

        if let Ok(name) = std::env::var("MDNS_SERVICE_NAME") {
            config.service_name = name;
        }
        if let Ok(port) = std::env::var("PORT")
            && let Ok(p) = port.parse()
        {
            config.port = p;
        }
        config
    }

    /// Build TXT record properties for advertisement.
    pub fn build_txt_record(&self) -> HashMap<String, String> {
        let mut txt = self.txt_properties.clone();
        txt.insert("version".to_string(), env!("CARGO_PKG_VERSION").to_string());
        txt.insert("api".to_string(), "v1".to_string());
        txt
    }
}

/// A discovered service on the network.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredService {
    /// Instance name.
    pub name: String,
    /// Service type.
    pub service_type: String,
    /// Hostname.
    pub hostname: String,
    /// IP addresses.
    pub addresses: Vec<String>,
    /// Port.
    pub port: u16,
    /// TXT record properties.
    pub txt: HashMap<String, String>,
    /// When this service was discovered.
    pub discovered_at: String,
}

impl DiscoveredService {
    /// Get the API base URL for this service.
    pub fn api_url(&self) -> Option<String> {
        self.addresses
            .first()
            .map(|addr| format!("http://{}:{}", addr, self.port))
    }

    /// Get the version from TXT records.
    pub fn version(&self) -> Option<&str> {
        self.txt.get("version").map(|v| v.as_str())
    }
}

/// Discovery tracker — maintains a list of discovered services.
pub struct DiscoveryTracker {
    services: HashMap<String, DiscoveredService>,
}

impl DiscoveryTracker {
    pub fn new() -> Self {
        Self {
            services: HashMap::new(),
        }
    }

    /// Add or update a discovered service.
    pub fn upsert(&mut self, service: DiscoveredService) {
        self.services.insert(service.name.clone(), service);
    }

    /// Remove a service (when it goes offline).
    pub fn remove(&mut self, name: &str) -> Option<DiscoveredService> {
        self.services.remove(name)
    }

    /// List all discovered services.
    pub fn list(&self) -> Vec<&DiscoveredService> {
        self.services.values().collect()
    }

    /// Find a service by name.
    pub fn find(&self, name: &str) -> Option<&DiscoveredService> {
        self.services.get(name)
    }

    /// Count of discovered services.
    pub fn count(&self) -> usize {
        self.services.len()
    }
}

impl Default for DiscoveryTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_type() {
        assert_eq!(SERVICE_TYPE, "_ironclaw._tcp");
    }

    #[test]
    fn test_default_config() {
        let config = MdnsConfig::default();
        assert!(!config.enabled);
        assert!(config.service_name.contains("IronClaw"));
    }

    #[test]
    fn test_build_txt_record() {
        let config = MdnsConfig::default();
        let txt = config.build_txt_record();
        assert!(txt.contains_key("version"));
        assert!(txt.contains_key("api"));
    }

    #[test]
    fn test_discovered_service_api_url() {
        let svc = DiscoveredService {
            name: "Test".into(),
            service_type: SERVICE_TYPE.into(),
            hostname: "test.local".into(),
            addresses: vec!["192.168.1.10".into()],
            port: 3000,
            txt: HashMap::new(),
            discovered_at: "now".into(),
        };
        assert_eq!(svc.api_url(), Some("http://192.168.1.10:3000".into()));
    }

    #[test]
    fn test_tracker_upsert() {
        let mut tracker = DiscoveryTracker::new();
        tracker.upsert(DiscoveredService {
            name: "Test".into(),
            service_type: SERVICE_TYPE.into(),
            hostname: "test.local".into(),
            addresses: vec![],
            port: 3000,
            txt: HashMap::new(),
            discovered_at: "now".into(),
        });
        assert_eq!(tracker.count(), 1);
    }

    #[test]
    fn test_tracker_remove() {
        let mut tracker = DiscoveryTracker::new();
        tracker.upsert(DiscoveredService {
            name: "Test".into(),
            service_type: SERVICE_TYPE.into(),
            hostname: "test.local".into(),
            addresses: vec![],
            port: 3000,
            txt: HashMap::new(),
            discovered_at: "now".into(),
        });
        assert!(tracker.remove("Test").is_some());
        assert_eq!(tracker.count(), 0);
    }

    #[test]
    fn test_tracker_find() {
        let mut tracker = DiscoveryTracker::new();
        tracker.upsert(DiscoveredService {
            name: "MyNode".into(),
            service_type: SERVICE_TYPE.into(),
            hostname: "node.local".into(),
            addresses: vec!["10.0.0.1".into()],
            port: 3000,
            txt: HashMap::new(),
            discovered_at: "now".into(),
        });
        assert!(tracker.find("MyNode").is_some());
        assert!(tracker.find("Other").is_none());
    }
}
