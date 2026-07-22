//! Shared OAuth infrastructure: optional provider credentials, callback server, landing pages.
//!
//! Every OAuth flow in the codebase (WASM tool auth, MCP server auth, NEAR AI login)
//! uses the same callback port, landing page, and listener logic from this module.
//!
//! # Supported Providers
//!
//! - **Google** (Desktop App): Calendar, Drive, Gmail, Sheets, etc.
//! - **GitHub** (OAuth App): GitHub API access for code, issues, PRs.
//! - **Notion** (Integration): Notion workspace access.
//! - **Gmail** (Desktop App): Gmail-specific variant with pub/sub scopes.
//!
//! # OAuth Credentials
//!
//! Provider client credentials are never committed to the repository. They can
//! be supplied at:
//!
//! - **Compile time**: Set THINCLAW_GOOGLE_CLIENT_ID / THINCLAW_GOOGLE_CLIENT_SECRET
//!   before building to inject a distributor-owned OAuth client.
//! - **Runtime**: Users can set GOOGLE_OAUTH_CLIENT_ID / GOOGLE_OAUTH_CLIENT_SECRET
//!   env vars, which take priority over optional compile-time values.

use std::time::Duration;

use rand::RngExt;
use subtle::ConstantTimeEq;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

const OAUTH_CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);
const OAUTH_CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_OAUTH_CALLBACK_CONNECTIONS: usize = 16;
const MAX_OAUTH_REQUEST_LINE_BYTES: usize = 16 * 1024;
const MAX_OAUTH_CALLBACK_PARAMS: usize = 32;
const MAX_OAUTH_CALLBACK_VALUE_BYTES: usize = 8 * 1024;

// ── Optional compile-time credentials ──────────────────────────────────

pub struct OAuthCredentials {
    pub client_id: &'static str,
    pub client_secret: &'static str,
}

fn complete_credentials(
    client_id: &'static str,
    client_secret: &'static str,
) -> Option<OAuthCredentials> {
    if client_id.is_empty() || client_secret.is_empty() {
        return None;
    }
    Some(OAuthCredentials {
        client_id,
        client_secret,
    })
}

/// Optional Google OAuth "Desktop App" credentials shared by Google tools.
/// Runtime credentials from a tool capability take precedence over these
/// compile-time values.
const GOOGLE_CLIENT_ID: &str = match option_env!("THINCLAW_GOOGLE_CLIENT_ID") {
    Some(v) => v,
    None => "",
};
const GOOGLE_CLIENT_SECRET: &str = match option_env!("THINCLAW_GOOGLE_CLIENT_SECRET") {
    Some(v) => v,
    None => "",
};

/// GitHub OAuth App credentials.
///
/// **Note:** The GitHub WASM tool uses a Personal Access Token (PAT), not OAuth.
/// Set it with: `thinclaw secret set github_token <token>`
/// This OAuth config only applies if you register a GitHub OAuth App for
/// a future OAuth-based integration.
///
/// Override at compile time with THINCLAW_GITHUB_CLIENT_ID / THINCLAW_GITHUB_CLIENT_SECRET,
/// or at runtime via GITHUB_OAUTH_CLIENT_ID / GITHUB_OAUTH_CLIENT_SECRET env vars.
const GITHUB_CLIENT_ID: &str = match option_env!("THINCLAW_GITHUB_CLIENT_ID") {
    Some(v) => v,
    None => "Ov23liIronClawGHApp01",
};
const GITHUB_CLIENT_SECRET: &str = match option_env!("THINCLAW_GITHUB_CLIENT_SECRET") {
    Some(v) => v,
    // No built-in secret: users must register their own GitHub OAuth App
    // and provide credentials via env var or compile-time override.
    // The GitHub WASM tool uses PAT auth, not OAuth.
    None => "",
};

/// Notion Integration credentials.
///
/// **Note:** No Notion WASM tool exists yet. These credentials are reserved
/// for a future Notion integration. Users must register their own Notion
/// integration and provide credentials via env var or compile-time override.
const NOTION_CLIENT_ID: &str = match option_env!("THINCLAW_NOTION_CLIENT_ID") {
    Some(v) => v,
    None => "",
};
const NOTION_CLIENT_SECRET: &str = match option_env!("THINCLAW_NOTION_CLIENT_SECRET") {
    Some(v) => v,
    None => "",
};

