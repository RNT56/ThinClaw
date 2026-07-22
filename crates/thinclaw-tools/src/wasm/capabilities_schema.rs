//! JSON schema for WASM tool capabilities files.
//!
//! External WASM tools declare their required capabilities via a sidecar JSON file
//! (e.g., `slack.capabilities.json`). This module defines the schema for those files
//! and provides conversion to runtime [`Capabilities`].
//!
//! # Example Capabilities File
//!
//! ```json
//! {
//!   "http": {
//!     "allowlist": [
//!       { "host": "slack.com", "path_prefix": "/api/", "methods": ["GET", "POST"] }
//!     ],
//!     "credentials": {
//!       "slack_bot_token": {
//!         "secret_name": "slack_bot_token",
//!         "location": { "type": "bearer" },
//!         "host_patterns": ["slack.com"]
//!       }
//!     },
//!     "rate_limit": { "requests_per_minute": 50, "requests_per_hour": 1000 }
//!   },
//!   "secrets": {
//!     "allowed_names": ["slack_bot_token"]
//!   }
//! }
//! ```

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use thinclaw_secrets::{CredentialLocation, CredentialMapping};

use crate::wasm::{
    Capabilities, EndpointPattern, HttpCapability, RateLimitConfig, SecretsCapability,
    ToolInvokeCapability, WorkspaceCapability,
};

/// Root schema for a capabilities JSON file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilitiesFile {
    /// HTTP request capability.
    #[serde(default)]
    pub http: Option<HttpCapabilitySchema>,

    /// Secret existence checks.
    #[serde(default)]
    pub secrets: Option<SecretsCapabilitySchema>,

    /// Tool invocation via aliases.
    #[serde(default)]
    pub tool_invoke: Option<ToolInvokeCapabilitySchema>,

    /// Workspace file read access.
    #[serde(default)]
    pub workspace: Option<WorkspaceCapabilitySchema>,

    /// Authentication setup instructions.
    /// Used by `thinclaw config` to guide users through auth setup.
    #[serde(default)]
    pub auth: Option<AuthCapabilitySchema>,

    /// Legacy package layout used by early bundled tools. Parsing normalizes
    /// these fields into the direct capability fields above.
    #[serde(default)]
    pub capabilities: Option<CapabilitySetSchema>,

    /// Bounded guest configuration retained for package-level consumers.
    #[serde(default)]
    pub config: HashMap<String, serde_json::Value>,

    /// Optional hook bundle interpreted by the root hook registry.
    #[serde(default)]
    pub hooks: Option<serde_json::Value>,
}

/// Capability subset accepted inside the legacy `capabilities` wrapper.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilitySetSchema {
    #[serde(default)]
    pub http: Option<HttpCapabilitySchema>,
    #[serde(default)]
    pub secrets: Option<SecretsCapabilitySchema>,
    #[serde(default)]
    pub tool_invoke: Option<ToolInvokeCapabilitySchema>,
    #[serde(default)]
    pub workspace: Option<WorkspaceCapabilitySchema>,
    #[serde(default)]
    pub hooks: Option<serde_json::Value>,
}

