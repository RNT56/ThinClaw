//! Lifecycle management for extensions: discovery, installation, authentication,
//! and activation of channels, tools, and MCP servers.
//!
//! Extensions are the user-facing abstraction that unifies three runtime kinds:
//! - **Channels** (Telegram, Slack, Discord) — messaging integrations (WASM)
//! - **Tools** — sandboxed capabilities (WASM)
//! - **MCP servers** — external API integrations via Model Context Protocol
//!
//! The agent can search a built-in registry (or discover online), install,
//! authenticate, and activate extensions at runtime without CLI commands.
//!
//! ```text
//!  User: "add telegram"
//!    -> tool_search("telegram")    -> finds channel in registry
//!    -> tool_install("telegram")   -> copies bundled WASM to channels dir
//!    -> tool_activate("telegram")  -> configures credentials, starts channel
//! ```

pub mod clawhub;
pub mod discovery;
pub mod ext_health_monitor;
pub mod lifecycle_hooks;
pub mod manager;
pub mod manifest;
pub mod manifest_validator;
pub mod native;
pub mod registry;
pub mod signing;

pub use discovery::OnlineDiscovery;
pub use manager::ExtensionManager;
pub use manifest::{PluginManifest, validate_plugin_manifest};
pub use native::NativePluginRuntime;
pub use registry::ExtensionRegistry;

use serde::{Deserialize, Serialize};
use thinclaw_types::{IntegrationSetupStatus, SetupAction, SetupAuthMode, SetupState};

/// The kind of extension, determining how it's installed, authenticated, and activated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionKind {
    /// Hosted MCP server, HTTP transport, OAuth 2.1 auth.
    McpServer,
    /// Sandboxed WASM module, file-based, capabilities auth.
    WasmTool,
    /// WASM channel module with hot-activation support.
    WasmChannel,
}

impl std::fmt::Display for ExtensionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExtensionKind::McpServer => write!(f, "mcp_server"),
            ExtensionKind::WasmTool => write!(f, "wasm_tool"),
            ExtensionKind::WasmChannel => write!(f, "wasm_channel"),
        }
    }
}

/// A registry entry describing a known or discovered extension.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryEntry {
    /// Unique identifier (e.g., "notion", "weather", "telegram").
    pub name: String,
    /// Human-readable name (e.g., "Notion", "Weather Tool").
    pub display_name: String,
    /// What kind of extension this is.
    pub kind: ExtensionKind,
    /// Short description of what this extension does.
    pub description: String,
    /// Search keywords beyond the name.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Where to get this extension.
    pub source: ExtensionSource,
    /// Fallback source when the primary source fails (e.g., download 404 → build from source).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_source: Option<Box<ExtensionSource>>,
    /// How authentication works.
    pub auth_hint: AuthHint,
}

/// Where the extension binary or server lives.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExtensionSource {
    /// URL to a hosted MCP server.
    McpUrl { url: String },
    /// Downloadable WASM binary.
    WasmDownload {
        wasm_url: String,
        #[serde(default)]
        capabilities_url: Option<String>,
    },
    /// Build from source repository.
    WasmBuildable {
        repo_url: String,
        #[serde(default)]
        build_dir: Option<String>,
        /// Crate name used to locate the build artifact binary.
        #[serde(default)]
        crate_name: Option<String>,
    },
    /// Discovered online (not yet validated for a specific source type).
    Discovered { url: String },
}

/// Hint about what authentication method is needed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthHint {
    /// MCP server supports Dynamic Client Registration (zero-config OAuth).
    Dcr,
    /// MCP server needs a pre-configured OAuth client_id.
    OAuthPreConfigured {
        /// URL where the user can create an OAuth app.
        setup_url: String,
    },
    /// WASM tool has auth defined in its capabilities.json file.
    CapabilitiesAuth,
    /// No authentication needed.
    None,
}

/// Where a search result came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultSource {
    /// From the built-in curated registry.
    Registry,
    /// From online discovery (validated).
    Discovered,
}

/// Result of searching for extensions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// The registry entry.
    #[serde(flatten)]
    pub entry: RegistryEntry,
    /// Where this result came from.
    pub source: ResultSource,
    /// Whether the endpoint was validated (for discovered entries).
    #[serde(default)]
    pub validated: bool,
}

/// Result of installing an extension.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallResult {
    pub name: String,
    pub kind: ExtensionKind,
    pub message: String,
}

/// Result of authenticating an extension.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResult {
    pub name: String,
    pub kind: ExtensionKind,
    /// Auth mode for the current flow (`oauth`, `manual_token`, `secrets`, `none`).
    pub auth_mode: String,
    /// Detailed auth status (`authenticated`, `awaiting_authorization`, etc.).
    pub auth_status: String,
    /// OAuth URL to open (for OAuth flows).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_url: Option<String>,
    /// Whether using local or remote callback.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callback_type: Option<String>,
    /// Instructions for manual token entry (for WASM tools).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// URL for manual token setup.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setup_url: Option<String>,
    /// Shared auth provider name (for shared credentials like Google).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_auth_provider: Option<String>,
    /// Missing OAuth scopes, if the stored credential needs reauth.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_scopes: Vec<String>,
    /// Whether the tool is waiting for a token from the user.
    #[serde(default)]
    pub awaiting_token: bool,
    /// Current auth status.
    pub status: String,
}

/// Result of activating an extension.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivateResult {
    pub name: String,
    pub kind: ExtensionKind,
    /// Names of tools that were loaded/registered.
    pub tools_loaded: Vec<String>,
    pub message: String,
}

fn default_true() -> bool {
    true
}

