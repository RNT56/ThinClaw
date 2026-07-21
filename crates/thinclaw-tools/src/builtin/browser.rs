//! Browser automation tool using CDP (Chrome DevTools Protocol).
//!
//! Provides the agent with web browsing capabilities: navigating pages,
//! reading content via accessibility tree snapshots, clicking elements,
//! typing text, taking screenshots, and evaluating JavaScript.
//!
//! Uses `chromiumoxide` to connect to a Chrome/Chromium instance via CDP.
//!
//! **Chrome resolution order:**
//! 1. `BROWSER_EXECUTABLE` override, then local Chrome/Chromium/Brave/Edge
//!    binary (macOS, Linux, Windows)
//! 2. Docker container with Chromium + Xvfb (automatic fallback when no
//!    local binary is found and Docker is available, or forced via
//!    `BROWSER_DOCKER=always` env var). Set `BROWSER_DOCKER=never` to disable.

use std::collections::HashMap;
use std::fmt::Write;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chromiumoxide::auth::Credentials;
use chromiumoxide::cdp::browser_protocol::accessibility::{AxNode, GetFullAxTreeParams};
use chromiumoxide::cdp::browser_protocol::dom::{BackendNodeId, FocusParams, ResolveNodeParams};
use chromiumoxide::cdp::browser_protocol::fetch::{
    ContinueRequestParams, EventRequestPaused, FailRequestParams,
};
use chromiumoxide::cdp::browser_protocol::input::{DispatchKeyEventParams, DispatchKeyEventType};
use chromiumoxide::cdp::browser_protocol::network::ErrorReason;
use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
use chromiumoxide::cdp::js_protocol::runtime::CallFunctionOnParams;
use chromiumoxide::handler::HandlerConfig;
use chromiumoxide::{Browser, BrowserConfig, Page};
use futures::StreamExt;
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, RwLock};

use super::browser_cloud::{
    CloudBrowserProvider, CloudBrowserSession, DEFAULT_CLOUD_IDLE_TIMEOUT, build_provider,
};
use thinclaw_tools_core::{
    ApprovalRequirement, Tool, ToolError, ToolOutput, is_public_outbound_ip,
};
use thinclaw_types::JobContext;

// ── Limits ───────────────────────────────────────────────────────────

/// Maximum number of open tabs before the oldest is auto-closed.
const MAX_TABS: usize = 8;

/// Character limit for `get_text` content returned to the LLM.
const TEXT_CHAR_LIMIT: usize = 16_000;

/// Navigation page-load timeout.
const PAGE_LOAD_TIMEOUT: Duration = Duration::from_secs(30);

/// Deadline for closing CDP pages, the browser, and its handler task.
const BROWSER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// Deadline for DNS validation of every intercepted network destination.
const BROWSER_DNS_TIMEOUT: Duration = Duration::from_secs(5);

/// Bound concurrent paused requests so a page cannot create unbounded guard
/// tasks or event backlog.
const MAX_INTERCEPTED_REQUESTS: usize = 64;

const MAX_BROWSER_URL_BYTES: usize = 8 * 1024;

/// Browser inputs and page-controlled outputs must stay bounded even when a
/// hostile page returns pathological CDP values.
const MAX_TYPE_TEXT_BYTES: usize = 16 * 1024;
const MAX_EVALUATE_EXPRESSION_BYTES: usize = 64 * 1024;
const MAX_EVALUATE_RESULT_BYTES: usize = 256 * 1024;
const MAX_EVALUATE_RESULT_NODES: usize = 2_048;
const MAX_SNAPSHOT_BYTES: usize = 64 * 1024;
const MAX_SNAPSHOT_NODES: usize = 4_096;
const MAX_SNAPSHOT_REFS: usize = 2_048;
const MAX_AX_ROLE_CHARS: usize = 64;
const MAX_AX_NAME_CHARS: usize = 512;
const MAX_SCREENSHOT_BYTES: usize = 16 * 1024 * 1024;
const MAX_KEY_BYTES: usize = 64;
const MAX_SCROLL_PIXELS: i64 = 100_000;

fn truncate_chars(value: &str, max_chars: usize) -> (String, bool) {
    let mut chars = value.chars();
    let bounded = chars.by_ref().take(max_chars).collect::<String>();
    (bounded, chars.next().is_some())
}

fn bounded_browser_string(value: &str, max_chars: usize) -> String {
    truncate_chars(value, max_chars).0
}

fn validate_ref_id(value: &str) -> Result<(), ToolError> {
    if value.len() < 2
        || value.len() > 16
        || !value.starts_with('e')
        || !value[1..].bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(ToolError::InvalidParameters(
            "Element ref must use the snapshot form e<number>".to_string(),
        ));
    }
    Ok(())
}

fn prepare_browser_profile_dir(path: &std::path::Path) -> Result<(), ToolError> {
    if path.as_os_str().is_empty() || path.as_os_str().to_string_lossy().len() > 4_096 {
        return Err(ToolError::ExecutionFailed(
            "Browser profile path is empty or oversized".to_string(),
        ));
    }
    std::fs::create_dir_all(path).map_err(|error| {
        ToolError::ExecutionFailed(format!(
            "Failed to create browser profile directory: {error}"
        ))
    })?;
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        ToolError::ExecutionFailed(format!(
            "Failed to inspect browser profile directory: {error}"
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ToolError::ExecutionFailed(
            "Browser profile path must be a real directory, not a symlink".to_string(),
        ));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        // SAFETY: geteuid has no preconditions and does not mutate process state.
        let effective_uid = unsafe { libc::geteuid() };
        if metadata.uid() != effective_uid {
            return Err(ToolError::ExecutionFailed(
                "Browser profile directory is not owned by the current user".to_string(),
            ));
        }
        if metadata.mode() & 0o077 != 0 {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).map_err(
                |error| {
                    ToolError::ExecutionFailed(format!(
                        "Failed to secure browser profile permissions: {error}"
                    ))
                },
            )?;
        }
    }
    Ok(())
}

fn push_bounded_text(target: &mut String, value: &str, max_bytes: usize) -> bool {
    let remaining = max_bytes.saturating_sub(target.len());
    if value.len() <= remaining {
        target.push_str(value);
        return false;
    }
    let boundary = value.floor_char_boundary(remaining);
    target.push_str(&value[..boundary]);
    true
}

/// Consume an arbitrary JSON value into a bounded shape. Chromium has already
/// decoded the CDP response at this point, but this prevents a page from
/// feeding an unbounded value into tool output, history, or the next model
/// request. The string budget is intentionally conservative because JSON
/// escaping can expand a character by up to six bytes.
fn bound_json_value(value: serde_json::Value) -> (serde_json::Value, bool) {
    fn visit(
        value: serde_json::Value,
        string_budget: &mut usize,
        nodes: &mut usize,
        truncated: &mut bool,
        depth: usize,
    ) -> serde_json::Value {
        if *nodes >= MAX_EVALUATE_RESULT_NODES || depth > 16 {
            *truncated = true;
            return serde_json::Value::String("[truncated]".to_string());
        }
        *nodes += 1;
        match value {
            serde_json::Value::String(value) => {
                let allowance = (*string_budget).min(16 * 1024);
                let (bounded, was_truncated) = truncate_chars(&value, allowance);
                *string_budget = string_budget.saturating_sub(bounded.chars().count());
                *truncated |= was_truncated;
                serde_json::Value::String(bounded)
            }
            serde_json::Value::Array(values) => {
                let original_len = values.len();
                let mut bounded = Vec::with_capacity(values.len().min(256));
                for value in values.into_iter().take(256) {
                    bounded.push(visit(value, string_budget, nodes, truncated, depth + 1));
                    if *nodes >= MAX_EVALUATE_RESULT_NODES {
                        break;
                    }
                }
                *truncated |= bounded.len() < original_len;
                serde_json::Value::Array(bounded)
            }
            serde_json::Value::Object(values) => {
                let original_len = values.len();
                let mut bounded = serde_json::Map::new();
                for (key, value) in values.into_iter().take(256) {
                    let allowance = (*string_budget).min(1_024);
                    let (key, key_truncated) = truncate_chars(&key, allowance);
                    *string_budget = string_budget.saturating_sub(key.chars().count());
                    *truncated |= key_truncated;
                    bounded.insert(
                        key,
                        visit(value, string_budget, nodes, truncated, depth + 1),
                    );
                    if *nodes >= MAX_EVALUATE_RESULT_NODES {
                        break;
                    }
                }
                *truncated |= bounded.len() < original_len;
                serde_json::Value::Object(bounded)
            }
            scalar => scalar,
        }
    }

    let mut string_budget = MAX_EVALUATE_RESULT_BYTES / 8;
    let mut nodes = 0;
    let mut truncated = false;
    let bounded = visit(value, &mut string_budget, &mut nodes, &mut truncated, 0);
    // The conservative budgets above should keep this below the hard limit.
    // Retain a final fail-closed check in case structural overhead changes.
    if serde_json::to_vec(&bounded).is_ok_and(|bytes| bytes.len() <= MAX_EVALUATE_RESULT_BYTES) {
        (bounded, truncated)
    } else {
        (
            serde_json::json!({
                "error": "JavaScript result exceeded the browser output limit"
            }),
            true,
        )
    }
}

