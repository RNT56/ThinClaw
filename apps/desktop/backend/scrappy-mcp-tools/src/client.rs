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
    #[error("invalid MCP endpoint: {0}")]
    InvalidEndpoint(String),
    #[error("invalid MCP request: {0}")]
    InvalidRequest(String),
    #[error("MCP destination denied: {0}")]
    DestinationDenied(String),
    #[error("MCP credential rejected")]
    Unauthorized,
    #[error("MCP request timed out")]
    Timeout,
    #[error("MCP redirect denied")]
    RedirectDenied,
    #[error("MCP response exceeds the size limit")]
    ResponseTooLarge,
    #[error("malformed MCP response: {0}")]
    MalformedResponse(String),
    #[error("MCP network error: {0}")]
    Network(String),
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
            return Err(McpError::InvalidEndpoint(
                "MCP base URL is missing or invalid".into(),
            ));
        }
        let mut base = reqwest::Url::parse(&config.base_url)
            .map_err(|_| McpError::InvalidEndpoint("MCP base URL is invalid".into()))?;
        if !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(McpError::InvalidEndpoint(
                "MCP base URL must not contain credentials, a query, or a fragment".into(),
            ));
        }
        let host = base
            .host_str()
            .ok_or_else(|| McpError::InvalidEndpoint("MCP base URL has no host".into()))?;
        let is_loopback = host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback());
        if (is_loopback && !matches!(base.scheme(), "http" | "https"))
            || (!is_loopback && base.scheme() != "https")
        {
            return Err(McpError::InvalidEndpoint(
                "Remote MCP endpoints require HTTPS; local endpoints require loopback HTTP(S)"
                    .into(),
            ));
        }
        if config.auth_token.trim() != config.auth_token
            || config.auth_token.len() > 16 * 1024
            || config.auth_token.chars().any(char::is_control)
        {
            return Err(McpError::InvalidRequest(
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
            .ok_or_else(|| McpError::InvalidEndpoint("MCP endpoint has no host".into()))?;
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
                McpError::DestinationDenied(
                    "endpoint is not an allowed public HTTPS destination".into(),
                )
            })?;
            if !guarded.pinned_addrs.is_empty() {
                builder = builder.resolve_to_addrs(host, &guarded.pinned_addrs);
            }
        }
        builder
            .build()
            .map_err(|_| McpError::Network("could not create the bounded HTTP client".into()))
    }

    fn tool_url(&self) -> McpResult<reqwest::Url> {
        let mut url = reqwest::Url::parse(&self.base_url)
            .map_err(|_| McpError::InvalidEndpoint("stored MCP base URL is invalid".into()))?;
        let path = format!("{}/tools/call", url.path().trim_end_matches('/'));
        url.set_path(&path);
        Ok(url)
    }

    async fn bounded_body(response: reqwest::Response, limit: usize) -> McpResult<Vec<u8>> {
        if response
            .content_length()
            .is_some_and(|length| length > u64::try_from(limit).unwrap_or(u64::MAX))
        {
            return Err(McpError::ResponseTooLarge);
        }
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| {
                if error.is_timeout() {
                    McpError::Timeout
                } else {
                    McpError::Network("response stream failed".into())
                }
            })?;
            if body.len().saturating_add(chunk.len()) > limit {
                return Err(McpError::ResponseTooLarge);
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
            return Err(McpError::InvalidRequest("MCP tool name is invalid".into()));
        }
        let url = self.tool_url()?;
        debug!(tool, "[mcp-client] calling bounded MCP tool");

        let body = ToolCallRequest {
            tool: tool.to_string(),
            arguments,
        };

        let encoded = serde_json::to_vec(&body)
            .map_err(|_| McpError::InvalidRequest("MCP request is not valid JSON".into()))?;
        if encoded.len() > MAX_MCP_REQUEST_BYTES {
            return Err(McpError::InvalidRequest(
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
        let resp = request.send().await.map_err(|error| {
            if error.is_timeout() {
                McpError::Timeout
            } else {
                McpError::Network("request failed before a response was received".into())
            }
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            if status.is_redirection() {
                return Err(McpError::RedirectDenied);
            }
            if matches!(status.as_u16(), 401 | 403) {
                return Err(McpError::Unauthorized);
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
        let wrapper: ToolCallResponse = serde_json::from_slice(&body)
            .map_err(|_| McpError::MalformedResponse("response is not valid tool JSON".into()))?;

        if let Some(err) = wrapper.error {
            if err.len() > MAX_MCP_ERROR_BYTES || err.chars().any(char::is_control) {
                return Err(McpError::MalformedResponse(
                    "MCP server returned an invalid error".into(),
                ));
            }
            return Err(McpError::Server(err));
        }

        let typed: T = serde_json::from_value(wrapper.result)
            .map_err(|_| McpError::MalformedResponse("tool result has an unexpected shape".into()))?;
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

    async fn serve_once(response: &'static [u8]) -> (String, tokio::task::JoinHandle<String>) {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let read = stream.read(&mut chunk).await.unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..read]);
                let header_end = request
                    .windows(4)
                    .position(|window| window == b"\r\n\r\n")
                    .map(|position| position + 4);
                if let Some(header_end) = header_end {
                    let headers = String::from_utf8_lossy(&request[..header_end]);
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            line.split_once(':').and_then(|(name, value)| {
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().ok())
                                    .flatten()
                            })
                        })
                        .unwrap_or(0);
                    if request.len() >= header_end + content_length {
                        break;
                    }
                }
            }
            stream.write_all(response).await.unwrap();
            String::from_utf8(request).unwrap()
        });
        (format!("http://{address}"), handle)
    }

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

    #[tokio::test]
    async fn production_probe_uses_post_tool_protocol_and_bearer_auth() {
        let body = r#"{"result":{"tools":[{"name":"weather"}]}}"#;
        let response = Box::leak(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .into_boxed_str(),
        )
        .as_bytes();
        let (url, request) = serve_once(response).await;
        let client = McpClient::new(config(&url)).unwrap();
        let result = client
            .call_tool_raw("search_tools", serde_json::json!({ "query": "" }))
            .await
            .unwrap();
        assert_eq!(result["tools"][0]["name"], "weather");
        let request = request.await.unwrap();
        assert!(request.starts_with("POST /tools/call HTTP/1.1"));
        assert!(request.to_lowercase().contains("authorization: bearer very-secret-token"));
        assert!(request.contains("\"tool\":\"search_tools\""));
    }

    #[tokio::test]
    async fn redirects_auth_failures_and_oversized_bodies_are_typed() {
        for (response, expected) in [
            (
                b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1/elsewhere\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .as_slice(),
                "redirect",
            ),
            (
                b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .as_slice(),
                "unauthorized",
            ),
            (
                b"HTTP/1.1 200 OK\r\nContent-Length: 9000000\r\nConnection: close\r\n\r\n"
                    .as_slice(),
                "oversized",
            ),
        ] {
            let response: &'static [u8] = Box::leak(response.to_vec().into_boxed_slice());
            let (url, request) = serve_once(response).await;
            let error = McpClient::new(config(&url))
                .unwrap()
                .call_tool_raw("search_tools", serde_json::json!({}))
                .await
                .unwrap_err();
            match expected {
                "redirect" => assert!(matches!(error, McpError::RedirectDenied)),
                "unauthorized" => assert!(matches!(error, McpError::Unauthorized)),
                "oversized" => assert!(matches!(error, McpError::ResponseTooLarge)),
                _ => unreachable!(),
            }
            request.await.unwrap();
        }
    }

    #[tokio::test]
    async fn malformed_responses_and_private_remote_destinations_are_typed() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\nnot-json";
        let (url, request) = serve_once(response).await;
        let error = McpClient::new(config(&url))
            .unwrap()
            .call_tool_raw("search_tools", serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(error, McpError::MalformedResponse(_)));
        request.await.unwrap();

        let private = McpClient::new(config("https://10.0.0.1")).unwrap();
        let error = private
            .call_tool_raw("search_tools", serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(error, McpError::DestinationDenied(_)));
    }

    #[tokio::test]
    async fn bounded_request_deadline_is_reported_as_timeout() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_millis(1_500)).await;
        });
        let client = McpClient::new(McpConfig {
            base_url: format!("http://{address}"),
            auth_token: String::new(),
            timeout_ms: 1_000,
        })
        .unwrap();
        let error = client
            .call_tool_raw("search_tools", serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(error, McpError::Timeout));
        server.abort();
    }
}