/// Returns compile-time OAuth credentials for a provider, keyed by secret_name.
///
/// The secret_name comes from the tool's capabilities.json `auth.secret_name` field.
/// Returns `None` if no complete credential pair is configured for that provider.
pub fn builtin_credentials(secret_name: &str) -> Option<OAuthCredentials> {
    match secret_name {
        "google_oauth_token" | "gmail_oauth_token" => {
            complete_credentials(GOOGLE_CLIENT_ID, GOOGLE_CLIENT_SECRET)
        }
        "github_oauth_token"
            if !GITHUB_CLIENT_ID.is_empty() && !GITHUB_CLIENT_SECRET.is_empty() =>
        {
            Some(OAuthCredentials {
                client_id: GITHUB_CLIENT_ID,
                client_secret: GITHUB_CLIENT_SECRET,
            })
        }
        "notion_oauth_token"
            if !NOTION_CLIENT_ID.is_empty() && !NOTION_CLIENT_SECRET.is_empty() =>
        {
            Some(OAuthCredentials {
                client_id: NOTION_CLIENT_ID,
                client_secret: NOTION_CLIENT_SECRET,
            })
        }
        _ => None,
    }
}

/// Gmail-specific OAuth configuration.
///
/// Uses the same Google Desktop App credentials as `google_oauth_token`,
/// but with scopes specific to Gmail access and pub/sub notifications.
/// This is what `cloud_oauth_start("gmail")` dispatches to in ThinClaw Desktop.
pub struct GmailOAuthConfig;

impl GmailOAuthConfig {
    /// OAuth scopes required for Gmail integration.
    pub const SCOPES: &'static [&'static str] = &[
        "https://www.googleapis.com/auth/gmail.readonly",
        "https://www.googleapis.com/auth/gmail.send",
        "https://www.googleapis.com/auth/pubsub",
    ];

    /// The authorization endpoint.
    pub const AUTH_URL: &'static str = "https://accounts.google.com/o/oauth2/v2/auth";

    /// The token exchange endpoint.
    pub const TOKEN_URL: &'static str = "https://oauth2.googleapis.com/token";

    /// The redirect URI (uses the shared callback port).
    pub fn redirect_uri() -> String {
        format!("{}/callback", callback_url())
    }

    /// Build the full authorization URL with PKCE.
    pub fn auth_url(state: &str, code_challenge: &str) -> Option<String> {
        let credentials = builtin_credentials("gmail_oauth_token")?;
        Some(format!(
            "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}&code_challenge={}&code_challenge_method=S256&access_type=offline&prompt=consent",
            Self::AUTH_URL,
            credentials.client_id,
            urlencoding::encode(&Self::redirect_uri()),
            urlencoding::encode(&Self::SCOPES.join(" ")),
            urlencoding::encode(state),
            code_challenge,
        ))
    }
}

// ── Shared callback server ──────────────────────────────────────────────

/// Fixed port for all OAuth callbacks.
///
/// Every redirect URI registered with providers must use this port:
/// `http://localhost:9876/callback` (or `/auth/callback` for NEAR AI).
pub const OAUTH_CALLBACK_PORT: u16 = 9876;

/// Returns the OAuth callback base URL.
///
/// Checks `THINCLAW_OAUTH_CALLBACK_URL` env var first (useful for remote/VPS
/// deployments where `127.0.0.1` is unreachable from the user's browser),
/// then falls back to `http://{callback_host()}:{OAUTH_CALLBACK_PORT}`.
pub fn callback_url() -> String {
    if let Ok(value) = std::env::var("THINCLAW_OAUTH_CALLBACK_URL")
        && value.len() <= 16 * 1024
        && let Ok(mut parsed) = url::Url::parse(&value)
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.query().is_none()
        && parsed.fragment().is_none()
        && parsed.host_str().is_some()
        && (parsed.scheme() == "https"
            || (parsed.scheme() == "http" && parsed.host_str().is_some_and(is_loopback_host)))
    {
        let path = parsed.path().trim_end_matches('/').to_string();
        parsed.set_path(&path);
        return parsed.to_string().trim_end_matches('/').to_string();
    }
    // The built-in listener is intentionally loopback-only. Remote callbacks
    // require an explicit HTTPS reverse-proxy URL above.
    format!("http://127.0.0.1:{OAUTH_CALLBACK_PORT}")
}

/// Returns the hostname used in OAuth callback URLs.
///
/// Reads `OAUTH_CALLBACK_HOST` from the environment (default: `127.0.0.1`).
///
/// **Remote server usage:** set `OAUTH_CALLBACK_HOST` to the network interface
/// address you want to listen on (e.g. the server's LAN IP or `0.0.0.0`).
/// The callback listener will bind to that specific address instead of the
/// loopback interface, so the OAuth redirect can reach an external browser.
/// Note: this transmits the session token over plain HTTP — prefer SSH port
/// forwarding (`ssh -L 9876:127.0.0.1:9876 user@host`) when possible.
///
/// # Example
///
/// ```bash
/// export OAUTH_CALLBACK_HOST=203.0.113.10
/// thinclaw login
/// # Opens: http://203.0.113.10:9876/auth/callback
/// ```
pub fn callback_host() -> String {
    std::env::var("OAUTH_CALLBACK_HOST").unwrap_or_else(|_| "127.0.0.1".to_string())
}

