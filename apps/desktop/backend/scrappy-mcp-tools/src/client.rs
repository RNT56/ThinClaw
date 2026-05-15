use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, error};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum McpError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
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

#[derive(Debug, Clone)]
pub struct McpClient {
    http: reqwest::Client,
    base_url: String,
}

impl McpClient {
    pub fn new(config: McpConfig) -> McpResult<Self> {
        let mut headers = HeaderMap::new();
        if !config.auth_token.is_empty() {
            let val = HeaderValue::from_str(&format!("Bearer {}", config.auth_token))
                .map_err(|e| McpError::Server(format!("Invalid auth token header: {}", e)))?;
            headers.insert(AUTHORIZATION, val);
        }

        let http = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_millis(config.timeout_ms))
            .build()?;

        Ok(Self {
            http,
            base_url: config.base_url.trim_end_matches('/').to_string(),
        })
    }

    /// Call an MCP tool by name with JSON arguments.
    /// Returns the server response deserialized into `T`.
    pub async fn call_tool<T: DeserializeOwned>(
        &self,
        tool: &str,
        arguments: serde_json::Value,
    ) -> McpResult<T> {
        let url = format!("{}/tools/call", self.base_url);

        debug!("[mcp-client] calling tool '{}' -> {}", tool, url);

        let body = ToolCallRequest {
            tool: tool.to_string(),
            arguments,
        };

        let resp = self.http.post(&url).json(&body).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            error!("[mcp-client] server returned {}: {}", status, text);
            return Err(McpError::Server(format!("HTTP {}: {}", status, text)));
        }

        let wrapper: ToolCallResponse = resp.json().await?;

        if let Some(err) = wrapper.error {
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
