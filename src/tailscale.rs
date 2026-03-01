//! Tailscale discovery module.
//!
//! Queries the local Tailscale daemon API to discover other devices on the
//! tailnet. This enables the Tauri thin client to auto-find the headless
//! orchestrator without manual IP configuration.
//!
//! The Tailscale local API runs on `localhost:41112` by default (macOS/Linux)
//! and provides information about the tailnet, connected peers, and the local
//! node's status.

use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};

/// Tailscale local API client.
pub struct TailscaleDiscovery {
    client: Client,
    /// Base URL of the Tailscale local API (default: http://localhost:41112)
    base_url: String,
}

/// Tailscale status response (subset of fields we care about).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TailscaleStatus {
    /// The local machine's tailnet IP addresses.
    #[serde(rename = "TailscaleIPs")]
    pub tailscale_ips: Option<Vec<String>>,
    /// Current backend state (e.g., "Running", "NeedsLogin").
    pub backend_state: Option<String>,
    /// Self-node info.
    #[serde(rename = "Self")]
    pub self_node: Option<TailscalePeer>,
    /// Peer map (keyed by public key).
    pub peer: Option<std::collections::HashMap<String, TailscalePeer>>,
}

/// Information about a Tailscale peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TailscalePeer {
    /// Hostname of the peer.
    pub host_name: Option<String>,
    /// DNS name of the peer (e.g., "myhost.tailnet-name.ts.net").
    #[serde(rename = "DNSName")]
    pub dns_name: Option<String>,
    /// Tailscale IP addresses.
    #[serde(rename = "TailscaleIPs")]
    pub tailscale_ips: Option<Vec<String>>,
    /// Whether the peer is currently online.
    pub online: Option<bool>,
    /// OS of the peer.
    #[serde(rename = "OS")]
    pub os: Option<String>,
    /// Tags assigned to this peer (e.g., ["tag:server"]).
    pub tags: Option<Vec<String>>,
}

/// A discovered orchestrator on the tailnet.
#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredOrchestrator {
    pub hostname: String,
    pub dns_name: Option<String>,
    pub ip: String,
    pub os: Option<String>,
    pub online: bool,
}

impl TailscaleDiscovery {
    /// Create a new Tailscale discovery client with default settings.
    pub fn new() -> Self {
        Self::with_base_url("http://localhost:41112")
    }

    /// Create with a custom base URL.
    pub fn with_base_url(base_url: &str) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap_or_default(),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Check if Tailscale is available and running.
    pub async fn is_available(&self) -> bool {
        match self.get_status().await {
            Ok(status) => status
                .backend_state
                .as_deref()
                .map(|s| s == "Running")
                .unwrap_or(false),
            Err(_) => false,
        }
    }

    /// Get the full Tailscale status.
    pub async fn get_status(&self) -> Result<TailscaleStatus, String> {
        let resp = self
            .client
            .get(format!("{}/localapi/v0/status", self.base_url))
            .send()
            .await
            .map_err(|e| format!("Tailscale API: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!("Tailscale API returned {}", resp.status()));
        }

        resp.json().await.map_err(|e| format!("Parse status: {e}"))
    }

    /// Get the local machine's Tailscale IP.
    pub async fn local_ip(&self) -> Result<Option<String>, String> {
        let status = self.get_status().await?;
        Ok(status.tailscale_ips.and_then(|ips| ips.into_iter().next()))
    }

    /// Discover orchestrator instances on the tailnet.
    ///
    /// Looks for peers with the tag "tag:ironclaw" or matching hostname pattern.
    /// Falls back to listing all online peers if no specific orchestrators found.
    pub async fn discover_orchestrators(&self) -> Result<Vec<DiscoveredOrchestrator>, String> {
        let status = self.get_status().await?;

        let peers = status.peer.unwrap_or_default();
        let mut orchestrators = Vec::new();

        for peer in peers.values() {
            let online = peer.online.unwrap_or(false);
            if !online {
                continue;
            }

            let hostname = peer.host_name.as_deref().unwrap_or("unknown").to_string();

            // Check for ironclaw tag or hostname pattern
            let is_orchestrator = peer
                .tags
                .as_ref()
                .map(|tags| tags.iter().any(|t| t.contains("ironclaw")))
                .unwrap_or(false)
                || hostname.contains("ironclaw")
                || hostname.contains("molty");

            if !is_orchestrator {
                continue;
            }

            let ip = peer
                .tailscale_ips
                .as_ref()
                .and_then(|ips| ips.first())
                .cloned()
                .unwrap_or_default();

            if ip.is_empty() {
                continue;
            }

            orchestrators.push(DiscoveredOrchestrator {
                hostname,
                dns_name: peer.dns_name.clone(),
                ip,
                os: peer.os.clone(),
                online,
            });
        }

        Ok(orchestrators)
    }

    /// Try to find the best orchestrator URL for connection.
    /// Returns `http://<ip>:3000` for the first discovered orchestrator.
    pub async fn find_orchestrator_url(&self) -> Result<Option<String>, String> {
        let orchestrators = self.discover_orchestrators().await?;
        Ok(orchestrators
            .first()
            .map(|o| format!("http://{}:3000", o.ip)))
    }
}

impl Default for TailscaleDiscovery {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_base_url() {
        let discovery = TailscaleDiscovery::new();
        assert_eq!(discovery.base_url, "http://localhost:41112");
    }

    #[test]
    fn test_custom_base_url() {
        let discovery = TailscaleDiscovery::with_base_url("http://localhost:9999/");
        assert_eq!(discovery.base_url, "http://localhost:9999");
    }

    #[test]
    fn test_peer_deserialization() {
        let json = r#"{
            "HostName": "my-server",
            "DNSName": "my-server.tailnet.ts.net",
            "TailscaleIPs": ["100.64.1.2"],
            "Online": true,
            "OS": "linux",
            "Tags": ["tag:ironclaw"]
        }"#;
        let peer: TailscalePeer = serde_json::from_str(json).unwrap();
        assert_eq!(peer.host_name.as_deref(), Some("my-server"));
        assert_eq!(peer.online, Some(true));
        assert!(
            peer.tags
                .as_ref()
                .unwrap()
                .contains(&"tag:ironclaw".to_string())
        );
    }

    #[test]
    fn test_discovered_orchestrator_serialization() {
        let orch = DiscoveredOrchestrator {
            hostname: "server-1".to_string(),
            dns_name: Some("server-1.tail.ts.net".to_string()),
            ip: "100.64.1.2".to_string(),
            os: Some("linux".to_string()),
            online: true,
        };
        let json = serde_json::to_value(&orch).unwrap();
        assert_eq!(json["hostname"], "server-1");
        assert_eq!(json["ip"], "100.64.1.2");
    }
}