/// Returns `true` if `host` is a loopback address that only accepts local connections.
///
/// Covers `localhost` (case-insensitive), the full `127.0.0.0/8` IPv4 loopback
/// range, and `::1` for IPv6.
pub fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

/// Returns true when the current shell looks like SSH/headless operation.
pub fn ssh_or_headless_detected() -> bool {
    std::env::var_os("SSH_CONNECTION").is_some()
        || std::env::var_os("SSH_TTY").is_some()
        || (cfg!(target_os = "linux")
            && std::env::var_os("DISPLAY").is_none()
            && std::env::var_os("WAYLAND_DISPLAY").is_none())
}

/// SSH port-forwarding command for the shared OAuth callback listener.
pub fn ssh_callback_tunnel_command() -> String {
    format!(
        "ssh -L {port}:127.0.0.1:{port} user@host",
        port = OAUTH_CALLBACK_PORT
    )
}

/// Print OAuth callback tunnel guidance for SSH/headless hosts.
pub fn print_ssh_callback_hint() {
    println!("  SSH/headless OAuth callback tunnel:");
    println!("  {}", ssh_callback_tunnel_command());
    println!("  Keep that tunnel open, then open the auth URL in your local browser.");
}

/// Error from the OAuth callback listener.
#[derive(Debug, thiserror::Error)]
pub enum OAuthCallbackError {
    #[error("Port {0} is in use (another auth flow running?): {1}")]
    PortInUse(u16, String),

    #[error("Authorization denied by user")]
    Denied,

    #[error("Timed out waiting for authorization")]
    Timeout,

    #[error("IO error: {0}")]
    Io(String),

    #[error("OAuth callback listener must bind to a loopback host")]
    InsecureHost,
}

/// Map a `std::io::Error` from a bind attempt to an `OAuthCallbackError`.
fn bind_error(e: std::io::Error) -> OAuthCallbackError {
    if e.kind() == std::io::ErrorKind::AddrInUse {
        OAuthCallbackError::PortInUse(OAUTH_CALLBACK_PORT, e.to_string())
    } else {
        OAuthCallbackError::Io(e.to_string())
    }
}

/// Bind the OAuth callback listener on the fixed port.
///
/// When `OAUTH_CALLBACK_HOST` is a loopback address (the default `127.0.0.1`),
/// binds to `127.0.0.1` first and falls back to `[::1]` so local-only auth
/// flows remain restricted to the local machine.
///
/// When `OAUTH_CALLBACK_HOST` is set to a remote address, binds to that
/// specific address so only connections directed to it are accepted.
pub async fn bind_callback_listener() -> Result<TcpListener, OAuthCallbackError> {
    let host = callback_host();

    if is_loopback_host(&host) {
        // Local mode: prefer IPv4 loopback, fall back to IPv6.
        let ipv4_addr = format!("127.0.0.1:{}", OAUTH_CALLBACK_PORT);
        match TcpListener::bind(&ipv4_addr).await {
            Ok(listener) => return Ok(listener),
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                return Err(OAuthCallbackError::PortInUse(
                    OAUTH_CALLBACK_PORT,
                    e.to_string(),
                ));
            }
            Err(_) => {
                // IPv4 not available, fall back to IPv6
            }
        }
        TcpListener::bind(format!("[::1]:{}", OAUTH_CALLBACK_PORT))
            .await
            .map_err(bind_error)
    } else {
        Err(OAuthCallbackError::InsecureHost)
    }
}

/// Generate a cryptographically random, URL-safe OAuth `state` value.
///
/// The `state` parameter is the CSRF defense for the authorization-code flow:
/// it is sent on the authorization request and must be echoed back unchanged on
/// the loopback callback. Callers should generate one with this helper, thread
/// it into the authorization URL, and validate the callback with
/// [`wait_for_callback_with_state`].
///
/// Returns 32 bytes of randomness rendered as 64 lowercase hex characters, which
/// is comfortably above the entropy needed to make guessing infeasible while
/// staying within typical provider `state` length limits.
pub fn generate_oauth_state() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Returns `true` if `received` matches `expected` in constant time.
///
/// Uses a constant-time comparison so a timing side channel cannot be used to
/// recover the expected `state` byte by byte. Mirrors the `ConstantTimeEq`
/// usage in `src/orchestrator/auth.rs` and `src/hooks/webhook_signing.rs`.
fn oauth_state_matches(expected: &str, received: &str) -> bool {
    expected.as_bytes().ct_eq(received.as_bytes()).into()
}

