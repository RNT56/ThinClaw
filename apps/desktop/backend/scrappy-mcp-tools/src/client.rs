use futures::StreamExt;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;
use tracing::debug;

const MAX_MCP_REQUEST_BYTES: usize = 4 * 1024 * 1024;
const MAX_MCP_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_MCP_ERROR_BYTES: usize = 16 * 1024;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum McpError {
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("MCP server error: {0}")]
    Server(String),
}

pub type McpResult<T> = Result<T, McpError>;

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct McpConfig {
    /// Base URL of the FastAPI MCP server (e.g. "https://api.thinclaw.dev")
    pub base_url: String,
    /// JWT bearer token
    pub auth_token: String,
    /// Request timeout in milliseconds (default 30 000)
    pub timeout_ms: u64,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            auth_token: String::new(),
            timeout_ms: 30_000,
        }
    }
}

// ---------------------------------------------------------------------------
// MCP JSON-RPC types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct ToolCallRequest {
    tool: String,
    arguments: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct ToolCallResponse {
    #[serde(default)]
    result: serde_json::Value,
    #[serde(default)]
    error: Option<String>,
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct McpClient {
    base_url: String,
    auth_token: String,
    timeout: Duration,
    is_loopback: bool,
}

impl std::fmt::Debug for McpClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let endpoint_origin = reqwest::Url::parse(&self.base_url)
            .ok()
            .map(|url| url.origin().ascii_serialization())
            .unwrap_or_else(|| "<invalid>".to_string());
        formatter
            .debug_struct("McpClient")
            .field("endpoint_origin", &endpoint_origin)
            .field("has_auth_token", &!self.auth_token.is_empty())
            .field("timeout", &self.timeout)
            .field("is_loopback", &self.is_loopback)
            .finish()
    }
}

