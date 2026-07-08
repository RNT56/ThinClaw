//! Core [`ExtensionManager`] state, construction, post-construction setters,
//! the public setup/auth DTOs, and the small shared helpers reused across the
//! lifecycle/install/mcp/wasm/native/setup submodules.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{RwLock, oneshot};
use tokio::task::JoinHandle;

use crate::channels::ChannelManager;
use crate::channels::wasm::{WasmChannelHostConfig, WasmChannelRouter, WasmChannelRuntime};
use crate::extensions::clawhub::CatalogCache;
use crate::extensions::discovery::OnlineDiscovery;
use crate::extensions::native_activation::NativePluginState;
use crate::extensions::registry::ExtensionRegistry;
use crate::extensions::{AuthResult, ExtensionError, ExtensionKind, InstallResult, RegistryEntry};
use crate::hooks::HookRegistry;
use crate::pairing::PairingStore;
use crate::secrets::SecretsStore;
use crate::settings::Settings;
use crate::tools::ToolRegistry;
use crate::tools::mcp::McpClient;
use crate::tools::mcp::session::McpSessionManager;
use crate::tools::wasm::{
    WasmToolAuthCheck, WasmToolAuthMode, WasmToolAuthStatus, WasmToolRuntime,
};
use thinclaw_tools::builtin::extension_tools::{
    ExtensionInstallErrorKind, ExtensionInstallOutcome, ToolExtensionKind,
};

pub(super) type McpWatcherHandle = (JoinHandle<()>, oneshot::Sender<()>);
use thinclaw_types::SetupAuthMode;

/// Pending OAuth authorization state.
pub(super) struct PendingAuth {
    pub(super) name: String,
    pub(super) kind: ExtensionKind,
    pub(super) code_verifier: Option<String>,
    pub(super) redirect_uri: Option<String>,
    pub(super) thread_id: Option<String>,
    pub(super) created_at: std::time::Instant,
}

/// Runtime infrastructure needed for hot-activating WASM channels.
///
/// Set after construction via [`ExtensionManager::set_channel_runtime`] once the
/// channel manager, WASM runtime, pairing store, and webhook router are available.
pub(super) struct ChannelRuntimeState {
    pub(super) channel_manager: Arc<ChannelManager>,
    pub(super) wasm_channel_runtime: Arc<WasmChannelRuntime>,
    pub(super) pairing_store: Arc<PairingStore>,
    pub(super) wasm_channel_router: Arc<WasmChannelRouter>,
    pub(super) host_config: WasmChannelHostConfig,
}

/// Result of saving setup secrets and attempting activation.
pub struct SetupResult {
    /// Human-readable status message.
    pub message: String,
    /// Whether the channel was successfully activated after saving secrets.
    pub activated: bool,
}

#[derive(Debug, Clone, Default)]
pub struct AuthRequestContext {
    pub callback_base_url: Option<String>,
    pub callback_type: Option<String>,
    pub thread_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ExtensionSetupSchema {
    pub mode: String,
    pub auth_status: String,
    pub fields: Vec<crate::channels::web::types::SecretFieldInfo>,
    pub auth_url: Option<String>,
    pub instructions: Option<String>,
    pub setup_url: Option<String>,
    pub validation_url: Option<String>,
    pub shared_auth_provider: Option<String>,
    pub missing_scopes: Vec<String>,
}

pub(super) fn normalize_callback_base(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    if let Some(stripped) = trimmed.strip_suffix("/oauth/callback") {
        return Some(stripped.trim_end_matches('/').to_string());
    }
    if let Some(stripped) = trimmed.strip_suffix("/callback") {
        return Some(stripped.trim_end_matches('/').to_string());
    }
    Some(trimmed.to_string())
}

pub(super) fn setup_auth_mode_from_schema(mode: &str, fallback: SetupAuthMode) -> SetupAuthMode {
    match mode {
        "manual_token" | "secrets" | "manual_secrets" => SetupAuthMode::ManualSecrets,
        "oauth" => SetupAuthMode::OAuth,
        "shared_oauth" | "external_oauth_sync" => SetupAuthMode::SharedOAuth,
        "native_plugin" => SetupAuthMode::NativePlugin,
        "remote_secret_backend" => SetupAuthMode::RemoteSecretBackend,
        "none" => SetupAuthMode::None,
        _ => fallback,
    }
}

/// Central manager for extension lifecycle operations.
pub struct ExtensionManager {
    pub(super) registry: ExtensionRegistry,
    pub(super) discovery: OnlineDiscovery,

