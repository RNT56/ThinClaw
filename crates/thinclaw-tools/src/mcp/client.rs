//! MCP client for connecting to MCP servers.
//!
//! Supports both local (unauthenticated) and hosted (OAuth-authenticated) servers.
//! Uses Streamable HTTP or stdio transport, enforces strict protocol negotiation,
//! and preserves structured MCP tool outputs.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use tokio::sync::{Mutex, OnceCell, RwLock, oneshot};

use thinclaw_secrets::{SecretAccessContext, SecretError, SecretsStore};
use thinclaw_tools_core::{ApprovalRequirement, Tool, ToolArtifact, ToolError, ToolOutput};
use thinclaw_types::JobContext;

use super::auth::refresh_access_token;
use super::config::{
    McpCapabilityPolicy, McpConfigStore, McpLoggingLevel, McpServerConfig, McpTransport,
};
use super::protocol::{
    CallToolResult, CancelledNotification, ClientCapabilities, ClientElicitationCapability,
    ClientRootsCapability, ClientSamplingCapability, ClientSamplingToolsCapability,
    CompleteArgument, CompleteResult, ContentBlock, ElicitationCreateRequest, GetPromptResult,
    InitializeResult, ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult,
    ListToolsResult, LoggingMessageNotification, McpError, McpNotification, McpPrompt, McpRequest,
    McpResource, McpResourceContents, McpResourceTemplate, McpResponse, McpTool,
    McpTransportMessage, PROTOCOL_VERSION, ProgressNotification, ReadResourceResult,
    ResourceUpdatedNotification, SamplingCreateMessageRequest,
};
use super::session::McpSessionManager;
use super::stdio::{McpInboundHandler, StdioTransport};

/// Shared runtime state for a connected MCP server.
struct McpRuntimeState {
    server_name: String,
    tools_cache: RwLock<Option<Vec<McpTool>>>,
    resources_cache: RwLock<Option<Vec<McpResource>>>,
    resource_templates_cache: RwLock<Option<Vec<McpResourceTemplate>>>,
    prompts_cache: RwLock<Option<Vec<McpPrompt>>>,
    initialize_result: RwLock<Option<InitializeResult>>,
    capability_policy: McpCapabilityPolicy,
    roots_grants: RwLock<Vec<String>>,
    config_store: Option<McpConfigStore>,
    pending_interactions: Mutex<PendingInteractionState>,
}

// MCP interaction DTOs moved to `thinclaw-tools-core` so light consumers can use
// them without the heavyweight tool runtime; re-exported for path stability.
pub use thinclaw_tools_core::mcp_interaction::{McpInteractionKind, McpPendingInteraction};

enum PendingInteractionResolution {
    Approved(serde_json::Value),
    Denied(String),
}

#[derive(Default)]
struct PendingInteractionState {
    interactions: HashMap<String, McpPendingInteraction>,
    waiters: HashMap<String, oneshot::Sender<PendingInteractionResolution>>,
    request_ids: HashMap<u64, String>,
}

impl PendingInteractionState {
    fn remove(
        &mut self,
        interaction_id: &str,
        request_id: Option<u64>,
    ) -> Option<oneshot::Sender<PendingInteractionResolution>> {
        self.interactions.remove(interaction_id);
        if let Some(request_id) = request_id {
            self.request_ids.remove(&request_id);
        } else if let Some(request_id) =
            self.request_ids
                .iter()
                .find_map(|(request_id, pending_id)| {
                    (pending_id == interaction_id).then_some(*request_id)
                })
        {
            self.request_ids.remove(&request_id);
        }
        self.waiters.remove(interaction_id)
    }
}

impl McpRuntimeState {
    fn new(
        server_name: impl Into<String>,
        config: Option<&McpServerConfig>,
        config_store: Option<McpConfigStore>,
    ) -> Self {
        let (capability_policy, roots_grants) = config.map_or_else(
            || (McpCapabilityPolicy::default(), Vec::new()),
            |config| {
                (
                    config.capability_policy.clone(),
                    config.roots_grants.clone(),
                )
            },
        );

        Self {
            server_name: server_name.into(),
            tools_cache: RwLock::new(None),
            resources_cache: RwLock::new(None),
            resource_templates_cache: RwLock::new(None),
            prompts_cache: RwLock::new(None),
            initialize_result: RwLock::new(None),
            capability_policy,
            roots_grants: RwLock::new(roots_grants),
            config_store,
            pending_interactions: Mutex::new(PendingInteractionState::default()),
        }
    }

    fn client_capabilities(&self) -> ClientCapabilities {
        ClientCapabilities {
            roots: self
                .capability_policy
                .roots
                .then_some(ClientRootsCapability { list_changed: true }),
            sampling: self
                .capability_policy
                .sampling
                .then_some(ClientSamplingCapability {
                    tools: self
                        .capability_policy
                        .sampling_tools
                        .then_some(ClientSamplingToolsCapability {}),
                }),
            elicitation: self
                .capability_policy
                .form_elicitation
                .then_some(ClientElicitationCapability { forms: true }),
        }
    }

    async fn remember_initialize(&self, result: InitializeResult) {
        *self.initialize_result.write().await = Some(result);
    }

    async fn cached_initialize(&self) -> Option<InitializeResult> {
        self.initialize_result.read().await.clone()
    }

    async fn clear_all_caches(&self) {
        *self.tools_cache.write().await = None;
        *self.resources_cache.write().await = None;
        *self.resource_templates_cache.write().await = None;
        *self.prompts_cache.write().await = None;
    }

    async fn handle_notification_inner(&self, notification: &McpNotification) {
        match notification.method.as_str() {
            "notifications/tools/list_changed" => {
                *self.tools_cache.write().await = None;
                tracing::debug!(server = %self.server_name, "MCP tools cache invalidated");
            }
            "notifications/resources/list_changed" | "notifications/resources/updated" => {
                *self.resources_cache.write().await = None;
                *self.resource_templates_cache.write().await = None;
                if notification.method == "notifications/resources/updated"
                    && let Some(params) = notification.params.as_ref()
                    && let Ok(updated) =
                        serde_json::from_value::<ResourceUpdatedNotification>(params.clone())
                {
                    tracing::debug!(
                        server = %self.server_name,
                        uri = updated.uri.as_deref().unwrap_or(""),
                        "MCP resource updated"
                    );
                }
                tracing::debug!(server = %self.server_name, "MCP resources cache invalidated");
            }
            "notifications/prompts/list_changed" => {
                *self.prompts_cache.write().await = None;
                tracing::debug!(server = %self.server_name, "MCP prompts cache invalidated");
            }
            "notifications/message" => {
                let parsed = notification.params.as_ref().and_then(|params| {
                    serde_json::from_value::<LoggingMessageNotification>(params.clone()).ok()
                });
                tracing::debug!(
                    server = %self.server_name,
                    level = ?parsed.as_ref().and_then(|message| message.level),
                    logger = parsed.as_ref().and_then(|message| message.logger.as_deref()).unwrap_or(""),
                    data = ?parsed.as_ref().and_then(|message| message.data.clone()).or_else(|| notification.params.clone()),
                    "MCP log notification"
                );
            }
            "notifications/progress" => {
                let parsed = notification.params.as_ref().and_then(|params| {
                    serde_json::from_value::<ProgressNotification>(params.clone()).ok()
                });
                tracing::debug!(
                    server = %self.server_name,
                    progress = ?parsed.as_ref().and_then(|progress| progress.progress),
                    total = ?parsed.as_ref().and_then(|progress| progress.total),
                    progress_token = ?parsed.as_ref().and_then(|progress| progress.progress_token.clone()),
                    message = parsed.as_ref().and_then(|progress| progress.message.as_deref()).unwrap_or(""),
                    "MCP progress notification"
                );
            }
            "notifications/cancelled" => {
                let parsed = notification.params.as_ref().and_then(|params| {
                    serde_json::from_value::<CancelledNotification>(params.clone()).ok()
                });
                if let Some(cancelled) = parsed.as_ref() {
                    self.cancel_pending_server_request(
                        cancelled.request_id,
                        cancelled.reason.clone().unwrap_or_else(|| {
                            "MCP interaction was cancelled by the server".to_string()
                        }),
                    )
                    .await;
                }
                tracing::debug!(
                    server = %self.server_name,
                    request_id = ?parsed.as_ref().and_then(|cancelled| cancelled.request_id),
                    reason = parsed.as_ref().and_then(|cancelled| cancelled.reason.as_deref()).unwrap_or(""),
                    "MCP cancellation notification"
                );
            }
            other => {
                tracing::trace!(server = %self.server_name, method = %other, "Unhandled MCP notification");
            }
        }
    }

