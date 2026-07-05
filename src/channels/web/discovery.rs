//! LAN discovery advertiser (mDNS/Bonjour, milestone B3).
//!
//! Advertises the running gateway on `_thinclaw._tcp` so a paired mobile client
//! can relocate it after an address change. Discovery is a **locator only**
//! (docs/MOBILE_SECURITY.md D-X3): the advertised TXT record carries a
//! non-reversible fingerprint of the instance id plus display/version hints, and
//! the client must still verify the pinned SPKI and pairing-time instance id
//! before any credential is sent. Tokens, secrets, and home paths never enter
//! the TXT record.
//!
//! Default-off. The advertiser starts only when the operator opts in — either
//! `discovery.enabled = true` in settings, or the `MDNS_ENABLED` env override.
//! The whole responder is behind `#[cfg(feature = "mdns")]`; a stub keeps
//! default builds (and `--features edge`) compiling with discovery disabled.

use std::net::SocketAddr;

use thinclaw_config::mdns_discovery::MdnsConfig;

#[cfg(feature = "mdns")]
use super::server::GatewayState;

/// Resolve whether LAN discovery should advertise, and with what display name.
///
/// Affirmative when settings `discovery.enabled` is true OR the `MDNS_ENABLED`
/// env override is set (`from_env` already parsed it). Default-off: with neither
/// set, nothing is advertised. Returns the resolved service name when enabled.
#[cfg(feature = "mdns")]
pub(crate) async fn resolve_discovery(state: &GatewayState) -> Option<String> {
    let env_config = MdnsConfig::from_env();
    let mut enabled = env_config.enabled;
    let mut service_name: Option<String> = None;

    if let Some(store) = state.store.as_ref() {
        use thinclaw_gateway::web::ports::SettingsPort;
        if let Ok(snapshot) = state.load_settings(&state.user_id).await
            && let Some(discovery) = snapshot.values.get("discovery")
        {
            if let Some(b) = discovery.get("enabled").and_then(|v| v.as_bool()) {
                enabled = enabled || b;
            }
            if let Some(name) = discovery
                .get("service_name")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                service_name = Some(name.to_string());
            }
        }
        let _ = store;
    }

    if !enabled {
        return None;
    }

    // Env `MDNS_SERVICE_NAME` (already applied to `env_config`) is honored when
    // settings did not provide a name; otherwise fall back to the host-derived
    // default from `MdnsConfig`.
    Some(service_name.unwrap_or(env_config.service_name))
}

#[cfg(feature = "mdns")]
mod imp {
    use super::*;
    use thinclaw_config::mdns_discovery::SERVICE_TYPE;

    /// Handle for a running advertiser. Dropping it unregisters the service and
    /// shuts the mDNS daemon down.
    pub struct MdnsAdvertiserHandle {
        daemon: mdns_sd::ServiceDaemon,
        fullname: String,
    }

    impl Drop for MdnsAdvertiserHandle {
        fn drop(&mut self) {
            // Best-effort: unregister and shut down. Errors here only matter at
            // process teardown, so log at debug.
            if let Err(e) = self.daemon.unregister(&self.fullname) {
                tracing::debug!(error = %e, "mDNS unregister failed during shutdown");
            }
            if let Err(e) = self.daemon.shutdown() {
                tracing::debug!(error = %e, "mDNS daemon shutdown failed");
            }
        }
    }

    /// Register the gateway on `_thinclaw._tcp` at `bound` with locator TXT
    /// records. Returns `None` (and logs) when advertising is impossible or
    /// pointless — notably when `bound` is loopback, which no other host on the
    /// LAN can reach.
    pub fn spawn_mdns_advertiser(
        mut config: MdnsConfig,
        bound: SocketAddr,
        name: String,
    ) -> Option<MdnsAdvertiserHandle> {
        if bound.ip().is_loopback() {
            tracing::warn!(
                addr = %bound,
                "LAN discovery requested but gateway is bound to loopback; \
                 skipping mDNS advertisement (no LAN peer can reach loopback)"
            );
            return None;
        }

        config.service_name = name.clone();
        config.port = bound.port();
        let txt = config.build_txt_record();

        let hostname = hostname_for_mdns();
        // The instance name must be unique on the LAN; the fingerprint (or the
        // port, if unpaired) keeps two hosts advertising the same display name
        // from colliding.
        let instance_label = txt
            .get("fp")
            .map(|fp| format!("thinclaw-{}", &fp[..fp.len().min(12)]))
            .unwrap_or_else(|| format!("thinclaw-{}", bound.port()));

        // When the gateway binds to an unspecified address (`0.0.0.0` / `::`,
        // the common `host = 0.0.0.0` case) there is no single interface IP to
        // advertise. Pass an empty address and rely on `enable_addr_auto()`,
        // which fills in — and keeps updated — every real interface address.
        // A specific bound IP is advertised verbatim.
        let addr_seed = if bound.ip().is_unspecified() {
            String::new()
        } else {
            bound.ip().to_string()
        };

        let daemon = match mdns_sd::ServiceDaemon::new() {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to start mDNS daemon; LAN discovery disabled");
                return None;
            }
        };

