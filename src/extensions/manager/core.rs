//! Core [`ExtensionManager`] state, construction, post-construction setters,
//! the public setup/auth DTOs, and the small shared helpers reused across the
//! lifecycle/install/mcp/wasm/native/setup submodules.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
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

const MAX_EXTENSION_CAPABILITIES_BYTES: usize = 2 * 1024 * 1024;

/// A package snapshot protected against concurrent replacement.
///
/// Keep this value alive while making authorization decisions and loading the
/// corresponding WASM module. The publisher lock ensures both operations see
/// the same artifact generation.
pub(super) struct LockedWasmPackage {
    pub(super) capabilities_path: PathBuf,
    pub(super) capabilities_bytes: Option<Vec<u8>>,
    _publication_guard: thinclaw_platform::ArtifactReadGuard,
}

pub(super) async fn lock_wasm_package(
    directory: &Path,
    name: &str,
) -> Result<LockedWasmPackage, ExtensionError> {
    ExtensionManager::validate_extension_name(name)?;

    let directory_metadata = tokio::fs::symlink_metadata(directory)
        .await
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                ExtensionError::NotInstalled(format!(
                    "WASM package directory '{}' does not exist",
                    directory.display()
                ))
            } else {
                ExtensionError::Other(format!(
                    "failed to inspect WASM package directory '{}': {error}",
                    directory.display()
                ))
            }
        })?;
    if directory_metadata.file_type().is_symlink() || !directory_metadata.is_dir() {
        return Err(ExtensionError::Other(format!(
            "WASM package directory '{}' is not a real directory",
            directory.display()
        )));
    }

    let wasm_path = directory.join(format!("{name}.wasm"));
    let publication_guard = thinclaw_platform::acquire_artifact_read_lock(wasm_path.clone())
        .await
        .map_err(|error| {
            ExtensionError::Other(format!(
                "failed to lock WASM package '{}': {error}",
                wasm_path.display()
            ))
        })?;

    let marker_path = directory.join(format!(".{name}.wasm.installing.json"));
    match tokio::fs::symlink_metadata(&marker_path).await {
        Ok(_) => {
            return Err(ExtensionError::Other(format!(
                "WASM package '{}' has an incomplete publication journal",
                name
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(ExtensionError::Other(format!(
                "failed to inspect publication state for '{}': {error}",
                name
            )));
        }
    }

    let wasm_metadata = tokio::fs::symlink_metadata(&wasm_path)
        .await
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                ExtensionError::NotInstalled(format!("WASM package '{}' is not installed", name))
            } else {
                ExtensionError::Other(format!(
                    "failed to inspect WASM package '{}': {error}",
                    wasm_path.display()
                ))
            }
        })?;
    if wasm_metadata.file_type().is_symlink() || !wasm_metadata.is_file() {
        return Err(ExtensionError::Other(format!(
            "WASM package '{}' is not a regular file",
            wasm_path.display()
        )));
    }

    let capabilities_path = directory.join(format!("{name}.capabilities.json"));
    let capabilities_bytes = crate::registry::installer::read_regular_file_bounded(
        capabilities_path.clone(),
        MAX_EXTENSION_CAPABILITIES_BYTES,
    )
    .await
    .map_err(|error| {
        ExtensionError::Other(format!(
            "failed to read capabilities for '{}': {error}",
            name
        ))
    })?;

    Ok(LockedWasmPackage {
        capabilities_path,
        capabilities_bytes,
        _publication_guard: publication_guard,
    })
}