// ── Navigation guard ─────────────────────────────────────────────────

/// Hostnames whose namespaces are local or infrastructure-defined.
const BLOCKED_HOST_SUFFIXES: &[&str] = &[
    "localhost",
    "local",
    "internal",
    "home.arpa",
    "metadata.google.internal",
];

fn is_url_allowed(raw_url: &str) -> Result<(), String> {
    validate_browser_url(raw_url, false).map(|_| ())
}

fn validate_browser_url(raw_url: &str, allow_websocket: bool) -> Result<(String, u16), String> {
    if raw_url.is_empty() || raw_url.len() > MAX_BROWSER_URL_BYTES {
        return Err(format!(
            "URL must be non-empty and at most {MAX_BROWSER_URL_BYTES} bytes"
        ));
    }
    let parsed = url::Url::parse(raw_url).map_err(|e| format!("Invalid URL: {e}"))?;

    let default_port = match parsed.scheme() {
        "http" => 80,
        "https" => 443,
        "ws" if allow_websocket => 80,
        "wss" if allow_websocket => 443,
        scheme => return Err(format!("Blocked browser URL scheme: {scheme}")),
    };
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("Browser URLs cannot contain embedded credentials".to_string());
    }

    let (host, literal_ip) = match parsed
        .host()
        .ok_or_else(|| "Browser URL must contain a host".to_string())?
    {
        url::Host::Domain(domain) => (domain.trim_end_matches('.').to_ascii_lowercase(), None),
        url::Host::Ipv4(ip) => (ip.to_string(), Some(IpAddr::V4(ip))),
        url::Host::Ipv6(ip) => (ip.to_string(), Some(IpAddr::V6(ip))),
    };
    if host.is_empty() || host.len() > 253 {
        return Err("Browser URL host is empty or too long".to_string());
    }
    if literal_ip.is_none() {
        for suffix in BLOCKED_HOST_SUFFIXES {
            if host == *suffix || host.ends_with(&format!(".{suffix}")) {
                return Err(format!("Blocked host: {host}"));
            }
        }
    }

    if let Some(ip) = literal_ip
        && !is_public_outbound_ip(ip)
    {
        return Err(format!("Blocked non-public address: {ip}"));
    }

    let port = parsed.port().unwrap_or(default_port);
    // The pinned egress proxy intentionally supports only ordinary HTTP and
    // HTTPS. Reject other ports here so local, Docker, and cloud modes expose
    // the same network surface and cannot be used as generic TCP clients.
    if port != default_port {
        return Err(format!(
            "Browser egress is restricted to the standard {default_port} port"
        ));
    }

    Ok((host, port))
}

pub(crate) async fn is_network_url_allowed(raw_url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(raw_url).map_err(|error| format!("Invalid URL: {error}"))?;
    if matches!(parsed.scheme(), "data" | "blob") {
        // These schemes do not initiate a new network connection. Top-level
        // navigation still rejects them through `is_url_allowed`.
        return Ok(());
    }
    let (host, port) = validate_browser_url(raw_url, true)?;
    if let Ok(ip) = host.parse::<IpAddr>() {
        return is_public_outbound_ip(ip)
            .then_some(())
            .ok_or_else(|| format!("Blocked non-public address: {ip}"));
    }

    let addresses = tokio::time::timeout(
        BROWSER_DNS_TIMEOUT,
        tokio::net::lookup_host((host.as_str(), port)),
    )
    .await
    .map_err(|_| format!("DNS lookup for {host} timed out"))?
    .map_err(|error| format!("DNS lookup for {host} failed: {error}"))?
    .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err(format!("DNS lookup for {host} returned no addresses"));
    }
    if let Some(blocked) = addresses
        .iter()
        .find(|address| !is_public_outbound_ip(address.ip()))
    {
        return Err(format!(
            "Blocked host {host}: it resolves to non-public address {}",
            blocked.ip()
        ));
    }
    Ok(())
}

// ── Interactive role detection ───────────────────────────────────────

fn is_interactive_role(role: &str) -> bool {
    matches!(
        role,
        "button"
            | "link"
            | "textbox"
            | "checkbox"
            | "radio"
            | "combobox"
            | "menuitem"
            | "tab"
            | "slider"
            | "switch"
            | "searchbox"
            | "spinbutton"
            | "option"
            | "menuitemcheckbox"
            | "menuitemradio"
    )
}

// ── AX node ref tracking ─────────────────────────────────────────────

/// Reference to an accessibility tree node, mapped from a short ref ID.
#[derive(Debug, Clone)]
struct AXNodeRef {
    /// CDP backend node ID for DOM interaction.
    backend_node_id: Option<BackendNodeId>,
    /// The accessibility role.
    role: String,
    /// The accessible name.
    name: String,
}

/// State for a single browser page/tab.
struct PageState {
    page: Page,
    /// Guards redirects, subresources, fetch/XHR, and WebSocket destinations.
    request_guard_handle: tokio::task::JoinHandle<()>,
    /// Cached accessibility tree refs from last snapshot.
    role_refs: HashMap<String, AXNodeRef>,
}

impl Drop for PageState {
    fn drop(&mut self) {
        self.request_guard_handle.abort();
    }
}

struct PendingPageGuard {
    page: Option<Page>,
    request_guard_abort: Option<tokio::task::AbortHandle>,
}

impl PendingPageGuard {
    fn new(page: Page) -> Self {
        Self {
            page: Some(page),
            request_guard_abort: None,
        }
    }

    fn own_request_guard(&mut self, handle: &tokio::task::JoinHandle<()>) {
        self.request_guard_abort = Some(handle.abort_handle());
    }

    fn disarm(&mut self) {
        self.page = None;
        self.request_guard_abort = None;
    }
}

impl Drop for PendingPageGuard {
    fn drop(&mut self) {
        if let Some(handle) = self.request_guard_abort.take() {
            handle.abort();
        }
        let Some(page) = self.page.take() else {
            return;
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            tracing::error!("Unable to schedule cleanup for pending browser page");
            return;
        };
        handle.spawn(async move {
            let _ = tokio::time::timeout(BROWSER_SHUTDOWN_TIMEOUT, page.close()).await;
        });
    }
}

struct OwnedScreenshotPath(PathBuf);

impl Drop for OwnedScreenshotPath {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_file(&self.0)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::debug!(path = %self.0.display(), %error, "Failed to remove browser screenshot");
        }
    }
}

