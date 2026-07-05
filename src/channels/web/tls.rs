//! Optional rustls TLS listener for the web gateway (`docs/MOBILE_SECURITY.md`
//! decision D-X1).
//!
//! Serves the same [`axum::Router`] as the plain-HTTP listener, over a
//! self-signed P-256 certificate whose SPKI fingerprint travels in the
//! pairing QR (see `docs/MOBILE_APP.md`), so the very first connection is
//! already pinned — no trust-on-first-use window. Gated behind the
//! `gateway-tls` cargo feature; the `#[cfg(not(...))]` stub below keeps
//! callers compiling with the feature off.
//!
//! Persistence mirrors `crates/thinclaw-channels/src/pairing.rs`: files live
//! under `~/.thinclaw/tls/`, private key material is written with `0600`
//! permissions, and existing material is reused across restarts rather than
//! regenerated. The SPKI fingerprint is computed once at generation time
//! (from the `KeyPair` we just built, before it's ever serialized) and
//! persisted alongside the cert/key so reuse never needs to re-parse X.509.

#[cfg(feature = "gateway-tls")]
mod imp {
    use std::io;
    use std::net::{IpAddr, SocketAddr};
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    use axum::Router;
    use axum_server::Handle;
    use axum_server::tls_rustls::RustlsConfig;
    use base64::Engine as _;
    use rcgen::string::Ia5String;
    use rcgen::{CertificateParams, DnType, KeyPair, PublicKeyData, SanType};
    use sha2::{Digest, Sha256};

    /// Default port for the TLS listener (`GATEWAY_TLS_PORT` overrides).
    const DEFAULT_TLS_PORT: u16 = 3443;
    /// Env var carrying comma-separated URLs to append as SANs / advertise
    /// verbatim, for operators behind NAT/port-forwarding setups the local
    /// interface scan can't infer.
    const ADVERTISED_URLS_ENV: &str = "GATEWAY_ADVERTISED_URLS";
    /// Env var selecting the TLS policy: `off` | `auto` | `on` (default `auto`).
    const GATEWAY_TLS_ENV: &str = "GATEWAY_TLS";
    /// Env var overriding the TLS listener port.
    const GATEWAY_TLS_PORT_ENV: &str = "GATEWAY_TLS_PORT";