    async fn refresh_roots_grants(&self) {
        let Some(ref config_store) = self.config_store else {
            return;
        };

        let Ok(Some(config)) = config_store.get_server(&self.server_name).await else {
            return;
        };

        let mut roots = self.roots_grants.write().await;
        if *roots != config.roots_grants {
            *roots = config.roots_grants;
            tracing::debug!(server = %self.server_name, "Reloaded MCP roots grants from persisted config");
        }
    }

    async fn update_roots_grants(&self, roots_grants: Vec<String>) -> bool {
        let mut current = self.roots_grants.write().await;
        if *current == roots_grants {
            return false;
        }
        *current = roots_grants;
        true
    }

    async fn roots_result(&self) -> serde_json::Value {
        self.refresh_roots_grants().await;
        let roots_grants = self.roots_grants.read().await.clone();
        let roots = roots_grants
            .iter()
            .map(|root| {
                serde_json::json!({
                    "uri": normalize_root_uri(root),
                    "name": root_name(root),
                })
            })
            .collect::<Vec<_>>();
        serde_json::json!({ "roots": roots })
    }

    async fn list_pending_interactions(&self) -> Vec<McpPendingInteraction> {
        let mut pending = self
            .pending_interactions
            .lock()
            .await
            .interactions
            .values()
            .cloned()
            .collect::<Vec<_>>();
        pending.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        pending
    }

    async fn resolve_pending_interaction(
        &self,
        interaction_id: &str,
        resolution: PendingInteractionResolution,
    ) -> Result<(), ToolError> {
        let sender = {
            let mut pending = self.pending_interactions.lock().await;
            let request_id = pending
                .request_ids
                .iter()
                .find_map(|(request_id, pending_id)| {
                    (pending_id == interaction_id).then_some(*request_id)
                })
                .ok_or_else(|| {
                    ToolError::InvalidParameters(format!(
                        "No pending MCP interaction with id '{}'",
                        interaction_id
                    ))
                })?;
            pending
                .remove(interaction_id, Some(request_id))
                .ok_or_else(|| {
                    ToolError::InvalidParameters(format!(
                        "No pending MCP interaction with id '{}'",
                        interaction_id
                    ))
                })?
        };
        sender.send(resolution).map_err(|_| {
            ToolError::ExecutionFailed(format!(
                "Pending MCP interaction '{}' was already dropped",
                interaction_id
            ))
        })
    }

    async fn finish_pending_interaction(&self, interaction_id: &str, request_id: u64) {
        self.pending_interactions
            .lock()
            .await
            .remove(interaction_id, Some(request_id));
    }

    async fn cancel_pending_server_request(&self, request_id: Option<u64>, reason: String) {
        let Some(request_id) = request_id else {
            return;
        };
        let sender = {
            let mut pending = self.pending_interactions.lock().await;
            let Some(interaction_id) = pending.request_ids.get(&request_id).cloned() else {
                return;
            };
            pending.remove(&interaction_id, Some(request_id))
        };
        if let Some(sender) = sender {
            let _ = sender.send(PendingInteractionResolution::Denied(reason));
        }
    }

    async fn cancel_all_pending_interactions(&self, reason: &str) {
        let waiters = {
            let mut pending = self.pending_interactions.lock().await;
            pending.interactions.clear();
            pending.request_ids.clear();
            std::mem::take(&mut pending.waiters)
        };
        for (_, sender) in waiters {
            let _ = sender.send(PendingInteractionResolution::Denied(reason.to_string()));
        }
    }

    async fn build_pending_interaction(
        &self,
        request: &McpRequest,
        kind: McpInteractionKind,
    ) -> Result<
        (
            McpPendingInteraction,
            oneshot::Receiver<PendingInteractionResolution>,
        ),
        McpError,
    > {
        let interaction_id = uuid::Uuid::new_v4().to_string();
        let params = request.params.clone().unwrap_or(serde_json::Value::Null);
        let (title, description, schema) = describe_pending_interaction(&kind, &params);
        let pending_interaction = McpPendingInteraction {
            id: interaction_id.clone(),
            server_name: self.server_name.clone(),
            method: request.method.clone(),
            kind,
            title,
            description,
            params,
            schema,
            created_at: Utc::now().to_rfc3339(),
        };
        let (sender, receiver) = oneshot::channel();
        // A single lock makes registration cancellation-safe: the future has
        // no suspension point between these related inserts.
        let mut pending = self.pending_interactions.lock().await;
        if pending.request_ids.contains_key(&request.id) {
            return Err(McpError::invalid_request(format!(
                "MCP client request id {} is already active",
                request.id
            )));
        }
        if pending.interactions.len() >= MAX_PENDING_MCP_INTERACTIONS {
            return Err(McpError::request_cancelled(
                "MCP pending interaction capacity is exhausted",
            ));
        }
        pending
            .interactions
            .insert(interaction_id.clone(), pending_interaction.clone());
        pending.waiters.insert(interaction_id, sender);
        pending
            .request_ids
            .insert(request.id, pending_interaction.id.clone());
        drop(pending);
        Ok((pending_interaction, receiver))
    }

    async fn run_pending_interaction(
        &self,
        request: McpRequest,
        kind: McpInteractionKind,
    ) -> McpResponse {
        let request_id = request.id;
        let (pending, receiver) = match self.build_pending_interaction(&request, kind).await {
            Ok(pending) => pending,
            Err(error) => return McpResponse::error(request_id, error),
        };

        match tokio::time::timeout(MCP_INTERACTION_TIMEOUT, receiver).await {
            Ok(Ok(PendingInteractionResolution::Approved(result))) => {
                self.finish_pending_interaction(&pending.id, request_id)
                    .await;
                McpResponse::success(request_id, result)
            }
            Ok(Ok(PendingInteractionResolution::Denied(message))) => {
                self.finish_pending_interaction(&pending.id, request_id)
                    .await;
                McpResponse::error(request_id, McpError::request_cancelled(message))
            }
            Ok(Err(_)) => {
                self.finish_pending_interaction(&pending.id, request_id)
                    .await;
                McpResponse::error(
                    request_id,
                    McpError::request_cancelled("MCP interaction was cancelled".to_string()),
                )
            }
            Err(_) => {
                self.finish_pending_interaction(&pending.id, request_id)
                    .await;
                McpResponse::error(
                    request_id,
                    McpError::request_cancelled(
                        "Timed out waiting for MCP interaction response".to_string(),
                    ),
                )
            }
        }
    }
}

#[async_trait]
impl McpInboundHandler for McpRuntimeState {
    async fn handle_request(&self, request: McpRequest) -> McpResponse {
        match request.method.as_str() {
            "roots/list" if self.capability_policy.roots => {
                McpResponse::success(request.id, self.roots_result().await)
            }
            "sampling/createMessage" if self.capability_policy.sampling => {
                self.run_pending_interaction(request, McpInteractionKind::Sampling)
                    .await
            }
            "elicitation/create" if self.capability_policy.form_elicitation => {
                self.run_pending_interaction(request, McpInteractionKind::Elicitation)
                    .await
            }
            other => McpResponse::error(request.id, McpError::method_not_found(other)),
        }
    }