        // `mdns-sd` expects the fully-qualified registration type ending in
        // `.local.`; `SERVICE_TYPE` is the bare `_thinclaw._tcp` form used for
        // browsing and matching.
        let registration_type = format!("{SERVICE_TYPE}.local.");
        let service = match mdns_sd::ServiceInfo::new(
            &registration_type,
            &instance_label,
            &hostname,
            addr_seed.as_str(),
            bound.port(),
            txt,
        ) {
            Ok(s) => s.enable_addr_auto(),
            Err(e) => {
                tracing::warn!(error = %e, "Failed to build mDNS service info; LAN discovery disabled");
                let _ = daemon.shutdown();
                return None;
            }
        };

        let fullname = service.get_fullname().to_string();
        if let Err(e) = daemon.register(service) {
            tracing::warn!(error = %e, "Failed to register mDNS service; LAN discovery disabled");
            let _ = daemon.shutdown();
            return None;
        }

        tracing::info!(
            service = %SERVICE_TYPE,
            addr = %bound,
            name = %name,
            "LAN discovery advertising (mDNS)"
        );

        Some(MdnsAdvertiserHandle { daemon, fullname })
    }

    /// Resolve a `.local` hostname for the service record. `mdns-sd` requires a
    /// trailing dot; derive it from the OS hostname and fall back to a stable
    /// literal.
    fn hostname_for_mdns() -> String {
        let base = std::process::Command::new("hostname")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "thinclaw".to_string());
        // Strip any existing domain, keep the leaf label, ensure a single
        // `.local.` suffix as mdns-sd expects.
        let leaf = base.split('.').next().unwrap_or("thinclaw");
        format!("{leaf}.local.")
    }
}

#[cfg(feature = "mdns")]
pub use imp::{MdnsAdvertiserHandle, spawn_mdns_advertiser};

#[cfg(not(feature = "mdns"))]
mod imp {
    use super::*;

    /// Discovery-disabled stub handle (keeps default builds compiling).
    pub struct MdnsAdvertiserHandle;

    /// No-op advertiser used when the `mdns` feature is off.
    pub fn spawn_mdns_advertiser(
        _config: MdnsConfig,
        _bound: SocketAddr,
        _name: String,
    ) -> Option<MdnsAdvertiserHandle> {
        None
    }
}

#[cfg(not(feature = "mdns"))]
pub use imp::{MdnsAdvertiserHandle, spawn_mdns_advertiser};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_bind_is_not_advertised() {
        let bound: SocketAddr = "127.0.0.1:3000".parse().unwrap();
        let handle = spawn_mdns_advertiser(MdnsConfig::default(), bound, "ThinClaw test".into());
        assert!(
            handle.is_none(),
            "advertising a loopback bind is useless and must be skipped"
        );
    }

    #[cfg(feature = "mdns")]
    #[test]
    fn advertiser_lifecycle_smoke() {
        // `0.0.0.0` is the common production bind (`host = 0.0.0.0`): not
        // loopback, so the guard does not fire, and `enable_addr_auto` fills in
        // the real interface addresses. Assert the daemon registers and then
        // tears down cleanly via Drop (unregister + shutdown).
        let bound: SocketAddr = "0.0.0.0:0".parse().unwrap();
        let handle = spawn_mdns_advertiser(MdnsConfig::default(), bound, "ThinClaw smoke".into());
        assert!(
            handle.is_some(),
            "a non-loopback bind should start an advertiser"
        );
        drop(handle);
    }
}