/// Shared browser instance with page tracking.
struct BrowserInstance {
    browser: Browser,
    /// Drives the CDP websocket and must not detach from the browser lifecycle.
    handler_handle: tokio::task::JoinHandle<()>,
    /// Active pages by tab index.
    pages: Vec<PageState>,
    /// The "current" page index.
    current_page: Option<usize>,
    /// Ephemeral Chrome profile. Dropping it removes cookies, local storage,
    /// and other page-controlled state rather than sharing them across jobs.
    _profile_dir: Option<tempfile::TempDir>,
    /// Whether this instance is connected to a Docker container.
    is_docker: bool,
    /// Whether this local browser owns the root-provided pinned egress proxy.
    uses_local_egress: bool,
    /// Credentials installed into every new page for proxy challenges.
    proxy_credentials: Option<Credentials>,
    /// Cloud session metadata when connected to a managed remote browser.
    cloud_session: Option<CloudBrowserSession>,
    /// Cancellation-safe ownership of the exact external runtime/provider.
    cleanup_guard: BrowserInstanceCleanupGuard,
    /// Security scope that owns every tab and credential in this instance.
    owner_scope: BrowserSessionScope,
    /// Last observed tool activity for idle cloud-session cleanup.
    last_activity: Instant,
    /// Most recent screenshot owned by this instance. Replaced on the next
    /// capture and removed during shutdown so temporary files do not leak.
    screenshot_path: Option<OwnedScreenshotPath>,
}

impl Drop for BrowserInstance {
    fn drop(&mut self) {
        self.handler_handle.abort();
    }
}

type SharedBrowser = Arc<RwLock<Option<BrowserInstance>>>;

/// Opaque per-principal conversation scope. Browser state may be reused by
/// follow-up jobs in one conversation, but never by another principal or an
/// unrelated job that has no conversation.
#[derive(Clone, Copy, PartialEq, Eq)]
struct BrowserSessionScope([u8; 32]);

impl BrowserSessionScope {
    fn from_context(context: &JobContext) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"thinclaw-browser-session-scope-v1\0");
        hasher.update(&(context.principal_id.len() as u64).to_le_bytes());
        hasher.update(context.principal_id.as_bytes());
        match context.conversation_id {
            Some(conversation_id) => {
                hasher.update(b"conversation\0");
                hasher.update(conversation_id.as_bytes());
            }
            None => {
                hasher.update(b"job\0");
                hasher.update(context.job_id.as_bytes());
            }
        }
        Self(*hasher.finalize().as_bytes())
    }
}

/// Root-provided Docker Chromium lifecycle used by the browser tool.
#[async_trait]
pub trait BrowserDockerRuntime: Send + Sync {
    fn image_label(&self) -> String;
    fn http_endpoint(&self) -> String;
    fn is_available(&self) -> bool;

    /// Credentials for the authenticated egress proxy used by the isolated
    /// Docker browser. Local and cloud runtimes leave this unset.
    fn proxy_credentials(&self) -> Option<(String, String)> {
        None
    }

    async fn start(&self) -> Result<(), String>;
    async fn wait_for_ready(&self, timeout: Duration) -> Result<(), String>;
    async fn stop(&self) -> Result<(), String>;
}

/// Root-provided host proxy used by local Chrome. It resolves every target,
/// rejects non-public addresses, and connects to the validated socket address
/// itself, eliminating the DNS-validation/Chrome-resolution race.
#[derive(Clone)]
pub struct BrowserProxyConfig {
    pub endpoint: String,
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for BrowserProxyConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BrowserProxyConfig")
            .field("endpoint", &"[REDACTED URL]")
            .field("username", &"[REDACTED]")
            .field("password", &"[REDACTED]")
            .finish()
    }
}

impl BrowserProxyConfig {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.endpoint.len() > MAX_BROWSER_URL_BYTES
            || self.username.is_empty()
            || self.username.len() > 128
            || self.password.is_empty()
            || self.password.len() > 512
            || self.username.chars().any(char::is_control)
            || self.password.chars().any(char::is_control)
        {
            return Err("Browser proxy configuration is malformed".to_string());
        }
        let endpoint = url::Url::parse(&self.endpoint)
            .map_err(|error| format!("Invalid browser proxy endpoint: {error}"))?;
        if endpoint.scheme() != "http"
            || endpoint.host_str() != Some("127.0.0.1")
            || endpoint.port().is_none()
            || endpoint.path() != "/"
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
        {
            return Err(
                "Browser proxy must be an authenticated loopback HTTP endpoint".to_string(),
            );
        }
        Ok(())
    }

    pub(crate) fn authenticated_endpoint(&self) -> Result<String, String> {
        self.validate()?;
        let mut endpoint = url::Url::parse(&self.endpoint)
            .map_err(|error| format!("Invalid browser proxy endpoint: {error}"))?;
        endpoint
            .set_username(&self.username)
            .map_err(|()| "failed to encode browser proxy username".to_string())?;
        endpoint
            .set_password(Some(&self.password))
            .map_err(|()| "failed to encode browser proxy password".to_string())?;
        Ok(endpoint.to_string())
    }

    fn credentials(&self) -> Credentials {
        Credentials {
            username: self.username.clone(),
            password: self.password.clone(),
        }
    }
}

#[async_trait]
pub trait BrowserEgressRuntime: Send + Sync {
    async fn start(&self) -> Result<BrowserProxyConfig, String>;
    async fn stop(&self) -> Result<(), String>;
}

enum PendingBrowserRuntime {
    Docker(Arc<dyn BrowserDockerRuntime>),
    Egress(Arc<dyn BrowserEgressRuntime>),
}

/// Ensures cancellation between starting an external runtime and publishing a
/// `BrowserInstance` cannot orphan a proxy, relay, or container.
struct PendingBrowserRuntimeGuard {
    runtime: Option<PendingBrowserRuntime>,
}

impl PendingBrowserRuntimeGuard {
    fn docker(runtime: Arc<dyn BrowserDockerRuntime>) -> Self {
        Self {
            runtime: Some(PendingBrowserRuntime::Docker(runtime)),
        }
    }

    fn egress(runtime: Arc<dyn BrowserEgressRuntime>) -> Self {
        Self {
            runtime: Some(PendingBrowserRuntime::Egress(runtime)),
        }
    }

    fn disarm(&mut self) {
        self.runtime = None;
    }
}

impl Drop for PendingBrowserRuntimeGuard {
    fn drop(&mut self) {
        let Some(runtime) = self.runtime.take() else {
            return;
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            tracing::error!("Unable to schedule cleanup for pending browser runtime");
            return;
        };
        handle.spawn(async move {
            let result = match runtime {
                PendingBrowserRuntime::Docker(runtime) => runtime.stop().await,
                PendingBrowserRuntime::Egress(runtime) => runtime.stop().await,
            };
            if let Err(error) = result {
                tracing::warn!(%error, "Failed to clean cancelled browser runtime start");
            }
        });
    }
}

/// Browserbase keep-alive sessions need an explicit release request. Retain a
/// cancellation guard until ownership has moved into `BrowserInstance`.
struct PendingCloudSessionGuard {
    pending: Option<(Arc<dyn CloudBrowserProvider>, CloudBrowserSession)>,
}

impl PendingCloudSessionGuard {
    fn new(provider: Arc<dyn CloudBrowserProvider>, session: CloudBrowserSession) -> Self {
        Self {
            pending: Some((provider, session)),
        }
    }

    fn disarm(&mut self) {
        self.pending = None;
    }
}

impl Drop for PendingCloudSessionGuard {
    fn drop(&mut self) {
        let Some((provider, session)) = self.pending.take() else {
            return;
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            tracing::error!("Unable to schedule cleanup for pending cloud browser session");
            return;
        };
        handle.spawn(async move {
            if let Err(error) = provider.close_session(&session).await {
                tracing::warn!(%error, "Failed to clean cancelled cloud browser session start");
            }
        });
    }
}

enum OwnedBrowserRuntime {
    Docker(Arc<dyn BrowserDockerRuntime>),
    Egress(Arc<dyn BrowserEgressRuntime>),
    Cloud {
        provider: Arc<dyn CloudBrowserProvider>,
        session: CloudBrowserSession,
    },
}

/// Once a browser is published, this guard is its authoritative external
/// resource owner. If an action or shutdown future is cancelled, `Drop`
/// schedules idempotent cleanup instead of detaching infrastructure.
struct BrowserInstanceCleanupGuard {
    runtime: Option<OwnedBrowserRuntime>,
}