    /// TLS policy read from `GATEWAY_TLS` (default `auto`).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum TlsPolicy {
        /// Never start the TLS listener.
        Off,
        /// Start at boot only if `~/.thinclaw/devices.json` exists and has at
        /// least one paired device; otherwise lazily start on first pairing
        /// via the gateway's pairing handler.
        Auto,
        /// Always start at boot.
        On,
    }

    impl TlsPolicy {
        /// Read the policy from `GATEWAY_TLS` (default [`TlsPolicy::Auto`]).
        pub fn from_env() -> Self {
            match std::env::var(GATEWAY_TLS_ENV) {
                Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
                    "off" => TlsPolicy::Off,
                    "on" => TlsPolicy::On,
                    _ => TlsPolicy::Auto,
                },
                Err(_) => TlsPolicy::Auto,
            }
        }
    }

    /// Persisted (or freshly generated) TLS material for the gateway listener.
    #[derive(Debug, Clone)]
    pub struct TlsMaterial {
        pub cert_pem_path: PathBuf,
        pub key_pem_path: PathBuf,
        /// SHA-256 of the certificate's SubjectPublicKeyInfo DER, raw bytes.
        pub spki_sha256: [u8; 32],
    }

    impl TlsMaterial {
        /// SPKI fingerprint, base64url (no padding) — the value carried in
        /// the pairing QR's `fp` field.
        pub fn fingerprint_base64url(&self) -> String {
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(self.spki_sha256)
        }

        /// First 12 chars of the fingerprint, safe to log (never the key).
        pub fn fingerprint_prefix(&self) -> String {
            self.fingerprint_base64url().chars().take(12).collect()
        }

        /// Load existing TLS material from `<base_dir>/tls/`, generating a
        /// fresh self-signed cert if none is present yet.
        pub fn load_or_generate(base_dir: &Path) -> io::Result<Self> {
            let tls_dir = base_dir.join("tls");
            std::fs::create_dir_all(&tls_dir)?;
            let cert_pem_path = tls_dir.join("gateway-cert.pem");
            let key_pem_path = tls_dir.join("gateway-key.pem");
            let fingerprint_path = tls_dir.join("gateway-cert.fingerprint");

            if cert_pem_path.exists() && key_pem_path.exists() {
                if let Some(material) =
                    Self::read_existing(&cert_pem_path, &key_pem_path, &fingerprint_path)
                {
                    return Ok(material);
                }
                tracing::warn!(
                    "Existing TLS material at {} could not be reused; regenerating",
                    tls_dir.display()
                );
            }

            Self::generate(&cert_pem_path, &key_pem_path, &fingerprint_path)
        }

        fn read_existing(
            cert_pem_path: &Path,
            key_pem_path: &Path,
            fingerprint_path: &Path,
        ) -> Option<Self> {
            // The fingerprint sidecar is written atomically alongside the cert
            // at generation time (see `generate`), so its presence implies a
            // valid, matching cert/key pair.
            let fingerprint_hex = std::fs::read_to_string(fingerprint_path).ok()?;
            let spki_sha256 = hex_to_32_bytes(fingerprint_hex.trim())?;
            Some(Self {
                cert_pem_path: cert_pem_path.to_path_buf(),
                key_pem_path: key_pem_path.to_path_buf(),
                spki_sha256,
            })
        }

        fn generate(
            cert_pem_path: &Path,
            key_pem_path: &Path,
            fingerprint_path: &Path,
        ) -> io::Result<Self> {
            let key_pair = KeyPair::generate()
                .map_err(|e| io::Error::other(format!("failed to generate TLS key: {e}")))?;

            // Compute the SPKI fingerprint directly from the key pair we just
            // generated — this is the canonical SubjectPublicKeyInfo DER that
            // will end up in the certificate, so there's no need to re-parse
            // the signed certificate afterward.
            let spki_der = key_pair.subject_public_key_info();
            let spki_sha256: [u8; 32] = Sha256::digest(&spki_der).into();

            // `CertificateParams::new` defaults `not_before`/`not_after` to a
            // very long validity window (1975-01-01..4096-01-01) — well past
            // the 10-year target for a self-signed, pin-verified dev cert
            // that outlives any realistic gateway uptime. Left as-is rather
            // than pulling in the `time` crate directly just to narrow it.
            let mut params = CertificateParams::new(local_san_names())
                .map_err(|e| io::Error::other(format!("failed to build cert params: {e}")))?;
            params
                .distinguished_name
                .push(DnType::CommonName, "ThinClaw Gateway");
            for ip in local_san_ips() {
                params.subject_alt_names.push(SanType::IpAddress(ip));
            }

            let cert = params
                .self_signed(&key_pair)
                .map_err(|e| io::Error::other(format!("failed to self-sign cert: {e}")))?;

            std::fs::write(cert_pem_path, cert.pem())?;
            write_private_pem(key_pem_path, &key_pair.serialize_pem())?;
            std::fs::write(fingerprint_path, hex::encode(spki_sha256))?;

            tracing::info!(
                cert_path = %cert_pem_path.display(),
                "Generated self-signed gateway TLS certificate"
            );

            Ok(Self {
                cert_pem_path: cert_pem_path.to_path_buf(),
                key_pem_path: key_pem_path.to_path_buf(),
                spki_sha256,
            })
        }

        /// Load this material into an axum-server rustls config.
        pub async fn rustls_config(&self) -> io::Result<RustlsConfig> {
            RustlsConfig::from_pem_file(&self.cert_pem_path, &self.key_pem_path).await
        }
    }

    fn hex_to_32_bytes(hex_str: &str) -> Option<[u8; 32]> {
        let bytes = hex::decode(hex_str).ok()?;
        bytes.try_into().ok()
    }

    /// Write PEM key material with owner-only permissions.
    ///
    /// On Unix the file is *created* with `0600` (via `OpenOptions::mode`)
    /// rather than written then chmod'd — a write-then-chmod sequence leaves a
    /// window where the private key is readable under the process umask on a
    /// multi-user host. `set_permissions` afterwards covers the pre-existing-
    /// file case, where `mode` does not apply. On Windows the `0600` bit has
    /// no direct equivalent; the key inherits the (user-scoped) `~/.thinclaw`
    /// directory ACL, so a plain write is used.
    #[cfg(unix)]
    fn write_private_pem(path: &Path, contents: &str) -> io::Result<()> {
        use std::io::Write as _;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(contents.as_bytes())?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        Ok(())
    }

    #[cfg(not(unix))]
    fn write_private_pem(path: &Path, contents: &str) -> io::Result<()> {
        std::fs::write(path, contents)
    }

    /// Non-loopback local IPs to embed as SAN entries.
    fn local_san_ips() -> Vec<IpAddr> {
        let Ok(interfaces) = if_addrs::get_if_addrs() else {
            return Vec::new();
        };
        interfaces
            .into_iter()
            .filter(|iface| !iface.is_loopback())
            .map(|iface| iface.addr.ip())
            .collect()
    }

    /// SAN DNS names: the machine's `.local` hostname plus any
    /// `GATEWAY_ADVERTISED_URLS` host segments.
    fn local_san_names() -> Vec<String> {
        let mut names = Vec::new();
        if let Some(local_host) = local_hostname_dot_local() {
            names.push(local_host);
        }
        for url in advertised_urls_env_override().unwrap_or_default() {
            if let Some(host) = host_from_url(&url) {
                names.push(host);
            }
        }
        if names.is_empty() {
            // rcgen requires at least one SAN; fall back to localhost so
            // generation never fails on a host with no discoverable name.
            names.push("localhost".to_string());
        }
        names.retain(|name| Ia5String::try_from(name.as_str()).is_ok());
        names.sort();
        names.dedup();
        names
    }

    fn local_hostname_dot_local() -> Option<String> {
        let raw =
            command_output_trimmed("hostname", &[]).or_else(|| std::env::var("HOSTNAME").ok())?;
        let short = raw.split('.').next().unwrap_or(&raw);
        if short.is_empty() {
            return None;
        }
        Some(format!("{short}.local"))
    }

    fn command_output_trimmed(program: &str, args: &[&str]) -> Option<String> {
        let output = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if text.is_empty() { None } else { Some(text) }
    }

    fn host_from_url(url: &str) -> Option<String> {
        let without_scheme = match url.split_once("://") {
            Some((_, rest)) => rest,
            None => url,
        };
        let host_port = without_scheme.split('/').next()?;
        let host = host_port.rsplit_once(':').map_or(host_port, |(h, _)| h);
        let host = host.trim();
        if host.is_empty() {
            None
        } else {
            Some(host.to_string())
        }
    }

    fn advertised_urls_env_override() -> Option<Vec<String>> {
        let raw = std::env::var(ADVERTISED_URLS_ENV).ok()?;
        let urls: Vec<String> = raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        if urls.is_empty() { None } else { Some(urls) }
    }

    /// Classify an IP for advertised-URL ordering: Tailscale (100.64.0.0/10)
    /// first, then private LAN ranges, then everything else.
    fn ip_priority(ip: &IpAddr) -> u8 {
        match ip {
            IpAddr::V4(v4) => {
                let octets = v4.octets();
                if octets[0] == 100 && (64..=127).contains(&octets[1]) {
                    0 // Tailscale CGNAT range 100.64.0.0/10
                } else if v4.is_private() {
                    1
                } else {
                    2
                }
            }
            IpAddr::V6(_) => 3,
        }
    }

    /// Build the `https://` URLs to advertise for pairing QR payloads and
    /// operator-facing "connect from" hints, honoring `GATEWAY_ADVERTISED_URLS`
    /// verbatim when set.
    pub fn advertised_urls(port: u16) -> Vec<String> {
        if let Some(urls) = advertised_urls_env_override() {
            return urls;
        }

        let mut ips = local_san_ips();
        ips.sort_by_key(ip_priority);
        ips.into_iter()
            .map(|ip| match ip {
                IpAddr::V4(_) => format!("https://{ip}:{port}"),
                IpAddr::V6(ip) => format!("https://[{ip}]:{port}"),
            })
            .collect()
    }

    /// Resolve the TLS listener port: `GATEWAY_TLS_PORT` env override, else
    /// [`DEFAULT_TLS_PORT`].
    pub fn tls_port() -> u16 {
        std::env::var(GATEWAY_TLS_PORT_ENV)
            .ok()
            .and_then(|v| v.trim().parse::<u16>().ok())
            .unwrap_or(DEFAULT_TLS_PORT)
    }

    /// Whether `<base_dir>/devices.json` exists and has at least one paired
    /// device — the condition `auto` mode uses to decide whether to start
    /// the TLS listener at boot (vs. waiting for `ensure_started` on first
    /// pairing). Reads the file directly rather than depending on the
    /// `thinclaw-gateway` devices store, since this module only needs a
    /// cheap non-empty check, not the full device registry.
    pub fn has_paired_devices(base_dir: &Path) -> bool {
        let path = base_dir.join("devices.json");
        let Ok(content) = std::fs::read_to_string(path) else {
            return false;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
            return false;
        };
        value
            .get("devices")
            .and_then(|devices| devices.as_array())
            .is_some_and(|devices| !devices.is_empty())
    }

    /// A running TLS listener handle, used to shut the listener down.
    #[derive(Clone)]
    pub struct TlsListenerHandle {
        handle: Handle<SocketAddr>,
        pub addr: SocketAddr,
        pub fingerprint_prefix: String,
    }

    impl TlsListenerHandle {
        /// Gracefully shut down the TLS listener.
        pub fn shutdown(&self) {
            self.handle
                .graceful_shutdown(Some(std::time::Duration::from_secs(5)));
        }
    }

    /// Spawn the rustls listener on `port`, serving `router` (the same router
    /// the plain-HTTP listener serves). Returns a handle once the listener is
    /// bound; the serve future itself runs on a background task.
    pub async fn spawn_tls_listener(
        router: Router,
        port: u16,
        material: &TlsMaterial,
    ) -> io::Result<TlsListenerHandle> {
        let config = material.rustls_config().await?;
        let addr: SocketAddr = ([0, 0, 0, 0], port).into();
        let handle = Handle::new();

        // Bind eagerly so a port conflict (or any bind failure) surfaces as
        // an `Err` to the caller instead of a background log line under a
        // registry stuck in `Started` with a dead listener.
        let std_listener = std::net::TcpListener::bind(addr)?;
        std_listener.set_nonblocking(true)?;

        let fingerprint_prefix = material.fingerprint_prefix();
        tracing::info!(
            %addr,
            fingerprint_prefix = %fingerprint_prefix,
            "Starting gateway TLS listener"
        );

        let serve_handle = handle.clone();
        let server = axum_server::from_tcp_rustls(std_listener, config)?;
        tokio::spawn(async move {
            let result = server
                .handle(serve_handle)
                .serve(router.into_make_service_with_connect_info::<SocketAddr>())
                .await;
            if let Err(e) = result {
                tracing::error!("Gateway TLS listener error: {}", e);
            }
        });

        Ok(TlsListenerHandle {
            handle,
            addr,
            fingerprint_prefix,
        })
    }

    /// Process-wide registry for the lazy-start (`auto` mode) case: the
    /// gateway registers its router once at boot via [`register_router`],
    /// and the pairing handler (or anything else) calls [`ensure_started`]
    /// on first use without needing a reference threaded through
    /// `GatewayState`. A module-level registry was chosen over a
    /// `GatewayState` field because `GatewayState` is constructed as a
    /// struct literal in ~7 places (production + test helpers); a registry
    /// keeps this feature self-contained to `tls.rs`.
    mod registry {
        use std::sync::OnceLock;

        use axum::Router;
        use tokio::sync::Mutex;

        use super::{TlsListenerHandle, TlsMaterial, spawn_tls_listener, tls_port};

        struct Registered {
            router: Router,
            base_dir: std::path::PathBuf,
        }

        enum RegistryState {
            /// Router registered but the TLS listener has not been started
            /// yet (auto mode, no paired devices at boot).
            Pending(Registered),
            /// Listener is running.
            Started(TlsListenerHandle),
            /// TLS is disabled for this process (`GATEWAY_TLS=off`) or was
            /// never registered.
            Inactive,
        }

        fn cell() -> &'static Mutex<RegistryState> {
            static CELL: OnceLock<Mutex<RegistryState>> = OnceLock::new();
            CELL.get_or_init(|| Mutex::new(RegistryState::Inactive))
        }

        /// Register the router to serve over TLS once started. Called once
        /// at gateway boot with the same `Router<()>` the plain-HTTP
        /// listener serves. Overwrites any previous registration (relevant
        /// only across gateway restarts within the same process, e.g. tests).
        pub async fn register_router(router: Router, base_dir: std::path::PathBuf) {
            let mut guard = cell().lock().await;
            if matches!(*guard, RegistryState::Started(_)) {
                // Already running (e.g. `on` mode already started it);
                // nothing to do — don't clobber a live handle.
                return;
            }
            *guard = RegistryState::Pending(Registered { router, base_dir });
        }

        /// Start the TLS listener now if a router is registered and it
        /// isn't already running. Safe to call repeatedly (idempotent) —
        /// this is the hook `auto` mode uses to lazily start on first
        /// pairing. Returns the listener's fingerprint prefix once running.
        pub async fn ensure_started() -> std::io::Result<Option<String>> {
            let mut guard = cell().lock().await;
            // Clone out of `Pending` instead of consuming it, so a transient
            // failure (unwritable ~/.thinclaw/tls, port conflict) leaves the
            // registration intact and the next `ensure_started` call can
            // retry — never permanently disable lazy-start on first failure.
            let (router, base_dir) = match &*guard {
                RegistryState::Started(handle) => {
                    return Ok(Some(handle.fingerprint_prefix.clone()));
                }
                RegistryState::Inactive => return Ok(None),
                RegistryState::Pending(registered) => {
                    (registered.router.clone(), registered.base_dir.clone())
                }
            };
            let material = TlsMaterial::load_or_generate(&base_dir)?;
            let handle = spawn_tls_listener(router, tls_port(), &material).await?;
            let fingerprint_prefix = handle.fingerprint_prefix.clone();
            *guard = RegistryState::Started(handle);
            Ok(Some(fingerprint_prefix))
        }

        /// Mark TLS as inactive for this process (used when `GATEWAY_TLS=off`
        /// so `ensure_started` is a guaranteed no-op even if a router was
        /// registered earlier in the process lifetime, e.g. re-tested state).
        pub async fn mark_inactive() {
            let mut guard = cell().lock().await;
            if !matches!(*guard, RegistryState::Started(_)) {
                *guard = RegistryState::Inactive;
            }
        }
    }

    pub use registry::{ensure_started, mark_inactive, register_router};

    #[cfg(test)]
    mod tests {
        use super::*;
        use tempfile::TempDir;

        #[test]
        fn generates_material_in_tempdir() {
            let dir = TempDir::new().unwrap();
            let material = TlsMaterial::load_or_generate(dir.path()).unwrap();
            assert!(material.cert_pem_path.exists());
            assert!(material.key_pem_path.exists());
            assert_eq!(material.spki_sha256.len(), 32);
        }

        #[test]
        fn reuses_existing_material_on_second_call() {
            let dir = TempDir::new().unwrap();
            let first = TlsMaterial::load_or_generate(dir.path()).unwrap();
            let second = TlsMaterial::load_or_generate(dir.path()).unwrap();
            assert_eq!(first.spki_sha256, second.spki_sha256);
            assert_eq!(
                first.fingerprint_base64url(),
                second.fingerprint_base64url()
            );
            // Cert/key bytes on disk are untouched by the second call.
            let cert_bytes_1 = std::fs::read(&first.cert_pem_path).unwrap();
            let cert_bytes_2 = std::fs::read(&second.cert_pem_path).unwrap();
            assert_eq!(cert_bytes_1, cert_bytes_2);
        }

        #[test]
        fn fingerprint_is_stable_and_urlsafe() {
            let dir = TempDir::new().unwrap();
            let material = TlsMaterial::load_or_generate(dir.path()).unwrap();
            let fp = material.fingerprint_base64url();
            assert!(!fp.is_empty());
            assert!(!fp.contains('+'));
            assert!(!fp.contains('/'));
            assert!(!fp.contains('='));
            assert_eq!(material.fingerprint_prefix().len(), 12.min(fp.len()));
        }

        #[cfg(unix)]
        #[test]
        fn key_file_has_owner_only_permissions() {
            use std::os::unix::fs::PermissionsExt;
            let dir = TempDir::new().unwrap();
            let material = TlsMaterial::load_or_generate(dir.path()).unwrap();
            let perms = std::fs::metadata(&material.key_pem_path)
                .unwrap()
                .permissions();
            assert_eq!(perms.mode() & 0o777, 0o600);
        }

        #[test]
        fn tls_policy_defaults_to_auto() {
            // SAFETY (test-only): scoped to this process; no other test in this
            // module reads GATEWAY_TLS concurrently in a way that would race.
            unsafe {
                std::env::remove_var(GATEWAY_TLS_ENV);
            }
            assert_eq!(TlsPolicy::from_env(), TlsPolicy::Auto);
        }

        #[test]
        fn host_from_url_strips_scheme_and_port() {
            assert_eq!(
                host_from_url("https://100.64.1.2:3443"),
                Some("100.64.1.2".to_string())
            );
            assert_eq!(
                host_from_url("https://host.local:3443/"),
                Some("host.local".to_string())
            );
        }

        #[test]
        fn ip_priority_orders_tailscale_before_private_before_public() {
            let tailscale: IpAddr = "100.64.1.2".parse().unwrap();
            let private: IpAddr = "192.168.1.5".parse().unwrap();
            let public: IpAddr = "8.8.8.8".parse().unwrap();
            assert!(ip_priority(&tailscale) < ip_priority(&private));
            assert!(ip_priority(&private) < ip_priority(&public));
        }
    }
}