    async fn handle_notification(&self, notification: McpNotification) {
        self.handle_notification_inner(&notification).await;
    }

    async fn cancel_request(&self, request_id: u64, reason: &str) {
        self.cancel_pending_server_request(Some(request_id), reason.to_string())
            .await;
    }
}

/// Build the long-lived HTTP client for an MCP Streamable-HTTP server, pinning
/// the connection to the socket addresses `server_url` resolved to at build time
/// (F-02). This closes the DNS-rebinding TOCTOU window: once pinned, `reqwest`
/// connects to the validated address(es) for the client's lifetime instead of
/// re-resolving the hostname at connect time (when it could rebind to a private
/// address such as the cloud metadata endpoint).
///
/// Local endpoints are accepted only when the persisted config explicitly sets
/// `allow_local_http` (or when a caller uses the low-level unconfigured
/// constructor). Validation, DNS resolution, pinning, redirects, and client
/// construction all fail closed.
async fn build_pinned_mcp_client(
    server_url: &str,
    allow_local: bool,
) -> Result<reqwest::Client, ToolError> {
    super::build_pinned(reqwest::Client::builder(), server_url, allow_local).await
}

/// MCP client for communicating with MCP servers.
pub struct McpClient {
    /// Server URL (for HTTP transport).
    server_url: String,

    /// Server name (for logging and session management).
    server_name: String,

    /// HTTP client (used for HTTP transport only).
    http_client: Arc<OnceCell<Result<reqwest::Client, String>>>,

    /// Whether an explicitly configured local HTTP endpoint is permitted.
    http_client_allow_local: bool,

    /// Structural configuration errors captured by synchronous constructors.
    http_client_validation_error: Option<String>,

    /// Stdio transport (used for stdio transport only).
    stdio_transport: Option<Arc<StdioTransport>>,

    /// Request ID counter.
    next_id: AtomicU64,

    /// Shared runtime state.
    runtime: Arc<McpRuntimeState>,

    /// Session manager (shared across clients).
    session_manager: Option<Arc<McpSessionManager>>,

    /// Secrets store for retrieving access tokens.
    secrets: Option<Arc<dyn SecretsStore + Send + Sync>>,

    /// User ID for secrets lookup.
    user_id: String,

    /// Server configuration (for token secret name lookup).
    server_config: Option<McpServerConfig>,

    /// Serializes refreshes across cloned clients so a burst of requests near
    /// expiry does not rotate the same refresh token concurrently.
    token_refresh_lock: Arc<Mutex<()>>,
}

const MCP_INTERACTION_TIMEOUT: Duration = Duration::from_secs(1800);
const MCP_HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(1830);
const MCP_HTTP_CONTROL_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_PENDING_MCP_INTERACTIONS: usize = 32;
const MAX_MCP_HTTP_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_MCP_HTTP_ERROR_BYTES: usize = 16 * 1024;
const MAX_MCP_SSE_STREAM_BYTES: usize = 64 * 1024 * 1024;
const MAX_MCP_SSE_EVENTS: usize = 4096;
const MAX_MCP_SESSION_ID_BYTES: usize = 4 * 1024;
const MAX_MCP_CATALOG_PAGES: usize = 128;
const MAX_MCP_CATALOG_ITEMS: usize = 8 * 1024;
const MAX_MCP_CATALOG_BYTES: usize = 32 * 1024 * 1024;
const MAX_MCP_CURSOR_BYTES: usize = 8 * 1024;
const MCP_OAUTH_MAX_REFRESH_SKEW_SECS: i64 = 60;

enum StoredAccessToken {
    Missing,
    RefreshRequired,
    Available(String),
}

impl McpClient {
    /// Create a new simple MCP client (no authentication).
    pub fn new(server_url: impl Into<String>) -> Self {
        let url = server_url.into();
        let name = extract_server_name(&url);
        let runtime = Arc::new(McpRuntimeState::new(name.clone(), None, None));
        Self::new_internal(
            url,
            name,
            None,
            runtime,
            None,
            None,
            "default".to_string(),
            None,
        )
    }

    /// Create a new simple MCP client with a specific name.
    pub fn new_with_name(server_name: impl Into<String>, server_url: impl Into<String>) -> Self {
        let server_name = server_name.into();
        let runtime = Arc::new(McpRuntimeState::new(server_name.clone(), None, None));
        Self::new_internal(
            server_url.into(),
            server_name,
            None,
            runtime,
            None,
            None,
            "default".to_string(),
            None,
        )
    }

    /// Create a non-authenticated HTTP client from saved config so policy fields are preserved.
    pub fn new_configured(config: McpServerConfig) -> Self {
        Self::new_configured_with_store(config, None)
    }

