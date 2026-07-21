//! HTTP proxy server for sandboxed network access.
//!
//! This proxy runs on the host and handles all network requests from containers.
//! It validates every request against the allowlist. Credential injection is
//! **HTTP-only**: for plaintext `http://` forwards (`forward_request`) the proxy
//! resolves the secret through the configured [`CredentialResolver`] and adds it
//! to the outbound request. HTTPS traffic is tunneled opaquely via `CONNECT`
//! (`handle_connect`) and cannot be inspected or modified without MITM, so HTTPS
//! credentials are *not* injected here — containers that need authenticated HTTPS
//! fetch them out-of-band from the orchestrator's `/worker/{id}/credentials`
//! endpoint. The production [`StoreCredentialResolver`] reads secrets from the
//! encrypted [`SecretsStore`]; [`EnvCredentialResolver`] is the env fallback.
//!
//! ```text
//! Container ──► http_proxy=host.docker.internal:PORT ──► This Proxy ──► Internet
//!                                                             │
//!                                                             ├─► Validate domain
//!                                                             ├─► Inject credentials (HTTP forward only)
//!                                                             └─► Log requests
//! ```

use std::collections::HashSet;
use std::convert::Infallible;
use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use bytes::{Bytes, BytesMut};
use http_body_util::{BodyExt, Empty, Full, Limited, combinators::BoxBody};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use subtle::ConstantTimeEq;
use thinclaw_tools_core::is_public_outbound_ip;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, RwLock, Semaphore};
use tokio::task::{JoinHandle, JoinSet};

use crate::sandbox::error::{Result, SandboxError};
use crate::sandbox::proxy::policy::{NetworkDecision, NetworkPolicyDecider, NetworkRequest};
use crate::secrets::{CredentialLocation, SecretAccessContext, SecretsStore};

const PROXY_TASK_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);
const PROXY_SERVER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);
const PROXY_POLICY_TIMEOUT: Duration = Duration::from_secs(5);
const PROXY_CREDENTIAL_TIMEOUT: Duration = Duration::from_secs(5);
const PROXY_DNS_TIMEOUT: Duration = Duration::from_secs(10);
const PROXY_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const PROXY_TLS_CLIENT_HELLO_TIMEOUT: Duration = Duration::from_secs(10);
const PROXY_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const PROXY_HEADER_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_PROXY_TASKS: usize = 256;
const MAX_PROXY_HTTP_REQUESTS: usize = 64;
const MAX_PROXY_TUNNELS: usize = 64;
const MAX_PROXY_REQUEST_BYTES: usize = 16 * 1024 * 1024;
const MAX_PROXY_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
const MAX_TLS_CLIENT_HELLO_BYTES: usize = 64 * 1024;

/// State shared across proxy connections.
struct ProxyState {
    /// Policy decider for network requests.
    decider: Arc<dyn NetworkPolicyDecider>,
    /// Credential resolver (maps secret names to values).
    credential_resolver: Arc<dyn CredentialResolver>,
    /// Shared HTTP client for forwarding requests.
    http_client: Option<reqwest::Client>,
    /// Ephemeral proxy authentication. Binding on all interfaces is required
    /// for Docker bridge access, so every request must prove it came from a
    /// container that received this per-runtime credential.
    #[cfg(any(feature = "docker-sandbox", feature = "browser"))]
    proxy_token: String,
    expected_proxy_authorization: String,
    /// Request counter for logging.
    request_count: std::sync::atomic::AtomicU64,
    /// Whether the proxy is running.
    running: std::sync::atomic::AtomicBool,
    /// Every accepted connection and CONNECT tunnel is owned here so proxy
    /// shutdown can close the entire task tree rather than just the listener.
    tasks: std::sync::Mutex<JoinSet<()>>,
    accepting_tasks: std::sync::atomic::AtomicBool,
    request_slots: Arc<Semaphore>,
    tunnel_slots: Arc<Semaphore>,
}

fn spawn_proxy_task(
    state: &Arc<ProxyState>,
    task: impl Future<Output = ()> + Send + 'static,
) -> bool {
    let mut tasks = state
        .tasks
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if !state
        .accepting_tasks
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return false;
    }
    while let Some(result) = tasks.try_join_next() {
        if let Err(error) = result
            && !error.is_cancelled()
        {
            tracing::warn!(%error, "Sandbox proxy task failed");
        }
    }
    if tasks.len() >= MAX_PROXY_TASKS {
        tracing::warn!(limit = MAX_PROXY_TASKS, "Sandbox proxy task limit reached");
        return false;
    }
    tasks.spawn(task);
    true
}

async fn drain_proxy_tasks(state: &ProxyState) {
    state
        .accepting_tasks
        .store(false, std::sync::atomic::Ordering::Release);
    let mut tasks = {
        let mut guard = state
            .tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::mem::take(&mut *guard)
    };
    let graceful = async {
        while let Some(result) = tasks.join_next().await {
            if let Err(error) = result
                && !error.is_cancelled()
            {
                tracing::warn!(%error, "Sandbox proxy task failed while draining");
            }
        }
    };
    if tokio::time::timeout(PROXY_TASK_DRAIN_TIMEOUT, graceful)
        .await
        .is_err()
    {
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
        tracing::debug!("Aborted remaining sandbox proxy connections");
    }
}

/// Resolves secret names to their values.
#[async_trait::async_trait]
pub trait CredentialResolver: Send + Sync {
    /// Get the value of a secret by name.
    async fn resolve(&self, name: &str) -> Option<String>;
}

/// A credential resolver that uses environment variables.
pub struct EnvCredentialResolver;