impl BrowserInstanceCleanupGuard {
    fn docker(runtime: Arc<dyn BrowserDockerRuntime>) -> Self {
        Self {
            runtime: Some(OwnedBrowserRuntime::Docker(runtime)),
        }
    }

    fn egress(runtime: Arc<dyn BrowserEgressRuntime>) -> Self {
        Self {
            runtime: Some(OwnedBrowserRuntime::Egress(runtime)),
        }
    }

    fn cloud(provider: Arc<dyn CloudBrowserProvider>, session: CloudBrowserSession) -> Self {
        Self {
            runtime: Some(OwnedBrowserRuntime::Cloud { provider, session }),
        }
    }

    async fn cleanup(&mut self) -> Result<(), String> {
        let result = match self.runtime.as_ref() {
            Some(OwnedBrowserRuntime::Docker(runtime)) => runtime.stop().await,
            Some(OwnedBrowserRuntime::Egress(runtime)) => runtime.stop().await,
            Some(OwnedBrowserRuntime::Cloud { provider, session }) => provider
                .close_session(session)
                .await
                .map_err(|error| error.to_string()),
            None => return Ok(()),
        };
        if result.is_ok() {
            self.runtime = None;
        }
        result
    }
}

impl Drop for BrowserInstanceCleanupGuard {
    fn drop(&mut self) {
        let Some(runtime) = self.runtime.take() else {
            return;
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            tracing::error!("Unable to schedule cleanup for dropped browser instance");
            return;
        };
        handle.spawn(async move {
            let result = match runtime {
                OwnedBrowserRuntime::Docker(runtime) => runtime.stop().await,
                OwnedBrowserRuntime::Egress(runtime) => runtime.stop().await,
                OwnedBrowserRuntime::Cloud { provider, session } => provider
                    .close_session(&session)
                    .await
                    .map_err(|error| error.to_string()),
            };
            if let Err(error) = result {
                tracing::warn!(%error, "Failed to clean dropped browser instance");
            }
        });
    }
}

/// Browser automation tool.
///
/// Exposes browser actions (navigate, snapshot, click, type, screenshot, eval)
/// as a single unified tool with an `action` parameter. The tool lazily
/// launches Chrome on first use and reuses the instance.
///
/// When no local Chrome binary is found, the tool automatically falls back to
/// running Chromium inside a Docker container (if Docker is available).
/// Set `BROWSER_DOCKER=always` to force Docker mode even when a local browser
/// exists, or `BROWSER_DOCKER=never` to disable the fallback.
pub struct BrowserTool {
    instance: SharedBrowser,
    /// Browser operations are stateful and must remain atomic across the
    /// scope check, CDP operation, and state update.
    operation_lock: Mutex<()>,
    active_scope: std::sync::Mutex<Option<BrowserSessionScope>>,
    profile_dir: PathBuf,
    /// Docker config for Chromium fallback (or forced mode).
    docker_config: Option<Arc<dyn BrowserDockerRuntime>>,
    /// Pinned public-network proxy required for a locally launched browser.
    egress_runtime: Option<Arc<dyn BrowserEgressRuntime>>,
    /// Optional cloud browser provider selection.
    cloud_provider: Option<String>,
}

impl BrowserTool {
    /// Create a BrowserTool that uses local Chrome.
    pub fn new(profile_dir: PathBuf) -> Self {
        Self {
            instance: Arc::new(RwLock::new(None)),
            operation_lock: Mutex::new(()),
            active_scope: std::sync::Mutex::new(None),
            profile_dir,
            docker_config: None,
            egress_runtime: None,
            cloud_provider: None,
        }
    }

    /// Create a local browser backed by a root-owned pinned egress proxy.
    pub fn new_with_egress(
        profile_dir: PathBuf,
        egress_runtime: Arc<dyn BrowserEgressRuntime>,
    ) -> Self {
        Self {
            instance: Arc::new(RwLock::new(None)),
            operation_lock: Mutex::new(()),
            active_scope: std::sync::Mutex::new(None),
            profile_dir,
            docker_config: None,
            egress_runtime: Some(egress_runtime),
            cloud_provider: None,
        }
    }

    /// Create a BrowserTool with Docker Chromium fallback (or forced mode).
    pub fn new_with_docker(
        profile_dir: PathBuf,
        docker_config: Arc<dyn BrowserDockerRuntime>,
    ) -> Self {
        Self {
            instance: Arc::new(RwLock::new(None)),
            operation_lock: Mutex::new(()),
            active_scope: std::sync::Mutex::new(None),
            profile_dir,
            docker_config: Some(docker_config),
            egress_runtime: None,
            cloud_provider: None,
        }
    }

    /// Create a browser that uses pinned local egress when Chrome is present
    /// and the isolated Docker runtime as its fallback.
    pub fn new_with_docker_and_egress(
        profile_dir: PathBuf,
        docker_config: Arc<dyn BrowserDockerRuntime>,
        egress_runtime: Arc<dyn BrowserEgressRuntime>,
    ) -> Self {
        Self {
            instance: Arc::new(RwLock::new(None)),
            operation_lock: Mutex::new(()),
            active_scope: std::sync::Mutex::new(None),
            profile_dir,
            docker_config: Some(docker_config),
            egress_runtime: Some(egress_runtime),
            cloud_provider: None,
        }
    }

    /// Create a BrowserTool that prefers a managed cloud browser provider.
    pub fn new_with_cloud(profile_dir: PathBuf, cloud_provider: Option<String>) -> Self {
        Self {
            instance: Arc::new(RwLock::new(None)),
            operation_lock: Mutex::new(()),
            active_scope: std::sync::Mutex::new(None),
            profile_dir,
            docker_config: None,
            egress_runtime: None,
            cloud_provider,
        }
    }

    /// Find Chrome/Chromium/Brave/Edge executable on the system.
    fn find_chrome() -> Option<PathBuf> {
        thinclaw_platform::find_browser_executable()
    }

    fn execution_scope(&self) -> Result<BrowserSessionScope, ToolError> {
        self.active_scope
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .copied()
            .ok_or_else(|| {
                ToolError::ExecutionFailed(
                    "Browser operation has no authenticated job scope".to_string(),
                )
            })
    }

    async fn activate_scope(&self, scope: BrowserSessionScope) {
        *self
            .active_scope
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(scope);

        let stale = {
            let mut instance = self.instance.write().await;
            if instance
                .as_ref()
                .is_some_and(|instance| instance.owner_scope != scope)
            {
                instance.take()
            } else {
                None
            }
        };
        if let Some(instance) = stale {
            tracing::info!("Closing browser session at authenticated scope boundary");
            self.shutdown_browser_instance(instance).await;
        }
    }