/// Wait for an OAuth callback and extract a query parameter value.
///
/// Listens for a GET request matching `path_prefix` (e.g., "/callback" or "/auth/callback"),
/// extracts the value of `param_name` (e.g., "code" or "token"), and shows a branded
/// landing page using `display_name` (e.g., "Google", "Notion", "NEAR AI").
///
/// Times out after 5 minutes.
///
/// This is the state-less variant kept for callers that do not yet thread an
/// OAuth `state` nonce. New code should prefer [`wait_for_callback_with_state`]
/// and pass the value returned by [`generate_oauth_state`] so the loopback
/// callback is protected against CSRF / authorization-code injection.
pub async fn wait_for_callback(
    listener: TcpListener,
    path_prefix: &str,
    param_name: &str,
    display_name: &str,
) -> Result<String, OAuthCallbackError> {
    wait_for_callback_with_state(listener, path_prefix, param_name, display_name, None).await
}

/// Wait for an OAuth callback, optionally validating the `state` parameter.
///
/// Behaves like [`wait_for_callback`], but when `expected_state` is `Some`, the
/// callback's `state` query parameter must be present and match `expected_state`
/// (compared in constant time). A missing or mismatched `state` is rejected with
/// [`OAuthCallbackError::Denied`] and the failure landing page, preventing an
/// attacker from injecting their own authorization code into the loopback flow.
///
/// When `expected_state` is `None`, no `state` checking is performed (legacy
/// behavior).
pub async fn wait_for_callback_with_state(
    listener: TcpListener,
    path_prefix: &str,
    param_name: &str,
    display_name: &str,
    expected_state: Option<&str>,
) -> Result<String, OAuthCallbackError> {
    if path_prefix.is_empty()
        || path_prefix.len() > 1024
        || !path_prefix.starts_with('/')
        || path_prefix.contains(['?', '#'])
        || param_name.is_empty()
        || param_name.len() > 128
        || !param_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        || expected_state.is_some_and(|state| {
            state.is_empty()
                || state.len() > MAX_OAUTH_CALLBACK_VALUE_BYTES
                || state.chars().any(char::is_control)
        })
    {
        return Err(OAuthCallbackError::Io(
            "invalid OAuth callback configuration".to_string(),
        ));
    }
    let path_prefix = path_prefix.to_string();
    let param_name = param_name.to_string();
    let display_name = display_name
        .chars()
        .filter(|character| !character.is_control())
        .take(128)
        .collect::<String>();
    let expected_state = expected_state.map(str::to_string);

    tokio::time::timeout(OAUTH_CALLBACK_TIMEOUT, async move {
        let mut connections = JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept(), if connections.len() < MAX_OAUTH_CALLBACK_CONNECTIONS => {
                    let (socket, peer) = accepted
                        .map_err(|error| OAuthCallbackError::Io(error.to_string()))?;
                    let path_prefix = path_prefix.clone();
                    let param_name = param_name.clone();
                    let display_name = display_name.clone();
                    let expected_state = expected_state.clone();
                    connections.spawn(async move {
                        tokio::time::timeout(
                            OAUTH_CONNECTION_TIMEOUT,
                            process_callback_connection(
                                socket,
                                peer,
                                &path_prefix,
                                &param_name,
                                &display_name,
                                expected_state.as_deref(),
                            ),
                        )
                        .await
                        .unwrap_or(CallbackAttempt::Ignore)
                    });
                }
                Some(completed) = connections.join_next(), if !connections.is_empty() => {
                    match completed {
                        Ok(CallbackAttempt::Success(value)) => return Ok(value),
                        Ok(CallbackAttempt::Denied) => return Err(OAuthCallbackError::Denied),
                        Ok(CallbackAttempt::Ignore) => {}
                        Err(error) if error.is_cancelled() => {}
                        Err(error) => tracing::debug!(error = %error, "OAuth callback connection task failed"),
                    }
                }
            }
        }
    })
    .await
    .map_err(|_| OAuthCallbackError::Timeout)?
}

enum CallbackAttempt {
    Success(String),
    Denied,
    Ignore,
}