#[async_trait::async_trait]
impl CredentialResolver for EnvCredentialResolver {
    async fn resolve(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
}

/// A credential resolver that returns nothing (for testing).
pub struct NoCredentialResolver;

#[async_trait::async_trait]
impl CredentialResolver for NoCredentialResolver {
    async fn resolve(&self, _name: &str) -> Option<String> {
        None
    }
}

/// A credential resolver backed by the encrypted [`SecretsStore`].
///
/// This is the production resolver: it pulls credentials from the AES-256-GCM
/// secrets store via the audited injection path (`get_for_injection`) rather
/// than from the host process environment. Wire it through
/// [`NetworkProxyBuilder::with_credential_resolver`](super::NetworkProxyBuilder::with_credential_resolver)
/// (or [`NetworkProxyBuilder::from_config_with_store`](super::NetworkProxyBuilder::from_config_with_store))
/// when a store and the owning user are available; callers without a store fall
/// back to [`EnvCredentialResolver`].
pub struct StoreCredentialResolver {
    store: Arc<dyn SecretsStore + Send + Sync>,
    user_id: String,
}

impl StoreCredentialResolver {
    /// Create a resolver that reads secrets for `user_id` from `store`.
    pub fn new(store: Arc<dyn SecretsStore + Send + Sync>, user_id: impl Into<String>) -> Self {
        Self {
            store,
            user_id: user_id.into(),
        }
    }
}

#[async_trait::async_trait]
impl CredentialResolver for StoreCredentialResolver {
    async fn resolve(&self, name: &str) -> Option<String> {
        // Use the audited runtime-injection path so the access is recorded and
        // the value is decrypted only at the point of use, mirroring the WASM
        // credential injector (`thinclaw-tools` wasm::credential_injector).
        let context =
            SecretAccessContext::new("sandbox.proxy.credential_resolver", "http_proxy_injection");
        match self
            .store
            .get_for_injection(&self.user_id, name, context)
            .await
        {
            Ok(secret) => Some(secret.expose().to_string()),
            Err(err) => {
                // Never log the secret value; `SecretError`'s Display is safe and
                // names only the secret, not its plaintext.
                tracing::debug!("Proxy: secret {} unavailable from store: {}", name, err);
                None
            }
        }
    }
}

/// HTTP proxy server.
pub struct HttpProxy {
    state: Arc<ProxyState>,
    addr: RwLock<Option<SocketAddr>>,
    shutdown_tx: RwLock<Option<tokio::sync::oneshot::Sender<()>>>,
    server_handle: Mutex<Option<JoinHandle<()>>>,
    lifecycle_lock: Mutex<()>,
}

impl HttpProxy {
    /// Create a new HTTP proxy.
    pub fn new(
        decider: Arc<dyn NetworkPolicyDecider>,
        credential_resolver: Arc<dyn CredentialResolver>,
    ) -> Self {
        let proxy_token = generate_proxy_token();
        let expected_proxy_authorization = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("thinclaw:{proxy_token}"))
        );
        let http_client = match reqwest::Client::builder()
            .connect_timeout(PROXY_CONNECT_TIMEOUT)
            .timeout(PROXY_REQUEST_TIMEOUT)
            // Redirects must return to the container and traverse policy
            // validation again; following them here bypasses the allowlist.
            .redirect(reqwest::redirect::Policy::none())
            .build()
        {
            Ok(client) => Some(client),
            Err(error) => {
                tracing::error!(%error, "Failed to initialize sandbox proxy HTTP client");
                None
            }
        };
        Self {
            state: Arc::new(ProxyState {
                decider,
                credential_resolver,
                http_client,
                #[cfg(any(feature = "docker-sandbox", feature = "browser"))]
                proxy_token,
                expected_proxy_authorization,
                request_count: std::sync::atomic::AtomicU64::new(0),
                running: std::sync::atomic::AtomicBool::new(false),
                tasks: std::sync::Mutex::new(JoinSet::new()),
                accepting_tasks: std::sync::atomic::AtomicBool::new(false),
                request_slots: Arc::new(Semaphore::new(MAX_PROXY_HTTP_REQUESTS)),
                tunnel_slots: Arc::new(Semaphore::new(MAX_PROXY_TUNNELS)),
            }),
            addr: RwLock::new(None),
            shutdown_tx: RwLock::new(None),
            server_handle: Mutex::new(None),
            lifecycle_lock: Mutex::new(()),
        }
    }

    /// Start the proxy server on the given port (0 for auto-assign).
    pub async fn start(&self, port: u16) -> Result<SocketAddr> {
        self.start_on(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port).await
    }

    /// Start a host-only proxy for local clients. Docker callers must use
    /// [`Self::start`] because bridge traffic cannot reach loopback.
    pub async fn start_loopback(&self, port: u16) -> Result<SocketAddr> {
        self.start_on(IpAddr::V4(Ipv4Addr::LOCALHOST), port).await
    }

    async fn start_on(&self, bind_ip: IpAddr, port: u16) -> Result<SocketAddr> {
        let _lifecycle_guard = self.lifecycle_lock.lock().await;
        if self.is_running()
            && let Some(addr) = *self.addr.read().await
        {
            return Ok(addr);
        }
        if let Some(previous) = self.server_handle.lock().await.take() {
            previous.abort();
            let _ = previous.await;
            drain_proxy_tasks(self.state.as_ref()).await;
        }
        // Docker bridge traffic cannot reach a loopback-only listener. The
        // ephemeral Basic credential above prevents this all-interface bind
        // from becoming an unauthenticated LAN proxy.
        let listener = TcpListener::bind(SocketAddr::new(bind_ip, port))
            .await
            .map_err(|e| SandboxError::ProxyError {
                reason: format!("failed to bind: {}", e),
            })?;

        let addr = listener
            .local_addr()
            .map_err(|e| SandboxError::ProxyError {
                reason: format!("failed to get local addr: {}", e),
            })?;

        *self.addr.write().await = Some(addr);

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        *self.shutdown_tx.write().await = Some(shutdown_tx);

        self.state
            .running
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.state
            .accepting_tasks
            .store(true, std::sync::atomic::Ordering::Release);

        let state = self.state.clone();

        let handle = tokio::spawn(async move {
            tracing::info!("Sandbox proxy started on {}", addr);

            loop {
                tokio::select! {
                    accept_result = listener.accept() => {
                        match accept_result {
                            Ok((stream, _)) => {
                                let io = TokioIo::new(stream);
                                let connection_state = state.clone();

                                let connection = async move {
                                    let service = service_fn(move |req| {
                                        let state = connection_state.clone();
                                        async move { handle_request(req, state).await }
                                    });

                                    let mut builder = http1::Builder::new();
                                    builder
                                        .timer(TokioTimer::new())
                                        .header_read_timeout(PROXY_HEADER_TIMEOUT)
                                        .preserve_header_case(true)
                                        .title_case_headers(true);
                                    if let Err(e) = builder
                                        .serve_connection(io, service)
                                        .with_upgrades()
                                        .await
                                    {
                                        tracing::debug!("Proxy connection error: {}", e);
                                    }
                                };
                                if !spawn_proxy_task(&state, connection) {
                                    tracing::debug!("Proxy rejected connection during shutdown");
                                }
                            }
                            Err(e) => {
                                tracing::error!("Proxy accept error: {}", e);
                                break;
                            }
                        }
                    }
                    _ = &mut shutdown_rx => {
                        tracing::info!("Sandbox proxy shutting down");
                        break;
                    }
                }
            }

            drain_proxy_tasks(state.as_ref()).await;
            state
                .running
                .store(false, std::sync::atomic::Ordering::SeqCst);
        });
        *self.server_handle.lock().await = Some(handle);

        Ok(addr)
    }

    /// Stop the proxy server.
    pub async fn stop(&self) {
        let _lifecycle_guard = self.lifecycle_lock.lock().await;
        if let Some(tx) = self.shutdown_tx.write().await.take() {
            let _ = tx.send(());
        }
        if let Some(mut handle) = self.server_handle.lock().await.take() {
            match tokio::time::timeout(PROXY_SERVER_SHUTDOWN_TIMEOUT, &mut handle).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::warn!(%error, "Sandbox proxy server task failed during shutdown");
                    drain_proxy_tasks(self.state.as_ref()).await;
                }
                Err(_) => {
                    tracing::warn!("Sandbox proxy server did not stop before timeout; aborting");
                    handle.abort();
                    let _ = handle.await;
                    drain_proxy_tasks(self.state.as_ref()).await;
                }
            }
        }
        self.state
            .running
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    /// Get the address the proxy is listening on.
    pub async fn addr(&self) -> Option<SocketAddr> {
        *self.addr.read().await
    }

    /// Check if the proxy is running.
    pub fn is_running(&self) -> bool {
        self.state.running.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Get the number of requests handled.
    pub fn request_count(&self) -> u64 {
        self.state
            .request_count
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Ephemeral token embedded only in sandbox proxy URLs.
    #[cfg(any(feature = "docker-sandbox", feature = "browser"))]
    pub(crate) fn proxy_token(&self) -> String {
        self.state.proxy_token.clone()
    }
}

fn generate_proxy_token() -> String {
    use rand::RngExt as _;

    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    bytes
        .iter()
        .fold(String::with_capacity(64), |mut token, byte| {
            use std::fmt::Write as _;
            let _ = write!(token, "{byte:02x}");
            token
        })
}

fn take_bytes<'a>(
    bytes: &'a [u8],
    cursor: &mut usize,
    length: usize,
) -> std::result::Result<&'a [u8], String> {
    let end = cursor
        .checked_add(length)
        .ok_or_else(|| "TLS ClientHello length overflow".to_string())?;
    let value = bytes
        .get(*cursor..end)
        .ok_or_else(|| "truncated TLS ClientHello".to_string())?;
    *cursor = end;
    Ok(value)
}