    /// Get or launch the browser instance.
    ///
    /// Resolution order:
    /// 1. If `BROWSER_DOCKER=always`, skip local browser and use Docker.
    /// 2. Try to find and launch a local Chrome/Chromium/Brave/Edge binary.
    /// 3. If no local binary found and Docker is available with a
    ///    `DockerChromiumConfig`, start a container and connect via CDP.
    ///
    /// If Chrome was previously launched but has since crashed (CDP connection
    /// lost), the dead instance is dropped and a fresh one is started.
    async fn ensure_browser(&self) -> Result<(), ToolError> {
        let owner_scope = self.execution_scope()?;
        let mut guard = self.instance.write().await;

        // This should already have been handled by `activate_scope`, but keep a
        // defensive check adjacent to browser reuse so future internal callers
        // cannot accidentally cross the identity boundary.
        if guard
            .as_ref()
            .is_some_and(|instance| instance.owner_scope != owner_scope)
        {
            let stale = guard.take();
            if let Some(instance) = stale {
                self.shutdown_browser_instance(instance).await;
            }
        }

        // If we have an instance, verify Chrome is still alive by pinging CDP.
        let mut stale_instance = None;
        if let Some(ref instance) = *guard {
            let should_expire_cloud = instance.cloud_session.is_some()
                && instance.last_activity.elapsed() >= DEFAULT_CLOUD_IDLE_TIMEOUT;
            let lifecycle_task_failed = instance.handler_handle.is_finished()
                || instance
                    .pages
                    .iter()
                    .any(|page| page.request_guard_handle.is_finished());
            if should_expire_cloud || lifecycle_task_failed {
                tracing::info!(
                    timeout_secs = DEFAULT_CLOUD_IDLE_TIMEOUT.as_secs(),
                    lifecycle_task_failed,
                    "Closing stale browser session before reconnect"
                );
                stale_instance = guard.take();
            } else {
                // `browser.pages()` makes a CDP call — if the process died this
                // will return an error, signalling we must re-launch.
                if instance.browser.pages().await.is_err() {
                    tracing::warn!("Chrome process appears dead, re-launching");
                    stale_instance = guard.take();
                } else {
                    return Ok(());
                }
            }
        }

        if let Some(instance) = stale_instance.take() {
            self.shutdown_browser_instance(instance).await;
        }

        if let Some(provider) = build_provider(self.cloud_provider.as_deref())? {
            return self
                .connect_cloud_chrome(&mut guard, provider, owner_scope)
                .await;
        }

        let docker_mode =
            thinclaw_platform::BrowserDockerMode::from_env().map_err(ToolError::ExecutionFailed)?;

        // Try local Chrome first (unless Docker is forced).
        if !docker_mode.forces_docker()
            && let Some(chrome_path) = Self::find_chrome()
        {
            return self
                .launch_local_chrome(&mut guard, chrome_path, owner_scope)
                .await;
        }

        // Fall back to Docker Chromium.
        if docker_mode.allows_docker()
            && let Some(ref docker_config) = self.docker_config
        {
            if docker_config.is_available() {
                return self
                    .connect_docker_chrome(&mut guard, docker_config, owner_scope)
                    .await;
            }
            tracing::warn!("Docker not available for browser fallback");
        }

        // Neither local Chrome nor Docker available.
        Err(ToolError::ExecutionFailed(if cfg!(target_os = "windows") {
            "Chrome, Edge, or Brave not found. Install a supported local browser, set BROWSER_EXECUTABLE, or install Docker Desktop and set BROWSER_DOCKER=auto/always."
                .to_string()
        } else {
            "Chrome, Chromium, Edge, or Brave not found. Install a supported local browser, \
             set BROWSER_EXECUTABLE to its path, or install Docker and use BROWSER_DOCKER=auto/always. \
             On macOS: brew install --cask google-chrome. On Linux: apt install chromium-browser \
             or install google-chrome-stable, brave-browser, or microsoft-edge-stable."
                .to_string()
        }))
    }

    /// Launch a local Chrome binary and store the instance.
    async fn launch_local_chrome(
        &self,
        guard: &mut tokio::sync::RwLockWriteGuard<'_, Option<BrowserInstance>>,
        chrome_path: PathBuf,
        owner_scope: BrowserSessionScope,
    ) -> Result<(), ToolError> {
        prepare_browser_profile_dir(&self.profile_dir)?;
        let profile_dir = tempfile::Builder::new()
            .prefix("session-")
            .tempdir_in(&self.profile_dir)
            .map_err(|error| {
                ToolError::ExecutionFailed(format!(
                    "Failed to create ephemeral browser profile: {error}"
                ))
            })?;
        prepare_browser_profile_dir(profile_dir.path())?;
        let egress_runtime = self.egress_runtime.as_ref().cloned().ok_or_else(|| {
            ToolError::ExecutionFailed(
                "Local browser launch requires ThinClaw's pinned egress proxy".to_string(),
            )
        })?;
        let proxy = egress_runtime.start().await.map_err(|error| {
            ToolError::ExecutionFailed(format!("Failed to start local browser proxy: {error}"))
        })?;
        let mut runtime_cleanup = PendingBrowserRuntimeGuard::egress(Arc::clone(&egress_runtime));
        if let Err(error) = proxy.validate() {
            let _ = egress_runtime.stop().await;
            runtime_cleanup.disarm();
            return Err(ToolError::ExecutionFailed(error));
        }

        let config = match BrowserConfig::builder()
            .chrome_executable(chrome_path)
            .user_data_dir(profile_dir.path())
            .window_size(1280, 900)
            // Use the new headless mode via the proper builder method.
            .new_headless_mode()
            .enable_request_intercept()
            .disable_cache()
            .respect_https_errors()
            // Only pass args NOT already in chromiumoxide's DEFAULT_ARGS.
            .arg("disable-gpu")
            .arg("no-default-browser-check")
            .arg(format!("proxy-server={}", proxy.endpoint))
            .arg("proxy-bypass-list=<-loopback>")
            .arg("disable-quic")
            .arg("force-webrtc-ip-handling-policy=disable_non_proxied_udp")
            .arg("webrtc-ip-handling-policy=disable_non_proxied_udp")
            .build()
        {
            Ok(config) => config,
            Err(error) => {
                let _ = egress_runtime.stop().await;
                runtime_cleanup.disarm();
                return Err(ToolError::ExecutionFailed(format!(
                    "BrowserConfig error: {error}"
                )));
            }
        };

        let (browser, mut handler) = match Browser::launch(config).await {
            Ok(browser) => browser,
            Err(error) => {
                let _ = egress_runtime.stop().await;
                runtime_cleanup.disarm();
                return Err(ToolError::ExecutionFailed(format!(
                    "Failed to launch Chrome: {error}"
                )));
            }
        };

        // Spawn the CDP handler loop (Handler implements futures::Stream)
        let handler_handle = tokio::spawn(async move { while handler.next().await.is_some() {} });

        tracing::info!("Chrome launched locally for browser tool");

        **guard = Some(BrowserInstance {
            browser,
            handler_handle,
            pages: Vec::new(),
            current_page: None,
            _profile_dir: Some(profile_dir),
            is_docker: false,
            uses_local_egress: true,
            proxy_credentials: Some(proxy.credentials()),
            cloud_session: None,
            cleanup_guard: BrowserInstanceCleanupGuard::egress(egress_runtime),
            owner_scope,
            last_activity: Instant::now(),
            screenshot_path: None,
        });
        runtime_cleanup.disarm();

        Ok(())
    }

