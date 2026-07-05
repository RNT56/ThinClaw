use super::*;

/// Optional LAN discovery (mDNS/Bonjour) settings.
///
/// Default-off, mirroring the other optional subsystems: nothing is advertised
/// on the local network unless the operator explicitly enables it (settings
/// `enabled = true` or the `MDNS_ENABLED` env override). Discovery is a locator
/// only — a rediscovered endpoint must still present the pinned SPKI and the
/// pairing-time instance id before any credential is sent (D-X3). The advertised
/// TXT record therefore carries only non-sensitive locator hints (version, api,
/// display name, instance-id fingerprint) — never tokens, secrets, or home paths.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiscoverySettings {
    /// Master toggle for mDNS/Bonjour advertisement of this gateway.
    /// Default-off: `false`.
    #[serde(default)]
    pub enabled: bool,
    /// Optional human-readable service name to advertise. When unset, the
    /// advertiser derives a name from the host (e.g. "ThinClaw on <hostname>").
    #[serde(default)]
    pub service_name: Option<String>,
}