fn take_u8(bytes: &[u8], cursor: &mut usize) -> std::result::Result<u8, String> {
    Ok(take_bytes(bytes, cursor, 1)?[0])
}

fn take_u16(bytes: &[u8], cursor: &mut usize) -> std::result::Result<usize, String> {
    let value = take_bytes(bytes, cursor, 2)?;
    Ok(u16::from_be_bytes([value[0], value[1]]) as usize)
}

/// Parse the SNI value from a complete TLS ClientHello handshake message.
/// CONNECT is otherwise a generic byte tunnel: without this check, a client
/// could CONNECT to an allowlisted CDN hostname and send TLS for a disallowed
/// virtual host that happens to share the same IP address.
fn parse_client_hello_sni(handshake: &[u8]) -> std::result::Result<String, String> {
    if handshake.first().copied() != Some(1) {
        return Err("CONNECT tunnel did not begin with a TLS ClientHello".to_string());
    }
    let length_bytes = handshake
        .get(1..4)
        .ok_or_else(|| "truncated TLS handshake header".to_string())?;
    let hello_length = ((length_bytes[0] as usize) << 16)
        | ((length_bytes[1] as usize) << 8)
        | length_bytes[2] as usize;
    let hello = handshake
        .get(4..4 + hello_length)
        .ok_or_else(|| "truncated TLS ClientHello".to_string())?;
    let mut cursor = 0;
    take_bytes(hello, &mut cursor, 2 + 32)?; // legacy_version + random
    let session_id_length = take_u8(hello, &mut cursor)? as usize;
    take_bytes(hello, &mut cursor, session_id_length)?;
    let cipher_suites_length = take_u16(hello, &mut cursor)?;
    if cipher_suites_length == 0 || cipher_suites_length % 2 != 0 {
        return Err("invalid TLS ClientHello cipher-suite list".to_string());
    }
    take_bytes(hello, &mut cursor, cipher_suites_length)?;
    let compression_methods_length = take_u8(hello, &mut cursor)? as usize;
    if compression_methods_length == 0 {
        return Err("invalid TLS ClientHello compression-method list".to_string());
    }
    take_bytes(hello, &mut cursor, compression_methods_length)?;
    let extensions_length = take_u16(hello, &mut cursor)?;
    let extensions = take_bytes(hello, &mut cursor, extensions_length)?;
    if cursor != hello.len() {
        return Err("invalid trailing bytes in TLS ClientHello".to_string());
    }

    let mut extension_cursor = 0;
    while extension_cursor < extensions.len() {
        let extension_type = take_u16(extensions, &mut extension_cursor)?;
        let extension_length = take_u16(extensions, &mut extension_cursor)?;
        let extension = take_bytes(extensions, &mut extension_cursor, extension_length)?;
        if extension_type != 0 {
            continue;
        }

        let mut name_cursor = 0;
        let name_list_length = take_u16(extension, &mut name_cursor)?;
        let names = take_bytes(extension, &mut name_cursor, name_list_length)?;
        if name_cursor != extension.len() {
            return Err("invalid TLS server-name extension".to_string());
        }
        let mut names_cursor = 0;
        while names_cursor < names.len() {
            let name_type = take_u8(names, &mut names_cursor)?;
            let name_length = take_u16(names, &mut names_cursor)?;
            let name = take_bytes(names, &mut names_cursor, name_length)?;
            if name_type == 0 {
                let name = std::str::from_utf8(name)
                    .map_err(|_| "TLS server name is not UTF-8".to_string())?;
                if name.is_empty()
                    || name.len() > 253
                    || !name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
                {
                    return Err("TLS server name is invalid".to_string());
                }
                return Ok(name.trim_end_matches('.').to_ascii_lowercase());
            }
        }
        return Err("TLS ClientHello has no DNS server name".to_string());
    }

    Err("TLS ClientHello has no SNI extension".to_string())
}

