//! Authenticated Tailscale identity adapter for the web gateway.
//!
//! The gateway crate owns policy and RBAC. This runtime adapter owns the one
//! host capability required by that policy: a bounded call to the local
//! Tailscale CLI, which in turn queries the authenticated local daemon.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value};
use thinclaw_gateway::web::auth::{TailscaleIdentityResolver, TailscalePeerIdentity};

const WHOIS_TIMEOUT: Duration = Duration::from_secs(2);
const WHOIS_STDOUT_LIMIT: usize = 256 * 1024;
const WHOIS_STDERR_LIMIT: usize = 64 * 1024;
const MAX_CONCURRENT_WHOIS: usize = 4;

/// CLI-backed resolver with a hard concurrency bound. It deliberately does
/// not cache successful identities: node/user revocation is reflected on the
/// next HTTP request instead of living behind an application TTL.
pub(crate) struct TailscaleWhoisResolver {
    concurrency: Arc<tokio::sync::Semaphore>,
}

impl TailscaleWhoisResolver {
    pub(crate) fn new() -> Self {
        Self {
            concurrency: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_WHOIS)),
        }
    }
}

#[async_trait::async_trait]
impl TailscaleIdentityResolver for TailscaleWhoisResolver {
    async fn resolve(&self, source: SocketAddr) -> Result<Option<TailscalePeerIdentity>, String> {
        let _permit = Arc::clone(&self.concurrency)
            .try_acquire_owned()
            .map_err(|_| "Tailscale whois concurrency limit reached".to_string())?;
        let binary = crate::util::resolve_binary("tailscale");
        let mut command = thinclaw_platform::tokio_process_command!(
            "src.channels.web.tailscale_identity.tokio.101",
            binary
        );
        command.args(["whois", "--json", &source.to_string()]);
        let output = thinclaw_platform::bounded_command_output(
            &mut command,
            WHOIS_TIMEOUT,
            WHOIS_STDOUT_LIMIT,
            WHOIS_STDERR_LIMIT,
        )
        .await
        .map_err(|error| format!("bounded tailscale whois failed: {error}"))?;
        if !output.status.success() {
            // A non-zero result includes unknown, expired, and revoked peers.
            // Treat all of them alike and never reflect CLI stderr to clients.
            return Ok(None);
        }
        parse_whois_response(&output.stdout, source.ip()).map(Some)
    }
}

fn parse_whois_response(bytes: &[u8], source_ip: IpAddr) -> Result<TailscalePeerIdentity, String> {
    let document: Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("invalid tailscale whois JSON: {error}"))?;
    let root = document
        .as_object()
        .ok_or_else(|| "tailscale whois JSON root was not an object".to_string())?;
    let node = object_field(root, "Node")
        .ok_or_else(|| "tailscale whois JSON omitted Node".to_string())?;
    let node_id = id_field(node, "ID")
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "tailscale whois JSON omitted Node.ID".to_string())?;

    let addresses = string_array_field(node, "Addresses");
    if !addresses
        .iter()
        .any(|address| address_matches_ip(address, source_ip))
    {
        return Err("tailscale whois response did not bind the queried source address".to_string());
    }

    let user = object_field(root, "UserProfile");
    Ok(TailscalePeerIdentity {
        node_id,
        node_name: string_field(node, "Name"),
        user_id: user.and_then(|profile| id_field(profile, "ID")),
        user_login: user
            .and_then(|profile| string_field(profile, "LoginName"))
            .map(|login| login.trim().to_ascii_lowercase())
            .filter(|login| !login.is_empty()),
        tags: string_array_field(node, "Tags"),
    })
}

fn object_field<'a>(object: &'a Map<String, Value>, key: &str) -> Option<&'a Map<String, Value>> {
    object.get(key)?.as_object()
}

fn string_field(object: &Map<String, Value>, key: &str) -> Option<String> {
    object
        .get(key)?
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn id_field(object: &Map<String, Value>, key: &str) -> Option<String> {
    match object.get(key)? {
        Value::String(value) => Some(value.trim().to_string()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn string_array_field(object: &Map<String, Value>, key: &str) -> Vec<String> {
    object
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn address_matches_ip(address: &str, expected: IpAddr) -> bool {
    address
        .split_once('/')
        .map_or(address, |(address, _prefix)| address)
        .parse::<IpAddr>()
        .is_ok_and(|address| address == expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_user_owned_peer_and_binds_source_address() {
        let parsed = parse_whois_response(
            br#"{
                "Node": {
                    "ID": 1234,
                    "Name": "alice-phone.tailnet.ts.net.",
                    "Addresses": ["100.64.10.20", "fd7a:115c:a1e0::10"],
                    "Tags": []
                },
                "UserProfile": {
                    "ID": 5678,
                    "LoginName": "Alice@Example.COM"
                }
            }"#,
            "100.64.10.20".parse().unwrap(),
        )
        .unwrap();
        assert_eq!(parsed.node_id, "1234");
        assert_eq!(parsed.user_id.as_deref(), Some("5678"));
        assert_eq!(parsed.user_login.as_deref(), Some("alice@example.com"));
        assert!(parsed.tags.is_empty());
    }

    #[test]
    fn parses_tagged_peer_without_user_profile() {
        let parsed = parse_whois_response(
            br#"{
                "Node": {
                    "ID": "node-44",
                    "Name": "build-agent",
                    "Addresses": ["100.100.10.20/32"],
                    "Tags": ["tag:ci", "tag:thinclaw"]
                }
            }"#,
            "100.100.10.20".parse().unwrap(),
        )
        .unwrap();
        assert_eq!(parsed.node_id, "node-44");
        assert_eq!(parsed.user_login, None);
        assert_eq!(parsed.tags, ["tag:ci", "tag:thinclaw"]);
    }

    #[test]
    fn rejects_response_for_a_different_source_address() {
        let error = parse_whois_response(
            br#"{"Node":{"ID":1,"Addresses":["100.64.10.21"]}}"#,
            "100.64.10.20".parse().unwrap(),
        )
        .unwrap_err();
        assert!(error.contains("bind the queried source"));
    }
}