    /// Create a non-authenticated HTTP client backed by a persisted config store.
    pub fn new_configured_with_store(
        config: McpServerConfig,
        config_store: Option<McpConfigStore>,
    ) -> Self {
        let runtime = Arc::new(McpRuntimeState::new(
            config.name.clone(),
            Some(&config),
            config_store,
        ));
        Self::new_internal(
            config.url.clone(),
            config.name.clone(),
            None,
            runtime,
            None,
            None,
            "default".to_string(),
            Some(config),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new_internal(
        server_url: String,
        server_name: String,
        stdio_transport: Option<Arc<StdioTransport>>,
        runtime: Arc<McpRuntimeState>,
        session_manager: Option<Arc<McpSessionManager>>,
        secrets: Option<Arc<dyn SecretsStore + Send + Sync>>,
        user_id: String,
        server_config: Option<McpServerConfig>,
    ) -> Self {
        let http_client_allow_local = server_config
            .as_ref()
            .map(|config| config.allow_local_http)
            // Low-level constructors are used by tests and embedded callers;
            // an explicit local URL remains an operator choice.
            .unwrap_or(true);
        let http_client_validation_error = if stdio_transport.is_some() {
            None
        } else {
            server_config
                .as_ref()
                .and_then(|config| config.validate().err())
                .map(|error| error.to_string())
        };
        Self {
            http_client: Arc::new(OnceCell::new()),
            http_client_allow_local,
            http_client_validation_error,
            server_url,
            server_name,
            stdio_transport,
            next_id: AtomicU64::new(1),
            runtime,
            session_manager,
            secrets,
            user_id,
            server_config,
            token_refresh_lock: Arc::new(Mutex::new(())),
        }
    }

    async fn http_client(&self) -> Result<&reqwest::Client, ToolError> {
        if let Some(error) = self.http_client_validation_error.as_deref() {
            return Err(ToolError::NotAuthorized(format!(
                "MCP HTTP endpoint is not permitted: {error}"
            )));
        }
        self.http_client
            .get_or_init(|| async {
                build_pinned_mcp_client(&self.server_url, self.http_client_allow_local)
                    .await
                    .map_err(|error| error.to_string())
            })
            .await
            .as_ref()
            .map_err(|error| {
                ToolError::NotAuthorized(format!("MCP HTTP endpoint is not permitted: {error}"))
            })
    }

    /// Create a new MCP client with stdio transport.
    pub fn new_stdio(config: &McpServerConfig) -> Result<Self, ToolError> {
        Self::new_stdio_with_store(config, None)
    }

    /// Create a new MCP client with stdio transport and a persisted config store.
    pub fn new_stdio_with_store(
        config: &McpServerConfig,
        config_store: Option<McpConfigStore>,
    ) -> Result<Self, ToolError> {
        let command = config.command.as_deref().ok_or_else(|| {
            ToolError::ExternalService(format!(
                "MCP server '{}' is configured for stdio but has no command",
                config.name
            ))
        })?;

        let runtime = Arc::new(McpRuntimeState::new(
            config.name.clone(),
            Some(config),
            config_store,
        ));
        let handler: Arc<dyn McpInboundHandler> = runtime.clone();
        let transport = StdioTransport::spawn(
            &config.name,
            command,
            &config.args,
            &config.env,
            Some(handler),
        )?;

        Ok(Self::new_internal(
            String::new(),
            config.name.clone(),
            Some(Arc::new(transport)),
            runtime,
            None,
            None,
            "default".to_string(),
            Some(config.clone()),
        ))
    }

    /// Create an MCP client from a server config, choosing the appropriate transport.
    pub fn from_config(
        config: &McpServerConfig,
        session_manager: Option<Arc<McpSessionManager>>,
        secrets: Option<Arc<dyn SecretsStore + Send + Sync>>,
        user_id: &str,
    ) -> Result<Self, ToolError> {
        Self::from_config_with_store(config, session_manager, secrets, user_id, None)
    }

    /// Create an MCP client from a server config with a persisted config store.
    pub fn from_config_with_store(
        config: &McpServerConfig,
        session_manager: Option<Arc<McpSessionManager>>,
        secrets: Option<Arc<dyn SecretsStore + Send + Sync>>,
        user_id: &str,
        config_store: Option<McpConfigStore>,
    ) -> Result<Self, ToolError> {
        match config.transport {
            McpTransport::Stdio => Self::new_stdio_with_store(config, config_store),
            McpTransport::Http => {
                if let (Some(sm), Some(sec)) = (session_manager, secrets) {
                    Ok(Self::new_authenticated_with_store(
                        config.clone(),
                        sm,
                        sec,
                        user_id,
                        config_store,
                    ))
                } else {
                    Ok(Self::new_configured_with_store(
                        config.clone(),
                        config_store,
                    ))
                }
            }
        }
    }

    /// Create a new authenticated MCP client.
    pub fn new_authenticated(
        config: McpServerConfig,
        session_manager: Arc<McpSessionManager>,
        secrets: Arc<dyn SecretsStore + Send + Sync>,
        user_id: impl Into<String>,
    ) -> Self {
        Self::new_authenticated_with_store(config, session_manager, secrets, user_id, None)
    }

    /// Create a new authenticated MCP client backed by a persisted config store.
    pub fn new_authenticated_with_store(
        config: McpServerConfig,
        session_manager: Arc<McpSessionManager>,
        secrets: Arc<dyn SecretsStore + Send + Sync>,
        user_id: impl Into<String>,
        config_store: Option<McpConfigStore>,
    ) -> Self {
        let runtime = Arc::new(McpRuntimeState::new(
            config.name.clone(),
            Some(&config),
            config_store,
        ));
        Self::new_internal(
            config.url.clone(),
            config.name.clone(),
            None,
            runtime,
            Some(session_manager),
            Some(secrets),
            user_id.into(),
            Some(config),
        )
    }

    /// Get the server name.
    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    /// Get the server URL.
    pub fn server_url(&self) -> &str {
        &self.server_url
    }

    /// Registered ThinClaw prefix for a server's MCP tools.
    pub fn registered_tool_prefix(server_name: &str) -> String {
        format!("mcp__{}__", encode_tool_component(server_name))
    }

    /// Full registered ThinClaw tool name for an MCP tool.
    pub fn registered_tool_name(server_name: &str, tool_name: &str) -> String {
        format!(
            "{}{}",
            Self::registered_tool_prefix(server_name),
            encode_tool_component(tool_name)
        )
    }

    /// Get the next request ID.
    fn next_request_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    async fn inspect_access_token(&self) -> Result<StoredAccessToken, ToolError> {
        let Some(ref secrets) = self.secrets else {
            return Ok(StoredAccessToken::Missing);
        };

        let Some(ref config) = self.server_config else {
            return Ok(StoredAccessToken::Missing);
        };

        let token_name = config.token_secret_name();
        match secrets.get(&self.user_id, &token_name).await {
            Ok(secret)
                if secret.expires_at.is_some_and(|expires_at| {
                    let lifetime_seconds = expires_at
                        .signed_duration_since(secret.updated_at)
                        .num_seconds()
                        .max(0);
                    let skew_seconds =
                        (lifetime_seconds / 10).clamp(1, MCP_OAUTH_MAX_REFRESH_SKEW_SECS);
                    expires_at <= Utc::now() + chrono::Duration::seconds(skew_seconds)
                }) =>
            {
                return Ok(StoredAccessToken::RefreshRequired);
            }
            Ok(_) => {}
            Err(SecretError::Expired) => return Ok(StoredAccessToken::RefreshRequired),
            Err(SecretError::NotFound(_)) => return Ok(StoredAccessToken::Missing),
            Err(error) => {
                return Err(ToolError::ExternalService(format!(
                    "Failed to inspect access token: {error}"
                )));
            }
        }

        match secrets
            .get_for_injection(
                &self.user_id,
                &token_name,
                SecretAccessContext::new("mcp.client", "oauth_access_token"),
            )
            .await
        {
            Ok(token) => Ok(StoredAccessToken::Available(token.expose().to_string())),
            Err(SecretError::Expired) => Ok(StoredAccessToken::RefreshRequired),
            Err(SecretError::NotFound(_)) => Ok(StoredAccessToken::Missing),
            Err(error) => Err(ToolError::ExternalService(format!(
                "Failed to get access token: {error}"
            ))),
        }
    }

    /// Get the access token for this server (if authenticated), refreshing it
    /// before its persisted expiry under a clone-shared single-flight lock.
    async fn get_access_token(&self) -> Result<Option<String>, ToolError> {
        match self.inspect_access_token().await? {
            StoredAccessToken::Available(token) => return Ok(Some(token)),
            StoredAccessToken::Missing => return Ok(None),
            StoredAccessToken::RefreshRequired => {}
        }

        let _refresh_guard = self.token_refresh_lock.lock().await;
        match self.inspect_access_token().await? {
            StoredAccessToken::Available(token) => return Ok(Some(token)),
            StoredAccessToken::Missing => return Ok(None),
            StoredAccessToken::RefreshRequired => {}
        }

        let (Some(config), Some(secrets)) = (&self.server_config, &self.secrets) else {
            return Ok(None);
        };
        let refreshed = refresh_access_token(config, secrets, &self.user_id)
            .await
            .map_err(|error| {
                ToolError::ExternalService(format!(
                    "MCP access token expired and refresh failed: {error}"
                ))
            })?;
        Ok(Some(refreshed.access_token))
    }

    async fn refresh_after_unauthorized(
        &self,
        rejected_token: Option<&str>,
    ) -> Result<(), ToolError> {
        let _refresh_guard = self.token_refresh_lock.lock().await;
        if let StoredAccessToken::Available(current_token) = self.inspect_access_token().await?
            && rejected_token.is_some_and(|rejected| rejected != current_token.as_str())
        {
            // Another request already replaced the credential that received
            // this 401, so retry with that value without rotating again.
            return Ok(());
        }

        let (Some(config), Some(secrets)) = (&self.server_config, &self.secrets) else {
            return Err(ToolError::NotAuthorized(
                "MCP client has no OAuth configuration".to_string(),
            ));
        };
        refresh_access_token(config, secrets, &self.user_id)
            .await
            .map(|_| ())
            .map_err(|error| ToolError::ExternalService(error.to_string()))
    }

    async fn send_notification(&self, notification: McpNotification) -> Result<(), ToolError> {
        if let Some(ref transport) = self.stdio_transport {
            return transport.send_notification(notification).await;
        }

        let payload = serialize_http_payload(&notification)?;

        let mut req_builder = self
            .http_client()
            .await?
            .post(&self.server_url)
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .header("MCP-Protocol-Version", PROTOCOL_VERSION)
            .timeout(MCP_HTTP_CONTROL_TIMEOUT)
            .body(payload);

        if let Some(token) = self.get_access_token().await? {
            req_builder = req_builder.header("Authorization", format!("Bearer {}", token));
        }

        if let Some(ref session_manager) = self.session_manager
            && let Some(session_id) = session_manager.get_session_id(&self.server_name).await
        {
            req_builder = req_builder.header("MCP-Session-Id", session_id);
        }

        let response = req_builder
            .send()
            .await
            .map_err(|e| ToolError::ExternalService(format!("MCP notification failed: {e}")))?;

        self.capture_session_id(response.headers()).await;

        if !response.status().is_success() {
            let status = response.status();
            let body = bounded_error_body(response).await;
            return Err(ToolError::ExternalService(format!(
                "MCP notification returned status: {} - {}",
                status, body
            )));
        }

        Ok(())
    }

    async fn send_response(&self, response: McpResponse) -> Result<(), ToolError> {
        if self.stdio_transport.is_some() {
            return Ok(());
        }

        let payload = serialize_http_payload(&response)?;

        let mut req_builder = self
            .http_client()
            .await?
            .post(&self.server_url)
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .header("MCP-Protocol-Version", PROTOCOL_VERSION)
            .timeout(MCP_HTTP_CONTROL_TIMEOUT)
            .body(payload);

        if let Some(token) = self.get_access_token().await? {
            req_builder = req_builder.header("Authorization", format!("Bearer {}", token));
        }

        if let Some(ref session_manager) = self.session_manager
            && let Some(session_id) = session_manager.get_session_id(&self.server_name).await
        {
            req_builder = req_builder.header("MCP-Session-Id", session_id);
        }

        let http_response = req_builder.send().await.map_err(|e| {
            ToolError::ExternalService(format!("Failed to send MCP client response: {e}"))
        })?;
        self.capture_session_id(http_response.headers()).await;
        if !http_response.status().is_success() {
            let status = http_response.status();
            let body = bounded_error_body(http_response).await;
            return Err(ToolError::ExternalService(format!(
                "MCP client response returned status: {} - {}",
                status, body
            )));
        }
        Ok(())
    }

    /// Send a request to the MCP server.
    async fn send_request(&self, request: McpRequest) -> Result<McpResponse, ToolError> {
        if let Some(ref transport) = self.stdio_transport {
            return transport.send_request(request).await;
        }

        self.send_request_http(request).await
    }

    async fn send_request_http(&self, request: McpRequest) -> Result<McpResponse, ToolError> {
        let payload = serialize_http_payload(&request)?;
        for attempt in 0..2 {
            let mut req_builder = self
                .http_client()
                .await?
                .post(&self.server_url)
                .header("Accept", "application/json, text/event-stream")
                .header("Content-Type", "application/json")
                .header("MCP-Protocol-Version", PROTOCOL_VERSION)
                .timeout(MCP_HTTP_REQUEST_TIMEOUT)
                .body(payload.clone());

            let access_token = self.get_access_token().await?;
            if let Some(ref token) = access_token {
                req_builder = req_builder.header("Authorization", format!("Bearer {}", token));
            }

            if let Some(ref session_manager) = self.session_manager
                && let Some(session_id) = session_manager.get_session_id(&self.server_name).await
            {
                req_builder = req_builder.header("MCP-Session-Id", session_id);
            }

            let response = req_builder
                .send()
                .await
                .map_err(|e| ToolError::ExternalService(format!("MCP request failed: {e}")))?;

            if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                if attempt == 0
                    && self
                        .refresh_after_unauthorized(access_token.as_deref())
                        .await
                        .is_ok()
                {
                    continue;
                }

                return Err(ToolError::ExternalService(format!(
                    "MCP server '{}' requires authentication. Run: thinclaw mcp auth {}",
                    self.server_name, self.server_name
                )));
            }

            let parsed = self.parse_response(response).await?;
            if parsed.id != request.id {
                return Err(ToolError::ExternalService(format!(
                    "MCP server '{}' returned response id {} for request id {}",
                    self.server_name, parsed.id, request.id
                )));
            }
            return Ok(parsed);
        }

        Err(ToolError::ExternalService(
            "MCP request failed after retry".to_string(),
        ))
    }

    async fn capture_session_id(&self, headers: &reqwest::header::HeaderMap) {
        if let Some(ref session_manager) = self.session_manager {
            let session_id = headers
                .get("MCP-Session-Id")
                .or_else(|| headers.get("Mcp-Session-Id"))
                .and_then(|v| v.to_str().ok())
                .filter(|value| !value.is_empty() && value.len() <= MAX_MCP_SESSION_ID_BYTES)
                .map(str::to_string);
            if session_id.is_some() {
                session_manager
                    .update_session_id(&self.server_name, session_id)
                    .await;
            }
        }
    }

    /// Parse the HTTP response into an MCP response.
    async fn parse_response(&self, response: reqwest::Response) -> Result<McpResponse, ToolError> {
        self.capture_session_id(response.headers()).await;

        if !response.status().is_success() {
            let status = response.status();
            let body = bounded_error_body(response).await;
            return Err(ToolError::ExternalService(format!(
                "MCP server returned status: {} - {}",
                status, body
            )));
        }

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        if content_type.contains("text/event-stream") {
            return self.parse_sse_response(response).await;
        }

        let body = read_limited_response(response, MAX_MCP_HTTP_MESSAGE_BYTES, "MCP response body")
            .await?;
        let body = std::str::from_utf8(&body).map_err(|error| {
            ToolError::ExternalService(format!("MCP response body is not valid UTF-8: {error}"))
        })?;

        if let Some(parsed) = self.process_transport_payload(body).await? {
            return Ok(parsed);
        }

        Err(ToolError::ExternalService(
            "Received notification without a matching response".to_string(),
        ))
    }

    async fn process_transport_payload(
        &self,
        payload: &str,
    ) -> Result<Option<McpResponse>, ToolError> {
        match McpTransportMessage::parse_str(payload)
            .map_err(|e| ToolError::ExternalService(format!("Failed to parse MCP response: {e}")))?
        {
            McpTransportMessage::Response(response) => Ok(Some(response)),
            McpTransportMessage::Notification(notification) => {
                self.runtime.handle_notification_inner(&notification).await;
                Ok(None)
            }
            McpTransportMessage::Request(request) => {
                let response = self.runtime.handle_request(request).await;
                self.send_response(response).await?;
                Ok(None)
            }
        }
    }

    async fn process_sse_event(
        &self,
        current_data: &mut Vec<u8>,
    ) -> Result<Option<McpResponse>, ToolError> {
        if current_data.is_empty() {
            return Ok(None);
        }

        let payload = std::str::from_utf8(current_data).map_err(|error| {
            ToolError::ExternalService(format!("MCP SSE data is not valid UTF-8: {error}"))
        })?;
        let result = self.process_transport_payload(payload).await;
        current_data.clear();
        result
    }

    async fn process_sse_line(
        &self,
        line: &[u8],
        current_data: &mut Vec<u8>,
        event_count: &mut usize,
    ) -> Result<Option<McpResponse>, ToolError> {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            if current_data.is_empty() {
                return Ok(None);
            }
            *event_count = event_count.checked_add(1).ok_or_else(|| {
                ToolError::ExternalService("MCP SSE event count overflow".to_string())
            })?;
            if *event_count > MAX_MCP_SSE_EVENTS {
                return Err(ToolError::ExternalService(format!(
                    "MCP SSE response exceeds the {MAX_MCP_SSE_EVENTS} event limit"
                )));
            }
            return self.process_sse_event(current_data).await;
        }

        let Some(data) = line.strip_prefix(b"data:") else {
            return Ok(None);
        };
        let data = data.strip_prefix(b" ").unwrap_or(data);
        let separator_bytes = usize::from(!current_data.is_empty());
        let new_len = current_data
            .len()
            .checked_add(separator_bytes)
            .and_then(|len| len.checked_add(data.len()))
            .ok_or_else(|| ToolError::ExternalService("MCP SSE event size overflow".to_string()))?;
        if new_len > MAX_MCP_HTTP_MESSAGE_BYTES {
            return Err(ToolError::ExternalService(format!(
                "MCP SSE event exceeds the {MAX_MCP_HTTP_MESSAGE_BYTES} byte limit"
            )));
        }
        if separator_bytes != 0 {
            current_data.push(b'\n');
        }
        current_data.extend_from_slice(data);
        Ok(None)
    }