    // MCP infrastructure
    pub(super) mcp_session_manager: Arc<McpSessionManager>,
    /// Active MCP clients keyed by server name.
    pub(super) mcp_clients: RwLock<HashMap<String, Arc<McpClient>>>,
    /// Background config sync tasks keyed by server name.
    pub(super) mcp_watchers: RwLock<HashMap<String, McpWatcherHandle>>,
    /// Handle to the background MCP health monitor (probe + auto-reconnect).
    pub(super) mcp_health_monitor: RwLock<Option<JoinHandle<()>>>,
    /// Cooperative shutdown signal for the MCP health monitor.
    pub(super) mcp_health_monitor_shutdown: RwLock<Option<oneshot::Sender<()>>>,

    // WASM tool infrastructure
    pub(super) wasm_tool_runtime: Option<Arc<WasmToolRuntime>>,
    pub(super) wasm_tools_dir: PathBuf,
    pub(super) wasm_channels_dir: PathBuf,

    // WASM channel hot-activation infrastructure (set post-construction)
    pub(super) channel_runtime: RwLock<Option<ChannelRuntimeState>>,

    // Shared
    pub(super) secrets: Arc<dyn SecretsStore + Send + Sync>,
    pub(super) tool_registry: Arc<ToolRegistry>,
    pub(super) wasm_tool_invoker: Option<Arc<crate::tools::execution::HostMediatedToolInvoker>>,
    pub(super) hooks: Option<Arc<HookRegistry>>,
    pub(super) pending_auth: RwLock<HashMap<String, PendingAuth>>,
    pub(super) user_id: String,
    /// Optional database store for DB-backed MCP config.
    pub(super) store: Option<Arc<dyn crate::db::Database>>,
    /// Names of WASM channels that were successfully loaded at startup.
    pub(super) active_channel_names: RwLock<HashSet<String>>,
    /// Last activation error for each WASM channel (ephemeral, cleared on success).
    pub(super) activation_errors: RwLock<HashMap<String, String>>,
    /// SSE broadcast sender (set post-construction via `set_sse_sender()`).
    pub(super) sse_sender:
        RwLock<Option<tokio::sync::broadcast::Sender<crate::channels::web::types::SseEvent>>>,
    /// In-memory ClawHub catalog — populated by background prefetch at startup.
    pub(super) catalog_cache: Arc<tokio::sync::Mutex<CatalogCache>>,
    /// Optional lifecycle audit hook for recording install/activate/remove events.
    pub(super) lifecycle_audit_hook:
        RwLock<Option<Arc<crate::extensions::lifecycle_hooks::AuditLogHook>>>,
    /// Native dynamic-library plugin state (registered manifests, loaded
    /// runtimes, health). Strictly opt-in and default-off: nothing here loads
    /// native code unless `extensions.allow_native_plugins` is enabled and an
    /// operator has placed a signed manifest in an allowlisted directory.
    pub(super) native_plugins: RwLock<NativePluginState>,
}

impl ExtensionManager {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        mcp_session_manager: Arc<McpSessionManager>,
        secrets: Arc<dyn SecretsStore + Send + Sync>,
        tool_registry: Arc<ToolRegistry>,
        wasm_tool_invoker: Option<Arc<crate::tools::execution::HostMediatedToolInvoker>>,
        hooks: Option<Arc<HookRegistry>>,
        wasm_tool_runtime: Option<Arc<WasmToolRuntime>>,
        wasm_tools_dir: PathBuf,
        wasm_channels_dir: PathBuf,
        user_id: String,
        store: Option<Arc<dyn crate::db::Database>>,
        catalog_entries: Vec<RegistryEntry>,
    ) -> Self {
        let registry = if catalog_entries.is_empty() {
            ExtensionRegistry::new()
        } else {
            ExtensionRegistry::new_with_catalog(catalog_entries)
        };
        Self {
            registry,
            discovery: OnlineDiscovery::new(),
            mcp_session_manager,
            mcp_clients: RwLock::new(HashMap::new()),
            mcp_watchers: RwLock::new(HashMap::new()),
            mcp_health_monitor: RwLock::new(None),
            mcp_health_monitor_shutdown: RwLock::new(None),
            wasm_tool_runtime,
            wasm_tools_dir,
            wasm_channels_dir,
            channel_runtime: RwLock::new(None),
            secrets,
            tool_registry,
            wasm_tool_invoker,
            hooks,
            pending_auth: RwLock::new(HashMap::new()),
            user_id,
            store,
            active_channel_names: RwLock::new(HashSet::new()),
            activation_errors: RwLock::new(HashMap::new()),
            sse_sender: RwLock::new(None),
            catalog_cache: Arc::new(tokio::sync::Mutex::new(CatalogCache::new(3600))),
            lifecycle_audit_hook: RwLock::new(None),
            native_plugins: RwLock::new(NativePluginState::new()),
        }
    }