impl CapabilitiesFile {
    /// Parse from JSON string.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        let parsed: Self = serde_json::from_str(json)?;
        parsed
            .normalize_and_validate()
            .map_err(<serde_json::Error as serde::de::Error>::custom)
    }

    /// Parse from JSON bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        let parsed: Self = serde_json::from_slice(bytes)?;
        parsed
            .normalize_and_validate()
            .map_err(<serde_json::Error as serde::de::Error>::custom)
    }

    fn normalize_and_validate(mut self) -> Result<Self, String> {
        if let Some(nested) = self.capabilities.take() {
            if self.http.is_some()
                || self.secrets.is_some()
                || self.tool_invoke.is_some()
                || self.workspace.is_some()
            {
                return Err(
                    "capabilities cannot mix direct fields with the legacy wrapper".to_string(),
                );
            }
            self.http = nested.http;
            self.secrets = nested.secrets;
            self.tool_invoke = nested.tool_invoke;
            self.workspace = nested.workspace;
            if self.hooks.is_some() && nested.hooks.is_some() {
                return Err("hooks cannot be declared in two locations".to_string());
            }
            self.hooks = self.hooks.or(nested.hooks);
        }
        self.validate()?;
        Ok(self)
    }

    /// Convert to runtime Capabilities.
    pub fn to_capabilities(&self) -> Capabilities {
        let mut caps = Capabilities::default();

        if let Some(http) = &self.http {
            caps.http = Some(http.to_http_capability());
        }

        if let Some(secrets) = &self.secrets {
            caps.secrets = Some(SecretsCapability {
                allowed_names: secrets.allowed_names.clone(),
            });
        }

        if let Some(tool_invoke) = &self.tool_invoke {
            caps.tool_invoke = Some(ToolInvokeCapability {
                aliases: tool_invoke.aliases.clone(),
                rate_limit: tool_invoke
                    .rate_limit
                    .as_ref()
                    .map(|r| r.to_rate_limit_config())
                    .unwrap_or_default(),
            });
        }

        if let Some(workspace) = &self.workspace {
            caps.workspace_read = Some(WorkspaceCapability {
                allowed_prefixes: workspace.allowed_prefixes.clone(),
                reader: None, // Injected at runtime
            });
        }

        caps
    }

    /// Validate all operator-controlled sizes and policy shapes before the
    /// manifest is converted into runtime capabilities.
    pub fn validate(&self) -> Result<(), String> {
        if self.config.len() > 256
            || self.config.keys().any(|key| !valid_identifier(key, 128))
            || serialized_value_exceeds(&self.config, 1024 * 1024)
            || self
                .hooks
                .as_ref()
                .is_some_and(|hooks| serialized_value_exceeds(hooks, 1024 * 1024))
        {
            return Err("tool config or hook metadata exceeds its key or byte limit".to_string());
        }

        if let Some(http) = &self.http {
            if http.allowlist.len() > 128
                || http.credentials.len() > 64
                || http
                    .rate_limit
                    .as_ref()
                    .is_some_and(|rate| !valid_rate_limit(rate))
                || http
                    .max_request_bytes
                    .is_some_and(|value| value == 0 || value > 20 * 1024 * 1024)
                || http
                    .max_response_bytes
                    .is_some_and(|value| value == 0 || value > 20 * 1024 * 1024)
                || http
                    .timeout_secs
                    .is_some_and(|value| value == 0 || value > 120)
            {
                return Err(
                    "HTTP capability exceeds a count, size, rate, or timeout limit".to_string(),
                );
            }
            for endpoint in &http.allowlist {
                if !valid_host_pattern(&endpoint.host)
                    || endpoint.path_prefix.as_deref().is_some_and(|prefix| {
                        prefix.is_empty()
                            || prefix.len() > 2048
                            || !prefix.starts_with('/')
                            || prefix.contains(['\\', '?', '#'])
                            || prefix.chars().any(char::is_control)
                            || prefix.split('/').any(|part| matches!(part, "." | ".."))
                    })
                    || endpoint.methods.len() > 16
                    || endpoint.methods.iter().any(|method| {
                        !matches!(
                            method.as_str(),
                            "GET" | "POST" | "PUT" | "DELETE" | "PATCH" | "HEAD"
                        )
                    })
                {
                    return Err("HTTP allowlist entry is malformed or oversized".to_string());
                }
            }

            let allowed_hosts = http
                .allowlist
                .iter()
                .map(|endpoint| endpoint.host.as_str())
                .collect::<std::collections::HashSet<_>>();
            let mut credential_secret_names = std::collections::HashSet::new();
            for (key, credential) in &http.credentials {
                if !valid_identifier(key, 128)
                    || !valid_identifier(&credential.secret_name, 128)
                    || !credential_secret_names.insert(credential.secret_name.as_str())
                    || credential.host_patterns.is_empty()
                    || credential.host_patterns.len() > 32
                    || credential.host_patterns.iter().any(|host| {
                        !valid_host_pattern(host) || !allowed_hosts.contains(host.as_str())
                    })
                    || !valid_credential_location(&credential.location)
                {
                    return Err(
                        "HTTP credential mapping is ambiguous, malformed, or overbroad".to_string(),
                    );
                }
            }
        }

        if let Some(secrets) = &self.secrets
            && (!valid_unique_list(&secrets.allowed_names, 64, valid_secret_pattern))
        {
            return Err("secret capability names are malformed or excessive".to_string());
        }
        if let Some(tool_invoke) = &self.tool_invoke
            && (tool_invoke.aliases.len() > 64
                || tool_invoke.aliases.iter().any(|(alias, target)| {
                    !valid_identifier(alias, 128) || !valid_identifier(target, 128)
                })
                || tool_invoke
                    .rate_limit
                    .as_ref()
                    .is_some_and(|rate| !valid_rate_limit(rate)))
        {
            return Err("tool-invoke capability is malformed or excessive".to_string());
        }
        if let Some(workspace) = &self.workspace
            && (workspace.allowed_prefixes.is_empty()
                || !valid_unique_list(&workspace.allowed_prefixes, 64, valid_workspace_prefix))
        {
            return Err(
                "workspace capability must contain bounded traversal-safe prefixes".to_string(),
            );
        }
        if let Some(auth) = &self.auth {
            validate_auth(auth)?;
        }
        Ok(())
    }
}

