//! Bonjour/mDNS device discovery.
//!
//! Enables automatic discovery of ThinClaw instances on the local network
//! using mDNS (Bonjour/Avahi) service advertisement and browsing.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// mDNS service type for ThinClaw.
pub const SERVICE_TYPE: &str = "_thinclaw._tcp";

/// Compute the discovery fingerprint of the persisted gateway instance id:
/// unpadded base64url of `sha256(instance-id bytes)`.
///
/// This is a stable, non-reversible tag advertised in the `fp` TXT record so a
/// previously paired client can recognize a rediscovered endpoint without the
/// raw instance id (or any secret) ever crossing the wire. It is a *locator
/// hint only* — the client still verifies the pinned SPKI and the pairing-time
/// instance id before sending a credential (D-X3). Returns `None` when no
/// instance id has been persisted yet (i.e. no pairing has happened).
pub fn instance_fingerprint() -> Option<String> {
    thinclaw_platform::read_instance_id().map(|id| fingerprint_instance_id(&id))
}

/// Fingerprint a specific instance id (see [`instance_fingerprint`]). Split out
/// so the TXT-record hashing is unit-testable without touching the filesystem.
pub fn fingerprint_instance_id(instance_id: &str) -> String {
    let digest = Sha256::digest(instance_id.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

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
            .unwrap_or_else(|| "thinclaw".to_string());

        Self {
            enabled: false,
            service_name: format!("ThinClaw on {}", hostname),
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
    ///
    /// The advertised record carries only non-sensitive locator hints:
    /// - `version` — advertising build version
    /// - `api` — gateway API version (`v1`)
    /// - `name` — human-readable instance display name (from settings or host)
    /// - `fp` — [`instance_fingerprint`] of the persisted instance id, present
    ///   only once the gateway has been paired at least once
    ///
    /// It NEVER contains tokens, secrets, credentials, or filesystem/home paths.
    pub fn build_txt_record(&self) -> HashMap<String, String> {
        let mut txt = self.txt_properties.clone();
        txt.insert("version".to_string(), env!("CARGO_PKG_VERSION").to_string());
        txt.insert("api".to_string(), "v1".to_string());
        txt.insert("name".to_string(), self.service_name.clone());
        if let Some(fp) = instance_fingerprint() {
            txt.insert("fp".to_string(), fp);
        }
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
        assert_eq!(SERVICE_TYPE, "_thinclaw._tcp");
    }

    #[test]
    fn test_default_config() {
        let config = MdnsConfig::default();
        assert!(!config.enabled);
        assert!(config.service_name.contains("ThinClaw"));
    }

    #[test]
    fn test_build_txt_record() {
        let config = MdnsConfig::default();
        let txt = config.build_txt_record();
        assert!(txt.contains_key("version"));
        assert!(txt.contains_key("api"));
    }

    #[test]
    fn test_build_txt_record_includes_name() {
        let config = MdnsConfig {
            service_name: "ThinClaw on test-host".to_string(),
            ..Default::default()
        };
        let txt = config.build_txt_record();
        assert_eq!(
            txt.get("name").map(String::as_str),
            Some("ThinClaw on test-host")
        );
    }

    #[test]
    fn test_build_txt_record_has_no_secrets_or_paths() {
        let config = MdnsConfig::default();
        let txt = config.build_txt_record();
        // Locator hints only — never tokens, secrets, or filesystem/home paths.
        let forbidden_keys = [
            "token",
            "secret",
            "iid",
            "instance_id",
            "path",
            "home",
            "key",
        ];
        for key in forbidden_keys {
            assert!(!txt.contains_key(key), "TXT record must not carry `{key}`");
        }
        for (k, v) in &txt {
            assert!(
                !v.contains("/.thinclaw")
                    && !v.contains(&*std::env::var("HOME").unwrap_or_default()),
                "TXT value for `{k}` leaked a home path: {v}"
            );
        }
    }

    #[test]
    fn test_fingerprint_is_stable_and_base64url() {
        let fp1 = fingerprint_instance_id("11111111-2222-3333-4444-555555555555");
        let fp2 = fingerprint_instance_id("11111111-2222-3333-4444-555555555555");
        assert_eq!(fp1, fp2, "fingerprint must be deterministic");
        // Unpadded base64url of a sha256 digest: 43 chars, url-safe alphabet.
        assert_eq!(fp1.len(), 43);
        assert!(!fp1.contains('='), "must be unpadded");
        assert!(
            fp1.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "must use the url-safe alphabet: {fp1}"
        );
    }

    #[test]
    fn test_fingerprint_differs_per_instance() {
        let a = fingerprint_instance_id("instance-a");
        let b = fingerprint_instance_id("instance-b");
        assert_ne!(a, b);
    }

    #[test]
    fn test_fingerprint_is_not_reversible_to_raw_id() {
        let raw = "11111111-2222-3333-4444-555555555555";
        let fp = fingerprint_instance_id(raw);
        assert!(
            !fp.contains(raw),
            "fingerprint must not embed the raw instance id"
        );
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