    /// Get a clone of the shared catalog cache Arc — for background prefetch by `app.rs`.
    pub fn catalog_cache(&self) -> Arc<tokio::sync::Mutex<CatalogCache>> {
        Arc::clone(&self.catalog_cache)
    }

    /// Configure the channel runtime infrastructure for hot-activating WASM channels.
    ///
    /// Call after construction (and after wrapping in `Arc`) once the channel
    /// manager, WASM runtime, pairing store, and webhook router are available.
    /// Without this, channel activation returns an error.
    pub async fn set_channel_runtime(
        &self,
        channel_manager: Arc<ChannelManager>,
        wasm_channel_runtime: Arc<WasmChannelRuntime>,
        pairing_store: Arc<PairingStore>,
        wasm_channel_router: Arc<WasmChannelRouter>,
        host_config: WasmChannelHostConfig,
    ) {
        *self.channel_runtime.write().await = Some(ChannelRuntimeState {
            channel_manager,
            wasm_channel_runtime,
            pairing_store,
            wasm_channel_router,
            host_config,
        });
    }

    /// Register channel names that were loaded at startup.
    /// Called after WASM channels are loaded so `list()` reports accurate active status.
    pub async fn set_active_channels(&self, names: Vec<String>) {
        let mut active = self.active_channel_names.write().await;
        active.extend(names);
    }

    /// Persist the set of active channel names to the settings store.
    ///
    /// Saved under key `activated_channels` so channels auto-activate on restart.
    pub(super) async fn persist_active_channels(&self) {
        let Some(ref store) = self.store else {
            return;
        };
        let names: Vec<String> = self
            .active_channel_names
            .read()
            .await
            .iter()
            .cloned()
            .collect();
        let value = serde_json::json!(names);
        if let Err(e) = store
            .set_setting(&self.user_id, "activated_channels", &value)
            .await
        {
            tracing::warn!(error = %e, "Failed to persist activated_channels setting");
        }
    }