fn serialized_value_exceeds(value: &impl Serialize, max_bytes: usize) -> bool {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len() > max_bytes)
        .unwrap_or(true)
}

fn valid_identifier(value: &str, max_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_bytes
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn valid_env_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn valid_rate_limit(rate: &RateLimitSchema) -> bool {
    rate.requests_per_minute > 0
        && rate.requests_per_minute <= 10_000
        && rate.requests_per_hour >= rate.requests_per_minute
        && rate.requests_per_hour <= 1_000_000
}

fn valid_host_pattern(value: &str) -> bool {
    let host = value.strip_prefix("*.").unwrap_or(value);
    !host.is_empty()
        && host.len() <= 253
        && !host.starts_with('.')
        && !host.ends_with('.')
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                && label
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && label
                    .as_bytes()
                    .last()
                    .is_some_and(u8::is_ascii_alphanumeric)
        })
}

fn valid_unique_list(
    values: &[String],
    max_entries: usize,
    validator: impl Fn(&str) -> bool,
) -> bool {
    let mut unique = std::collections::HashSet::with_capacity(values.len());
    values.len() <= max_entries
        && values
            .iter()
            .all(|value| validator(value) && unique.insert(value.as_str()))
}

fn valid_secret_pattern(value: &str) -> bool {
    let base = value.strip_suffix('*').unwrap_or(value);
    valid_identifier(base, 128) && !base.contains('*')
}

fn valid_workspace_prefix(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 1024
        && !value.starts_with('/')
        && !value.contains('\\')
        && !value.chars().any(char::is_control)
        && value
            .trim_end_matches('/')
            .split('/')
            .all(|part| !part.is_empty() && !matches!(part, "." | ".."))
}

fn valid_credential_location(location: &CredentialLocationSchema) -> bool {
    match location {
        CredentialLocationSchema::Bearer => true,
        CredentialLocationSchema::Basic { username } => {
            !username.is_empty()
                && username.len() <= 256
                && !username.contains(':')
                && !username.chars().any(char::is_control)
        }
        CredentialLocationSchema::Header { name, prefix } => {
            reqwest::header::HeaderName::from_bytes(name.as_bytes()).is_ok()
                && prefix.as_deref().is_none_or(|prefix| {
                    prefix.len() <= 256
                        && !prefix.chars().any(char::is_control)
                        && reqwest::header::HeaderValue::from_str(prefix).is_ok()
                })
        }
        CredentialLocationSchema::QueryParam { name } => valid_identifier(name, 128),
        CredentialLocationSchema::UrlPath { placeholder } => placeholder
            .strip_prefix('{')
            .and_then(|value| value.strip_suffix('}'))
            .is_some_and(|name| {
                !name.is_empty()
                    && name.len() <= 128
                    && name.bytes().all(|byte| {
                        byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'
                    })
            }),
    }
}

fn valid_https_url(value: &str) -> bool {
    if value.is_empty() || value.len() > 4096 {
        return false;
    }
    url::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str().is_some_and(|host| {
                host.parse::<std::net::IpAddr>().is_ok()
                    || (!host.starts_with("*.") && valid_host_pattern(host))
            })
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
    })
}

fn valid_optional_text(value: Option<&str>, max_bytes: usize) -> bool {
    value.is_none_or(|value| {
        value.len() <= max_bytes
            && !value
                .chars()
                .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    })
}