async fn read_tls_client_hello<R>(stream: &mut R) -> std::result::Result<(Vec<u8>, String), String>
where
    R: AsyncRead + Unpin,
{
    let mut wire_bytes = Vec::new();
    let mut handshake = Vec::new();
    loop {
        let mut header = [0u8; 5];
        stream
            .read_exact(&mut header)
            .await
            .map_err(|error| format!("failed to read TLS record header: {error}"))?;
        if header[0] != 22 {
            return Err("CONNECT tunnel only permits TLS handshakes".to_string());
        }
        let record_length = u16::from_be_bytes([header[3], header[4]]) as usize;
        if record_length == 0
            || wire_bytes
                .len()
                .saturating_add(5)
                .saturating_add(record_length)
                > MAX_TLS_CLIENT_HELLO_BYTES
        {
            return Err("TLS ClientHello exceeds proxy limit".to_string());
        }
        let mut record = vec![0u8; record_length];
        stream
            .read_exact(&mut record)
            .await
            .map_err(|error| format!("failed to read TLS record: {error}"))?;
        wire_bytes.extend_from_slice(&header);
        wire_bytes.extend_from_slice(&record);
        handshake.extend_from_slice(&record);

        if handshake.len() >= 4 {
            let hello_length = ((handshake[1] as usize) << 16)
                | ((handshake[2] as usize) << 8)
                | handshake[3] as usize;
            let full_length = 4usize
                .checked_add(hello_length)
                .ok_or_else(|| "TLS ClientHello length overflow".to_string())?;
            if full_length > MAX_TLS_CLIENT_HELLO_BYTES {
                return Err("TLS ClientHello exceeds proxy limit".to_string());
            }
            if handshake.len() >= full_length {
                let server_name = parse_client_hello_sni(&handshake[..full_length])?;
                return Ok((wire_bytes, server_name));
            }
        }
    }
}

fn canonical_connect_host(host: &str) -> String {
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

fn proxy_request_is_authorized(req: &Request<hyper::body::Incoming>, state: &ProxyState) -> bool {
    req.headers()
        .get("proxy-authorization")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|supplied| {
            supplied
                .as_bytes()
                .ct_eq(state.expected_proxy_authorization.as_bytes())
                .into()
        })
}

/// Handle an incoming proxy request.
async fn handle_request(
    req: Request<hyper::body::Incoming>,
    state: Arc<ProxyState>,
) -> std::result::Result<Response<BoxBody<Bytes, Infallible>>, Infallible> {
    state
        .request_count
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    if !proxy_request_is_authorized(&req, state.as_ref()) {
        tracing::warn!("Rejected unauthenticated sandbox proxy request");
        return Ok(proxy_auth_required_response());
    }

    // Handle CONNECT method for HTTPS tunneling
    if req.method() == Method::CONNECT {
        return Ok(handle_connect(req, state).await);
    }

    let _request_permit = match state.request_slots.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            return Ok(error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "Sandbox proxy request capacity is exhausted".to_string(),
            ));
        }
    };

    // For HTTP requests, validate and forward
    let uri = req.uri().to_string();
    let method = req.method().to_string();

    let network_req = match NetworkRequest::from_url(&method, &uri) {
        Some(r) => r,
        None => {
            // The raw URI can contain query-string credentials. Never include
            // it in logs when parsing has failed.
            tracing::warn!(method, "Proxy rejected an invalid target URL");
            return Ok(error_response(
                StatusCode::BAD_REQUEST,
                "Invalid URL".to_string(),
            ));
        }
    };

    // Make policy decision
    let decision = match tokio::time::timeout(
        PROXY_POLICY_TIMEOUT,
        state.decider.decide(&network_req),
    )
    .await
    {
        Ok(decision) => decision,
        Err(_) => {
            tracing::warn!(
                %method,
                host = %network_req.host,
                path = %network_req.path,
                "Sandbox proxy policy decision timed out"
            );
            return Ok(error_response(
                StatusCode::GATEWAY_TIMEOUT,
                "Network policy decision timed out".to_string(),
            ));
        }
    };

    match decision {
        NetworkDecision::Deny { reason } => {
            tracing::info!(
                %method,
                host = %network_req.host,
                path = %network_req.path,
                %reason,
                "Sandbox proxy blocked request"
            );
            Ok(error_response(StatusCode::FORBIDDEN, reason))
        }
        NetworkDecision::Allow | NetworkDecision::AllowWithCredentials { .. } => {
            // Forward the request
            forward_request(req, decision, state).await
        }
    }
}