    /// Load previously activated channel names from the settings store.
    ///
    /// Returns channel names that were activated in a prior session so they can
    /// be auto-activated at startup.
    pub async fn load_persisted_active_channels(&self) -> Vec<String> {
        let Some(ref store) = self.store else {
            return Vec::new();
        };
        match store.get_setting(&self.user_id, "activated_channels").await {
            Ok(Some(value)) => match serde_json::from_value(value) {
                Ok(names) => names,
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to deserialize activated_channels");
                    Vec::new()
                }
            },
            Ok(None) => Vec::new(),
            Err(e) => {
                tracing::warn!(error = %e, "Failed to load activated_channels setting");
                Vec::new()
            }
        }
    }

    /// Set the SSE broadcast sender for pushing extension status events to the web UI.
    pub async fn set_sse_sender(
        &self,
        sender: tokio::sync::broadcast::Sender<crate::channels::web::types::SseEvent>,
    ) {
        *self.sse_sender.write().await = Some(sender);
    }

    pub(super) fn auth_result(
        &self,
        name: &str,
        kind: ExtensionKind,
        auth_mode: impl Into<String>,
        auth_status: impl Into<String>,
    ) -> AuthResult {
        let auth_mode = auth_mode.into();
        let auth_status = auth_status.into();
        AuthResult {
            name: name.to_string(),
            kind,
            auth_mode,
            auth_status: auth_status.clone(),
            auth_url: None,
            callback_type: None,
            instructions: None,
            setup_url: None,
            shared_auth_provider: None,
            missing_scopes: Vec::new(),
            awaiting_token: false,
            status: auth_status,
        }
    }

    pub(super) async fn resolve_callback_base_url(
        &self,
        browser_origin: Option<&str>,
    ) -> Option<String> {
        if let Some(origin) = browser_origin
            && let Some(normalized) = normalize_callback_base(origin)
        {
            return Some(normalized);
        }

        if let Ok(override_url) = std::env::var("THINCLAW_OAUTH_CALLBACK_URL")
            && let Some(normalized) = normalize_callback_base(&override_url)
        {
            return Some(normalized);
        }

        if let Some(ref store) = self.store
            && let Ok(map) = store.get_all_settings(&self.user_id).await
        {
            let settings = Settings::from_db_map(&map);
            if let Some(url) = settings.tunnel.public_url
                && let Some(normalized) = normalize_callback_base(&url)
            {
                return Some(normalized);
            }
        }

        let settings = Settings::load();
        settings
            .tunnel
            .public_url
            .as_deref()
            .and_then(normalize_callback_base)
    }

    pub(super) fn apply_auth_check(result: &mut AuthResult, check: &WasmToolAuthCheck) {
        result.auth_mode = check.auth_mode.as_str().to_string();
        result.auth_status = check.auth_status.as_str().to_string();
        result.status = result.auth_status.clone();
        result.awaiting_token = matches!(
            (check.auth_mode, check.auth_status),
            (
                WasmToolAuthMode::ManualToken,
                WasmToolAuthStatus::AwaitingToken
            )
        );
        result.shared_auth_provider = check.shared_auth_provider.clone();
        result.missing_scopes = check.missing_scopes.clone();
    }

    pub(super) async fn broadcast_auth_completed(
        &self,
        extension_name: &str,
        success: bool,
        message: impl Into<String>,
        auth_result: Option<&AuthResult>,
        thread_id: Option<String>,
    ) {
        let Some(sender) = self.sse_sender.read().await.clone() else {
            return;
        };
        let (auth_mode, auth_status, shared_auth_provider, missing_scopes) = auth_result
            .map(|result| {
                (
                    Some(result.auth_mode.clone()),
                    Some(result.auth_status.clone()),
                    result.shared_auth_provider.clone(),
                    result.missing_scopes.clone(),
                )
            })
            .unwrap_or((None, None, None, Vec::new()));
        let _ = sender.send(crate::channels::web::types::SseEvent::AuthCompleted {
            extension_name: extension_name.to_string(),
            success,
            message: message.into(),
            auth_mode,
            auth_status,
            shared_auth_provider,
            missing_scopes,
            thread_id,
        });
    }

    /// Set the lifecycle audit hook for recording plugin install/activate/remove events.
    pub async fn set_lifecycle_audit_hook(
        &self,
        hook: Arc<crate::extensions::lifecycle_hooks::AuditLogHook>,
    ) {
        *self.lifecycle_audit_hook.write().await = Some(hook);
    }

    /// Fire a lifecycle event to the audit hook (if set).
    pub(super) async fn fire_lifecycle_event(
        &self,
        event: crate::extensions::lifecycle_hooks::LifecycleEvent,
    ) {
        if let Some(ref hook) = *self.lifecycle_audit_hook.read().await {
            use crate::extensions::lifecycle_hooks::LifecycleHook;
            hook.on_event(&event);
        }
    }

    /// Broadcast an extension status change to the web UI via SSE.
    pub(super) async fn broadcast_extension_status(
        &self,
        name: &str,
        status: &str,
        message: Option<&str>,
    ) {
        if let Some(ref sender) = *self.sse_sender.read().await {
            let _ = sender.send(crate::channels::web::types::SseEvent::ExtensionStatus {
                extension_name: name.to_string(),
                status: status.to_string(),
                message: message.map(|m| m.to_string()),
            });
        }
    }

    /// Determine what kind of installed extension this is.
    pub(super) async fn determine_installed_kind(
        &self,
        name: &str,
    ) -> Result<ExtensionKind, ExtensionError> {
        // Check MCP servers first
        if self.get_mcp_server(name).await.is_ok() {
            return Ok(ExtensionKind::McpServer);
        }

        // Check WASM tools
        let wasm_path = self.wasm_tools_dir.join(format!("{}.wasm", name));
        if wasm_path.exists() {
            return Ok(ExtensionKind::WasmTool);
        }

        // Check WASM channels
        let channel_path = self.wasm_channels_dir.join(format!("{}.wasm", name));
        if channel_path.exists() {
            return Ok(ExtensionKind::WasmChannel);
        }

        // Check native plugins. A native plugin is "installed" once its signed
        // manifest has been scanned and registered (which only happens when the
        // operator has opted in via allow_native_plugins). Registration loads no
        // code; activation is still a separate, gated step.
        if self.native_plugins.read().await.is_registered(name) {
            return Ok(ExtensionKind::NativePlugin);
        }

        Err(ExtensionError::NotInstalled(format!(
            "'{}' is not installed as an MCP server, WASM tool, WASM channel, or native plugin",
            name
        )))
    }

    /// Reject names containing path separators or traversal sequences.
    pub(super) fn validate_extension_name(name: &str) -> Result<(), ExtensionError> {
        if name.contains('/') || name.contains('\\') || name.contains("..") || name.contains('\0') {
            return Err(ExtensionError::InstallFailed(format!(
                "Invalid extension name '{}': contains path separator or traversal characters",
                name
            )));
        }
        Ok(())
    }

    pub(super) async fn cleanup_expired_auths(&self) {
        let mut pending = self.pending_auth.write().await;
        pending.retain(|_, auth| auth.created_at.elapsed() < std::time::Duration::from_secs(300));
    }

    /// Validate an OAuth state nonce against pending authentications.
    ///
    /// Returns true if the nonce matches any pending auth entry (and the entry
    /// hasn't expired). This prevents CSRF attacks on the OAuth callback.
    pub async fn validate_pending_auth_nonce(&self, nonce: &str) -> bool {
        self.cleanup_expired_auths().await;
        let pending = self.pending_auth.read().await;
        pending.contains_key(nonce)
    }

    pub(super) async fn take_pending_auth(&self, nonce: &str) -> Option<PendingAuth> {
        self.cleanup_expired_auths().await;
        self.pending_auth.write().await.remove(nonce)
    }

    pub(super) async fn unregister_hook_prefix(&self, prefix: &str) -> usize {
        let Some(ref hooks) = self.hooks else {
            return 0;
        };

        let names = hooks.list().await;
        let mut removed = 0;
        for hook_name in names {
            if hook_name.starts_with(prefix) && hooks.unregister(&hook_name).await {
                removed += 1;
            }
        }
        removed
    }
}

pub(super) fn extension_kind_from_tool_kind(kind: ToolExtensionKind) -> ExtensionKind {
    match kind {
        ToolExtensionKind::McpServer => ExtensionKind::McpServer,
        ToolExtensionKind::WasmTool => ExtensionKind::WasmTool,
        ToolExtensionKind::WasmChannel => ExtensionKind::WasmChannel,
    }
}

pub(super) fn install_outcome(
    result: &Result<InstallResult, ExtensionError>,
) -> ExtensionInstallOutcome {
    match result {
        Ok(_) => ExtensionInstallOutcome::Success,
        Err(ExtensionError::AlreadyInstalled(_)) => ExtensionInstallOutcome::AlreadyInstalled,
        Err(_) => ExtensionInstallOutcome::Failed,
    }
}

pub(super) fn install_error_kind(error: &ExtensionError) -> ExtensionInstallErrorKind {
    match error {
        ExtensionError::AlreadyInstalled(_) => ExtensionInstallErrorKind::AlreadyInstalled,
        _ => ExtensionInstallErrorKind::Other,
    }
}