fn validate_auth(auth: &AuthCapabilitySchema) -> Result<(), String> {
    if !valid_identifier(&auth.secret_name, 128)
        || !valid_optional_text(auth.display_name.as_deref(), 256)
        || !valid_optional_text(auth.instructions.as_deref(), 64 * 1024)
        || !valid_optional_text(auth.token_hint.as_deref(), 1024)
        || auth
            .setup_url
            .as_deref()
            .is_some_and(|url| !valid_https_url(url))
        || auth
            .env_var
            .as_deref()
            .is_some_and(|value| !valid_env_name(value))
        || auth
            .provider
            .as_deref()
            .is_some_and(|value| !valid_identifier(value, 128))
    {
        return Err("authentication capability is malformed or oversized".to_string());
    }

    if let Some(endpoint) = &auth.validation_endpoint
        && (!valid_https_url(&endpoint.url)
            || !matches!(
                endpoint.method.as_str(),
                "GET" | "POST" | "PUT" | "DELETE" | "PATCH" | "HEAD"
            )
            || !(100..=599).contains(&endpoint.success_status))
    {
        return Err("authentication validation endpoint is malformed".to_string());
    }

    if let Some(oauth) = &auth.oauth
        && (!valid_https_url(&oauth.authorization_url)
            || !valid_https_url(&oauth.token_url)
            || oauth.client_id.as_deref().is_some_and(|value| {
                value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control)
            })
            || oauth
                .client_id_env
                .as_deref()
                .is_some_and(|value| !valid_env_name(value))
            || oauth.client_secret.as_deref().is_some_and(|value| {
                value.is_empty() || value.len() > 16 * 1024 || value.chars().any(char::is_control)
            })
            || oauth
                .client_secret_env
                .as_deref()
                .is_some_and(|value| !valid_env_name(value))
            || !valid_unique_list(&oauth.scopes, 128, |scope| {
                !scope.is_empty() && scope.len() <= 1024 && !scope.chars().any(char::is_control)
            })
            || oauth.extra_params.len() > 64
            || oauth.extra_params.iter().any(|(key, value)| {
                !valid_identifier(key, 128)
                    || value.len() > 4096
                    || value.chars().any(char::is_control)
            })
            || !valid_identifier(&oauth.access_token_field, 128)
            || (!oauth.use_pkce
                && oauth.client_secret.is_none()
                && oauth.client_secret_env.is_none()))
    {
        return Err("OAuth capability is malformed, unsafe, or oversized".to_string());
    }
    Ok(())
}

/// HTTP capability schema.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpCapabilitySchema {
    /// Allowed endpoint patterns.
    #[serde(default)]
    pub allowlist: Vec<EndpointPatternSchema>,

    /// Credential mappings (key is an identifier, not the secret name).
    #[serde(default)]
    pub credentials: HashMap<String, CredentialMappingSchema>,

    /// Rate limiting configuration.
    #[serde(default)]
    pub rate_limit: Option<RateLimitSchema>,

    /// Maximum request body size in bytes.
    #[serde(default)]
    pub max_request_bytes: Option<usize>,

    /// Maximum response body size in bytes.
    #[serde(default)]
    pub max_response_bytes: Option<usize>,

    /// Request timeout in seconds.
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

impl HttpCapabilitySchema {
    fn to_http_capability(&self) -> HttpCapability {
        let mut cap = HttpCapability {
            allowlist: self
                .allowlist
                .iter()
                .map(|p| p.to_endpoint_pattern())
                .collect(),
            credentials: self
                .credentials
                .values()
                .map(|m| (m.secret_name.clone(), m.to_credential_mapping()))
                .collect(),
            rate_limit: self
                .rate_limit
                .as_ref()
                .map(|r| r.to_rate_limit_config())
                .unwrap_or_default(),
            ..Default::default()
        };

        if let Some(max) = self.max_request_bytes {
            cap.max_request_bytes = max;
        }
        if let Some(max) = self.max_response_bytes {
            cap.max_response_bytes = max;
        }
        if let Some(secs) = self.timeout_secs {
            cap.timeout = Duration::from_secs(secs);
        }

        cap
    }
}

/// Endpoint pattern schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointPatternSchema {
    /// Hostname (e.g., "api.slack.com" or "*.slack.com").
    pub host: String,

    /// Optional path prefix (e.g., "/api/").
    #[serde(default)]
    pub path_prefix: Option<String>,

    /// Allowed HTTP methods (empty = all).
    #[serde(default)]
    pub methods: Vec<String>,
}

impl EndpointPatternSchema {
    fn to_endpoint_pattern(&self) -> EndpointPattern {
        EndpointPattern {
            host: self.host.clone(),
            path_prefix: self.path_prefix.clone(),
            methods: self.methods.clone(),
        }
    }
}

/// Credential mapping schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialMappingSchema {
    /// Name of the secret to inject.
    pub secret_name: String,

    /// Where to inject the credential.
    pub location: CredentialLocationSchema,

    /// Host patterns this credential applies to.
    #[serde(default)]
    pub host_patterns: Vec<String>,
}

impl CredentialMappingSchema {
    fn to_credential_mapping(&self) -> CredentialMapping {
        CredentialMapping {
            secret_name: self.secret_name.clone(),
            location: self.location.to_credential_location(),
            host_patterns: self.host_patterns.clone(),
        }
    }
}

/// Credential injection location schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum CredentialLocationSchema {
    /// Bearer token in Authorization header.
    Bearer,

    /// Basic auth (password from secret, username in config).
    Basic { username: String },

    /// Custom header.
    Header {
        name: String,
        #[serde(default)]
        prefix: Option<String>,
    },

    /// Query parameter.
    QueryParam { name: String },

    /// URL/path placeholder replacement.
    UrlPath { placeholder: String },
}

