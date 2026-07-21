//! JSON schema for WASM channel capabilities files.
//!
//! External WASM channels declare their required capabilities via a sidecar JSON file
//! (e.g., `slack.capabilities.json`). This module defines the schema for those files
//! and provides conversion to runtime [`ChannelCapabilities`].
//!
//! # Example Capabilities File
//!
//! ```json
//! {
//!   "type": "channel",
//!   "name": "slack",
//!   "description": "Slack Events API channel",
//!   "capabilities": {
//!     "http": {
//!       "allowlist": [
//!         { "host": "slack.com", "path_prefix": "/api/" }
//!       ],
//!       "credentials": {
//!         "slack_bot": {
//!           "secret_name": "slack_bot_token",
//!           "location": { "type": "bearer" },
//!           "host_patterns": ["slack.com"]
//!         }
//!       }
//!     },
//!     "secrets": { "allowed_names": ["slack_*"] },
//!     "channel": {
//!       "allowed_paths": ["/webhook/slack"],
//!       "allow_polling": false,
//!       "workspace_prefix": "channels/slack/",
//!       "emit_rate_limit": { "messages_per_minute": 100 }
//!     }
//!   },
//!   "config": {
//!     "signing_secret_name": "slack_signing_secret"
//!   }
//! }
//! ```

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::wasm::capabilities::{
    ChannelCapabilities, CredentialLocation, CredentialMapping, EmitRateLimitConfig,
    EndpointPattern, HttpCapability, MIN_POLL_INTERVAL_MS, RateLimitConfig, SecretsCapability,
    ToolCapabilities, ToolInvokeCapability, WorkspaceCapability,
};

/// Operator-facing maturity / auth-correctness disposition for a channel (F-10).
///
/// Defaults to `Experimental` so an unmarked channel is never silently treated as
/// production-grade. `Beta` channels typically have weaker-than-native inbound
/// auth (e.g. a shared-secret `equals` compare rather than the platform's native
/// HMAC / Ed25519 / signed-JWT verification) and must say so in their README.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProductionStatus {
    /// Native-grade auth and a faithful integration; safe to rely on.
    Production,
    /// Functional, but with a documented auth-correctness caveat.
    Beta,
    /// Unproven / unmarked.
    #[default]
    Experimental,
}

/// Root schema for a channel capabilities JSON file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelCapabilitiesFile {
    /// File type, must be "channel".
    #[serde(default = "default_type")]
    pub r#type: String,

    /// Channel name.
    pub name: String,

    /// Operator-facing maturity disposition (F-10). Defaults to `experimental`.
    #[serde(default)]
    pub production_status: ProductionStatus,

    /// Channel description.
    #[serde(default)]
    pub description: Option<String>,

    /// Optional platform formatting guidance for prompt assembly.
    ///
    /// This lets WASM-backed channels declare their own formatting/rendering
    /// expectations so prompt assembly does not need channel-name switches.
    #[serde(default)]
    pub formatting_hints: Option<String>,

    /// Setup configuration for the wizard.
    #[serde(default)]
    pub setup: SetupSchema,

    /// Capabilities (tool + channel specific).
    #[serde(default)]
    pub capabilities: ChannelCapabilitiesSchema,

    /// Channel-specific configuration passed to on_start.
    #[serde(default)]
    pub config: HashMap<String, serde_json::Value>,
}

fn default_type() -> String {
    "channel".to_string()
}

