//! MCP server configuration.
//!
//! Stores configuration for connecting to hosted MCP servers.
//! Configuration is persisted at ~/.thinclaw/mcp-servers.json.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use serde::{Deserialize, Serialize};
use tokio::fs;

use thinclaw_tools_core::{OutboundUrlGuardOptions, ToolError, validate_outbound_url};

/// Transport type for MCP servers.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    /// HTTP/HTTPS transport (Streamable HTTP).
    #[default]
    Http,
    /// Stdio transport — spawn a child process and communicate via stdin/stdout.
    Stdio,
}

/// Configuration for connecting to a remote MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Unique name for this server (e.g., "notion", "github").
    #[serde(alias = "id")]
    pub name: String,

    /// Optional human-friendly name for display surfaces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,

    /// Server URL (must be HTTPS for remote servers).
    /// Required for HTTP transport, unused for stdio.
    #[serde(default)]
    pub url: String,

    /// Transport type: "http" (default) or "stdio".
    #[serde(default)]
    pub transport: McpTransport,

    /// Command to run for stdio transport (e.g., "npx", "uvx", "python").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,

    /// Arguments to pass to the command for stdio transport.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,

    /// Extra environment variables to set for the stdio child process.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,

    /// OAuth configuration (if server requires authentication).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth: Option<OAuthConfig>,

    /// Whether this server auto-activates at startup.
    #[serde(default = "default_true", alias = "auto_activate")]
    pub enabled: bool,

    /// Allow loopback/private HTTP endpoints for local development.
    #[serde(default)]
    pub allow_local_http: bool,

    /// Per-server capability policy for client features.
    #[serde(default)]
    pub capability_policy: McpCapabilityPolicy,

    /// Explicit roots granted to this server when roots capability is enabled.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roots_grants: Vec<String>,

    /// Desired logging level for this server.
    #[serde(default)]
    pub logging_level: McpLoggingLevel,

    /// Persisted metadata discovered during prior connections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,

    /// Last-known runtime health snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_health: Option<McpRuntimeHealth>,

    /// Optional description for the server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

fn default_true() -> bool {
    true
}