impl McpClient {
    pub fn new(config: McpConfig) -> McpResult<Self> {
        if config.base_url.trim() != config.base_url
            || config.base_url.is_empty()
            || config.base_url.len() > 4_096
            || config.base_url.chars().any(char::is_control)
        {
            return Err(McpError::Server(
                "MCP base URL is missing or invalid".into(),
            ));
        }
        let mut base = reqwest::Url::parse(&config.base_url)
            .map_err(|_| McpError::Server("MCP base URL is invalid".into()))?;
        if !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(McpError::Server(
                "MCP base URL must not contain credentials, a query, or a fragment".into(),
            ));
        }
        let host = base
            .host_str()
            .ok_or_else(|| McpError::Server("MCP base URL has no host".into()))?;
        let is_loopback = host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback());
        if (is_loopback && !matches!(base.scheme(), "http" | "https"))
            || (!is_loopback && base.scheme() != "https")
        {
            return Err(McpError::Server(
                "Remote MCP endpoints require HTTPS; local endpoints require loopback HTTP(S)"
                    .into(),
            ));
        }
        if config.auth_token.trim() != config.auth_token
            || config.auth_token.len() > 16 * 1024
            || config.auth_token.chars().any(char::is_control)
        {
            return Err(McpError::Server(
                "MCP authentication token is invalid".into(),
            ));
        }
        let timeout_ms = config.timeout_ms.clamp(1_000, 5 * 60 * 1_000);
        let path = base.path().trim_end_matches('/').to_string();
        base.set_path(if path.is_empty() { "/" } else { &path });

        Ok(Self {
            base_url: base.as_str().trim_end_matches('/').to_string(),
            auth_token: config.auth_token,
            timeout: Duration::from_millis(timeout_ms),
            is_loopback,
        })
    }

    async fn request_client(&self, url: &reqwest::Url) -> McpResult<reqwest::Client> {
        let host = url
            .host_str()
            .ok_or_else(|| McpError::Server("MCP endpoint has no host".into()))?;
        let mut builder = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(10))
            .timeout(self.timeout)
            .redirect(reqwest::redirect::Policy::none());
        if !self.is_loopback {
            let guarded = thinclaw_tools_core::validate_outbound_url_pinned_async(
                url.as_str(),
                &thinclaw_tools_core::OutboundUrlGuardOptions {
                    require_https: true,
                    upgrade_http_to_https: false,
                    allowlist: vec![host.to_string()],
                },
            )
            .await
            .map_err(|_| {
                McpError::Server("MCP endpoint is not a public HTTPS destination".into())
            })?;
            if !guarded.pinned_addrs.is_empty() {
                builder = builder.resolve_to_addrs(host, &guarded.pinned_addrs);
            }
        }
        builder
            .build()
            .map_err(|_| McpError::Http("Could not create MCP HTTP client".into()))
    }

    fn tool_url(&self) -> McpResult<reqwest::Url> {
        let mut url = reqwest::Url::parse(&self.base_url)
            .map_err(|_| McpError::Server("Stored MCP base URL is invalid".into()))?;
        let path = format!("{}/tools/call", url.path().trim_end_matches('/'));
        url.set_path(&path);
        Ok(url)
    }

    async fn bounded_body(response: reqwest::Response, limit: usize) -> McpResult<Vec<u8>> {
        if response
            .content_length()
            .is_some_and(|length| length > u64::try_from(limit).unwrap_or(u64::MAX))
        {
            return Err(McpError::Server(
                "MCP response exceeds the size limit".into(),
            ));
        }
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| McpError::Http("MCP response stream failed".into()))?;
            if body.len().saturating_add(chunk.len()) > limit {
                return Err(McpError::Server(
                    "MCP response exceeds the size limit".into(),
                ));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    /// Call an MCP tool by name with JSON arguments.
    /// Returns the server response deserialized into `T`.
    pub async fn call_tool<T: DeserializeOwned>(
        &self,
        tool: &str,
        arguments: serde_json::Value,
    ) -> McpResult<T> {
        if tool.is_empty() || tool.len() > 256 || tool.chars().any(char::is_control) {
            return Err(McpError::Server("MCP tool name is invalid".into()));
        }
        let url = self.tool_url()?;
        debug!(tool, "[mcp-client] calling bounded MCP tool");

        let body = ToolCallRequest {
            tool: tool.to_string(),
            arguments,
        };

        let encoded = serde_json::to_vec(&body)?;
        if encoded.len() > MAX_MCP_REQUEST_BYTES {
            return Err(McpError::Server(
                "MCP request exceeds the size limit".into(),
            ));
        }
        let client = self.request_client(&url).await?;
        let mut request = client
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(encoded);
        if !self.auth_token.is_empty() {
            request = request.bearer_auth(&self.auth_token);
        }
        let resp = request
            .send()
            .await
            .map_err(|_| McpError::Http("MCP request failed".into()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            if matches!(status.as_u16(), 401 | 403) {
                return Err(McpError::Server(format!(
                    "MCP server rejected the configured credential (HTTP {status})"
                )));
            }
            let detail = Self::bounded_body(resp, MAX_MCP_ERROR_BYTES)
                .await
                .ok()
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .map(|text| {
                    text.chars()
                        .filter(|character| !character.is_control() || *character == '\n')
                        .take(2_048)
                        .collect::<String>()
                })
                .filter(|text| !text.trim().is_empty())
                .unwrap_or_else(|| "no bounded error detail".into());
            return Err(McpError::Server(format!("HTTP {status}: {detail}")));
        }

        let body = Self::bounded_body(resp, MAX_MCP_RESPONSE_BYTES).await?;
        let wrapper: ToolCallResponse = serde_json::from_slice(&body)?;

        if let Some(err) = wrapper.error {
            if err.len() > MAX_MCP_ERROR_BYTES || err.chars().any(char::is_control) {
                return Err(McpError::Server(
                    "MCP server returned an invalid error".into(),
                ));
            }
            return Err(McpError::Server(err));
        }

        let typed: T = serde_json::from_value(wrapper.result)?;
        Ok(typed)
    }

    /// Raw call returning `serde_json::Value` for untyped consumers.
    pub async fn call_tool_raw(
        &self,
        tool: &str,
        arguments: serde_json::Value,
    ) -> McpResult<serde_json::Value> {
        self.call_tool::<serde_json::Value>(tool, arguments).await
    }

    /// Get the base URL (useful for health-check / diagnostics).
    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(base_url: &str) -> McpConfig {
        McpConfig {
            base_url: base_url.to_string(),
            auth_token: "very-secret-token".to_string(),
            timeout_ms: 30_000,
        }
    }

    #[test]
    fn requires_https_except_for_loopback() {
        assert!(McpClient::new(config("http://example.com")).is_err());
        assert!(McpClient::new(config("https://example.com/api")).is_ok());
        assert!(McpClient::new(config("http://127.0.0.1:8000")).is_ok());
        assert!(McpClient::new(config("http://localhost:8000")).is_ok());
    }

    #[test]
    fn rejects_ambiguous_or_credentialed_base_urls() {
        for url in [
            " https://example.com",
            "https://user@example.com",
            "https://example.com?token=secret",
            "https://example.com/#secret",
            "file:///tmp/socket",
        ] {
            assert!(McpClient::new(config(url)).is_err(), "accepted {url}");
        }
    }

    #[test]
    fn debug_output_redacts_token_and_endpoint_path() {
        let client = McpClient::new(config("https://example.com/private/token-path")).unwrap();
        let output = format!("{client:?}");
        assert!(output.contains("https://example.com"));
        assert!(!output.contains("very-secret-token"));
        assert!(!output.contains("token-path"));
    }

    #[test]
    fn constructs_tool_endpoint_without_losing_base_path() {
        let client = McpClient::new(config("https://example.com/mcp/v1/")).unwrap();
        assert_eq!(
            client.tool_url().unwrap().as_str(),
            "https://example.com/mcp/v1/tools/call"
        );
    }
}