impl ChannelCapabilitiesFile {
    /// Parse from JSON string.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        let parsed: Self = serde_json::from_str(json)?;
        parsed
            .validate()
            .map_err(<serde_json::Error as serde::de::Error>::custom)?;
        Ok(parsed)
    }

    /// Parse from JSON bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        let parsed: Self = serde_json::from_slice(bytes)?;
        parsed
            .validate()
            .map_err(<serde_json::Error as serde::de::Error>::custom)?;
        Ok(parsed)
    }

    /// Validate all operator-controlled bounds before any part of a channel
    /// manifest is converted into runtime policy or setup behavior.
    pub fn validate(&self) -> Result<(), String> {
        fn valid_token(value: &str, max: usize) -> bool {
            !value.is_empty()
                && value.len() <= max
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        }

        fn valid_rate_limit(rate: &RateLimitSchema) -> bool {
            rate.requests_per_minute > 0
                && rate.requests_per_minute <= 10_000
                && rate.requests_per_hour >= rate.requests_per_minute
                && rate.requests_per_hour <= 1_000_000
        }

        fn valid_host_pattern(value: &str) -> bool {
            if value == "*" {
                return true;
            }
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

        fn valid_http_path_prefix(value: &str) -> bool {
            !value.is_empty()
                && value.len() <= 2048
                && value.starts_with('/')
                && !value.contains(['\\', '?', '#'])
                && !value.chars().any(char::is_control)
                && !value.split('/').any(|part| matches!(part, "." | ".."))
        }

        if self.r#type != "channel" {
            return Err("capabilities file type must be 'channel'".to_string());
        }
        if !crate::wasm::capabilities::is_valid_channel_name(&self.name) {
            return Err("channel name must be a safe lowercase namespace key".to_string());
        }
        if self
            .description
            .as_deref()
            .is_some_and(|value| value.len() > 4096 || value.chars().any(char::is_control))
            || self
                .formatting_hints
                .as_deref()
                .is_some_and(|value| value.len() > 64 * 1024)
        {
            return Err("channel description or formatting hints are oversized".to_string());
        }
        if self.config.len() > 256
            || self.config.keys().any(|key| !valid_token(key, 128))
            || serde_json::to_vec(&self.config)
                .map(|bytes| bytes.len() > 1024 * 1024)
                .unwrap_or(true)
        {
            return Err("channel config exceeds its key or byte limit".to_string());
        }

        if self.setup.required_secrets.len() > 64 {
            return Err("setup secret list exceeds the entry limit".to_string());
        }
        let mut declared_setup_secrets = std::collections::HashSet::new();
        for secret in &self.setup.required_secrets {
            if !valid_token(&secret.name, 128)
                || !declared_setup_secrets.insert(secret.name.as_str())
                || secret.prompt.is_empty()
                || secret.prompt.len() > 4096
                || secret.prompt.chars().any(char::is_control)
                || secret.validation.as_deref().is_some_and(|value| {
                    value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control)
                })
                || secret
                    .auto_generate
                    .as_ref()
                    .is_some_and(|value| value.length == 0 || value.length > 1024)
            {
                return Err(
                    "setup secret declarations must be unique, bounded, and well formed"
                        .to_string(),
                );
            }
        }

        if let Some(http) = &self.capabilities.http {
            if http.allowlist.len() > 128
                || http.credentials.len() > 64
                || http
                    .max_request_bytes
                    .is_some_and(|value| value == 0 || value > 20 * 1024 * 1024)
                || http
                    .max_response_bytes
                    .is_some_and(|value| value == 0 || value > 20 * 1024 * 1024)
                || http
                    .timeout_secs
                    .is_some_and(|value| value == 0 || value > 120)
                || http
                    .rate_limit
                    .as_ref()
                    .is_some_and(|rate| !valid_rate_limit(rate))
            {
                return Err(
                    "HTTP capability exceeds a count, size, rate, or timeout limit".to_string(),
                );
            }
            for endpoint in &http.allowlist {
                if !valid_host_pattern(&endpoint.host)
                    || endpoint
                        .path_prefix
                        .as_deref()
                        .is_some_and(|path| !valid_http_path_prefix(path))
                    || endpoint.methods.len() > 8
                    || endpoint.methods.iter().any(|method| {
                        !matches!(
                            method.to_ascii_uppercase().as_str(),
                            "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD"
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
            let has_wildcard_base = http.credentials.values().any(|mapping| {
                matches!(mapping.location, CredentialLocationSchema::UrlBase { .. })
                    && mapping.host_patterns.iter().any(|host| host == "*")
            });
            let mut mapped_secret_names = std::collections::HashSet::new();
            for (alias, mapping) in &http.credentials {
                let expected_placeholder =
                    crate::wasm::capabilities::credential_placeholder_name(&mapping.secret_name)
                        .map(|name| format!("{{{name}}}"));
                if !valid_token(alias, 128)
                    || expected_placeholder.is_none()
                    || !mapped_secret_names.insert(mapping.secret_name.as_str())
                    || !declared_setup_secrets.contains(mapping.secret_name.as_str())
                    || mapping.host_patterns.is_empty()
                    || mapping.host_patterns.len() > 32
                    || mapping.host_patterns.iter().any(|host| {
                        !valid_host_pattern(host)
                            || !allowed_hosts.contains(host.as_str())
                            || (host == "*" && !has_wildcard_base)
                    })
                {
                    return Err(
                        "HTTP credential mapping is malformed, undeclared, or overbroad"
                            .to_string(),
                    );
                }
                match &mapping.location {
                    CredentialLocationSchema::Bearer => {}
                    CredentialLocationSchema::Basic { username } => {
                        if username.is_empty()
                            || username.len() > 1024
                            || username.contains(['\r', '\n'])
                        {
                            return Err("HTTP basic-auth username is invalid".to_string());
                        }
                    }
                    CredentialLocationSchema::Header { name, prefix } => {
                        if axum::http::HeaderName::from_bytes(name.as_bytes()).is_err()
                            || prefix.as_deref().is_some_and(|value| {
                                value.len() > 1024 || value.contains(['\r', '\n'])
                            })
                        {
                            return Err("HTTP credential header mapping is invalid".to_string());
                        }
                    }
                    CredentialLocationSchema::QueryParam { name } => {
                        if !valid_token(name, 128) {
                            return Err("HTTP credential query parameter is invalid".to_string());
                        }
                    }
                    CredentialLocationSchema::UrlPath { placeholder }
                    | CredentialLocationSchema::UrlBase { placeholder }
                    | CredentialLocationSchema::Body { placeholder } => {
                        let inner = placeholder
                            .strip_prefix('{')
                            .and_then(|value| value.strip_suffix('}'));
                        if inner.is_none_or(|value| {
                            value.is_empty()
                                || value.len() > 128
                                || !value.bytes().all(|byte| {
                                    byte.is_ascii_uppercase()
                                        || byte.is_ascii_digit()
                                        || byte == b'_'
                                })
                        }) || Some(placeholder) != expected_placeholder.as_ref()
                        {
                            return Err("HTTP credential placeholder is invalid".to_string());
                        }
                    }
                }
            }
        }

        if let Some(secrets) = &self.capabilities.secrets
            && (secrets.allowed_names.len() > 128
                || secrets.allowed_names.iter().any(|name| {
                    let base = name.strip_suffix('*').unwrap_or(name);
                    base.is_empty() || !valid_token(base, 128)
                }))
        {
            return Err("secret capability is malformed or oversized".to_string());
        }
        if let Some(tool_invoke) = &self.capabilities.tool_invoke
            && (tool_invoke.aliases.len() > 128
                || tool_invoke
                    .aliases
                    .iter()
                    .any(|(alias, target)| !valid_token(alias, 128) || !valid_token(target, 128))
                || tool_invoke
                    .rate_limit
                    .as_ref()
                    .is_some_and(|rate| !valid_rate_limit(rate)))
        {
            return Err("tool-invoke capability is malformed or oversized".to_string());
        }
        if let Some(workspace) = &self.capabilities.workspace
            && (workspace.allowed_prefixes.len() > 64
                || workspace.allowed_prefixes.iter().any(|prefix| {
                    prefix.is_empty()
                        || prefix.len() > 1024
                        || prefix.starts_with('/')
                        || prefix.contains('\\')
                        || prefix.chars().any(char::is_control)
                        || prefix
                            .trim_end_matches('/')
                            .split('/')
                            .any(|part| part.is_empty() || matches!(part, "." | ".."))
                }))
        {
            return Err("workspace capability is malformed or oversized".to_string());
        }

        if let Some(channel) = &self.capabilities.channel {
            let expected_path = format!("/webhook/{}", self.name);
            if channel.allowed_paths.len() > 32
                || channel.allowed_paths.iter().any(|path| {
                    path != &expected_path
                        && !path
                            .strip_prefix(&expected_path)
                            .is_some_and(|suffix| suffix.starts_with('/'))
                })
                || channel
                    .min_poll_interval_ms
                    .is_some_and(|value| value > 86_400_000)
                || channel
                    .max_message_size
                    .is_some_and(|value| value == 0 || value > 64 * 1024)
                || channel
                    .callback_timeout_secs
                    .is_some_and(|value| value == 0 || value > 120)
                || channel.emit_rate_limit.as_ref().is_some_and(|rate| {
                    rate.messages_per_minute == 0
                        || rate.messages_per_minute > 10_000
                        || rate.messages_per_hour < rate.messages_per_minute
                        || rate.messages_per_hour > 1_000_000
                })
            {
                return Err(
                    "channel capability exceeds its namespace or runtime limits".to_string()
                );
            }
            if let Some(prefix) = &channel.workspace_prefix {
                let probe = ChannelCapabilities {
                    workspace_prefix: prefix.clone(),
                    ..ChannelCapabilities::default()
                };
                probe.validate_workspace_path("validation-probe")?;
            }
            if let Some(webhook) = &channel.webhook {
                let signature_secret_name = webhook
                    .secret_name
                    .clone()
                    .unwrap_or_else(|| format!("{}_webhook_secret", self.name));
                let verify_secret_name = webhook
                    .verify_token_secret_name
                    .as_deref()
                    .unwrap_or(&signature_secret_name);
                if webhook
                    .secret_header
                    .as_deref()
                    .is_none_or(|name| axum::http::HeaderName::from_bytes(name.as_bytes()).is_err())
                    || !valid_token(&signature_secret_name, 128)
                    || !declared_setup_secrets.contains(signature_secret_name.as_str())
                    || webhook
                        .verify_token_param
                        .as_deref()
                        .is_some_and(|name| !valid_token(name, 128))
                    || webhook.verify_token_secret_name.is_some()
                        && webhook.verify_token_param.is_none()
                    || webhook.verify_token_param.is_some()
                        && (!valid_token(verify_secret_name, 128)
                            || !declared_setup_secrets.contains(verify_secret_name))
                {
                    return Err(
                        "webhook authentication must use declared setup secrets and valid header/query names"
                            .to_string(),
                    );
                }
            }
        }

        if let Some(endpoint) = &self.setup.validation_endpoint {
            let parsed_url = url::Url::parse(endpoint.url()).ok();
            if endpoint.url().is_empty()
                || endpoint.url().len() > 16 * 1024
                || parsed_url.as_ref().is_none_or(|url| {
                    url.scheme() != "https"
                        || url.host_str().is_none()
                        || !url.username().is_empty()
                        || url.password().is_some()
                        || url.fragment().is_some()
                })
            {
                return Err("setup validation URL is invalid or oversized".to_string());
            }
            let Some(parsed_url) = parsed_url else {
                return Err("setup validation URL is invalid or oversized".to_string());
            };
            let method = endpoint
                .request()
                .map_or("GET", |request| request.method.as_str());
            if let Some(request) = endpoint.request()
                && (!matches!(request.method.to_ascii_uppercase().as_str(), "GET" | "POST")
                    || !(200..400).contains(&request.success_status)
                    || request.secret_name.as_deref().is_some_and(|name| {
                        !valid_token(name, 128) || !declared_setup_secrets.contains(name)
                    })
                    || request.secret_name.is_some() != request.credential.is_some()
                    || request.credential.as_ref().is_some_and(|credential| {
                        !matches!(
                            credential,
                            CredentialLocationSchema::Bearer
                                | CredentialLocationSchema::Basic { .. }
                                | CredentialLocationSchema::Header { .. }
                        )
                    }))
            {
                return Err("setup validation request is malformed".to_string());
            }
            let validation_granted = self.capabilities.http.as_ref().is_some_and(|http| {
                http.allowlist.iter().any(|grant| {
                    grant.to_endpoint_pattern().matches(
                        parsed_url.host_str().unwrap_or_default(),
                        parsed_url.path(),
                        method,
                    )
                })
            });
            if !validation_granted {
                return Err(
                    "setup validation URL is not granted by the HTTP capability".to_string()
                );
            }
        }
        Ok(())
    }

    /// Convert to runtime ChannelCapabilities.
    pub fn to_capabilities(&self) -> ChannelCapabilities {
        self.capabilities.to_channel_capabilities(&self.name)
    }

    /// Get the channel config as JSON string.
    pub fn config_json(&self) -> String {
        serde_json::to_string(&self.config).unwrap_or_else(|_| "{}".to_string())
    }

    /// Get the webhook secret header name for this channel.
    ///
    /// Returns the configured header name from capabilities, or a sensible default.
    pub fn webhook_secret_header(&self) -> Option<&str> {
        self.capabilities
            .channel
            .as_ref()
            .and_then(|c| c.webhook.as_ref())
            .and_then(|w| w.secret_header.as_deref())
    }

    /// Get the webhook secret name for this channel.
    ///
    /// Returns the configured secret name or defaults to "{channel_name}_webhook_secret".
    pub fn webhook_secret_name(&self) -> String {
        self.capabilities
            .channel
            .as_ref()
            .and_then(|c| c.webhook.as_ref())
            .and_then(|w| w.secret_name.clone())
            .unwrap_or_else(|| format!("{}_webhook_secret", self.name))
    }

    /// Get the webhook secret validation mode for this channel.
    pub fn webhook_secret_validation(&self) -> WebhookSecretValidation {
        self.capabilities
            .channel
            .as_ref()
            .and_then(|c| c.webhook.as_ref())
            .map(|w| w.secret_validation)
            .unwrap_or_default()
    }

    /// Get the query parameter name used for GET/HEAD webhook verification.
    pub fn webhook_verify_token_param(&self) -> Option<&str> {
        self.capabilities
            .channel
            .as_ref()
            .and_then(|c| c.webhook.as_ref())
            .and_then(|w| w.verify_token_param.as_deref())
    }

    /// Get the verify-token secret name for GET/HEAD webhook verification.
    pub fn webhook_verify_token_secret_name(&self) -> Option<String> {
        let webhook = self
            .capabilities
            .channel
            .as_ref()
            .and_then(|c| c.webhook.as_ref())?;

        webhook.verify_token_param.as_ref()?;

        Some(
            webhook
                .verify_token_secret_name
                .clone()
                .unwrap_or_else(|| self.webhook_secret_name()),
        )
    }

    /// Get formatting hints declared by the channel package, if any.
    pub fn formatting_hints(&self) -> Option<&str> {
        self.formatting_hints.as_deref()
    }
}

/// Schema for channel capabilities.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelCapabilitiesSchema {
    #[serde(default)]
    pub http: Option<HttpCapabilitySchema>,
    #[serde(default)]
    pub secrets: Option<SecretsCapabilitySchema>,
    #[serde(default)]
    pub tool_invoke: Option<ToolInvokeCapabilitySchema>,
    #[serde(default)]
    pub workspace: Option<WorkspaceCapabilitySchema>,

    /// Channel-specific capabilities.
    #[serde(default)]
    pub channel: Option<ChannelSpecificCapabilitiesSchema>,
}

impl ChannelCapabilitiesSchema {
    fn to_tool_capabilities(&self) -> ToolCapabilities {
        let mut caps = ToolCapabilities::default();

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
                    .map(RateLimitSchema::to_rate_limit_config)
                    .unwrap_or_default(),
            });
        }
        if let Some(workspace) = &self.workspace {
            caps.workspace_read = Some(WorkspaceCapability {
                allowed_prefixes: workspace.allowed_prefixes.clone(),
            });
        }

        caps
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpCapabilitySchema {
    #[serde(default)]
    pub allowlist: Vec<EndpointPatternSchema>,
    #[serde(default)]
    pub credentials: HashMap<String, CredentialMappingSchema>,
    #[serde(default)]
    pub rate_limit: Option<RateLimitSchema>,
    #[serde(default)]
    pub max_request_bytes: Option<usize>,
    #[serde(default)]
    pub max_response_bytes: Option<usize>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

impl HttpCapabilitySchema {
    fn to_http_capability(&self) -> HttpCapability {
        let mut cap = HttpCapability {
            allowlist: self
                .allowlist
                .iter()
                .map(EndpointPatternSchema::to_endpoint_pattern)
                .collect(),
            credentials: self
                .credentials
                .iter()
                .map(|(alias, mapping)| (alias.clone(), mapping.to_credential_mapping()))
                .collect(),
            rate_limit: self
                .rate_limit
                .as_ref()
                .map(RateLimitSchema::to_rate_limit_config)
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointPatternSchema {
    pub host: String,
    #[serde(default)]
    pub path_prefix: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialMappingSchema {
    pub secret_name: String,
    pub location: CredentialLocationSchema,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum CredentialLocationSchema {
    Bearer,
    Basic {
        username: String,
    },
    Header {
        name: String,
        #[serde(default)]
        prefix: Option<String>,
    },
    QueryParam {
        name: String,
    },
    UrlPath {
        placeholder: String,
    },
    UrlBase {
        placeholder: String,
    },
    Body {
        placeholder: String,
    },
}

impl CredentialLocationSchema {
    fn to_credential_location(&self) -> CredentialLocation {
        match self {
            Self::Bearer => CredentialLocation::Bearer,
            Self::Basic { username } => CredentialLocation::Basic {
                username: username.clone(),
            },
            Self::Header { name, prefix } => CredentialLocation::Header {
                name: name.clone(),
                prefix: prefix.clone(),
            },
            Self::QueryParam { name } => CredentialLocation::QueryParam { name: name.clone() },
            Self::UrlPath { placeholder } => CredentialLocation::UrlPath {
                placeholder: placeholder.clone(),
            },
            Self::UrlBase { placeholder } => CredentialLocation::UrlBase {
                placeholder: placeholder.clone(),
            },
            Self::Body { placeholder } => CredentialLocation::Body {
                placeholder: placeholder.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RateLimitSchema {
    #[serde(default = "default_requests_per_minute")]
    pub requests_per_minute: u32,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretsCapabilitySchema {
    #[serde(default)]
    pub allowed_names: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolInvokeCapabilitySchema {
    #[serde(default)]
    pub aliases: HashMap<String, String>,
    #[serde(default)]
    pub rate_limit: Option<RateLimitSchema>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceCapabilitySchema {
    #[serde(default)]
    pub allowed_prefixes: Vec<String>,
}

impl ChannelCapabilitiesSchema {
    /// Convert to runtime ChannelCapabilities.
    pub fn to_channel_capabilities(&self, channel_name: &str) -> ChannelCapabilities {
        let tool_caps = self.to_tool_capabilities();

        let mut caps =
            ChannelCapabilities::for_channel(channel_name).with_tool_capabilities(tool_caps);

        if let Some(channel) = &self.channel {
            caps.allowed_paths = channel.allowed_paths.clone();
            caps.allow_polling = channel.allow_polling;
            caps.min_poll_interval_ms = channel
                .min_poll_interval_ms
                .unwrap_or(MIN_POLL_INTERVAL_MS)
                .max(MIN_POLL_INTERVAL_MS);

            if let Some(prefix) = &channel.workspace_prefix {
                caps.workspace_prefix = prefix.clone();
            }

            if let Some(rate) = &channel.emit_rate_limit {
                caps.emit_rate_limit = rate.to_emit_rate_limit();
            }

            if let Some(max_size) = channel.max_message_size {
                caps.max_message_size = max_size;
            }

            if let Some(timeout_secs) = channel.callback_timeout_secs {
                caps.callback_timeout = Duration::from_secs(timeout_secs);
            }
        }

        caps
    }
}

/// Channel-specific capabilities schema.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelSpecificCapabilitiesSchema {
    /// HTTP paths the channel can register for webhooks.
    #[serde(default)]
    pub allowed_paths: Vec<String>,

    /// Whether polling is allowed.
    #[serde(default)]
    pub allow_polling: bool,

    /// Minimum poll interval in milliseconds.
    #[serde(default)]
    pub min_poll_interval_ms: Option<u32>,

    /// Workspace prefix for storage (overrides default).
    #[serde(default)]
    pub workspace_prefix: Option<String>,

    /// Rate limiting for emit_message.
    #[serde(default)]
    pub emit_rate_limit: Option<EmitRateLimitSchema>,

    /// Maximum message content size in bytes.
    #[serde(default)]
    pub max_message_size: Option<usize>,

    /// Callback timeout in seconds.
    #[serde(default)]
    pub callback_timeout_secs: Option<u64>,

    /// Webhook configuration (secret header, etc.).
    #[serde(default)]
    pub webhook: Option<WebhookSchema>,
}

/// Webhook configuration schema.
///
/// Allows channels to specify their webhook validation requirements.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebhookSchema {
    /// HTTP header name for secret validation.
    ///
    /// Examples:
    /// - Telegram: "X-Telegram-Bot-Api-Secret-Token"
    /// - Slack: "X-Slack-Signature"
    /// - GitHub: "X-Hub-Signature-256"
    /// - Generic: "X-Webhook-Secret"
    #[serde(default)]
    pub secret_header: Option<String>,

    /// Secret name in secrets store for webhook validation.
    /// Default: "{channel_name}_webhook_secret"
    #[serde(default)]
    pub secret_name: Option<String>,

    /// How POST webhook secrets should be validated.
    #[serde(default)]
    pub secret_validation: WebhookSecretValidation,

    /// Query parameter name used for GET/HEAD webhook verification.
    #[serde(default)]
    pub verify_token_param: Option<String>,

    /// Secret name in secrets store for GET/HEAD verify-token validation.
    ///
    /// Defaults to `secret_name` when omitted.
    #[serde(default)]
    pub verify_token_secret_name: Option<String>,
}

/// Validation mode for webhook secrets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookSecretValidation {
    /// Compare the provided secret directly with the configured secret value.
    #[default]
    Equals,
    /// Validate the request body using a `sha256=<hex>` HMAC-SHA256 signature.
    HmacSha256Body,
    /// Validate the request body using a base64 HMAC-SHA256 signature.
    HmacSha256Base64Body,
    /// Validate Twitch EventSub signatures over id + timestamp + body.
    TwitchEventsubHmacSha256,
    /// Validate Twilio signatures over callback URL plus sorted form fields.
    TwilioRequestSignature,
    /// Validate Slack signatures: `v0=<hex>` HMAC-SHA256 over
    /// `v0:{X-Slack-Request-Timestamp}:{body}`, with a 5-minute replay window.
    SlackV0Signature,
    /// Validate Discord interaction signatures: an Ed25519 signature over
    /// `X-Signature-Timestamp` concatenated with the raw request body, verified
    /// against the application's hex-encoded public key (`discord_public_key`).
    DiscordEd25519,
}

/// Setup configuration schema.
///
/// Allows channels to declare their setup requirements for the wizard.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupSchema {
    /// Required secrets that must be configured during setup.
    #[serde(default)]
    pub required_secrets: Vec<SecretSetupSchema>,

    /// Optional validation endpoint to verify configuration. The legacy string
    /// form remains readable for static, unauthenticated checks. Credentialed
    /// validation should use the structured form so secrets stay in headers.
    #[serde(default)]
    pub validation_endpoint: Option<SetupValidationEndpointSchema>,
}

/// Setup validation endpoint, with backwards-compatible parsing of legacy
/// static URL strings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SetupValidationEndpointSchema {
    LegacyUrl(String),
    Request(SetupValidationRequestSchema),
}

impl SetupValidationEndpointSchema {
    pub fn url(&self) -> &str {
        match self {
            Self::LegacyUrl(url) => url,
            Self::Request(request) => &request.url,
        }
    }

    pub fn request(&self) -> Option<&SetupValidationRequestSchema> {
        match self {
            Self::LegacyUrl(_) => None,
            Self::Request(request) => Some(request),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupValidationRequestSchema {
    pub url: String,
    #[serde(default = "default_setup_validation_method")]
    pub method: String,
    #[serde(default = "default_setup_validation_status")]
    pub success_status: u16,
    #[serde(default)]
    pub secret_name: Option<String>,
    #[serde(default)]
    pub credential: Option<CredentialLocationSchema>,
}

fn default_setup_validation_method() -> String {
    "GET".to_string()
}

fn default_setup_validation_status() -> u16 {
    200
}

/// Configuration for a secret required during setup.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretSetupSchema {
    /// Secret name in the secrets store (e.g., "telegram_bot_token").
    pub name: String,

    /// Prompt to show the user during setup.
    pub prompt: String,

    /// Optional regex for validation.
    #[serde(default)]
    pub validation: Option<String>,

    /// Whether this secret is optional.
    #[serde(default)]
    pub optional: bool,

    /// Auto-generate configuration if the user doesn't provide a value.
    #[serde(default)]
    pub auto_generate: Option<AutoGenerateSchema>,
}

/// Configuration for auto-generating a secret value.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutoGenerateSchema {
    /// Length of the generated value in bytes (will be hex-encoded).
    #[serde(default = "default_auto_generate_length")]
    pub length: usize,
}

fn default_auto_generate_length() -> usize {
    32
}

/// Schema for emit rate limiting.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmitRateLimitSchema {
    /// Maximum messages per minute.
    #[serde(default = "default_messages_per_minute")]
    pub messages_per_minute: u32,

    /// Maximum messages per hour.
    #[serde(default = "default_messages_per_hour")]
    pub messages_per_hour: u32,
}

fn default_messages_per_minute() -> u32 {
    100
}

fn default_messages_per_hour() -> u32 {
    5000
}

impl EmitRateLimitSchema {
    fn to_emit_rate_limit(&self) -> EmitRateLimitConfig {
        EmitRateLimitConfig {
            messages_per_minute: self.messages_per_minute,
            messages_per_hour: self.messages_per_hour,
        }
    }
}

impl From<RateLimitSchema> for EmitRateLimitSchema {
    fn from(schema: RateLimitSchema) -> Self {
        Self {
            messages_per_minute: schema.requests_per_minute,
            messages_per_hour: schema.requests_per_hour,
        }
    }
}

/// Channel configuration returned by on_start.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelConfig {
    /// Display name for the channel.
    pub display_name: String,

    /// HTTP endpoints to register.
    #[serde(default)]
    pub http_endpoints: Vec<HttpEndpointConfigSchema>,

    /// Polling configuration.
    #[serde(default)]
    pub poll: Option<PollConfigSchema>,
}

impl Default for ChannelConfig {
    fn default() -> Self {
        Self {
            display_name: "WASM Channel".to_string(),
            http_endpoints: Vec::new(),
            poll: None,
        }
    }
}

/// HTTP endpoint configuration schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpEndpointConfigSchema {
    /// Path to register.
    pub path: String,

    /// HTTP methods to accept.
    #[serde(default)]
    pub methods: Vec<String>,

    /// Whether secret validation is required.
    #[serde(default)]
    pub require_secret: bool,
}

/// Polling configuration schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PollConfigSchema {
    /// Polling interval in milliseconds.
    pub interval_ms: u32,

    /// Whether polling is enabled.
    #[serde(default)]
    pub enabled: bool,
}

#[cfg(test)]
mod tests {
    use crate::wasm::schema::{ChannelCapabilitiesFile, WebhookSecretValidation};

    #[test]
    fn test_parse_minimal() {
        let json = r#"{
            "name": "test"
        }"#;
        let file = ChannelCapabilitiesFile::from_json(json).unwrap();
        assert_eq!(file.name, "test");
        assert_eq!(file.r#type, "channel");
    }

    #[test]
    fn test_parse_full_slack_example() {
        let json = r#"{
            "type": "channel",
            "name": "slack",
            "description": "Slack Events API channel",
            "formatting_hints": "Use Slack mrkdwn. Avoid triple backticks when plain text will do.",
            "setup": {
                "required_secrets": [{
                    "name": "slack_bot_token",
                    "prompt": "Slack bot token"
                }]
            },
            "capabilities": {
                "http": {
                    "allowlist": [
                        { "host": "slack.com", "path_prefix": "/api/" }
                    ],
                    "credentials": {
                        "slack_bot": {
                            "secret_name": "slack_bot_token",
                            "location": { "type": "bearer" },
                            "host_patterns": ["slack.com"]
                        }
                    },
                    "rate_limit": { "requests_per_minute": 50, "requests_per_hour": 1000 }
                },
                "secrets": { "allowed_names": ["slack_*"] },
                "channel": {
                    "allowed_paths": ["/webhook/slack"],
                    "allow_polling": false,
                    "emit_rate_limit": { "messages_per_minute": 100, "messages_per_hour": 5000 }
                }
            },
            "config": {
                "signing_secret_name": "slack_signing_secret"
            }
        }"#;

        let file = ChannelCapabilitiesFile::from_json(json).unwrap();
        assert_eq!(file.name, "slack");
        assert_eq!(
            file.description,
            Some("Slack Events API channel".to_string())
        );
        assert_eq!(
            file.formatting_hints(),
            Some("Use Slack mrkdwn. Avoid triple backticks when plain text will do.")
        );

        let caps = file.to_capabilities();
        assert!(caps.is_path_allowed("/webhook/slack"));
        assert!(!caps.allow_polling);
        assert_eq!(caps.workspace_prefix, "channels/slack/");

        // Check tool capabilities were parsed
        assert!(caps.tool_capabilities.http.is_some());
        assert!(caps.tool_capabilities.secrets.is_some());

        // Check config
        let config_json = file.config_json();
        assert!(config_json.contains("signing_secret_name"));
    }

    #[test]
    fn test_parse_with_polling() {
        let json = r#"{
            "name": "telegram",
            "capabilities": {
                "channel": {
                    "allowed_paths": [],
                    "allow_polling": true,
                    "min_poll_interval_ms": 60000
                }
            }
        }"#;

        let file = ChannelCapabilitiesFile::from_json(json).unwrap();
        let caps = file.to_capabilities();

        assert!(caps.allow_polling);
        assert_eq!(caps.min_poll_interval_ms, 60000);
    }

    #[test]
    fn test_min_poll_interval_enforced() {
        let json = r#"{
            "name": "test",
            "capabilities": {
                "channel": {
                    "allow_polling": true,
                    "min_poll_interval_ms": 1000
                }
            }
        }"#;

        let file = ChannelCapabilitiesFile::from_json(json).unwrap();
        let caps = file.to_capabilities();

        // Should be clamped to minimum
        assert_eq!(caps.min_poll_interval_ms, 30000);
    }

    #[test]
    fn test_workspace_prefix_override() {
        let json = r#"{
            "name": "custom",
            "capabilities": {
                "channel": {
                    "workspace_prefix": "integrations/custom/"
                }
            }
        }"#;

        let file = ChannelCapabilitiesFile::from_json(json).unwrap();
        let caps = file.to_capabilities();

        assert_eq!(caps.workspace_prefix, "integrations/custom/");
    }

    #[test]
    fn test_emit_rate_limit() {
        let json = r#"{
            "name": "test",
            "capabilities": {
                "channel": {
                    "emit_rate_limit": {
                        "messages_per_minute": 50,
                        "messages_per_hour": 1000
                    }
                }
            }
        }"#;

        let file = ChannelCapabilitiesFile::from_json(json).unwrap();
        let caps = file.to_capabilities();

        assert_eq!(caps.emit_rate_limit.messages_per_minute, 50);
        assert_eq!(caps.emit_rate_limit.messages_per_hour, 1000);
    }

    #[test]
    fn test_webhook_schema() {
        let json = r#"{
            "name": "telegram",
            "setup": {
                "required_secrets": [
                    {"name": "telegram_webhook_secret", "prompt": "Webhook secret"},
                    {"name": "telegram_verify_token", "prompt": "Verify token"}
                ]
            },
            "capabilities": {
                "channel": {
                    "allowed_paths": ["/webhook/telegram"],
                    "webhook": {
                        "secret_header": "X-Telegram-Bot-Api-Secret-Token",
                        "secret_name": "telegram_webhook_secret",
                        "secret_validation": "hmac_sha256_body",
                        "verify_token_param": "hub.verify_token",
                        "verify_token_secret_name": "telegram_verify_token"
                    }
                }
            }
        }"#;

        let file = ChannelCapabilitiesFile::from_json(json).unwrap();
        assert_eq!(
            file.webhook_secret_header(),
            Some("X-Telegram-Bot-Api-Secret-Token")
        );
        assert_eq!(file.webhook_secret_name(), "telegram_webhook_secret");
        assert_eq!(
            file.webhook_secret_validation(),
            WebhookSecretValidation::HmacSha256Body
        );
        assert_eq!(file.webhook_verify_token_param(), Some("hub.verify_token"));
        assert_eq!(
            file.webhook_verify_token_secret_name().as_deref(),
            Some("telegram_verify_token")
        );
    }

    #[test]
    fn test_webhook_secret_name_default() {
        let json = r#"{
            "name": "mybot",
            "capabilities": {}
        }"#;

        let file = ChannelCapabilitiesFile::from_json(json).unwrap();
        assert_eq!(file.webhook_secret_header(), None);
        assert_eq!(file.webhook_secret_name(), "mybot_webhook_secret");
        assert_eq!(
            file.webhook_secret_validation(),
            WebhookSecretValidation::Equals
        );
        assert_eq!(file.webhook_verify_token_param(), None);
        assert_eq!(file.webhook_verify_token_secret_name(), None);
    }

    #[test]
    fn bundled_channel_capabilities_all_validate() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("channels-src");
        let mut checked = 0usize;
        for directory in std::fs::read_dir(&root).unwrap() {
            let directory = directory.unwrap();
            if !directory.file_type().unwrap().is_dir() {
                continue;
            }
            for entry in std::fs::read_dir(directory.path()).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                if path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(".capabilities.json"))
                {
                    let bytes = std::fs::read(&path).unwrap();
                    ChannelCapabilitiesFile::from_bytes(&bytes).unwrap_or_else(|error| {
                        panic!("{} failed validation: {error}", path.display())
                    });
                    checked += 1;
                }
            }
        }
        assert!(checked >= 16, "expected all bundled channel manifests");
    }

    #[test]
    fn test_webhook_verify_token_secret_name_defaults_to_post_secret() {
        let json = r#"{
            "name": "whatsapp",
            "setup": {
                "required_secrets": [{
                    "name": "whatsapp_app_secret",
                    "prompt": "App secret"
                }]
            },
            "capabilities": {
                "channel": {
                    "webhook": {
                        "secret_header": "X-Hub-Signature-256",
                        "secret_name": "whatsapp_app_secret",
                        "verify_token_param": "hub.verify_token"
                    }
                }
            }
        }"#;

        let file = ChannelCapabilitiesFile::from_json(json).unwrap();
        assert_eq!(
            file.webhook_verify_token_secret_name().as_deref(),
            Some("whatsapp_app_secret")
        );
    }

    #[test]
    fn test_setup_schema() {
        let json = r#"{
            "name": "telegram",
            "setup": {
                "required_secrets": [
                    {
                        "name": "telegram_bot_token",
                        "prompt": "Enter your Telegram Bot Token",
                        "validation": "^[0-9]+:[A-Za-z0-9_-]+$"
                    },
                    {
                        "name": "telegram_webhook_secret",
                        "prompt": "Webhook secret (leave empty to auto-generate)",
                        "optional": true,
                        "auto_generate": { "length": 64 }
                    }
                ],
                "validation_endpoint": {
                    "url": "https://api.telegram.org/bot/getMe",
                    "secret_name": "telegram_bot_token",
                    "credential": { "type": "bearer" }
                }
            },
            "capabilities": {
                "http": {
                    "allowlist": [{
                        "host": "api.telegram.org",
                        "path_prefix": "/bot/",
                        "methods": ["GET"]
                    }]
                }
            }
        }"#;

        let file = ChannelCapabilitiesFile::from_json(json).unwrap();
        assert_eq!(file.setup.required_secrets.len(), 2);
        assert_eq!(file.setup.required_secrets[0].name, "telegram_bot_token");
        assert!(!file.setup.required_secrets[0].optional);
        assert!(file.setup.required_secrets[1].optional);
        assert_eq!(
            file.setup.required_secrets[1]
                .auto_generate
                .as_ref()
                .unwrap()
                .length,
            64
        );
    }
}