    async fn parse_sse_response(
        &self,
        mut response: reqwest::Response,
    ) -> Result<McpResponse, ToolError> {
        if response
            .content_length()
            .is_some_and(|length| length > MAX_MCP_SSE_STREAM_BYTES as u64)
        {
            return Err(ToolError::ExternalService(format!(
                "MCP SSE response exceeds the {MAX_MCP_SSE_STREAM_BYTES} byte stream limit"
            )));
        }

        let mut current_data = Vec::new();
        let mut line = Vec::new();
        let mut total_bytes = 0usize;
        let mut event_count = 0usize;

        while let Some(chunk) = response.chunk().await.map_err(|e| {
            ToolError::ExternalService(format!("Failed to read MCP SSE response chunk: {e}"))
        })? {
            total_bytes = total_bytes.checked_add(chunk.len()).ok_or_else(|| {
                ToolError::ExternalService("MCP SSE stream size overflow".to_string())
            })?;
            if total_bytes > MAX_MCP_SSE_STREAM_BYTES {
                return Err(ToolError::ExternalService(format!(
                    "MCP SSE response exceeds the {MAX_MCP_SSE_STREAM_BYTES} byte stream limit"
                )));
            }

            let mut fragment_start = 0usize;
            for (index, byte) in chunk.iter().enumerate() {
                if *byte != b'\n' {
                    continue;
                }
                append_sse_line_fragment(&mut line, &chunk[fragment_start..index])?;
                if let Some(parsed) = self
                    .process_sse_line(&line, &mut current_data, &mut event_count)
                    .await?
                {
                    return Ok(parsed);
                }
                line.clear();
                fragment_start = index + 1;
            }
            append_sse_line_fragment(&mut line, &chunk[fragment_start..])?;
        }

        if !line.is_empty()
            && let Some(parsed) = self
                .process_sse_line(&line, &mut current_data, &mut event_count)
                .await?
        {
            return Ok(parsed);
        }
        if !current_data.is_empty() {
            event_count = event_count.checked_add(1).ok_or_else(|| {
                ToolError::ExternalService("MCP SSE event count overflow".to_string())
            })?;
            if event_count > MAX_MCP_SSE_EVENTS {
                return Err(ToolError::ExternalService(format!(
                    "MCP SSE response exceeds the {MAX_MCP_SSE_EVENTS} event limit"
                )));
            }
            if let Some(parsed) = self.process_sse_event(&mut current_data).await? {
                return Ok(parsed);
            }
        }

        Err(ToolError::ExternalService(
            "No JSON-RPC response frame found in SSE body".to_string(),
        ))
    }