/// Handle CONNECT method for HTTPS tunneling.
///
/// Establishes a bidirectional TCP tunnel between the client and the target host.
/// Returns 200 OK to signal the client to begin TLS over the upgraded connection.
///
/// NOTE: Credential injection is not possible through CONNECT tunnels since the proxy
/// cannot inspect or modify TLS-encrypted traffic without MITM. Containers that need
/// authenticated HTTPS should fetch credentials via the orchestrator's
/// `GET /worker/{id}/credentials` endpoint and set them as environment variables.
async fn handle_connect(
    req: Request<hyper::body::Incoming>,
    state: Arc<ProxyState>,
) -> Response<BoxBody<Bytes, Infallible>> {
    let tunnel_permit = match state.tunnel_slots.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "Sandbox proxy tunnel capacity is exhausted".to_string(),
            );
        }
    };
    // Extract host:port from CONNECT target (e.g. "api.github.com:443")
    let authority = match req.uri().authority() {
        Some(a) => a.clone(),
        None => {
            return error_response(StatusCode::BAD_REQUEST, "Missing host".to_string());
        }
    };

    let host = authority.host().to_string();
    let target_port = authority.port_u16().unwrap_or(443);
    if target_port != 443 {
        return error_response(
            StatusCode::FORBIDDEN,
            "CONNECT is restricted to the standard TLS port".to_string(),
        );
    }

    // Check if host is allowed
    let network_req = NetworkRequest {
        method: "CONNECT".to_string(),
        url: format!("https://{}", host),
        host: host.clone(),
        path: "/".to_string(),
    };

    let decision = match tokio::time::timeout(
        PROXY_POLICY_TIMEOUT,
        state.decider.decide(&network_req),
    )
    .await
    {
        Ok(decision) => decision,
        Err(_) => {
            return error_response(
                StatusCode::GATEWAY_TIMEOUT,
                "Network policy decision timed out".to_string(),
            );
        }
    };

    if let NetworkDecision::Deny { reason } = decision {
        tracing::info!("Proxy: blocked CONNECT {} - {}", host, reason);
        return error_response(StatusCode::FORBIDDEN, reason);
    }

    let targets = match resolve_public_addresses(&host, target_port).await {
        Ok(targets) => targets,
        Err(reason) => {
            tracing::warn!(%host, %reason, "Proxy rejected CONNECT target resolution");
            return error_response(StatusCode::FORBIDDEN, reason);
        }
    };

    tracing::debug!("Proxy: allowing CONNECT to {}:{}", host, target_port);

    // Establish the tunnel after the upgrade completes. The task is registered
    // with the proxy lifecycle so `stop()` can abort it immediately instead of
    // leaving a 30-minute detached tunnel behind.
    let target_label = format!("{host}:{target_port}");
    let canonical_target_host = canonical_connect_host(&host);
    let tunnel = async move {
        let _tunnel_permit = tunnel_permit;
        match tokio::time::timeout(PROXY_CONNECT_TIMEOUT, hyper::upgrade::on(req)).await {
            Err(_) => {
                tracing::warn!("Proxy: upgrade timed out for {}", target_label);
            }
            Ok(Ok(upgraded)) => {
                let mut client_stream = TokioIo::new(upgraded);
                let (client_hello, server_name) = match tokio::time::timeout(
                    PROXY_TLS_CLIENT_HELLO_TIMEOUT,
                    read_tls_client_hello(&mut client_stream),
                )
                .await
                {
                    Ok(Ok(client_hello)) => client_hello,
                    Ok(Err(reason)) => {
                        tracing::warn!(%target_label, %reason, "Proxy rejected CONNECT preface");
                        return;
                    }
                    Err(_) => {
                        tracing::warn!(%target_label, "Proxy TLS ClientHello timed out");
                        return;
                    }
                };
                if server_name != canonical_target_host {
                    tracing::warn!(
                        %target_label,
                        %server_name,
                        "Proxy rejected CONNECT/SNI hostname mismatch"
                    );
                    return;
                }
                match connect_first_public_target(&targets).await {
                    Ok(mut server_stream) => {
                        if let Err(error) = server_stream.write_all(&client_hello).await {
                            tracing::debug!(
                                %target_label,
                                %error,
                                "Proxy failed to forward TLS ClientHello"
                            );
                            return;
                        }
                        let tunnel_timeout = std::time::Duration::from_secs(30 * 60);
                        match tokio::time::timeout(
                            tunnel_timeout,
                            tokio::io::copy_bidirectional(&mut client_stream, &mut server_stream),
                        )
                        .await
                        {
                            Ok(Ok(_)) => {}
                            Ok(Err(e)) => {
                                tracing::debug!("Proxy: tunnel to {} closed: {}", target_label, e);
                            }
                            Err(_) => {
                                tracing::info!(
                                    "Proxy: tunnel to {} timed out after 30m, closing",
                                    target_label
                                );
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("Proxy: failed to connect to {}: {}", target_label, e);
                    }
                }
            }
            Ok(Err(e)) => {
                tracing::error!("Proxy: upgrade failed for {}: {}", target_label, e);
            }
        }
    };
    if !spawn_proxy_task(&state, tunnel) {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "Proxy is shutting down".to_string(),
        );
    }

    // Return 200 OK so the client begins the TLS handshake over the upgraded connection
    make_response(StatusCode::OK, empty_body())
}

async fn resolve_public_addresses(
    host: &str,
    port: u16,
) -> std::result::Result<Vec<SocketAddr>, String> {
    let resolved = tokio::time::timeout(PROXY_DNS_TIMEOUT, tokio::net::lookup_host((host, port)))
        .await
        .map_err(|_| format!("DNS lookup for {host} timed out"))?
        .map_err(|error| format!("DNS lookup for {host} failed: {error}"))?
        .collect::<Vec<_>>();
    if resolved.is_empty() {
        return Err(format!("DNS lookup for {host} returned no addresses"));
    }
    if let Some(private) = resolved
        .iter()
        .find(|address| !is_public_outbound_ip(address.ip()))
    {
        return Err(format!(
            "target {host} resolved to non-public address {}",
            private.ip()
        ));
    }

    let mut public = resolved;
    public.sort_by_key(|address| address.is_ipv6());
    public.dedup();
    Ok(public)
}