impl McpServerConfig {
    /// Create a new MCP server configuration (HTTP transport).
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            transport: McpTransport::Http,
            command: None,
            args: Vec::new(),
            env: BTreeMap::new(),
            oauth: None,
            enabled: true,
            display_name: None,
            allow_local_http: false,
            capability_policy: McpCapabilityPolicy::default(),
            roots_grants: Vec::new(),
            logging_level: McpLoggingLevel::default(),
            metadata: None,
            runtime_health: None,
            description: None,
        }
    }

    /// Create a new stdio MCP server configuration.
    pub fn new_stdio(
        name: impl Into<String>,
        command: impl Into<String>,
        args: Vec<String>,
    ) -> Self {
        Self {
            name: name.into(),
            url: String::new(),
            transport: McpTransport::Stdio,
            command: Some(command.into()),
            args,
            env: BTreeMap::new(),
            oauth: None,
            enabled: true,
            display_name: None,
            allow_local_http: false,
            capability_policy: McpCapabilityPolicy::default(),
            roots_grants: Vec::new(),
            logging_level: McpLoggingLevel::default(),
            metadata: None,
            runtime_health: None,
            description: None,
        }
    }

    /// Set the display name.
    pub fn with_display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = Some(display_name.into());
        self
    }

    /// Set OAuth configuration.
    pub fn with_oauth(mut self, oauth: OAuthConfig) -> Self {
        self.oauth = Some(oauth);
        self
    }

    /// Set description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set environment variables for stdio servers.
    pub fn with_env(mut self, env: BTreeMap<String, String>) -> Self {
        self.env = env;
        self
    }

    /// Check whether this config uses stdio transport.
    pub fn is_stdio(&self) -> bool {
        self.transport == McpTransport::Stdio
    }

    /// Validate the server configuration.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.name.is_empty() {
            return Err(ConfigError::InvalidConfig {
                reason: "Server name cannot be empty".to_string(),
            });
        }

        match self.transport {
            McpTransport::Http => {
                if self.url.is_empty() {
                    return Err(ConfigError::InvalidConfig {
                        reason: "Server URL cannot be empty for HTTP transport".to_string(),
                    });
                }

                let options = OutboundUrlGuardOptions {
                    require_https: !self.allow_local_http,
                    upgrade_http_to_https: false,
                    allowlist: Vec::new(),
                };

                if let Err(error) = validate_outbound_url(&self.url, &options) {
                    if !self.allow_local_http && is_localhost_url(&self.url) {
                        return Ok(());
                    }

                    return Err(ConfigError::InvalidConfig {
                        reason: error.to_string(),
                    });
                }

                if self.allow_local_http {
                    let parsed =
                        url::Url::parse(&self.url).map_err(|e| ConfigError::InvalidConfig {
                            reason: format!("Invalid MCP server URL: {e}"),
                        })?;
                    if !matches!(parsed.scheme(), "http" | "https") {
                        return Err(ConfigError::InvalidConfig {
                            reason: "MCP server URLs must use http:// or https://".to_string(),
                        });
                    }
                }
            }
            McpTransport::Stdio => {
                if self.command.is_none() || self.command.as_deref() == Some("") {
                    return Err(ConfigError::InvalidConfig {
                        reason: "Command is required for stdio transport".to_string(),
                    });
                }
            }
        }

        Ok(())
    }

    /// Check if this server requires authentication.
    ///
    /// Returns true if OAuth is pre-configured OR if this is a remote HTTPS server
    /// (which likely supports Dynamic Client Registration even without pre-configured OAuth).
    pub fn requires_auth(&self) -> bool {
        // Stdio servers never use HTTP auth.
        if self.transport == McpTransport::Stdio {
            return false;
        }
        if self.oauth.is_some() {
            return true;
        }
        // Remote HTTPS servers need auth handling (DCR, token refresh, 401 detection).
        // Localhost/127.0.0.1 servers are assumed to be dev servers without auth.
        let url_lower = self.url.to_lowercase();
        let is_localhost = is_localhost_url(&url_lower);
        url_lower.starts_with("https://") && !is_localhost
    }

    /// Stable tool namespace used for registered ThinClaw tool names.
    pub fn tool_namespace(&self) -> String {
        self.name
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() {
                    ch.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect()
    }

    /// Display label for UI/CLI surfaces.
    pub fn display_label(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.name)
    }

    /// Whether this server should auto-activate at startup.
    pub fn auto_activate(&self) -> bool {
        self.enabled
    }

    /// Get the secret name used to store the access token.
    pub fn token_secret_name(&self) -> String {
        format!("mcp_{}_access_token", self.name)
    }

    /// Get the secret name used to store the refresh token.
    pub fn refresh_token_secret_name(&self) -> String {
        format!("mcp_{}_refresh_token", self.name)
    }

    /// Get the secret name used to store the DCR client ID.
    pub fn client_id_secret_name(&self) -> String {
        format!("mcp_{}_client_id", self.name)
    }
}

/// OAuth 2.1 configuration for an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthConfig {
    /// OAuth client ID.
    pub client_id: String,

    /// Authorization endpoint URL.
    /// If not provided, will be discovered from /.well-known/oauth-protected-resource.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_url: Option<String>,

    /// Token endpoint URL.
    /// If not provided, will be discovered from /.well-known/oauth-authorization-server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_url: Option<String>,

    /// Scopes to request.
    #[serde(default)]
    pub scopes: Vec<String>,

    /// Explicit protected-resource identifier to use for OAuth resource binding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,

    /// Whether to use PKCE (default: true, as required by OAuth 2.1).
    #[serde(default = "default_true")]
    pub use_pkce: bool,

    /// Extra parameters to include in the authorization request.
    #[serde(default)]
    pub extra_params: HashMap<String, String>,
}