async fn process_callback_connection(
    mut socket: TcpStream,
    peer: std::net::SocketAddr,
    expected_path: &str,
    param_name: &str,
    display_name: &str,
    expected_state: Option<&str>,
) -> CallbackAttempt {
    // The legacy state-less entry point is retained for compatibility, but it
    // must never accept a callback arriving through a non-loopback interface.
    if expected_state.is_none() && !peer.ip().is_loopback() {
        let _ = write_empty_response(&mut socket, "403 Forbidden").await;
        return CallbackAttempt::Ignore;
    }

    let Some(request_line) = read_bounded_request_line(&mut socket).await else {
        let _ = write_empty_response(&mut socket, "400 Bad Request").await;
        return CallbackAttempt::Ignore;
    };
    let mut request_parts = request_line.split_whitespace();
    let (Some(method), Some(target), Some(version)) = (
        request_parts.next(),
        request_parts.next(),
        request_parts.next(),
    ) else {
        let _ = write_empty_response(&mut socket, "400 Bad Request").await;
        return CallbackAttempt::Ignore;
    };
    if method != "GET"
        || !version.starts_with("HTTP/1.")
        || request_parts.next().is_some()
        || !target.starts_with('/')
        || target.contains('#')
    {
        let _ = write_empty_response(&mut socket, "400 Bad Request").await;
        return CallbackAttempt::Ignore;
    }
    let Some((path, query)) = target.split_once('?') else {
        let _ = write_empty_response(&mut socket, "404 Not Found").await;
        return CallbackAttempt::Ignore;
    };
    if path != expected_path {
        let _ = write_empty_response(&mut socket, "404 Not Found").await;
        return CallbackAttempt::Ignore;
    }
    if !valid_percent_encoding(query) {
        let _ = write_empty_response(&mut socket, "400 Bad Request").await;
        return CallbackAttempt::Ignore;
    }

    let mut params = std::collections::HashMap::new();
    for (index, (key, value)) in url::form_urlencoded::parse(query.as_bytes()).enumerate() {
        if index >= MAX_OAUTH_CALLBACK_PARAMS
            || key.is_empty()
            || key.len() > 128
            || value.len() > MAX_OAUTH_CALLBACK_VALUE_BYTES
            || key.chars().any(char::is_control)
            || value.chars().any(char::is_control)
            || params
                .insert(key.into_owned(), value.into_owned())
                .is_some()
        {
            let _ = write_empty_response(&mut socket, "400 Bad Request").await;
            return CallbackAttempt::Ignore;
        }
    }

    // Validate state before honoring either an OAuth error or a code. A forged
    // callback must not be able to cancel the legitimate flow.
    if let Some(expected) = expected_state {
        let state_matches = params
            .get("state")
            .is_some_and(|received| oauth_state_matches(expected, received));
        if !state_matches {
            tracing::warn!("[oauth] ignored callback with missing or mismatched state");
            let _ = write_landing(&mut socket, display_name, false, "400 Bad Request").await;
            return CallbackAttempt::Ignore;
        }
    }

    if params.contains_key("error") {
        let _ = write_landing(&mut socket, display_name, false, "400 Bad Request").await;
        return CallbackAttempt::Denied;
    }
    let Some(value) = params.get(param_name) else {
        let _ = write_empty_response(&mut socket, "400 Bad Request").await;
        return CallbackAttempt::Ignore;
    };
    if value.is_empty() || value.len() > MAX_OAUTH_CALLBACK_VALUE_BYTES {
        let _ = write_empty_response(&mut socket, "400 Bad Request").await;
        return CallbackAttempt::Ignore;
    }

    let _ = write_landing(&mut socket, display_name, true, "200 OK").await;
    let _ = socket.shutdown().await;
    CallbackAttempt::Success(value.clone())
}

async fn read_bounded_request_line(socket: &mut TcpStream) -> Option<String> {
    let mut line = Vec::new();
    let mut buffer = [0u8; 1024];
    loop {
        let read = socket.read(&mut buffer).await.ok()?;
        if read == 0 {
            return None;
        }
        let bytes = &buffer[..read];
        let line_end = bytes.iter().position(|byte| *byte == b'\n');
        let portion = line_end.map_or(bytes, |index| &bytes[..=index]);
        if line.len().saturating_add(portion.len()) > MAX_OAUTH_REQUEST_LINE_BYTES {
            return None;
        }
        line.extend_from_slice(portion);
        if line_end.is_some() {
            break;
        }
    }
    let line = std::str::from_utf8(&line)
        .ok()?
        .trim_end_matches(['\r', '\n']);
    Some(line.to_string())
}

fn valid_percent_encoding(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len()
                || !bytes[index + 1].is_ascii_hexdigit()
                || !bytes[index + 2].is_ascii_hexdigit()
            {
                return false;
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    true
}

async fn write_empty_response(socket: &mut TcpStream, status: &str) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n\r\n"
    );
    socket.write_all(response.as_bytes()).await
}

/// Write the branded OAuth landing page with the given HTTP status line.
async fn write_landing<W>(
    socket: &mut W,
    display_name: &str,
    success: bool,
    status: &str,
) -> std::io::Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let html = landing_html(display_name, success);
    let response = format!(
        "HTTP/1.1 {status}\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         Cache-Control: no-store\r\n\
         Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'\r\n\
         Referrer-Policy: no-referrer\r\n\
         X-Content-Type-Options: nosniff\r\n\
         X-Frame-Options: DENY\r\n\
         \r\n\
         {html}",
        html.len()
    );
    socket.write_all(response.as_bytes()).await
}

/// Escape a string for safe interpolation into HTML content.
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(c),
        }
    }
    out
}