/// Pending OAuth authorization state.
pub(super) struct PendingAuth {
    pub(super) name: String,
    pub(super) kind: ExtensionKind,
    pub(super) code_verifier: Option<String>,
    pub(super) mcp_authorization: Option<crate::tools::mcp::auth::PreparedMcpAuthorization>,
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
    /// Serializes MCP config/client/tool-registry lifecycle transitions. MCP
    /// administration is rare, while allowing remove/auth/reconnect to race can
    /// resurrect a stale transport or leak its subprocess.
    pub(super) mcp_operation_lock: tokio::sync::Mutex<()>,
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
    /// Serializes in-process WASM activation/removal so an extension cannot be
    /// resurrected in the runtime while its package is being uninstalled.
    pub(super) wasm_operation_lock: tokio::sync::Mutex<()>,

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
            mcp_operation_lock: tokio::sync::Mutex::new(()),
            mcp_watchers: RwLock::new(HashMap::new()),
            mcp_health_monitor: RwLock::new(None),
            mcp_health_monitor_shutdown: RwLock::new(None),
            wasm_tool_runtime,
            wasm_tools_dir,
            wasm_channels_dir,
            wasm_operation_lock: tokio::sync::Mutex::new(()),
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
        let mut matches = Vec::new();
        match self.get_mcp_server(name).await {
            Ok(_) => matches.push(ExtensionKind::McpServer),
            Err(crate::tools::mcp::config::ConfigError::ServerNotFound { .. }) => {}
            Err(error) => {
                return Err(ExtensionError::Config(format!(
                    "failed to inspect MCP extension configuration: {error}"
                )));
            }
        }

        let wasm_path = self.wasm_tools_dir.join(format!("{}.wasm", name));
        if is_real_extension_file(&wasm_path).await? {
            matches.push(ExtensionKind::WasmTool);
        }

        let channel_path = self.wasm_channels_dir.join(format!("{}.wasm", name));
        if is_real_extension_file(&channel_path).await? {
            matches.push(ExtensionKind::WasmChannel);
        }

        // Check native plugins. A native plugin is "installed" once its signed
        // manifest has been scanned and registered (which only happens when the
        // operator has opted in via allow_native_plugins). Registration loads no
        // code; activation is still a separate, gated step.
        if self.native_plugins.read().await.is_registered(name) {
            matches.push(ExtensionKind::NativePlugin);
        }

        match matches.as_slice() {
            [kind] => Ok(*kind),
            [] => Err(ExtensionError::NotInstalled(format!(
                "'{}' is not installed as an MCP server, WASM tool, WASM channel, or native plugin",
                name
            ))),
            _ => Err(ExtensionError::Config(format!(
                "Extension name '{}' is ambiguous across installed kinds: {}",
                name,
                matches
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ))),
        }
    }

    /// Reject names containing path separators or traversal sequences.
    pub(super) fn validate_extension_name(name: &str) -> Result<(), ExtensionError> {
        if name.is_empty()
            || name.len() > 128
            || matches!(name, "." | "..")
            || name.contains("..")
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(ExtensionError::InstallFailed(format!(
                "Invalid extension name '{}': use 1-128 ASCII letters, digits, dots, hyphens, or underscores without traversal sequences",
                name
            )));
        }
        Ok(())
    }

    pub(super) async fn cleanup_expired_auths(&self) {
        let mut pending = self.pending_auth.write().await;
        pending.retain(|_, auth| auth.created_at.elapsed() < std::time::Duration::from_secs(300));
    }

    pub(super) async fn insert_pending_auth(&self, nonce: String, auth: PendingAuth) {
        const MAX_PENDING_AUTHS: usize = 256;
        let mut pending = self.pending_auth.write().await;
        pending.retain(|_, existing| {
            existing.created_at.elapsed() < std::time::Duration::from_secs(300)
                && (existing.kind != auth.kind || existing.name != auth.name)
        });
        if pending.len() >= MAX_PENDING_AUTHS
            && let Some(oldest) = pending
                .iter()
                .min_by_key(|(_, existing)| existing.created_at)
                .map(|(nonce, _)| nonce.clone())
        {
            pending.remove(&oldest);
        }
        pending.insert(nonce, auth);
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

async fn is_real_extension_file(path: &Path) -> Result<bool, ExtensionError> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => Ok(true),
        Ok(_) => Err(ExtensionError::Config(format!(
            "Installed extension path '{}' is not a regular file",
            path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(ExtensionError::Config(format!(
            "failed to inspect installed extension '{}': {error}",
            path.display()
        ))),
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