    /// Initialize the connection to the MCP server.
    pub async fn initialize(&self) -> Result<InitializeResult, ToolError> {
        if let Some(ref session_manager) = self.session_manager
            && session_manager.is_initialized(&self.server_name).await
            && let Some(cached) = self.runtime.cached_initialize().await
        {
            return Ok(cached);
        }

        if let Some(ref session_manager) = self.session_manager {
            session_manager
                .get_or_create(&self.server_name, &self.server_url)
                .await;
        }

        let request = McpRequest::initialize_with_capabilities(
            self.next_request_id(),
            self.runtime.client_capabilities(),
        );
        let response = self.send_request(request).await?;

        if let Some(error) = response.error {
            return Err(ToolError::ExternalService(format!(
                "MCP initialization error: {} (code {})",
                error.message, error.code
            )));
        }

        let result: InitializeResult =
            serde_json::from_value(response.result.ok_or_else(|| {
                ToolError::ExternalService("No result in initialize response".to_string())
            })?)
            .map_err(|e| ToolError::ExternalService(format!("Invalid initialize result: {}", e)))?;

        let server_version = result.protocol_version.as_deref().ok_or_else(|| {
            ToolError::ExternalService(
                "MCP server did not advertise a protocolVersion during initialize".to_string(),
            )
        })?;
        if server_version != PROTOCOL_VERSION {
            return Err(ToolError::ExternalService(format!(
                "MCP server '{}' negotiated unsupported protocol version '{}' (expected '{}')",
                self.server_name, server_version, PROTOCOL_VERSION
            )));
        }

        self.runtime.remember_initialize(result.clone()).await;

        if let Some(ref session_manager) = self.session_manager {
            session_manager.mark_initialized(&self.server_name).await;
        }

        self.send_notification(McpNotification::initialized())
            .await?;

        Ok(result)
    }

    /// List available tools from the MCP server.
    pub async fn list_tools(&self) -> Result<Vec<McpTool>, ToolError> {
        if let Some(tools) = self.runtime.tools_cache.read().await.as_ref() {
            return Ok(tools.clone());
        }

        self.initialize().await?;

        let mut cursor = None::<String>;
        let mut tools = Vec::new();
        let mut pagination = CatalogPagination::default();
        loop {
            pagination.begin_page("tools")?;
            let request = McpRequest::list_tools(self.next_request_id(), cursor.as_deref());
            let response = self.send_request(request).await?;
            if let Some(error) = response.error {
                return Err(ToolError::ExternalService(format!(
                    "MCP error: {} (code {})",
                    error.message, error.code
                )));
            }
            let page: ListToolsResult =
                serde_json::from_value(response.result.ok_or_else(|| {
                    ToolError::ExternalService("No result in MCP tools/list response".to_string())
                })?)
                .map_err(|e| ToolError::ExternalService(format!("Invalid tools list: {}", e)))?;
            pagination.accept_items(tools.len(), &page.tools, "tools")?;
            tools.extend(page.tools);
            cursor = pagination.accept_cursor(page.cursor.next_cursor, "tools")?;
            if cursor.is_none() {
                break;
            }
        }

        *self.runtime.tools_cache.write().await = Some(tools.clone());
        Ok(tools)
    }

    /// List resources from the MCP server.
    pub async fn list_resources(&self) -> Result<Vec<McpResource>, ToolError> {
        if let Some(resources) = self.runtime.resources_cache.read().await.as_ref() {
            return Ok(resources.clone());
        }

        self.initialize().await?;

        let mut cursor = None::<String>;
        let mut resources = Vec::new();
        let mut pagination = CatalogPagination::default();
        loop {
            pagination.begin_page("resources")?;
            let request = McpRequest::list_resources(self.next_request_id(), cursor.as_deref());
            let response = self.send_request(request).await?;
            if let Some(error) = response.error {
                return Err(ToolError::ExternalService(format!(
                    "MCP resources/list error: {} (code {})",
                    error.message, error.code
                )));
            }
            let page: ListResourcesResult =
                serde_json::from_value(response.result.ok_or_else(|| {
                    ToolError::ExternalService(
                        "No result in MCP resources/list response".to_string(),
                    )
                })?)
                .map_err(|e| {
                    ToolError::ExternalService(format!("Invalid resources list: {}", e))
                })?;
            pagination.accept_items(resources.len(), &page.resources, "resources")?;
            resources.extend(page.resources);
            cursor = pagination.accept_cursor(page.cursor.next_cursor, "resources")?;
            if cursor.is_none() {
                break;
            }
        }

        *self.runtime.resources_cache.write().await = Some(resources.clone());
        Ok(resources)
    }