impl OAuthConfig {
    /// Create a new OAuth configuration with just a client ID.
    pub fn new(client_id: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            authorization_url: None,
            token_url: None,
            scopes: Vec::new(),
            resource: None,
            use_pkce: true,
            extra_params: HashMap::new(),
        }
    }

    /// Set authorization and token URLs.
    pub fn with_endpoints(
        mut self,
        authorization_url: impl Into<String>,
        token_url: impl Into<String>,
    ) -> Self {
        self.authorization_url = Some(authorization_url.into());
        self.token_url = Some(token_url.into());
        self
    }

    /// Set scopes.
    pub fn with_scopes(mut self, scopes: Vec<String>) -> Self {
        self.scopes = scopes;
        self
    }

    /// Set the OAuth resource identifier.
    pub fn with_resource(mut self, resource: impl Into<String>) -> Self {
        self.resource = Some(resource.into());
        self
    }
}

/// Configuration file containing all MCP servers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpServersFile {
    /// List of configured MCP servers.
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,

    /// Schema version for future compatibility.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
}

fn default_schema_version() -> u32 {
    2
}

impl McpServersFile {
    /// Upgrade older config payloads to the current schema defaults.
    pub fn migrate_in_place(&mut self) {
        if self.schema_version < 2 {
            for server in &mut self.servers {
                if server.display_name.is_none() {
                    server.display_name = Some(server.name.clone());
                }
            }
            self.schema_version = 2;
        }
    }

    /// Get a server by name.
    pub fn get(&self, name: &str) -> Option<&McpServerConfig> {
        self.servers.iter().find(|s| s.name == name)
    }

    /// Get a mutable server by name.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut McpServerConfig> {
        self.servers.iter_mut().find(|s| s.name == name)
    }

    /// Add or update a server configuration.
    pub fn upsert(&mut self, config: McpServerConfig) {
        if let Some(existing) = self.get_mut(&config.name) {
            *existing = config;
        } else {
            self.servers.push(config);
        }
    }

    /// Remove a server by name.
    pub fn remove(&mut self, name: &str) -> bool {
        let len_before = self.servers.len();
        self.servers.retain(|s| s.name != name);
        self.servers.len() < len_before
    }

    /// Get all enabled servers.
    pub fn enabled_servers(&self) -> impl Iterator<Item = &McpServerConfig> {
        self.servers.iter().filter(|s| s.enabled)
    }
}

/// Policy for advertising and serving MCP client capabilities.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpCapabilityPolicy {
    #[serde(default = "default_true")]
    pub tools: bool,
    #[serde(default = "default_true")]
    pub resources: bool,
    #[serde(default = "default_true")]
    pub prompts: bool,
    #[serde(default = "default_true")]
    pub completion: bool,
    #[serde(default)]
    pub roots: bool,
    #[serde(default)]
    pub sampling: bool,
    #[serde(default)]
    pub sampling_tools: bool,
    #[serde(default)]
    pub form_elicitation: bool,
    #[serde(default = "default_logging_enabled")]
    pub logging: bool,
}

fn default_logging_enabled() -> bool {
    true
}

impl Default for McpCapabilityPolicy {
    fn default() -> Self {
        Self {
            tools: true,
            resources: true,
            prompts: true,
            completion: true,
            roots: false,
            sampling: false,
            sampling_tools: false,
            form_elicitation: false,
            logging: true,
        }
    }
}

/// Last-known runtime health for a configured server.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpRuntimeHealth {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_connected_at: Option<String>,
    #[serde(default)]
    pub connected: bool,
}

/// Persisted per-server logging preference.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McpLoggingLevel {
    Debug,
    Info,
    #[default]
    Warning,
    Error,
}

/// Error type for MCP configuration operations.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Invalid configuration: {reason}")]
    InvalidConfig { reason: String },

    #[error("Server not found: {name}")]
    ServerNotFound { name: String },
}

impl From<ConfigError> for ToolError {
    fn from(err: ConfigError) -> Self {
        ToolError::ExternalService(err.to_string())
    }
}