async fn connect_first_public_target(targets: &[SocketAddr]) -> std::io::Result<TcpStream> {
    let mut last_error = None;
    for target in targets {
        match tokio::time::timeout(PROXY_CONNECT_TIMEOUT, TcpStream::connect(target)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(error)) => last_error = Some(error),
            Err(_) => {
                last_error = Some(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!("connection to {target} timed out"),
                ));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::AddrNotAvailable,
            "no public target address",
        )
    }))
}

/// Forward a request to the target server.
async fn forward_request(
    req: Request<hyper::body::Incoming>,
    decision: NetworkDecision,
    state: Arc<ProxyState>,
) -> std::result::Result<Response<BoxBody<Bytes, Infallible>>, Infallible> {
    let (parts, body) = req.into_parts();
    let method = parts.method;
    let uri = parts.uri;

    let mut target_url = match reqwest::Url::parse(&uri.to_string()) {
        Ok(url) if url.scheme() == "http" => url,
        Ok(_) => {
            return Ok(error_response(
                StatusCode::BAD_REQUEST,
                "HTTPS proxy requests must use CONNECT".to_string(),
            ));
        }
        Err(error) => {
            return Ok(error_response(
                StatusCode::BAD_REQUEST,
                format!("Invalid target URL: {error}"),
            ));
        }
    };
    let Some(target_host) = target_url.host_str().map(str::to_string) else {
        return Ok(error_response(
            StatusCode::BAD_REQUEST,
            "Target URL has no host".to_string(),
        ));
    };
    let target_port = target_url.port_or_known_default().unwrap_or(80);
    if target_port != 80 {
        return Ok(error_response(
            StatusCode::FORBIDDEN,
            "Plain HTTP proxying is restricted to port 80".to_string(),
        ));
    }
    if !target_url.username().is_empty() || target_url.password().is_some() {
        return Ok(error_response(
            StatusCode::BAD_REQUEST,
            "Target URLs may not contain user information".to_string(),
        ));
    }
    let validated_host_header = if target_host.contains(':') {
        format!("[{target_host}]")
    } else {
        target_host.clone()
    };
    let targets = match resolve_public_addresses(&target_host, target_port).await {
        Ok(targets) => targets,
        Err(reason) => {
            tracing::warn!(host = %target_host, %reason, "Proxy rejected HTTP target resolution");
            return Ok(error_response(StatusCode::FORBIDDEN, reason));
        }
    };
    let pinned_ip = targets[0].ip().to_string();
    if target_url.set_host(Some(&pinned_ip)).is_err() {
        return Ok(error_response(
            StatusCode::BAD_REQUEST,
            "Target host could not be pinned to its validated address".to_string(),
        ));
    }

    let Some(http_client) = state.http_client.as_ref() else {
        return Ok(error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "Sandbox proxy HTTP client is unavailable".to_string(),
        ));
    };

    // Build the forwarded request
    let mut builder = http_client.request(
        reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET),
        target_url,
    );

    // RFC 7230 permits `Connection` to nominate additional hop-by-hop
    // headers. Strip those as well as the fixed list. The client-supplied Host
    // header is never trusted: overriding it with the validated URL authority
    // prevents virtual-host allowlist bypass on shared IP/CDN infrastructure.
    let connection_headers = connection_header_names(&parts.headers);
    for (name, value) in &parts.headers {
        if name != hyper::header::HOST
            && !is_hop_by_hop_header(name.as_str())
            && !connection_headers.contains(name.as_str())
            && let Ok(v) = value.to_str()
        {
            builder = builder.header(name.as_str(), v);
        }
    }
    builder = builder.header(reqwest::header::HOST, validated_host_header);
    // Inject credentials if needed
    if let NetworkDecision::AllowWithCredentials {
        secret_name,
        location,
    } = decision
    {
        let credential = match tokio::time::timeout(
            PROXY_CREDENTIAL_TIMEOUT,
            state.credential_resolver.resolve(&secret_name),
        )
        .await
        {
            Ok(credential) => credential,
            Err(_) => {
                tracing::warn!(%secret_name, "Proxy credential resolution timed out");
                return Ok(error_response(
                    StatusCode::GATEWAY_TIMEOUT,
                    "Credential resolution timed out".to_string(),
                ));
            }
        };
        if let Some(credential) = credential {
            builder = match location {
                CredentialLocation::AuthorizationBearer => {
                    builder.header("Authorization", format!("Bearer {}", credential))
                }
                CredentialLocation::AuthorizationBasic { username } => {
                    let encoded = base64::engine::general_purpose::STANDARD
                        .encode(format!("{username}:{credential}"));
                    builder.header("Authorization", format!("Basic {encoded}"))
                }
                CredentialLocation::Header { name, prefix } => {
                    let value = match prefix {
                        Some(p) => format!("{}{}", p, credential),
                        None => credential.clone(),
                    };
                    builder.header(name, value)
                }
                CredentialLocation::QueryParam { name } => builder.query(&[(name, credential)]),
                CredentialLocation::UrlPath { .. }
                | CredentialLocation::UrlBase { .. }
                | CredentialLocation::Body { .. } => {
                    return Ok(error_response(
                        StatusCode::NOT_IMPLEMENTED,
                        "URL-path credential injection is not supported by the proxy".to_string(),
                    ));
                }
            };
            tracing::debug!("Proxy: injected credential for {}", secret_name);
        } else {
            tracing::warn!("Proxy: credential {} not found", secret_name);
            return Ok(error_response(
                StatusCode::BAD_GATEWAY,
                "Required proxy credential is unavailable".to_string(),
            ));
        }
    }

    // Copy body
    let body_bytes = match tokio::time::timeout(
        PROXY_REQUEST_TIMEOUT,
        Limited::new(body, MAX_PROXY_REQUEST_BYTES).collect(),
    )
    .await
    {
        Ok(Ok(collected)) => collected.to_bytes(),
        Ok(Err(e)) => {
            tracing::warn!("Proxy: rejected request body: {}", e);
            return Ok(error_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                format!(
                    "Request body exceeds the {} byte proxy limit",
                    MAX_PROXY_REQUEST_BYTES
                ),
            ));
        }
        Err(_) => {
            return Ok(error_response(
                StatusCode::REQUEST_TIMEOUT,
                "Timed out reading sandbox proxy request body".to_string(),
            ));
        }
    };

    if !body_bytes.is_empty() {
        builder = builder.body(body_bytes.to_vec());
    }

    // Send the request
    match builder.send().await {
        Ok(mut response) => {
            let status = response.status();
            let headers = response.headers().clone();

            if response
                .content_length()
                .is_some_and(|length| length > MAX_PROXY_RESPONSE_BYTES as u64)
            {
                return Ok(error_response(
                    StatusCode::BAD_GATEWAY,
                    format!(
                        "Upstream response exceeds the {} byte proxy limit",
                        MAX_PROXY_RESPONSE_BYTES
                    ),
                ));
            }
            let mut body = BytesMut::new();
            loop {
                match response.chunk().await {
                    Ok(Some(chunk)) => {
                        if body.len().saturating_add(chunk.len()) > MAX_PROXY_RESPONSE_BYTES {
                            return Ok(error_response(
                                StatusCode::BAD_GATEWAY,
                                format!(
                                    "Upstream response exceeds the {} byte proxy limit",
                                    MAX_PROXY_RESPONSE_BYTES
                                ),
                            ));
                        }
                        body.extend_from_slice(&chunk);
                    }
                    Ok(None) => break,
                    Err(e) => {
                        tracing::error!("Proxy: failed to read response body: {}", e);
                        return Ok(error_response(
                            StatusCode::BAD_GATEWAY,
                            "Failed to read response".to_string(),
                        ));
                    }
                }
            }

            let mut resp_builder = Response::builder().status(status.as_u16());
            for (name, value) in &headers {
                if !is_hop_by_hop_header(name.as_str()) {
                    resp_builder = resp_builder.header(name.as_str(), value.as_bytes());
                }
            }
            Ok(make_response_from_builder(
                resp_builder,
                full_body(body.freeze()),
            ))
        }
        Err(e) => {
            // reqwest errors can retain the full request URL, including query
            // credentials. Strip it before logging or constructing a response.
            let safe_error = e.without_url();
            tracing::error!("Proxy: request failed: {}", safe_error);
            Ok(error_response(
                StatusCode::BAD_GATEWAY,
                "Upstream request failed".to_string(),
            ))
        }
    }
}