    /// Read a resource.
    pub async fn read_resource(&self, uri: &str) -> Result<ReadResourceResult, ToolError> {
        self.initialize().await?;
        let request = McpRequest::read_resource(self.next_request_id(), uri);
        let response = self.send_request(request).await?;
        if let Some(error) = response.error {
            return Err(ToolError::ExternalService(format!(
                "MCP resources/read error: {} (code {})",
                error.message, error.code
            )));
        }
        serde_json::from_value(response.result.ok_or_else(|| {
            ToolError::ExternalService("No result in MCP resources/read response".to_string())
        })?)
        .map_err(|e| ToolError::ExternalService(format!("Invalid resources/read result: {}", e)))
    }

    /// List resource templates.
    pub async fn list_resource_templates(&self) -> Result<Vec<McpResourceTemplate>, ToolError> {
        if let Some(templates) = self.runtime.resource_templates_cache.read().await.as_ref() {
            return Ok(templates.clone());
        }

        self.initialize().await?;

        let mut cursor = None::<String>;
        let mut templates = Vec::new();
        let mut pagination = CatalogPagination::default();
        loop {
            pagination.begin_page("resource templates")?;
            let request =
                McpRequest::list_resource_templates(self.next_request_id(), cursor.as_deref());
            let response = self.send_request(request).await?;
            if let Some(error) = response.error {
                return Err(ToolError::ExternalService(format!(
                    "MCP resources/templates/list error: {} (code {})",
                    error.message, error.code
                )));
            }
            let page: ListResourceTemplatesResult =
                serde_json::from_value(response.result.ok_or_else(|| {
                    ToolError::ExternalService(
                        "No result in MCP resources/templates/list response".to_string(),
                    )
                })?)
                .map_err(|e| {
                    ToolError::ExternalService(format!("Invalid resource template list: {}", e))
                })?;
            pagination.accept_items(
                templates.len(),
                &page.resource_templates,
                "resource templates",
            )?;
            templates.extend(page.resource_templates);
            cursor = pagination.accept_cursor(page.cursor.next_cursor, "resource templates")?;
            if cursor.is_none() {
                break;
            }
        }

        *self.runtime.resource_templates_cache.write().await = Some(templates.clone());
        Ok(templates)
    }

    /// Subscribe to resource change notifications.
    pub async fn subscribe_resource(&self, uri: &str) -> Result<(), ToolError> {
        self.initialize().await?;
        let response = self
            .send_request(McpRequest::subscribe_resource(self.next_request_id(), uri))
            .await?;
        if let Some(error) = response.error {
            return Err(ToolError::ExternalService(format!(
                "MCP resources/subscribe error: {} (code {})",
                error.message, error.code
            )));
        }
        Ok(())
    }

    /// Unsubscribe from resource change notifications.
    pub async fn unsubscribe_resource(&self, uri: &str) -> Result<(), ToolError> {
        self.initialize().await?;
        let response = self
            .send_request(McpRequest::unsubscribe_resource(
                self.next_request_id(),
                uri,
            ))
            .await?;
        if let Some(error) = response.error {
            return Err(ToolError::ExternalService(format!(
                "MCP resources/unsubscribe error: {} (code {})",
                error.message, error.code
            )));
        }
        Ok(())
    }

    /// List prompts from the MCP server.
    pub async fn list_prompts(&self) -> Result<Vec<McpPrompt>, ToolError> {
        if let Some(prompts) = self.runtime.prompts_cache.read().await.as_ref() {
            return Ok(prompts.clone());
        }

        self.initialize().await?;

        let mut cursor = None::<String>;
        let mut prompts = Vec::new();
        let mut pagination = CatalogPagination::default();
        loop {
            pagination.begin_page("prompts")?;
            let request = McpRequest::list_prompts(self.next_request_id(), cursor.as_deref());
            let response = self.send_request(request).await?;
            if let Some(error) = response.error {
                return Err(ToolError::ExternalService(format!(
                    "MCP prompts/list error: {} (code {})",
                    error.message, error.code
                )));
            }
            let page: ListPromptsResult =
                serde_json::from_value(response.result.ok_or_else(|| {
                    ToolError::ExternalService("No result in MCP prompts/list response".to_string())
                })?)
                .map_err(|e| ToolError::ExternalService(format!("Invalid prompts list: {}", e)))?;
            pagination.accept_items(prompts.len(), &page.prompts, "prompts")?;
            prompts.extend(page.prompts);
            cursor = pagination.accept_cursor(page.cursor.next_cursor, "prompts")?;
            if cursor.is_none() {
                break;
            }
        }

        *self.runtime.prompts_cache.write().await = Some(prompts.clone());
        Ok(prompts)
    }

    /// Get an MCP prompt with optional arguments.
    pub async fn get_prompt(
        &self,
        name: &str,
        arguments: Option<serde_json::Value>,
    ) -> Result<GetPromptResult, ToolError> {
        self.initialize().await?;
        let response = self
            .send_request(McpRequest::get_prompt(
                self.next_request_id(),
                name,
                arguments,
            ))
            .await?;
        if let Some(error) = response.error {
            return Err(ToolError::ExternalService(format!(
                "MCP prompts/get error: {} (code {})",
                error.message, error.code
            )));
        }
        serde_json::from_value(response.result.ok_or_else(|| {
            ToolError::ExternalService("No result in MCP prompts/get response".to_string())
        })?)
        .map_err(|e| ToolError::ExternalService(format!("Invalid prompts/get result: {}", e)))
    }

    /// Run argument completion for prompts/resource templates.
    pub async fn complete(
        &self,
        reference: serde_json::Value,
        argument: CompleteArgument,
    ) -> Result<CompleteResult, ToolError> {
        self.initialize().await?;
        let response = self
            .send_request(McpRequest::complete(
                self.next_request_id(),
                reference,
                argument,
            ))
            .await?;
        if let Some(error) = response.error {
            return Err(ToolError::ExternalService(format!(
                "MCP completion/complete error: {} (code {})",
                error.message, error.code
            )));
        }
        serde_json::from_value(response.result.ok_or_else(|| {
            ToolError::ExternalService("No result in MCP completion/complete response".to_string())
        })?)
        .map_err(|e| ToolError::ExternalService(format!("Invalid completion result: {}", e)))
    }

    /// Set the server log verbosity.
    pub async fn set_logging_level(&self, level: McpLoggingLevel) -> Result<(), ToolError> {
        self.initialize().await?;
        let protocol_level = match level {
            McpLoggingLevel::Debug => super::protocol::McpLoggingLevel::Debug,
            McpLoggingLevel::Info => super::protocol::McpLoggingLevel::Info,
            McpLoggingLevel::Warning => super::protocol::McpLoggingLevel::Warning,
            McpLoggingLevel::Error => super::protocol::McpLoggingLevel::Error,
        };
        let response = self
            .send_request(McpRequest::set_logging_level(
                self.next_request_id(),
                protocol_level,
            ))
            .await?;
        if let Some(error) = response.error {
            return Err(ToolError::ExternalService(format!(
                "MCP logging/setLevel error: {} (code {})",
                error.message, error.code
            )));
        }
        Ok(())
    }

    /// Call a tool on the MCP server.
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<CallToolResult, ToolError> {
        self.initialize().await?;

        let request = McpRequest::call_tool(self.next_request_id(), name, arguments);
        let response = self.send_request(request).await?;

        if let Some(error) = response.error {
            return Err(ToolError::ExecutionFailed(format!(
                "MCP tool error: {} (code {})",
                error.message, error.code
            )));
        }

        serde_json::from_value(response.result.ok_or_else(|| {
            ToolError::ExternalService("No result in MCP tools/call response".to_string())
        })?)
        .map_err(|e| ToolError::ExternalService(format!("Invalid tool result: {}", e)))
    }

    /// Clear all catalog caches.
    pub async fn clear_cache(&self) {
        self.runtime.clear_all_caches().await;
    }

    /// Update the in-memory roots grants for an active client.
    pub async fn update_roots_grants(&self, roots_grants: Vec<String>) -> bool {
        self.runtime.update_roots_grants(roots_grants).await
    }