/// An installed extension with its current status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledExtension {
    pub name: String,
    pub kind: ExtensionKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Server or source URL (e.g. MCP server endpoint).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub authenticated: bool,
    pub auth_mode: String,
    pub auth_status: String,
    pub active: bool,
    /// Tool names if active.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Whether this extension has a setup schema (required_secrets) that can be configured.
    #[serde(default)]
    pub needs_setup: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_auth_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_scopes: Vec<String>,
    /// Whether this extension is installed locally (false = available in registry but not installed).
    #[serde(default = "default_true")]
    pub installed: bool,
    /// Last activation error for WASM channels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activation_error: Option<String>,
}

impl InstalledExtension {
    pub fn setup_status(&self) -> IntegrationSetupStatus {
        let auth_mode = setup_auth_mode_from_status(&self.auth_mode);
        if !self.installed {
            return IntegrationSetupStatus::not_installed(auth_mode);
        }
        if let Some(error) = &self.activation_error {
            return IntegrationSetupStatus::failed(auth_mode, error.clone());
        }

        let state = if self.active && self.authenticated {
            SetupState::Ready
        } else if !self.authenticated {
            SetupState::NeedsAuth
        } else if self.authenticated && !self.active {
            SetupState::InstalledUnconfigured
        } else {
            SetupState::Degraded
        };

        let mut actions = vec![SetupAction::Validate];
        if self.needs_setup {
            actions.push(SetupAction::ConfigureSecrets);
        }
        if matches!(auth_mode, SetupAuthMode::OAuth | SetupAuthMode::SharedOAuth)
            && !self.authenticated
        {
            actions.push(SetupAction::StartOAuth);
        }
        if self.active {
            actions.push(SetupAction::Disable);
        } else {
            actions.push(SetupAction::Activate);
        }
        actions.push(SetupAction::Remove);

        IntegrationSetupStatus {
            state,
            auth_mode,
            actions,
            message: (!self.auth_status.is_empty()).then(|| self.auth_status.clone()),
            ..IntegrationSetupStatus::default()
        }
    }
}

fn setup_auth_mode_from_status(auth_mode: &str) -> SetupAuthMode {
    match auth_mode {
        "oauth" => SetupAuthMode::OAuth,
        "shared_oauth" | "external_oauth_sync" => SetupAuthMode::SharedOAuth,
        "manual_token" | "secrets" | "manual_secrets" => SetupAuthMode::ManualSecrets,
        "native_plugin" => SetupAuthMode::NativePlugin,
        "remote_secret_backend" => SetupAuthMode::RemoteSecretBackend,
        _ => SetupAuthMode::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extension_status(
        installed: bool,
        authenticated: bool,
        active: bool,
        auth_mode: &str,
    ) -> InstalledExtension {
        InstalledExtension {
            name: "example".to_string(),
            kind: ExtensionKind::WasmChannel,
            description: None,
            url: None,
            authenticated,
            auth_mode: auth_mode.to_string(),
            auth_status: if authenticated {
                "authenticated".to_string()
            } else {
                "awaiting_authorization".to_string()
            },
            active,
            tools: Vec::new(),
            needs_setup: !authenticated,
            shared_auth_provider: None,
            missing_scopes: Vec::new(),
            installed,
            activation_error: None,
        }
    }

    #[test]
    fn setup_status_marks_available_extension_installable() {
        let setup = extension_status(false, false, false, "none").setup_status();
        assert_eq!(setup.state, SetupState::NotInstalled);
        assert_eq!(setup.actions, vec![SetupAction::Install]);
    }

    #[test]
    fn setup_status_marks_manual_secret_extension_needing_auth() {
        let setup = extension_status(true, false, false, "secrets").setup_status();
        assert_eq!(setup.state, SetupState::NeedsAuth);
        assert_eq!(setup.auth_mode, SetupAuthMode::ManualSecrets);
        assert!(setup.actions.contains(&SetupAction::ConfigureSecrets));
        assert!(setup.actions.contains(&SetupAction::Activate));
    }

    #[test]
    fn setup_status_marks_active_extension_ready() {
        let setup = extension_status(true, true, true, "oauth").setup_status();
        assert_eq!(setup.state, SetupState::Ready);
        assert_eq!(setup.auth_mode, SetupAuthMode::OAuth);
        assert!(setup.actions.contains(&SetupAction::Validate));
        assert!(setup.actions.contains(&SetupAction::Disable));
    }

    #[test]
    fn setup_status_preserves_activation_failure() {
        let mut ext = extension_status(true, true, false, "secrets");
        ext.activation_error = Some("bad config".to_string());
        let setup = ext.setup_status();
        assert_eq!(setup.state, SetupState::Failed);
        assert_eq!(setup.message.as_deref(), Some("bad config"));
    }
}

/// Error type for extension operations.
#[derive(Debug, thiserror::Error)]
pub enum ExtensionError {
    #[error("Extension not found: {0}")]
    NotFound(String),

    #[error("Extension already installed: {0}")]
    AlreadyInstalled(String),

    #[error("Extension not installed: {0}")]
    NotInstalled(String),

    #[error("Authentication failed: {0}")]
    AuthFailed(String),

    #[error("Activation failed: {0}")]
    ActivationFailed(String),

    #[error("Installation failed: {0}")]
    InstallFailed(String),

    #[error("Discovery failed: {0}")]
    DiscoveryFailed(String),

    #[error("Invalid URL: {0}")]
    InvalidUrl(String),

    #[error("Download failed: {0}")]
    DownloadFailed(String),

    #[error("Config error: {0}")]
    Config(String),

    #[error("{0}")]
    Other(String),
}