fn connection_header_names(headers: &hyper::HeaderMap) -> HashSet<String> {
    headers
        .get_all(hyper::header::CONNECTION)
        .iter()
        .chain(headers.get_all("proxy-connection").iter())
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

/// Check if a header is hop-by-hop (should not be forwarded).
fn is_hop_by_hop_header(name: &str) -> bool {
    matches!(
        name.to_lowercase().as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "proxy-connection"
            | "te"
            | "trailers"
            | "transfer-encoding"
            | "upgrade"
    )
}

/// Build a response with guaranteed success (valid status + simple body cannot fail).
fn make_response(
    status: StatusCode,
    body: BoxBody<Bytes, Infallible>,
) -> Response<BoxBody<Bytes, Infallible>> {
    Response::builder()
        .status(status)
        .body(body)
        .unwrap_or_else(|_| {
            let mut resp = Response::new(
                Full::new(Bytes::from("Internal error"))
                    .map_err(|_| unreachable!())
                    .boxed(),
            );
            *resp.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            resp
        })
}

/// Finalize a partially-built response, falling back to 500 on builder error.
fn make_response_from_builder(
    builder: hyper::http::response::Builder,
    body: BoxBody<Bytes, Infallible>,
) -> Response<BoxBody<Bytes, Infallible>> {
    builder.body(body).unwrap_or_else(|_| {
        Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(full_body(Bytes::from("Response build error")))
            .unwrap_or_else(|_| {
                Response::new(
                    Full::new(Bytes::from("Internal error"))
                        .map_err(|_| unreachable!())
                        .boxed(),
                )
            })
    })
}

/// Create an error response.
fn error_response(status: StatusCode, message: String) -> Response<BoxBody<Bytes, Infallible>> {
    make_response_from_builder(
        Response::builder()
            .status(status)
            .header("Content-Type", "text/plain"),
        full_body(Bytes::from(message)),
    )
}

fn proxy_auth_required_response() -> Response<BoxBody<Bytes, Infallible>> {
    make_response_from_builder(
        Response::builder()
            .status(StatusCode::PROXY_AUTHENTICATION_REQUIRED)
            .header("Proxy-Authenticate", "Basic realm=\"ThinClaw Sandbox\""),
        full_body(Bytes::from("Proxy authentication required")),
    )
}

/// Create an empty body.
fn empty_body() -> BoxBody<Bytes, Infallible> {
    Empty::<Bytes>::new().map_err(|_| unreachable!()).boxed()
}

/// Create a body from bytes.
fn full_body(bytes: Bytes) -> BoxBody<Bytes, Infallible> {
    Full::new(bytes).map_err(|_| unreachable!()).boxed()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::proxy::allowlist::DomainAllowlist;
    use crate::sandbox::proxy::policy::DefaultPolicyDecider;

    #[tokio::test]
    async fn test_proxy_starts_and_stops() {
        let allowlist = DomainAllowlist::new(&["example.com".to_string()]);
        let decider = Arc::new(DefaultPolicyDecider::new(allowlist, vec![]));
        let resolver = Arc::new(NoCredentialResolver);

        let proxy = HttpProxy::new(decider, resolver);

        let addr = proxy.start(0).await.unwrap();
        assert!(proxy.is_running());
        assert!(addr.port() > 0);

        proxy.stop().await;
        assert!(!proxy.is_running());

        let restarted = proxy.start(0).await.unwrap();
        assert!(proxy.is_running());
        assert!(restarted.port() > 0);
        proxy.stop().await;
        assert!(!proxy.is_running());
    }

    #[tokio::test]
    async fn proxy_requires_authentication_on_host_listener() {
        let allowlist = DomainAllowlist::new(&["example.com".to_string()]);
        let decider = Arc::new(DefaultPolicyDecider::new(allowlist, vec![]));
        let proxy = HttpProxy::new(decider, Arc::new(NoCredentialResolver));
        let address = proxy.start(0).await.unwrap();

        let mut stream = TcpStream::connect(("127.0.0.1", address.port()))
            .await
            .unwrap();
        stream
            .write_all(
                b"GET http://example.com/ HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        let mut response = Vec::new();
        tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut response))
            .await
            .unwrap()
            .unwrap();
        assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 407"));

        proxy.stop().await;
    }

    #[tokio::test]
    async fn local_proxy_can_bind_exclusively_to_loopback() {
        let allowlist = DomainAllowlist::new(&["example.com".to_string()]);
        let decider = Arc::new(DefaultPolicyDecider::new(allowlist, vec![]));
        let proxy = HttpProxy::new(decider, Arc::new(NoCredentialResolver));
        let address = proxy.start_loopback(0).await.unwrap();

        assert_eq!(address.ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert!(address.port() > 0);
        proxy.stop().await;
    }

    fn client_hello_with_sni(server_name: &str) -> Vec<u8> {
        let name = server_name.as_bytes();
        let mut server_name_entry = vec![0];
        server_name_entry.extend_from_slice(&(name.len() as u16).to_be_bytes());
        server_name_entry.extend_from_slice(name);
        let mut server_name_extension = Vec::new();
        server_name_extension.extend_from_slice(&(server_name_entry.len() as u16).to_be_bytes());
        server_name_extension.extend_from_slice(&server_name_entry);

        let mut extensions = Vec::new();
        extensions.extend_from_slice(&0u16.to_be_bytes());
        extensions.extend_from_slice(&(server_name_extension.len() as u16).to_be_bytes());
        extensions.extend_from_slice(&server_name_extension);

        let mut hello = vec![0x03, 0x03];
        hello.extend_from_slice(&[7; 32]);
        hello.push(0); // session id
        hello.extend_from_slice(&2u16.to_be_bytes());
        hello.extend_from_slice(&0x1301u16.to_be_bytes());
        hello.push(1);
        hello.push(0);
        hello.extend_from_slice(&(extensions.len() as u16).to_be_bytes());
        hello.extend_from_slice(&extensions);

        let mut handshake = vec![1];
        let hello_length = hello.len() as u32;
        handshake.extend_from_slice(&hello_length.to_be_bytes()[1..]);
        handshake.extend_from_slice(&hello);
        handshake
    }

    #[test]
    fn parses_client_hello_sni_for_connect_binding() {
        let hello = client_hello_with_sni("Api.Example.COM");
        assert_eq!(parse_client_hello_sni(&hello).unwrap(), "api.example.com");
    }

    #[test]
    fn rejects_client_hello_without_sni() {
        let mut hello = client_hello_with_sni("example.com");
        // Change the server-name extension to an unknown extension type.
        let extension_offset = 4 + 2 + 32 + 1 + 2 + 2 + 1 + 1 + 2;
        hello[extension_offset..extension_offset + 2].copy_from_slice(&42u16.to_be_bytes());
        assert!(parse_client_hello_sni(&hello).is_err());
    }

    #[test]
    fn rejects_non_public_proxy_targets() {
        for address in [
            "127.0.0.1",
            "10.0.0.1",
            "100.64.0.1",
            "169.254.1.1",
            "192.0.2.1",
            "198.18.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
            "2001:db8::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(
                !is_public_outbound_ip(address.parse().unwrap()),
                "{address} must not be proxyable"
            );
        }
        for address in ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"] {
            assert!(
                is_public_outbound_ip(address.parse().unwrap()),
                "{address} should be recognized as public"
            );
        }
    }

    #[test]
    fn connection_header_tokens_are_hop_by_hop() {
        let mut headers = hyper::HeaderMap::new();
        headers.insert("connection", "keep-alive, X-Internal-Hop".parse().unwrap());
        headers.insert("proxy-connection", "Another-Hop".parse().unwrap());
        let names = connection_header_names(&headers);
        assert!(names.contains("keep-alive"));
        assert!(names.contains("x-internal-hop"));
        assert!(names.contains("another-hop"));
    }

    #[test]
    fn test_hop_by_hop_headers() {
        assert!(is_hop_by_hop_header("connection"));
        assert!(is_hop_by_hop_header("Connection"));
        assert!(is_hop_by_hop_header("transfer-encoding"));
        assert!(!is_hop_by_hop_header("content-type"));
        assert!(!is_hop_by_hop_header("authorization"));
    }

    fn test_secrets_crypto() -> Arc<crate::secrets::SecretsCrypto> {
        // Matches the fixed-key fixture used by the WASM credential-injector
        // tests (`thinclaw-tools` wasm::credential_injector::tests::test_store).
        let key = "0123456789abcdef0123456789abcdef";
        Arc::new(
            crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(key.to_string()))
                .expect("test crypto key is valid"),
        )
    }

    #[tokio::test]
    async fn test_store_credential_resolver_returns_stored_value() {
        use crate::secrets::{CreateSecretParams, InMemorySecretsStore};

        let store = Arc::new(InMemorySecretsStore::new(test_secrets_crypto()));
        store
            .create(
                "user1",
                CreateSecretParams::new("OPENAI_API_KEY", "sk-test123"),
            )
            .await
            .expect("create secret");

        let resolver = StoreCredentialResolver::new(store, "user1");
        assert_eq!(
            resolver.resolve("OPENAI_API_KEY").await,
            Some("sk-test123".to_string())
        );
    }

    #[tokio::test]
    async fn test_store_credential_resolver_missing_secret_is_none() {
        use crate::secrets::InMemorySecretsStore;

        let store = Arc::new(InMemorySecretsStore::new(test_secrets_crypto()));
        let resolver = StoreCredentialResolver::new(store, "user1");
        assert_eq!(resolver.resolve("ABSENT_KEY").await, None);
    }

    #[test]
    fn test_make_response_does_not_panic() {
        let resp = make_response(StatusCode::OK, empty_body());
        assert_eq!(resp.status(), StatusCode::OK);

        let resp = error_response(StatusCode::FORBIDDEN, "denied".to_string());
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }
}