    /// Notify the connected server that roots grants changed.
    pub async fn notify_roots_list_changed(&self) -> Result<(), ToolError> {
        if self.runtime.cached_initialize().await.is_none() {
            return Ok(());
        }
        self.send_notification(McpNotification::roots_list_changed())
            .await
    }

    /// Snapshot all pending server-initiated interactions for this client.
    pub async fn pending_interactions(&self) -> Vec<McpPendingInteraction> {
        self.runtime.list_pending_interactions().await
    }

    /// Resolve a pending sampling or elicitation interaction.
    pub async fn resolve_pending_interaction(
        &self,
        interaction_id: &str,
        approved: bool,
        result: Option<serde_json::Value>,
        message: Option<String>,
    ) -> Result<(), ToolError> {
        let resolution = if approved {
            PendingInteractionResolution::Approved(result.ok_or_else(|| {
                ToolError::InvalidParameters(
                    "Approved MCP interactions require a response payload".to_string(),
                )
            })?)
        } else {
            PendingInteractionResolution::Denied(
                message.unwrap_or_else(|| "User denied the MCP interaction".to_string()),
            )
        };
        self.runtime
            .resolve_pending_interaction(interaction_id, resolution)
            .await
    }

    /// Create Tool implementations for all MCP tools.
    pub async fn create_tools(&self) -> Result<Vec<Arc<dyn Tool>>, ToolError> {
        let mcp_tools = self.list_tools().await?;
        let client = Arc::new(self.clone());
        let prefix = Self::registered_tool_prefix(&self.server_name);
        let mut seen_names = HashSet::new();

        let mut tools: Vec<Arc<dyn Tool>> = Vec::with_capacity(mcp_tools.len());
        for tool in mcp_tools {
            let registered_name = format!("{prefix}{}", encode_tool_component(&tool.name));
            if !seen_names.insert(registered_name.clone()) {
                return Err(ToolError::ExternalService(format!(
                    "MCP server '{}' has colliding tool names after encoding: '{}'",
                    self.server_name, tool.name
                )));
            }
            tools.push(Arc::new(McpToolWrapper {
                tool,
                registered_name,
                client: client.clone(),
            }) as Arc<dyn Tool>);
        }

        Ok(tools)
    }

    /// Test the connection to the MCP server.
    pub async fn test_connection(&self) -> Result<(), ToolError> {
        self.initialize().await?;
        self.list_tools().await?;
        Ok(())
    }

    /// Lightweight liveness probe for background health monitoring.
    ///
    /// - **stdio**: reports whether the child process's reader loop is still
    ///   alive (`is_running()`), which flips to `false` the moment the process
    ///   exits — free and immediate, no I/O.
    /// - **HTTP**: performs a real `tools/list` round trip that bypasses the
    ///   client-side tool cache, so a server that became unreachable after the
    ///   initial connection is detected rather than masked by cached results.
    ///
    /// Returns `Ok(())` when the server is reachable/alive, `Err` otherwise.
    pub async fn health_check(&self) -> Result<(), ToolError> {
        if let Some(ref transport) = self.stdio_transport {
            return if transport.is_running() {
                Ok(())
            } else {
                Err(ToolError::ExternalService(format!(
                    "MCP stdio server '{}' process is not running",
                    self.server_name
                )))
            };
        }

        // HTTP: a live round trip that does not consult the tools cache.
        let request = McpRequest::list_tools(self.next_request_id(), None);
        let response = self.send_request(request).await?;
        if let Some(error) = response.error {
            return Err(ToolError::ExternalService(format!(
                "MCP server '{}' health probe error: {} (code {})",
                self.server_name, error.message, error.code
            )));
        }
        Ok(())
    }

    /// Stop transport-owned processes and cancel pending server interactions.
    pub async fn shutdown(&self) {
        self.runtime
            .cancel_all_pending_interactions("MCP client shut down")
            .await;
        if let Some(transport) = self.stdio_transport.as_ref() {
            transport.shutdown().await;
        }
    }
}

impl Clone for McpClient {
    fn clone(&self) -> Self {
        Self {
            server_url: self.server_url.clone(),
            server_name: self.server_name.clone(),
            http_client: self.http_client.clone(),
            http_client_allow_local: self.http_client_allow_local,
            http_client_validation_error: self.http_client_validation_error.clone(),
            stdio_transport: self.stdio_transport.clone(),
            next_id: AtomicU64::new(self.next_id.load(Ordering::SeqCst)),
            runtime: self.runtime.clone(),
            session_manager: self.session_manager.clone(),
            secrets: self.secrets.clone(),
            user_id: self.user_id.clone(),
            server_config: self.server_config.clone(),
            token_refresh_lock: self.token_refresh_lock.clone(),
        }
    }
}

mod support;
use support::*;

/// Wrapper that implements Tool for an MCP tool.
struct McpToolWrapper {
    tool: McpTool,
    registered_name: String,
    client: Arc<McpClient>,
}

#[async_trait]
impl Tool for McpToolWrapper {
    fn name(&self) -> &str {
        &self.registered_name
    }

    fn description(&self) -> &str {
        &self.tool.description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        self.tool.input_schema.clone()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let result = self.client.call_tool(&self.tool.name, params).await?;
        let artifacts = result
            .content
            .iter()
            .filter_map(content_block_to_artifact)
            .collect::<Vec<_>>();
        let text_preview = collect_text_preview(&result.content);

        if result.is_error {
            return Err(ToolError::ExecutionFailed(if text_preview.is_empty() {
                "MCP tool returned an error".to_string()
            } else {
                text_preview
            }));
        }

        let mut output = ToolOutput::success(
            serde_json::json!({
                "content": result.content,
                "structuredContent": result.structured_content,
                "text": if text_preview.is_empty() {
                    serde_json::Value::Null
                } else {
                    serde_json::Value::String(text_preview.clone())
                },
                "isError": result.is_error,
            }),
            start.elapsed(),
        )
        .with_artifacts(artifacts);

        if !text_preview.is_empty() {
            output = output.with_raw(text_preview);
        }

        Ok(output)
    }

    fn requires_sanitization(&self) -> bool {
        true
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        if self.tool.requires_approval() {
            ApprovalRequirement::UnlessAutoApproved
        } else {
            ApprovalRequirement::Never
        }
    }
}

fn collect_text_preview(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(ContentBlock::as_text)
        .collect::<Vec<_>>()
        .join("\n")
}

fn content_block_to_artifact(block: &ContentBlock) -> Option<ToolArtifact> {
    match block {
        ContentBlock::Text { text } => Some(ToolArtifact::Text { text: text.clone() }),
        ContentBlock::Image { data, mime_type } => Some(ToolArtifact::Image {
            data: data.clone(),
            mime_type: mime_type.clone(),
        }),
        ContentBlock::Audio { data, mime_type } => Some(ToolArtifact::Audio {
            data: data.clone(),
            mime_type: mime_type.clone(),
        }),
        ContentBlock::ResourceLink {
            uri,
            name,
            title,
            mime_type,
            description,
        } => Some(ToolArtifact::ResourceLink {
            uri: uri.clone(),
            name: name.clone(),
            title: title.clone(),
            mime_type: mime_type.clone(),
            description: description.clone(),
        }),
        ContentBlock::EmbeddedResource { resource } => match resource {
            McpResourceContents::Text {
                uri,
                mime_type,
                text,
            } => Some(ToolArtifact::EmbeddedResource {
                uri: uri.clone(),
                mime_type: mime_type.clone(),
                text: Some(text.clone()),
                blob: None,
            }),
            McpResourceContents::Blob {
                uri,
                mime_type,
                blob,
            } => Some(ToolArtifact::EmbeddedResource {
                uri: uri.clone(),
                mime_type: mime_type.clone(),
                text: None,
                blob: Some(blob.clone()),
            }),
        },
    }
}

#[cfg(test)]
mod tests;