/// HTML landing page shown in the browser after an OAuth redirect.
pub fn landing_html(provider_name: &str, success: bool) -> String {
    let safe_name = html_escape(provider_name);
    let (icon, heading, subtitle, accent) = if success {
        (
            r##"<div style="width:64px;height:64px;border-radius:50%;background:#22c55e;display:flex;align-items:center;justify-content:center;margin:0 auto 24px">
                <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
              </div>"##,
            format!("{} Connected", safe_name),
            "You can close this window and return to your terminal.",
            "#22c55e",
        )
    } else {
        (
            r##"<div style="width:64px;height:64px;border-radius:50%;background:#ef4444;display:flex;align-items:center;justify-content:center;margin:0 auto 24px">
                <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
              </div>"##,
            "Authorization Failed".to_string(),
            "The request was denied. You can close this window and try again.",
            "#ef4444",
        )
    };

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>ThinClaw - {heading}</title>
<style>
  * {{ margin:0; padding:0; box-sizing:border-box }}
  body {{
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
    background: #0a0a0a;
    color: #e5e5e5;
    display: flex;
    justify-content: center;
    align-items: center;
    min-height: 100vh;
  }}
  .card {{
    text-align: center;
    padding: 48px 40px;
    max-width: 420px;
    border: 1px solid #262626;
    border-radius: 16px;
    background: #141414;
  }}
  h1 {{
    font-size: 22px;
    font-weight: 600;
    margin-bottom: 8px;
    color: #fafafa;
  }}
  p {{
    font-size: 14px;
    color: #a3a3a3;
    line-height: 1.5;
  }}
  .accent {{ color: {accent}; }}
  .brand {{
    margin-top: 32px;
    font-size: 12px;
    color: #525252;
    letter-spacing: 0.5px;
    text-transform: uppercase;
  }}
</style>
</head>
<body>
  <div class="card">
    {icon}
    <h1>{heading}</h1>
    <p>{subtitle}</p>
    <div class="brand">ThinClaw</div>
  </div>
</body>
</html>"#,
        heading = heading,
        icon = icon,
        subtitle = subtitle,
        accent = accent,
    )
}

#[cfg(test)]
mod tests {
    use crate::cli::oauth_defaults::{
        GmailOAuthConfig, builtin_credentials, callback_host, callback_url, is_loopback_host,
        landing_html,
    };
    use crate::config::helpers::lock_env;

    #[test]
    fn complete_credentials_requires_both_values() {
        assert!(super::complete_credentials("", "secret").is_none());
        assert!(super::complete_credentials("client", "").is_none());

        let credentials = super::complete_credentials("client", "secret").unwrap();
        assert_eq!(credentials.client_id, "client");
        assert_eq!(credentials.client_secret, "secret");
    }