    /// Start a Docker container running Chromium and connect via CDP.
    async fn connect_docker_chrome(
        &self,
        guard: &mut tokio::sync::RwLockWriteGuard<'_, Option<BrowserInstance>>,
        docker_config: &Arc<dyn BrowserDockerRuntime>,
        owner_scope: BrowserSessionScope,
    ) -> Result<(), ToolError> {
        // Start the container (idempotent — re-uses running container).
        docker_config.start().await.map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to start Docker Chromium: {e}"))
        })?;
        let mut runtime_cleanup = PendingBrowserRuntimeGuard::docker(Arc::clone(docker_config));

        // Wait for Chrome inside the container to be ready.
        if let Err(error) = docker_config.wait_for_ready(Duration::from_secs(30)).await {
            let _ = docker_config.stop().await;
            runtime_cleanup.disarm();
            return Err(ToolError::ExecutionFailed(format!(
                "Docker Chromium not ready: {error}. Is the image `{}` available?",
                docker_config.image_label()
            )));
        }
        let proxy_credentials = match docker_config.proxy_credentials() {
            Some((username, password))
                if !username.is_empty()
                    && username.len() <= 128
                    && !password.is_empty()
                    && password.len() <= 512
                    && !username.chars().any(char::is_control)
                    && !password.chars().any(char::is_control) =>
            {
                Credentials { username, password }
            }
            _ => {
                let _ = docker_config.stop().await;
                runtime_cleanup.disarm();
                return Err(ToolError::ExecutionFailed(
                    "Docker Chromium did not provide valid proxy credentials".to_string(),
                ));
            }
        };

        // Connect to Chrome via its HTTP endpoint. `chromiumoxide` will
        // automatically discover the WebSocket URL from /json/version.
        let endpoint = docker_config.http_endpoint();
        let handler_config = HandlerConfig {
            ignore_https_errors: false,
            request_intercept: true,
            cache_enabled: false,
            ..HandlerConfig::default()
        };
        let (browser, mut handler) =
            match Browser::connect_with_config(&endpoint, handler_config).await {
                Ok(connection) => connection,
                Err(error) => {
                    let _ = docker_config.stop().await;
                    runtime_cleanup.disarm();
                    return Err(ToolError::ExecutionFailed(format!(
                        "Failed to connect to Docker Chromium at {endpoint}: {error}"
                    )));
                }
            };

        // Spawn the CDP handler loop.
        let handler_handle = tokio::spawn(async move { while handler.next().await.is_some() {} });

        tracing::info!(
            endpoint = %endpoint,
            "Connected to Docker Chromium for browser tool"
        );

        **guard = Some(BrowserInstance {
            browser,
            handler_handle,
            pages: Vec::new(),
            current_page: None,
            _profile_dir: None,
            is_docker: true,
            uses_local_egress: false,
            proxy_credentials: Some(proxy_credentials),
            cloud_session: None,
            cleanup_guard: BrowserInstanceCleanupGuard::docker(Arc::clone(docker_config)),
            owner_scope,
            last_activity: Instant::now(),
            screenshot_path: None,
        });
        runtime_cleanup.disarm();

        Ok(())
    }

    /// Connect to a managed cloud browser provider over remote CDP.
    async fn connect_cloud_chrome(
        &self,
        guard: &mut tokio::sync::RwLockWriteGuard<'_, Option<BrowserInstance>>,
        provider: Arc<dyn CloudBrowserProvider>,
        owner_scope: BrowserSessionScope,
    ) -> Result<(), ToolError> {
        let session = provider.create_session().await?;
        let mut session_cleanup =
            PendingCloudSessionGuard::new(Arc::clone(&provider), session.clone());
        let connect_url = session.connect_url.clone();
        let endpoint_label = session.endpoint_label();
        let label = session.label.clone();

        let handler_config = HandlerConfig {
            ignore_https_errors: false,
            request_intercept: true,
            cache_enabled: false,
            ..HandlerConfig::default()
        };
        let (browser, mut handler) =
            match Browser::connect_with_config(&connect_url, handler_config).await {
                Ok(connection) => connection,
                Err(_error) => {
                    let _ = provider.close_session(&session).await;
                    session_cleanup.disarm();
                    return Err(ToolError::ExecutionFailed(format!(
                        "Failed to connect to {label} at {endpoint_label}"
                    )));
                }
            };

        let handler_handle = tokio::spawn(async move { while handler.next().await.is_some() {} });

        tracing::info!(
            provider = provider.label(),
            endpoint = %endpoint_label,
            "Connected to managed cloud browser"
        );

        **guard = Some(BrowserInstance {
            browser,
            handler_handle,
            pages: Vec::new(),
            current_page: None,
            _profile_dir: None,
            is_docker: false,
            uses_local_egress: false,
            proxy_credentials: None,
            cloud_session: Some(session.clone()),
            cleanup_guard: BrowserInstanceCleanupGuard::cloud(provider, session),
            owner_scope,
            last_activity: Instant::now(),
            screenshot_path: None,
        });
        session_cleanup.disarm();

        Ok(())
    }

    async fn shutdown_browser_instance(&self, mut instance: BrowserInstance) {
        let was_docker = instance.is_docker;
        let tabs = instance.pages.len();
        drop(instance.screenshot_path.take());

        let close_pages = async {
            for state in instance.pages.drain(..) {
                Self::close_page_state(state).await;
            }
        };
        let _ = tokio::time::timeout(BROWSER_SHUTDOWN_TIMEOUT, close_pages).await;

        match tokio::time::timeout(BROWSER_SHUTDOWN_TIMEOUT, instance.browser.close()).await {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => tracing::warn!(%error, "Browser close request failed"),
            Err(_) => tracing::warn!("Browser did not acknowledge close before timeout"),
        }

        if instance.browser.get_mut_child().is_some() {
            let needs_kill =
                match tokio::time::timeout(BROWSER_SHUTDOWN_TIMEOUT, instance.browser.wait()).await
                {
                    Ok(Ok(_)) => false,
                    Ok(Err(error)) => {
                        tracing::warn!(%error, "Failed to wait for local browser process");
                        true
                    }
                    Err(_) => true,
                };
            if needs_kill && let Some(Err(error)) = instance.browser.kill().await {
                tracing::warn!(%error, "Failed to kill local browser process");
            }
        }
        if tokio::time::timeout(BROWSER_SHUTDOWN_TIMEOUT, &mut instance.handler_handle)
            .await
            .is_err()
        {
            instance.handler_handle.abort();
            let _ = (&mut instance.handler_handle).await;
        }

        if let Err(error) = instance.cleanup_guard.cleanup().await {
            // The guard remains armed and retries from `Drop`, including when
            // this shutdown future itself is cancelled.
            tracing::warn!(%error, "Failed to release browser runtime cleanly");
        }

        tracing::debug!(tabs, was_docker, "Browser instance shut down");
    }

    fn touch_activity(instance: &mut BrowserInstance) {
        instance.last_activity = Instant::now();
    }

    async fn start_request_guard(page: &Page) -> Result<tokio::task::JoinHandle<()>, ToolError> {
        let mut requests = page
            .event_listener::<EventRequestPaused>()
            .await
            .map_err(|error| {
                ToolError::ExecutionFailed(format!(
                    "Failed to install browser network guard: {error}"
                ))
            })?;
        let guarded_page = page.clone();
        Ok(tokio::spawn(async move {
            let mut decisions = tokio::task::JoinSet::new();
            while let Some(event) = requests.next().await {
                while let Some(result) = decisions.try_join_next() {
                    if let Err(error) = result {
                        tracing::warn!(%error, "Browser request-guard task failed");
                    }
                }

                if decisions.len() >= MAX_INTERCEPTED_REQUESTS {
                    tracing::warn!(
                        limit = MAX_INTERCEPTED_REQUESTS,
                        "Browser page exceeded concurrent request-guard capacity; closing page"
                    );
                    let _ = guarded_page.close().await;
                    break;
                }

                let request_page = guarded_page.clone();
                let request_id = event.request_id.clone();
                let request_url = event.request.url.clone();
                let is_response_stage = event.response_status_code.is_some();
                decisions.spawn(async move {
                    let decision = if is_response_stage {
                        Ok(())
                    } else {
                        is_network_url_allowed(&request_url).await
                    };
                    let result = match decision {
                        Ok(()) => request_page
                            .execute(ContinueRequestParams::new(request_id))
                            .await
                            .map(|_| ()),
                        Err(reason) => {
                            tracing::warn!(%reason, "Blocked browser network request");
                            request_page
                                .execute(FailRequestParams::new(
                                    request_id,
                                    ErrorReason::BlockedByClient,
                                ))
                                .await
                                .map(|_| ())
                        }
                    };
                    if let Err(error) = result {
                        tracing::debug!(%error, "Failed to resolve paused browser request");
                    }
                });
            }
            decisions.abort_all();
            while decisions.join_next().await.is_some() {}
        }))
    }

    async fn close_page_state(mut state: PageState) {
        let _ = tokio::time::timeout(BROWSER_SHUTDOWN_TIMEOUT, state.page.clone().close()).await;
        state.request_guard_handle.abort();
        let _ = (&mut state.request_guard_handle).await;
    }

    /// List open tabs.
    async fn list_tabs(&self) -> Result<serde_json::Value, ToolError> {
        self.ensure_browser().await?;

        let mut guard = self.instance.write().await;
        let instance = guard.as_mut().ok_or_else(|| {
            ToolError::ExecutionFailed("Browser instance disappeared after startup".to_string())
        })?;

        let mut tabs = Vec::new();
        for (i, state) in instance.pages.iter().enumerate() {
            let url = bounded_browser_string(
                &state.page.url().await.ok().flatten().unwrap_or_default(),
                MAX_BROWSER_URL_BYTES,
            );
            let is_current = instance.current_page == Some(i);
            tabs.push(serde_json::json!({
                "index": i,
                "url": url,
                "current": is_current,
            }));
        }
        Self::touch_activity(instance);

        Ok(serde_json::json!({
            "tabs": tabs,
            "count": instance.pages.len(),
            "max_tabs": MAX_TABS,
        }))
    }

    /// Switch to a tab by index.
    async fn switch_tab(&self, tab_index: usize) -> Result<serde_json::Value, ToolError> {
        self.ensure_browser().await?;

        let mut guard = self.instance.write().await;
        let instance = guard.as_mut().ok_or_else(|| {
            ToolError::ExecutionFailed("Browser instance disappeared after startup".to_string())
        })?;

        if tab_index >= instance.pages.len() {
            return Err(ToolError::InvalidParameters(format!(
                "Tab index {} out of range (0..{})",
                tab_index,
                instance.pages.len()
            )));
        }

        instance.current_page = Some(tab_index);
        Self::touch_activity(instance);
        let url = bounded_browser_string(
            &instance.pages[tab_index]
                .page
                .url()
                .await
                .ok()
                .flatten()
                .unwrap_or_default(),
            MAX_BROWSER_URL_BYTES,
        );

        Ok(serde_json::json!({
            "status": "switched",
            "tab_index": tab_index,
            "url": url,
        }))
    }

    /// Scroll the current page.
    async fn scroll(
        &self,
        direction: &str,
        amount: Option<i64>,
    ) -> Result<serde_json::Value, ToolError> {
        self.ensure_browser().await?;

        let mut guard = self.instance.write().await;
        let instance = guard.as_mut().ok_or_else(|| {
            ToolError::ExecutionFailed("Browser instance disappeared after startup".to_string())
        })?;
        let state = Self::current_page_mut(instance)?;

        let pixels = amount.unwrap_or(500);
        if !(0..=MAX_SCROLL_PIXELS).contains(&pixels) {
            return Err(ToolError::InvalidParameters(format!(
                "Scroll amount must be between 0 and {MAX_SCROLL_PIXELS} pixels"
            )));
        }
        let js = match direction {
            "up" => format!("window.scrollBy(0, -{})", pixels),
            "down" => format!("window.scrollBy(0, {})", pixels),
            "top" => "window.scrollTo(0, 0)".to_string(),
            "bottom" => "window.scrollTo(0, document.body.scrollHeight)".to_string(),
            _ => {
                return Err(ToolError::InvalidParameters(
                    "Scroll direction must be up, down, top, or bottom".to_string(),
                ));
            }
        };

        state
            .page
            .evaluate_expression(&js)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Scroll failed: {e}")))?;

        // Allow time for lazy-loaded content
        tokio::time::sleep(Duration::from_millis(300)).await;
        Self::touch_activity(instance);

        Ok(serde_json::json!({
            "status": "scrolled",
            "direction": direction,
            "pixels": pixels,
        }))
    }

    /// Press a keyboard key (Enter, Tab, Escape, etc.) on the current page.
    async fn press_key(&self, key: &str) -> Result<serde_json::Value, ToolError> {
        if key.is_empty() || key.len() > MAX_KEY_BYTES || key.chars().any(char::is_control) {
            return Err(ToolError::InvalidParameters(format!(
                "Key must be non-empty, printable, and at most {MAX_KEY_BYTES} bytes"
            )));
        }
        self.ensure_browser().await?;

        let mut guard = self.instance.write().await;
        let instance = guard.as_mut().ok_or_else(|| {
            ToolError::ExecutionFailed("Browser instance disappeared after startup".to_string())
        })?;
        let state = Self::current_page_mut(instance)?;

        // Map user-friendly key names to CDP key identifiers
        let (key_code, text) = match key.to_lowercase().as_str() {
            "enter" | "return" => ("Enter", Some("\r")),
            "tab" => ("Tab", Some("\t")),
            "escape" | "esc" => ("Escape", None),
            "backspace" => ("Backspace", None),
            "delete" => ("Delete", None),
            "arrowup" | "up" => ("ArrowUp", None),
            "arrowdown" | "down" => ("ArrowDown", None),
            "arrowleft" | "left" => ("ArrowLeft", None),
            "arrowright" | "right" => ("ArrowRight", None),
            "space" => ("Space", Some(" ")),
            "home" => ("Home", None),
            "end" => ("End", None),
            "pageup" => ("PageUp", None),
            "pagedown" => ("PageDown", None),
            _ => (key, None),
        };

        let mut key_down_builder = DispatchKeyEventParams::builder()
            .r#type(DispatchKeyEventType::KeyDown)
            .key(key_code);

        if let Some(t) = text {
            key_down_builder = key_down_builder.text(t);
        }

        let key_down = key_down_builder
            .build()
            .map_err(|e| ToolError::ExecutionFailed(format!("KeyEvent build error: {e}")))?;

        state
            .page
            .execute(key_down)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Key press failed: {e}")))?;

        let key_up = DispatchKeyEventParams::builder()
            .r#type(DispatchKeyEventType::KeyUp)
            .key(key_code)
            .build()
            .map_err(|e| ToolError::ExecutionFailed(format!("KeyEvent build error: {e}")))?;

        state
            .page
            .execute(key_up)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Key release failed: {e}")))?;

        // Brief pause for any triggered navigation/updates
        tokio::time::sleep(Duration::from_millis(200)).await;
        Self::touch_activity(instance);

        Ok(serde_json::json!({
            "status": "key_pressed",
            "key": key_code,
        }))
    }

    /// Go back in browser history.
    async fn go_back(&self) -> Result<serde_json::Value, ToolError> {
        self.ensure_browser().await?;

        let mut guard = self.instance.write().await;
        let instance = guard.as_mut().ok_or_else(|| {
            ToolError::ExecutionFailed("Browser instance disappeared after startup".to_string())
        })?;
        let state = Self::current_page_mut(instance)?;

        state
            .page
            .evaluate_expression("window.history.back()")
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Back navigation failed: {e}")))?;

        tokio::time::sleep(Duration::from_millis(500)).await;
        let url = bounded_browser_string(
            &state.page.url().await.ok().flatten().unwrap_or_default(),
            MAX_BROWSER_URL_BYTES,
        );
        Self::touch_activity(instance);
        Ok(serde_json::json!({
            "status": "went_back",
            "url": url,
        }))
    }

    /// Go forward in browser history.
    async fn go_forward(&self) -> Result<serde_json::Value, ToolError> {
        self.ensure_browser().await?;

        let mut guard = self.instance.write().await;
        let instance = guard.as_mut().ok_or_else(|| {
            ToolError::ExecutionFailed("Browser instance disappeared after startup".to_string())
        })?;
        let state = Self::current_page_mut(instance)?;

        state
            .page
            .evaluate_expression("window.history.forward()")
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Forward navigation failed: {e}")))?;

        tokio::time::sleep(Duration::from_millis(500)).await;
        let url = bounded_browser_string(
            &state.page.url().await.ok().flatten().unwrap_or_default(),
            MAX_BROWSER_URL_BYTES,
        );
        Self::touch_activity(instance);
        Ok(serde_json::json!({
            "status": "went_forward",
            "url": url,
        }))
    }

    /// Get all images on the current page with their src and alt text.
    async fn get_images(&self) -> Result<serde_json::Value, ToolError> {
        self.ensure_browser().await?;

        let mut guard = self.instance.write().await;
        let instance = guard.as_mut().ok_or_else(|| {
            ToolError::ExecutionFailed("Browser instance disappeared after startup".to_string())
        })?;
        let state = Self::current_page_mut(instance)?;

        let js = r#"
        (function() {
            const imgs = Array.from(document.querySelectorAll('img'));
            return imgs.slice(0, 50).map(img => ({
                src: String(img.src || '').slice(0, 8192),
                alt: String(img.alt || '').slice(0, 1024),
                width: img.naturalWidth || img.width || 0,
                height: img.naturalHeight || img.height || 0,
            }));
        })()
        "#;

        let eval_result = state
            .page
            .evaluate_expression(js)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("get_images failed: {e}")))?;

        let images = eval_result
            .into_value::<serde_json::Value>()
            .unwrap_or(serde_json::json!([]));
        let (images, truncated) = bound_json_value(images);
        Self::touch_activity(instance);

        Ok(serde_json::json!({
            "images": images,
            "count": images.as_array().map(|a| a.len()).unwrap_or(0),
            "truncated": truncated,
        }))
    }

    /// Get browser console messages from the current page.
    async fn get_console(&self) -> Result<serde_json::Value, ToolError> {
        self.ensure_browser().await?;

        let mut guard = self.instance.write().await;
        let instance = guard.as_mut().ok_or_else(|| {
            ToolError::ExecutionFailed("Browser instance disappeared after startup".to_string())
        })?;
        let state = Self::current_page_mut(instance)?;

        // Inject a console capture script if not already present
        let js = r#"
        (function() {
            if (!window.__tc_console) {
                window.__tc_console = [];
                const orig = {
                    log: console.log,
                    warn: console.warn,
                    error: console.error,
                    info: console.info,
                };
                ['log', 'warn', 'error', 'info'].forEach(level => {
                    console[level] = function(...args) {
                        const safe = args.slice(0, 20).map(a => {
                            try {
                                const value = typeof a === 'object' ? JSON.stringify(a) : String(a);
                                return String(value === undefined ? '[undefined]' : value).slice(0, 2048);
                            } catch (_) {
                                return '[unserializable]';
                            }
                        }).join(' ').slice(0, 8192);
                        window.__tc_console.push({
                            level: level,
                            message: safe,
                            time: Date.now()
                        });
                        if (window.__tc_console.length > 100) window.__tc_console.shift();
                        orig[level].apply(console, args);
                    };
                });
            }
            const msgs = window.__tc_console.slice(-50);
            return msgs;
        })()
        "#;

        let eval_result = state
            .page
            .evaluate_expression(js)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("get_console failed: {e}")))?;

        let messages = eval_result
            .into_value::<serde_json::Value>()
            .unwrap_or(serde_json::json!([]));
        let (messages, truncated) = bound_json_value(messages);
        Self::touch_activity(instance);

        Ok(serde_json::json!({
            "console_messages": messages,
            "count": messages.as_array().map(|a| a.len()).unwrap_or(0),
            "truncated": truncated,
        }))
    }
}