impl CredentialLocationSchema {
    fn to_credential_location(&self) -> CredentialLocation {
        match self {
            CredentialLocationSchema::Bearer => CredentialLocation::AuthorizationBearer,
            CredentialLocationSchema::Basic { username } => {
                CredentialLocation::AuthorizationBasic {
                    username: username.clone(),
                }
            }
            CredentialLocationSchema::Header { name, prefix } => CredentialLocation::Header {
                name: name.clone(),
                prefix: prefix.clone(),
            },
            CredentialLocationSchema::QueryParam { name } => {
                CredentialLocation::QueryParam { name: name.clone() }
            }
            CredentialLocationSchema::UrlPath { placeholder } => CredentialLocation::UrlPath {
                placeholder: placeholder.clone(),
            },
        }
    }
}

/// Rate limit schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RateLimitSchema {
    /// Maximum requests per minute.
    #[serde(default = "default_requests_per_minute")]
    pub requests_per_minute: u32,

    /// Maximum requests per hour.
    #[serde(default = "default_requests_per_hour")]
    pub requests_per_hour: u32,
}

fn default_requests_per_minute() -> u32 {
    60
}

fn default_requests_per_hour() -> u32 {
    1000
}

impl RateLimitSchema {
    fn to_rate_limit_config(&self) -> RateLimitConfig {
        RateLimitConfig {
            requests_per_minute: self.requests_per_minute,
            requests_per_hour: self.requests_per_hour,
        }
    }
}

/// Secrets capability schema.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretsCapabilitySchema {
    /// Secret names the tool can check existence of (supports glob).
    #[serde(default)]
    pub allowed_names: Vec<String>,
}

/// Tool invocation capability schema.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolInvokeCapabilitySchema {
    /// Mapping from alias to real tool name.
    #[serde(default)]
    pub aliases: HashMap<String, String>,

    /// Rate limiting for tool calls.
    #[serde(default)]
    pub rate_limit: Option<RateLimitSchema>,
}

/// Workspace read capability schema.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceCapabilitySchema {
    /// Allowed path prefixes (e.g., ["context/", "daily/"]).
    #[serde(default)]
    pub allowed_prefixes: Vec<String>,
}

/// Authentication setup schema.
///
/// Tools declare their auth requirements here. The agent uses this to provide
/// generic auth flows without needing service-specific code in the main codebase.
///
/// Supports two auth methods:
/// 1. **OAuth** - Browser-based login (preferred for user-facing services)
/// 2. **Manual** - Copy/paste token from provider's dashboard
///
/// # Example (OAuth)
///
/// ```json
/// {
///   "auth": {
///     "secret_name": "notion_api_token",
///     "display_name": "Notion",
///     "oauth": {
///       "authorization_url": "https://api.notion.com/v1/oauth/authorize",
///       "token_url": "https://api.notion.com/v1/oauth/token",
///       "client_id": "your-client-id",
///       "scopes": []
///     },
///     "env_var": "NOTION_TOKEN"
///   }
/// }
/// ```
///
/// # Example (Manual)
///
/// ```json
/// {
///   "auth": {
///     "secret_name": "openai_api_key",
///     "display_name": "OpenAI",
///     "instructions": "Get your API key from platform.openai.com/api-keys",
///     "setup_url": "https://platform.openai.com/api-keys",
///     "token_hint": "Starts with 'sk-'",
///     "env_var": "OPENAI_API_KEY"
///   }
/// }
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthCapabilitySchema {
    /// Name of the secret to store (e.g., "notion_api_token").
    /// Must match the secret_name in credentials if HTTP capability is used.
    pub secret_name: String,

    /// Human-readable name for the service (e.g., "Notion", "Slack").
    #[serde(default)]
    pub display_name: Option<String>,

    /// OAuth configuration for browser-based login.
    /// If present, OAuth flow is used instead of manual token entry.
    #[serde(default)]
    pub oauth: Option<OAuthConfigSchema>,

    /// Instructions shown to the user for obtaining credentials (manual flow).
    /// Can include markdown formatting.
    #[serde(default)]
    pub instructions: Option<String>,

    /// URL to open for setting up credentials (manual flow).
    #[serde(default)]
    pub setup_url: Option<String>,

    /// Hint about expected token format (e.g., "Starts with 'sk-'").
    /// Used for validation feedback.
    #[serde(default)]
    pub token_hint: Option<String>,

    /// Environment variable to check before prompting.
    /// If this env var is set, its value is used automatically.
    #[serde(default)]
    pub env_var: Option<String>,

    /// Provider hint for organizing secrets (e.g., "notion", "openai").
    #[serde(default)]
    pub provider: Option<String>,

    /// Validation endpoint to check if the token works.
    /// Tool can specify an endpoint to call for validation.
    #[serde(default)]
    pub validation_endpoint: Option<ValidationEndpointSchema>,
}