/// Load MCP server configurations from a specific path.
pub async fn load_mcp_servers_from(path: impl AsRef<Path>) -> Result<McpServersFile, ConfigError> {
    let path = path.as_ref();

    if !path.exists() {
        return Ok(McpServersFile::default());
    }

    let content = fs::read_to_string(path).await?;
    let mut config: McpServersFile = serde_json::from_str(&content)?;
    config.migrate_in_place();

    Ok(config)
}

/// Save MCP server configurations to a specific path.
pub async fn save_mcp_servers_to(
    config: &McpServersFile,
    path: impl AsRef<Path>,
) -> Result<(), ConfigError> {
    let path = path.as_ref();

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }

    let mut config = config.clone();
    config.migrate_in_place();
    let content = serde_json::to_string_pretty(&config)?;
    fs::write(path, content).await?;

    Ok(())
}

/// Check if a URL points to a loopback address (localhost, 127.0.0.1, [::1]).
///
/// Uses `url::Url` for proper parsing so edge cases (IPv6, userinfo, ports)
/// are handled correctly without manual string splitting.
fn is_localhost_url(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    match parsed.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_config_path(name: &str) -> std::path::PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!("thinclaw-tools-mcp-config-{stamp}-{name}"))
            .join("mcp-servers.json")
    }

    #[test]
    fn test_is_localhost_url() {
        assert!(is_localhost_url("http://localhost:3000/path"));
        assert!(is_localhost_url("https://localhost/path"));
        assert!(is_localhost_url("http://127.0.0.1:8080"));
        assert!(is_localhost_url("http://127.0.0.1"));
        assert!(!is_localhost_url("https://notlocalhost.com/path"));
        assert!(!is_localhost_url("https://example-localhost.io"));
        assert!(!is_localhost_url("https://mcp.notion.com"));
        assert!(is_localhost_url("http://user:pass@localhost:3000/path"));
        // IPv6 loopback
        assert!(is_localhost_url("http://[::1]:8080/path"));
        assert!(is_localhost_url("http://[::1]/path"));
        assert!(!is_localhost_url("http://[::2]:8080/path"));
    }

    #[test]
    fn test_server_config_validation() {
        // Valid HTTPS server
        let config = McpServerConfig::new("notion", "https://mcp.notion.com");
        assert!(config.validate().is_ok());

        // Valid localhost (allowed for dev)
        let config = McpServerConfig::new("local", "http://localhost:8080");
        assert!(config.validate().is_ok());

        // Invalid: empty name
        let config = McpServerConfig::new("", "https://example.com");
        assert!(config.validate().is_err());

        // Invalid: HTTP for remote server
        let config = McpServerConfig::new("remote", "http://mcp.example.com");
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_stdio_config_validation() {
        // Valid stdio server
        let config = McpServerConfig::new_stdio(
            "filesystem",
            "npx",
            vec![
                "-y".to_string(),
                "@modelcontextprotocol/server-filesystem".to_string(),
            ],
        );
        assert!(config.validate().is_ok());
        assert!(config.is_stdio());
        assert!(!config.requires_auth());

        // Invalid: stdio without command
        let mut bad = McpServerConfig::new_stdio("bad", "", vec![]);
        bad.command = None;
        assert!(bad.validate().is_err());
    }

    #[test]
    fn test_stdio_config_serialization() {
        let config = McpServerConfig::new_stdio(
            "filesystem",
            "npx",
            vec![
                "-y".to_string(),
                "@modelcontextprotocol/server-filesystem".to_string(),
            ],
        );
        let json = serde_json::to_value(&config).unwrap();
        assert_eq!(json["transport"], "stdio");
        assert_eq!(json["command"], "npx");

        // Roundtrip
        let restored: McpServerConfig = serde_json::from_value(json).unwrap();
        assert_eq!(restored.transport, McpTransport::Stdio);
        assert_eq!(restored.command.as_deref(), Some("npx"));
    }

    #[test]
    fn test_oauth_config_builder() {
        let oauth = OAuthConfig::new("client-123")
            .with_endpoints(
                "https://auth.example.com/authorize",
                "https://auth.example.com/token",
            )
            .with_scopes(vec!["read".to_string(), "write".to_string()]);

        assert_eq!(oauth.client_id, "client-123");
        assert!(oauth.authorization_url.is_some());
        assert!(oauth.token_url.is_some());
        assert_eq!(oauth.scopes.len(), 2);
        assert!(oauth.use_pkce);
    }

    #[test]
    fn test_servers_file_operations() {
        let mut file = McpServersFile::default();

        // Add a server
        file.upsert(McpServerConfig::new("notion", "https://mcp.notion.com"));
        assert_eq!(file.servers.len(), 1);

        // Update the server
        let mut updated = McpServerConfig::new("notion", "https://mcp.notion.com/v2");
        updated.enabled = false;
        file.upsert(updated);
        assert_eq!(file.servers.len(), 1);
        assert!(!file.get("notion").unwrap().enabled);

        // Add another server
        file.upsert(McpServerConfig::new("github", "https://mcp.github.com"));
        assert_eq!(file.servers.len(), 2);

        // Remove a server
        assert!(file.remove("notion"));
        assert_eq!(file.servers.len(), 1);
        assert!(file.get("notion").is_none());

        // Remove non-existent server
        assert!(!file.remove("nonexistent"));
    }

    #[tokio::test]
    async fn test_load_save_config() {
        let path = temp_config_path("load-save");

        // Save a configuration
        let mut config = McpServersFile::default();
        config.upsert(
            McpServerConfig::new("notion", "https://mcp.notion.com").with_oauth(
                OAuthConfig::new("client-123")
                    .with_scopes(vec!["read".to_string(), "write".to_string()]),
            ),
        );

        save_mcp_servers_to(&config, &path).await.unwrap();

        // Load it back
        let loaded = load_mcp_servers_from(&path).await.unwrap();
        assert_eq!(loaded.servers.len(), 1);

        let server = loaded.get("notion").unwrap();
        assert_eq!(server.url, "https://mcp.notion.com");
        assert!(server.oauth.is_some());
        assert_eq!(server.oauth.as_ref().unwrap().client_id, "client-123");
    }

    #[tokio::test]
    async fn test_load_nonexistent_returns_empty() {
        let path = temp_config_path("nonexistent");

        let config = load_mcp_servers_from(&path).await.unwrap();
        assert!(config.servers.is_empty());
    }

    #[test]
    fn test_token_secret_names() {
        let config = McpServerConfig::new("notion", "https://mcp.notion.com");
        assert_eq!(config.token_secret_name(), "mcp_notion_access_token");
        assert_eq!(
            config.refresh_token_secret_name(),
            "mcp_notion_refresh_token"
        );
    }

    #[test]
    fn test_requires_auth_with_oauth() {
        let config = McpServerConfig::new("notion", "https://mcp.notion.com")
            .with_oauth(OAuthConfig::new("client-123"));
        assert!(config.requires_auth());
    }

    #[test]
    fn test_requires_auth_remote_https_without_oauth() {
        // Remote HTTPS servers need auth even without pre-configured OAuth (DCR)
        let config = McpServerConfig::new("github-copilot", "https://api.githubcopilot.com/mcp/");
        assert!(config.requires_auth());

        let config = McpServerConfig::new("notion", "https://mcp.notion.com");
        assert!(config.requires_auth());
    }

    #[test]
    fn test_requires_auth_localhost_no_auth() {
        // Localhost servers are dev servers, no auth needed
        let config = McpServerConfig::new("local", "http://localhost:8080");
        assert!(!config.requires_auth());

        let config = McpServerConfig::new("local", "http://127.0.0.1:3000/mcp");
        assert!(!config.requires_auth());

        // Even HTTPS localhost doesn't require auth
        let config = McpServerConfig::new("local", "https://localhost:8443");
        assert!(!config.requires_auth());
    }

    #[test]
    fn test_requires_auth_http_remote_no_auth() {
        // HTTP remote servers won't pass validation, but if they existed
        // they wouldn't trigger HTTPS auth detection
        let config = McpServerConfig::new("bad", "http://mcp.example.com");
        assert!(!config.requires_auth());
    }
}