#[async_trait]
impl Tool for BrowserTool {
    fn name(&self) -> &str {
        "browser"
    }

    fn description(&self) -> &str {
        "Browse the web: navigate pages, read content via accessibility snapshots, \
         click elements, type text, press keys, scroll, go back/forward, take \
         screenshots, extract images, read console output, and evaluate JavaScript. \
         Use 'snapshot' after navigation to see what's on the page — it returns an \
         accessibility tree with numbered refs (e.g., ref=\"e1\") that you can use \
         with 'click' and 'type' actions. Use 'close' when finished browsing."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["navigate", "snapshot", "click", "type", "press_key", "scroll", "screenshot", "evaluate", "get_text", "get_images", "console", "back", "forward", "close", "tabs", "switch_tab"],
                    "description": "The browser action to perform"
                },
                "url": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": MAX_BROWSER_URL_BYTES,
                    "description": "URL to navigate to (for 'navigate' action)"
                },
                "ref": {
                    "type": "string",
                    "pattern": "^e[0-9]{1,14}$",
                    "description": "Element ref ID from snapshot (for 'click' and 'type' actions, e.g. 'e1')"
                },
                "text": {
                    "type": "string",
                    "maxLength": MAX_TYPE_TEXT_BYTES,
                    "description": "Text to type (for 'type' action)"
                },
                "expression": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": MAX_EVALUATE_EXPRESSION_BYTES,
                    "description": "JavaScript expression to evaluate (for 'evaluate' action)"
                },
                "tab_index": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": MAX_TABS - 1,
                    "description": "Tab index to switch to (for 'switch_tab' action)"
                },
                "key": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": MAX_KEY_BYTES,
                    "description": "Key to press (for 'press_key' action, e.g. 'Enter', 'Tab', 'Escape', 'ArrowDown')"
                },
                "direction": {
                    "type": "string",
                    "enum": ["up", "down", "top", "bottom"],
                    "description": "Scroll direction (for 'scroll' action, default 'down')"
                },
                "amount": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": MAX_SCROLL_PIXELS,
                    "description": "Scroll amount in pixels (for 'scroll' action, default 500)"
                }
            },
            "required": ["action"],
            "additionalProperties": false
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();

        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("Missing 'action' parameter".into()))?;
        if action.len() > 32
            || !action
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
        {
            return Err(ToolError::InvalidParameters(
                "Browser action name is invalid".to_string(),
            ));
        }

        let _operation_guard = self.operation_lock.lock().await;
        self.activate_scope(BrowserSessionScope::from_context(ctx))
            .await;

        let result = match action {
            "navigate" => {
                let url = params.get("url").and_then(|v| v.as_str()).ok_or_else(|| {
                    ToolError::InvalidParameters("'navigate' requires 'url' parameter".into())
                })?;
                self.navigate(url).await?
            }
            "snapshot" => self.snapshot().await?,
            "click" => {
                let ref_id = params.get("ref").and_then(|v| v.as_str()).ok_or_else(|| {
                    ToolError::InvalidParameters("'click' requires 'ref' parameter".into())
                })?;
                self.click_ref(ref_id).await?
            }
            "type" => {
                let ref_id = params.get("ref").and_then(|v| v.as_str()).ok_or_else(|| {
                    ToolError::InvalidParameters("'type' requires 'ref' parameter".into())
                })?;
                let text = params.get("text").and_then(|v| v.as_str()).ok_or_else(|| {
                    ToolError::InvalidParameters("'type' requires 'text' parameter".into())
                })?;
                self.type_text(ref_id, text).await?
            }
            "screenshot" => self.screenshot().await?,
            "evaluate" => {
                let expression = params
                    .get("expression")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ToolError::InvalidParameters(
                            "'evaluate' requires 'expression' parameter".into(),
                        )
                    })?;
                self.evaluate(expression).await?
            }
            "get_text" => self.get_text().await?,
            "get_images" => self.get_images().await?,
            "console" => self.get_console().await?,
            "back" => self.go_back().await?,
            "forward" => self.go_forward().await?,
            "scroll" => {
                let direction = params
                    .get("direction")
                    .and_then(|v| v.as_str())
                    .unwrap_or("down");
                let amount = params.get("amount").and_then(|v| v.as_i64());
                self.scroll(direction, amount).await?
            }
            "press_key" => {
                let key = params.get("key").and_then(|v| v.as_str()).ok_or_else(|| {
                    ToolError::InvalidParameters("'press_key' requires 'key' parameter".into())
                })?;
                self.press_key(key).await?
            }
            "close" => self.close_session().await?,
            "tabs" => self.list_tabs().await?,
            "switch_tab" => {
                let raw_tab_index = params
                    .get("tab_index")
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| {
                        ToolError::InvalidParameters(
                            "'switch_tab' requires 'tab_index' parameter".into(),
                        )
                    })?;
                let tab_index = usize::try_from(raw_tab_index).map_err(|_| {
                    ToolError::InvalidParameters("Tab index is too large".to_string())
                })?;
                self.switch_tab(tab_index).await?
            }
            _ => {
                return Err(ToolError::InvalidParameters(format!(
                    "Unknown action: '{action}'. Use: navigate, snapshot, click, type, press_key, scroll, screenshot, evaluate, get_text, get_images, console, back, forward, close, tabs, switch_tab"
                )));
            }
        };

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    async fn shutdown(&self) -> Result<(), ToolError> {
        BrowserTool::shutdown(self).await;
        Ok(())
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn requires_sanitization(&self) -> bool {
        true // External web content must be sanitized
    }

    /// Browser operations (especially navigation + JS eval) can take longer
    /// than the default 60s timeout.
    fn execution_timeout(&self) -> Duration {
        Duration::from_secs(120)
    }
}

mod actions;
#[cfg(test)]
mod tests;