/// OAuth 2.0 configuration for browser-based login.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OAuthConfigSchema {
    /// OAuth authorization URL (e.g., "https://api.notion.com/v1/oauth/authorize").
    pub authorization_url: String,

    /// OAuth token exchange URL (e.g., "https://api.notion.com/v1/oauth/token").
    pub token_url: String,

    /// OAuth client ID.
    /// Can be set here or via environment variable (see client_id_env).
    #[serde(default)]
    pub client_id: Option<String>,

    /// Environment variable containing the client ID.
    /// Checked if client_id is not set directly.
    #[serde(default)]
    pub client_id_env: Option<String>,

    /// OAuth client secret (optional, some providers don't require it with PKCE).
    /// Can be set here or via environment variable (see client_secret_env).
    #[serde(default)]
    pub client_secret: Option<String>,

    /// Environment variable containing the client secret.
    /// Checked if client_secret is not set directly.
    #[serde(default)]
    pub client_secret_env: Option<String>,

    /// OAuth scopes to request.
    #[serde(default)]
    pub scopes: Vec<String>,

    /// Use PKCE (Proof Key for Code Exchange). Defaults to true and is required
    /// for public clients. It may be disabled only for a confidential client
    /// that supplies a client secret and whose provider has not enabled PKCE.
    #[serde(default = "default_true")]
    pub use_pkce: bool,

    /// Additional parameters to include in the authorization URL.
    #[serde(default)]
    pub extra_params: std::collections::HashMap<String, String>,

    /// Field name in token response containing the access token.
    /// Defaults to "access_token".
    #[serde(default = "default_access_token_field")]
    pub access_token_field: String,
}

impl std::fmt::Debug for OAuthConfigSchema {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OAuthConfigSchema")
            .field("authorization_url", &"[REDACTED URL]")
            .field("token_url", &"[REDACTED URL]")
            .field("client_id", &self.client_id)
            .field("client_id_env", &self.client_id_env)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("client_secret_env", &self.client_secret_env)
            .field("scopes", &self.scopes)
            .field("use_pkce", &self.use_pkce)
            .field(
                "extra_param_names",
                &self.extra_params.keys().collect::<Vec<_>>(),
            )
            .field("access_token_field", &self.access_token_field)
            .finish()
    }
}

fn default_true() -> bool {
    true
}

fn default_access_token_field() -> String {
    "access_token".to_string()
}

/// Schema for token validation endpoint.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationEndpointSchema {
    /// URL to call for validation (e.g., "https://api.notion.com/v1/users/me").
    pub url: String,

    /// HTTP method (defaults to GET).
    #[serde(default = "default_method")]
    pub method: String,

    /// Expected HTTP status code for success (defaults to 200).
    #[serde(default = "default_success_status")]
    pub success_status: u16,
}

fn default_method() -> String {
    "GET".to_string()
}

fn default_success_status() -> u16 {
    200
}

#[cfg(test)]
mod tests {
    use crate::wasm::capabilities_schema::{CapabilitiesFile, CredentialLocationSchema};

    #[test]
    fn test_parse_minimal() {
        let json = "{}";
        let caps = CapabilitiesFile::from_json(json).unwrap();
        assert!(caps.http.is_none());
        assert!(caps.secrets.is_none());
    }

    #[test]
    fn test_legacy_nested_capabilities_are_normalized() {
        let caps = CapabilitiesFile::from_json(
            r#"{
                "capabilities": {
                    "http": {"allowlist": [{"host": "api.example.com"}]},
                    "secrets": {"allowed_names": ["example_token"]}
                },
                "config": {"page_size": 25}
            }"#,
        )
        .unwrap();