#[cfg(feature = "gateway-tls")]
pub use imp::{
    TlsListenerHandle, TlsMaterial, TlsPolicy, advertised_urls, ensure_started, has_paired_devices,
    mark_inactive, register_router, spawn_tls_listener, tls_port,
};

/// Stub surface when `gateway-tls` is disabled, so callers can reference
/// `tls::TlsPolicy` etc. behind a single `#[cfg(feature = "gateway-tls")]` at
/// the call site without needing a second cfg for the type itself.
#[cfg(not(feature = "gateway-tls"))]
mod stub {
    /// TLS policy is always effectively "off" when the feature is disabled.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum TlsPolicy {
        Off,
        Auto,
        On,
    }

    impl TlsPolicy {
        pub fn from_env() -> Self {
            TlsPolicy::Off
        }
    }

    /// No-op stand-in for [`super::imp::register_router`].
    pub async fn register_router(_router: axum::Router, _base_dir: std::path::PathBuf) {}

    /// No-op stand-in for [`super::imp::ensure_started`]; always inactive.
    pub async fn ensure_started() -> std::io::Result<Option<String>> {
        Ok(None)
    }

    /// No-op stand-in for [`super::imp::mark_inactive`].
    pub async fn mark_inactive() {}

    /// No-op stand-in for [`super::imp::has_paired_devices`]; always false.
    pub fn has_paired_devices(_base_dir: &std::path::Path) -> bool {
        false
    }
}

#[cfg(not(feature = "gateway-tls"))]
pub use stub::{TlsPolicy, ensure_started, has_paired_devices, mark_inactive, register_router};