    #[test]
    fn test_is_loopback_host() {
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("127.0.0.2")); // full 127.0.0.0/8 range
        assert!(is_loopback_host("127.255.255.254"));
        assert!(is_loopback_host("::1"));
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("LOCALHOST"));
        assert!(!is_loopback_host("203.0.113.10"));
        assert!(!is_loopback_host("my-server.example.com"));
        assert!(!is_loopback_host("0.0.0.0"));
    }

    #[test]
    fn test_callback_host_default() {
        let _guard = lock_env();
        let original = std::env::var("OAUTH_CALLBACK_HOST").ok();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::remove_var("OAUTH_CALLBACK_HOST");
        }
        assert_eq!(callback_host(), "127.0.0.1");
        // Restore
        unsafe {
            if let Some(val) = original {
                std::env::set_var("OAUTH_CALLBACK_HOST", val);
            }
        }
    }

    #[test]
    fn test_callback_host_env_override() {
        let _guard = lock_env();
        let original_host = std::env::var("OAUTH_CALLBACK_HOST").ok();
        let original_url = std::env::var("THINCLAW_OAUTH_CALLBACK_URL").ok();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::set_var("OAUTH_CALLBACK_HOST", "203.0.113.10");
            std::env::remove_var("THINCLAW_OAUTH_CALLBACK_URL");
        }
        assert_eq!(callback_host(), "203.0.113.10");
        // A remote plain-HTTP callback host is never used as the redirect
        // fallback; remote deployments must provide an explicit HTTPS proxy.
        let url = callback_url();
        assert_eq!(url, "http://127.0.0.1:9876");
        // Restore
        unsafe {
            if let Some(val) = original_host {
                std::env::set_var("OAUTH_CALLBACK_HOST", val);
            } else {
                std::env::remove_var("OAUTH_CALLBACK_HOST");
            }
            if let Some(val) = original_url {
                std::env::set_var("THINCLAW_OAUTH_CALLBACK_URL", val);
            }
        }
    }

    #[test]
    fn test_callback_url_default() {
        let _guard = lock_env();
        // Clear both env vars to test default behavior
        let original_url = std::env::var("THINCLAW_OAUTH_CALLBACK_URL").ok();
        let original_host = std::env::var("OAUTH_CALLBACK_HOST").ok();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::remove_var("THINCLAW_OAUTH_CALLBACK_URL");
            std::env::remove_var("OAUTH_CALLBACK_HOST");
        }
        let url = callback_url();
        assert_eq!(url, "http://127.0.0.1:9876");
        // Restore
        unsafe {
            if let Some(val) = original_url {
                std::env::set_var("THINCLAW_OAUTH_CALLBACK_URL", val);
            }
            if let Some(val) = original_host {
                std::env::set_var("OAUTH_CALLBACK_HOST", val);
            }
        }
    }

    #[test]
    fn test_callback_url_env_override() {
        let _guard = lock_env();
        let original = std::env::var("THINCLAW_OAUTH_CALLBACK_URL").ok();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::set_var(
                "THINCLAW_OAUTH_CALLBACK_URL",
                "https://myserver.example.com:9876",
            );
        }
        let url = callback_url();
        assert_eq!(url, "https://myserver.example.com:9876");
        // Restore
        unsafe {
            if let Some(val) = original {
                std::env::set_var("THINCLAW_OAUTH_CALLBACK_URL", val);
            } else {
                std::env::remove_var("THINCLAW_OAUTH_CALLBACK_URL");
            }
        }
    }

    #[test]
    fn test_unknown_provider_returns_none() {
        assert!(builtin_credentials("unknown_token").is_none());
    }

    #[test]
    fn test_google_returns_based_on_compile_env() {
        let creds = builtin_credentials("google_oauth_token");
        if super::GOOGLE_CLIENT_ID.is_empty() || super::GOOGLE_CLIENT_SECRET.is_empty() {
            assert!(creds.is_none());
        } else {
            let creds = creds.unwrap();
            assert!(!creds.client_id.is_empty());
            assert!(!creds.client_secret.is_empty());
        }
    }

    #[test]
    fn test_github_returns_none_without_env_credentials() {
        // GitHub uses PAT auth (not OAuth), so builtin_credentials returns None
        // unless real OAuth App credentials are provided via env vars.
        let creds = builtin_credentials("github_oauth_token");
        // Without THINCLAW_GITHUB_CLIENT_SECRET set, this should be None.
        // If someone compiles with the env var set, it would be Some.
        if super::GITHUB_CLIENT_SECRET.is_empty() {
            assert!(creds.is_none());
        } else {
            assert!(creds.is_some());
        }
    }

    #[test]
    fn test_notion_returns_none_without_env_credentials() {
        // No Notion tool exists yet. Credentials are only available if
        // provided via env vars at compile time.
        let creds = builtin_credentials("notion_oauth_token");
        if super::NOTION_CLIENT_SECRET.is_empty() {
            assert!(creds.is_none());
        } else {
            assert!(creds.is_some());
        }
    }

    #[test]
    fn test_landing_html_success_contains_key_elements() {
        let html = landing_html("Google", true);
        assert!(html.contains("Google Connected"));
        assert!(html.contains("charset"));
        assert!(html.contains("ThinClaw"));
        assert!(html.contains("#22c55e")); // green accent
        assert!(!html.contains("Failed"));
    }

    #[test]
    fn test_landing_html_escapes_provider_name() {
        let html = landing_html("<script>alert(1)</script>", true);
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn test_landing_html_error_contains_key_elements() {
        let html = landing_html("Notion", false);
        assert!(html.contains("Authorization Failed"));
        assert!(html.contains("charset"));
        assert!(html.contains("ThinClaw"));
        assert!(html.contains("#ef4444")); // red accent
        assert!(!html.contains("Connected"));
    }

    #[test]
    fn test_gmail_returns_credentials() {
        let creds = builtin_credentials("gmail_oauth_token");
        if super::GOOGLE_CLIENT_ID.is_empty() || super::GOOGLE_CLIENT_SECRET.is_empty() {
            assert!(creds.is_none());
        } else {
            assert!(creds.is_some());
        }
    }

    #[test]
    fn test_gmail_shares_google_credentials() {
        let google = builtin_credentials("google_oauth_token");
        let gmail = builtin_credentials("gmail_oauth_token");
        assert_eq!(google.is_some(), gmail.is_some());
        if let (Some(google), Some(gmail)) = (google, gmail) {
            assert_eq!(google.client_id, gmail.client_id);
            assert_eq!(google.client_secret, gmail.client_secret);
        }
    }

    #[test]
    fn test_gmail_scopes() {
        assert_eq!(GmailOAuthConfig::SCOPES.len(), 3);
        assert!(
            GmailOAuthConfig::SCOPES
                .iter()
                .any(|s| s.contains("gmail.readonly"))
        );
        assert!(
            GmailOAuthConfig::SCOPES
                .iter()
                .any(|s| s.contains("gmail.send"))
        );
        assert!(
            GmailOAuthConfig::SCOPES
                .iter()
                .any(|s| s.contains("pubsub"))
        );
    }

    #[test]
    fn test_generate_oauth_state_is_random_and_url_safe() {
        let a = super::generate_oauth_state();
        let b = super::generate_oauth_state();
        // 32 bytes -> 64 hex chars
        assert_eq!(a.len(), 64);
        assert_eq!(b.len(), 64);
        // Overwhelmingly unlikely to collide
        assert_ne!(a, b);
        // Hex output needs no URL-encoding
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_oauth_state_matches_constant_time() {
        let state = super::generate_oauth_state();
        assert!(super::oauth_state_matches(&state, &state));
        assert!(!super::oauth_state_matches(&state, "wrong"));
        assert!(!super::oauth_state_matches(&state, ""));
        // Same length, different content
        let mut tampered = state.clone();
        tampered.replace_range(0..1, if state.starts_with('a') { "b" } else { "a" });
        assert!(!super::oauth_state_matches(&state, &tampered));
    }

    #[tokio::test]
    async fn test_wait_for_callback_with_state_ignores_mismatch_then_accepts_match() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::{TcpListener, TcpStream};

        // Bind an ephemeral loopback listener so the test never touches the
        // fixed OAuth port or a real browser.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let expected = super::generate_oauth_state();
        let expected_for_client = expected.clone();
        let server = tokio::spawn(async move {
            super::wait_for_callback_with_state(
                listener,
                "/callback",
                "code",
                "Test",
                Some(&expected),
            )
            .await
        });

        // Simulate the browser redirect with an attacker-controlled state value.
        let mut client = TcpStream::connect(addr).await.unwrap();
        client
            .write_all(
                b"GET /callback?code=injected&state=attacker HTTP/1.1\r\nHost: localhost\r\n\r\n",
            )
            .await
            .unwrap();
        // Drain the failure landing page so the server can finish writing.
        let mut buf = Vec::new();
        let _ = client.read_to_end(&mut buf).await;

        // A forged callback is ignored rather than being allowed to cancel the
        // real authorization flow.
        let mut client = TcpStream::connect(addr).await.unwrap();
        let request = format!(
            "GET /callback?code=good-code&state={expected_for_client} HTTP/1.1\r\nHost: localhost\r\n\r\n"
        );
        client.write_all(request.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        let _ = client.read_to_end(&mut buf).await;

        let result = server.await.unwrap();
        assert_eq!(result.unwrap(), "good-code");
    }

    #[tokio::test]
    async fn test_wait_for_callback_ignores_forged_error_and_slow_client() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::{TcpListener, TcpStream};

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let expected = super::generate_oauth_state();
        let expected_for_client = expected.clone();
        let server = tokio::spawn(async move {
            super::wait_for_callback_with_state(
                listener,
                "/callback",
                "code",
                "Test",
                Some(&expected),
            )
            .await
        });

        // Hold one accepted connection without sending a request line.
        let _slow_client = TcpStream::connect(addr).await.unwrap();

        // An attacker cannot terminate the flow by supplying an error under a
        // state value they do not possess.
        let mut forged = TcpStream::connect(addr).await.unwrap();
        forged
            .write_all(
                b"GET /callback?error=access_denied&state=attacker HTTP/1.1\r\nHost: localhost\r\n\r\n",
            )
            .await
            .unwrap();
        let mut response = Vec::new();
        let _ = forged.read_to_end(&mut response).await;

        let mut valid = TcpStream::connect(addr).await.unwrap();
        let request = format!(
            "GET /callback?code=good-code&state={expected_for_client} HTTP/1.1\r\nHost: localhost\r\n\r\n"
        );
        valid.write_all(request.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        let _ = valid.read_to_end(&mut response).await;

        let result = tokio::time::timeout(std::time::Duration::from_secs(2), server)
            .await
            .expect("slow peer must not monopolize the callback listener")
            .unwrap();
        assert_eq!(result.unwrap(), "good-code");
    }

    #[tokio::test]
    async fn test_wait_for_callback_with_state_accepts_match() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::{TcpListener, TcpStream};

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let expected = super::generate_oauth_state();
        let expected_for_client = expected.clone();
        let server = tokio::spawn(async move {
            super::wait_for_callback_with_state(
                listener,
                "/callback",
                "code",
                "Test",
                Some(&expected),
            )
            .await
        });

        let mut client = TcpStream::connect(addr).await.unwrap();
        let request = format!(
            "GET /callback?code=good-code&state={expected_for_client} HTTP/1.1\r\nHost: localhost\r\n\r\n"
        );
        client.write_all(request.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        let _ = client.read_to_end(&mut buf).await;

        let result = server.await.unwrap();
        assert_eq!(result.unwrap(), "good-code");
    }

    #[test]
    fn test_gmail_auth_url_contains_required_params() {
        let Some(url) = GmailOAuthConfig::auth_url("test-state", "test-challenge") else {
            assert!(builtin_credentials("gmail_oauth_token").is_none());
            return;
        };
        assert!(url.contains("accounts.google.com"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("code_challenge=test-challenge"));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("gmail"));
    }
}