        assert!(caps.capabilities.is_none());
        assert_eq!(caps.http.unwrap().allowlist[0].host, "api.example.com");
        assert_eq!(caps.config["page_size"], 25);
    }

    #[test]
    fn test_rejects_unbounded_workspace_and_overbroad_credentials() {
        assert!(CapabilitiesFile::from_json(r#"{"workspace":{"allowed_prefixes":[]}}"#).is_err());
        assert!(
            CapabilitiesFile::from_json(
                r#"{
                    "http": {
                        "allowlist": [{"host": "api.example.com"}],
                        "credentials": {
                            "token": {
                                "secret_name": "example_token",
                                "location": {"type": "bearer"},
                                "host_patterns": ["other.example.com"]
                            }
                        }
                    }
                }"#,
            )
            .is_err()
        );
    }

    #[test]
    fn bundled_tool_capabilities_all_validate() {
        let source_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools-src");
        for entry in std::fs::read_dir(source_root).unwrap() {
            let entry = entry.unwrap();
            if !entry.file_type().unwrap().is_dir() {
                continue;
            }
            for file in std::fs::read_dir(entry.path()).unwrap() {
                let file = file.unwrap();
                let path = file.path();
                if path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_none_or(|name| !name.ends_with("-tool.capabilities.json"))
                {
                    continue;
                }
                let bytes = std::fs::read(&path).unwrap();
                CapabilitiesFile::from_bytes(&bytes)
                    .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            }
        }
    }

    #[test]
    fn test_parse_http_allowlist() {
        let json = r#"{
            "http": {
                "allowlist": [
                    { "host": "api.slack.com", "path_prefix": "/api/", "methods": ["GET", "POST"] }
                ]
            }
        }"#;

        let caps = CapabilitiesFile::from_json(json).unwrap();
        let http = caps.http.unwrap();
        assert_eq!(http.allowlist.len(), 1);
        assert_eq!(http.allowlist[0].host, "api.slack.com");
        assert_eq!(http.allowlist[0].path_prefix, Some("/api/".to_string()));
        assert_eq!(http.allowlist[0].methods, vec!["GET", "POST"]);
    }

    #[test]
    fn test_parse_credentials() {
        let json = r#"{
            "http": {
                "allowlist": [{ "host": "slack.com" }, { "host": "*.slack.com" }],
                "credentials": {
                    "slack": {
                        "secret_name": "slack_bot_token",
                        "location": { "type": "bearer" },
                        "host_patterns": ["slack.com", "*.slack.com"]
                    }
                }
            }
        }"#;

        let caps = CapabilitiesFile::from_json(json).unwrap();
        let http = caps.http.unwrap();
        assert_eq!(http.credentials.len(), 1);
        let cred = http.credentials.get("slack").unwrap();
        assert_eq!(cred.secret_name, "slack_bot_token");
        assert!(matches!(cred.location, CredentialLocationSchema::Bearer));
        assert_eq!(cred.host_patterns, vec!["slack.com", "*.slack.com"]);
    }

    #[test]
    fn test_parse_custom_header_credential() {
        let json = r#"{
            "http": {
                "allowlist": [{ "host": "api.example.com" }],
                "credentials": {
                    "api_key": {
                        "secret_name": "my_api_key",
                        "location": { "type": "header", "name": "X-API-Key", "prefix": "Key " },
                        "host_patterns": ["api.example.com"]
                    }
                }
            }
        }"#;

        let caps = CapabilitiesFile::from_json(json).unwrap();
        let http = caps.http.unwrap();
        let cred = http.credentials.get("api_key").unwrap();
        match &cred.location {
            CredentialLocationSchema::Header { name, prefix } => {
                assert_eq!(name, "X-API-Key");
                assert_eq!(prefix, &Some("Key ".to_string()));
            }
            _ => panic!("Expected Header location"),
        }
    }

    #[test]
    fn test_parse_url_path_credential() {
        let json = r#"{
            "http": {
                "allowlist": [{ "host": "api.telegram.org" }],
                "credentials": {
                    "telegram_bot": {
                        "secret_name": "telegram_bot_token",
                        "location": {
                            "type": "url_path",
                            "placeholder": "{TELEGRAM_BOT_TOKEN}"
                        },
                        "host_patterns": ["api.telegram.org"]
                    }
                }
            }
        }"#;

        let caps = CapabilitiesFile::from_json(json).unwrap();
        let http = caps.http.unwrap();
        let cred = http.credentials.get("telegram_bot").unwrap();
        match &cred.location {
            CredentialLocationSchema::UrlPath { placeholder } => {
                assert_eq!(placeholder, "{TELEGRAM_BOT_TOKEN}");
            }
            _ => panic!("Expected UrlPath location"),
        }
    }

    #[test]
    fn test_parse_secrets_capability() {
        let json = r#"{
            "secrets": {
                "allowed_names": ["slack_*", "openai_key"]
            }
        }"#;

        let caps = CapabilitiesFile::from_json(json).unwrap();
        let secrets = caps.secrets.unwrap();
        assert_eq!(secrets.allowed_names, vec!["slack_*", "openai_key"]);
    }

    #[test]
    fn test_parse_tool_invoke() {
        let json = r#"{
            "tool_invoke": {
                "aliases": {
                    "search": "brave_search",
                    "calc": "calculator"
                },
                "rate_limit": {
                    "requests_per_minute": 10,
                    "requests_per_hour": 100
                }
            }
        }"#;

        let caps = CapabilitiesFile::from_json(json).unwrap();
        let tool_invoke = caps.tool_invoke.unwrap();
        assert_eq!(
            tool_invoke.aliases.get("search"),
            Some(&"brave_search".to_string())
        );
        let rate = tool_invoke.rate_limit.unwrap();
        assert_eq!(rate.requests_per_minute, 10);
    }

    #[test]
    fn test_parse_workspace() {
        let json = r#"{
            "workspace": {
                "allowed_prefixes": ["context/", "daily/"]
            }
        }"#;

        let caps = CapabilitiesFile::from_json(json).unwrap();
        let workspace = caps.workspace.unwrap();
        assert_eq!(workspace.allowed_prefixes, vec!["context/", "daily/"]);
    }

    #[test]
    fn test_to_capabilities() {
        let json = r#"{
            "http": {
                "allowlist": [{ "host": "api.slack.com", "path_prefix": "/api/" }],
                "rate_limit": { "requests_per_minute": 50, "requests_per_hour": 500 }
            },
            "secrets": {
                "allowed_names": ["slack_token"]
            }
        }"#;

        let file = CapabilitiesFile::from_json(json).unwrap();
        let caps = file.to_capabilities();

        assert!(caps.http.is_some());
        let http = caps.http.unwrap();
        assert_eq!(http.allowlist.len(), 1);
        assert_eq!(http.rate_limit.requests_per_minute, 50);

        assert!(caps.secrets.is_some());
        let secrets = caps.secrets.unwrap();
        assert!(secrets.is_allowed("slack_token"));
    }

    #[test]
    fn test_full_slack_example() {
        let json = r#"{
            "http": {
                "allowlist": [
                    { "host": "slack.com", "path_prefix": "/api/", "methods": ["GET", "POST"] }
                ],
                "credentials": {
                    "slack_bot_token": {
                        "secret_name": "slack_bot_token",
                        "location": { "type": "bearer" },
                        "host_patterns": ["slack.com"]
                    }
                },
                "rate_limit": { "requests_per_minute": 50, "requests_per_hour": 1000 }
            },
            "secrets": {
                "allowed_names": ["slack_bot_token"]
            }
        }"#;

        let file = CapabilitiesFile::from_json(json).unwrap();
        let caps = file.to_capabilities();

        let http = caps.http.unwrap();
        assert_eq!(http.allowlist[0].host, "slack.com");
        assert!(http.credentials.contains_key("slack_bot_token"));

        let secrets = caps.secrets.unwrap();
        assert!(secrets.is_allowed("slack_bot_token"));
    }

    #[test]
    fn test_parse_auth_capability() {
        let json = r#"{
            "auth": {
                "secret_name": "notion_api_token",
                "display_name": "Notion",
                "instructions": "Create an integration at notion.so/my-integrations",
                "setup_url": "https://www.notion.so/my-integrations",
                "token_hint": "Starts with 'secret_' or 'ntn_'",
                "env_var": "NOTION_TOKEN",
                "provider": "notion",
                "validation_endpoint": {
                    "url": "https://api.notion.com/v1/users/me",
                    "method": "GET",
                    "success_status": 200
                }
            }
        }"#;

        let caps = CapabilitiesFile::from_json(json).unwrap();
        let auth = caps.auth.unwrap();
        assert_eq!(auth.secret_name, "notion_api_token");
        assert_eq!(auth.display_name, Some("Notion".to_string()));
        assert_eq!(auth.env_var, Some("NOTION_TOKEN".to_string()));
        assert_eq!(auth.provider, Some("notion".to_string()));

        let validation = auth.validation_endpoint.unwrap();
        assert_eq!(validation.url, "https://api.notion.com/v1/users/me");
        assert_eq!(validation.method, "GET");
        assert_eq!(validation.success_status, 200);
    }

    #[test]
    fn test_parse_auth_minimal() {
        let json = r#"{
            "auth": {
                "secret_name": "my_api_key"
            }
        }"#;

        let caps = CapabilitiesFile::from_json(json).unwrap();
        let auth = caps.auth.unwrap();
        assert_eq!(auth.secret_name, "my_api_key");
        assert!(auth.display_name.is_none());
        assert!(auth.setup_url.is_none());
    }
}
