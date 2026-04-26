//! HTTP request tool.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;

use crate::context::JobContext;
use crate::safety::LeakDetector;
use crate::secrets::SecretsStore;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput, require_str};
use crate::tools::url_guard::{OutboundUrlGuardOptions, validate_outbound_url};
use crate::tools::wasm::{InjectedCredentials, SharedCredentialRegistry, inject_credential};

#[cfg(feature = "html-to-markdown")]
use crate::tools::builtin::convert_html_to_markdown;

/// Maximum response body size (5 MB).
///
/// 5 MB is large enough for typical JSON API responses and moderate HTML pages,
/// but small enough to prevent OOM from malicious or runaway servers.  The WASM
/// HTTP wrapper uses the same limit for consistency.
const MAX_RESPONSE_SIZE: usize = 5 * 1024 * 1024;

/// Tool for making HTTP requests.
pub struct HttpTool {
    client: Client,
    credential_registry: Option<Arc<SharedCredentialRegistry>>,
    secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
    /// Optional domain allowlist. When set (non-empty), only URLs whose host
    /// matches one of the glob patterns are permitted. Patterns are
    /// case-insensitive. Example: `*.openai.com, api.github.com`.
    url_allowlist: Vec<String>,
}

impl HttpTool {
    /// Create a new HTTP tool.
    pub fn new() -> Self {
        // Allow redirects with validation: each hop is checked against the
        // SSRF blocklist so we don't follow Location: http://internal-service.
        // We validate inside the execute() method after the response comes back.
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            // Follow up to 10 redirects. We re-validate the final URL manually
            // inside execute() to prevent SSRF via redirect chains.
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 10 {
                    attempt.error("too many redirects")
                } else if validate_outbound_url(
                    attempt.url().as_str(),
                    &OutboundUrlGuardOptions {
                        require_https: true,
                        upgrade_http_to_https: false,
                        allowlist: Vec::new(),
                    },
                )
                .is_ok()
                {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            }))
            .build()
            .expect("Failed to create HTTP client");

        // Parse URL allowlist from environment
        let url_allowlist: Vec<String> = std::env::var("HTTP_URL_ALLOWLIST")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        if !url_allowlist.is_empty() {
            tracing::info!(patterns = ?url_allowlist, "HTTP URL allowlist active");
        }

        Self {
            client,
            credential_registry: None,
            secrets_store: None,
            url_allowlist,
        }
    }

    /// Attach a credential registry and secrets store for auto-injection.
    pub fn with_credentials(
        mut self,
        registry: Arc<SharedCredentialRegistry>,
        secrets_store: Arc<dyn SecretsStore + Send + Sync>,
    ) -> Self {
        self.credential_registry = Some(registry);
        self.secrets_store = Some(secrets_store);
        self
    }
}

fn validate_url(url: &str, url_allowlist: &[String]) -> Result<reqwest::Url, ToolError> {
    validate_outbound_url(
        url,
        &OutboundUrlGuardOptions {
            require_https: true,
            upgrade_http_to_https: true,
            allowlist: url_allowlist.to_vec(),
        },
    )
}

#[cfg(feature = "html-to-markdown")]
/// Heuristic: treat as HTML if the `Content-Type` header contains `text/html`.
fn is_html_response(headers: &HashMap<String, String>) -> bool {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.to_lowercase().contains("text/html"))
        .unwrap_or(false)
}

fn parse_headers_param(
    headers: Option<&serde_json::Value>,
) -> Result<Vec<(String, String)>, ToolError> {
    match headers {
        None => Ok(Vec::new()),
        Some(serde_json::Value::Object(map)) => {
            let mut out = Vec::with_capacity(map.len());
            for (k, v) in map {
                let value = v.as_str().ok_or_else(|| {
                    ToolError::InvalidParameters(format!("header '{}' must have a string value", k))
                })?;
                out.push((k.clone(), value.to_string()));
            }
            Ok(out)
        }
        Some(serde_json::Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for (idx, item) in items.iter().enumerate() {
                let obj = item.as_object().ok_or_else(|| {
                    ToolError::InvalidParameters(format!(
                        "headers[{}] must be an object with 'name' and 'value'",
                        idx
                    ))
                })?;
                let name = obj.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
                    ToolError::InvalidParameters(format!("headers[{}].name must be a string", idx))
                })?;
                let value = obj.get("value").and_then(|v| v.as_str()).ok_or_else(|| {
                    ToolError::InvalidParameters(format!("headers[{}].value must be a string", idx))
                })?;
                out.push((name.to_string(), value.to_string()));
            }
            Ok(out)
        }
        Some(_) => Err(ToolError::InvalidParameters(
            "'headers' must be an object or an array of {name, value}".to_string(),
        )),
    }
}

/// Extract host from URL in params (for approval checks).
fn extract_host_from_params(params: &serde_json::Value) -> Option<String> {
    params
        .get("url")
        .and_then(|u| u.as_str())
        .and_then(|u| reqwest::Url::parse(u).ok())
        .and_then(|u| u.host_str().map(|h| h.to_string()))
}

impl Default for HttpTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for HttpTool {
    fn name(&self) -> &str {
        "http"
    }

    fn description(&self) -> &str {
        "Make raw HTTP requests to external URLs and APIs. Use this when you need a \
         network call and there is no more specific built-in or extension tool for the service."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "method": {
                    "type": "string",
                    "enum": ["GET", "POST", "PUT", "DELETE", "PATCH"],
                    "description": "HTTP method"
                },
                "url": {
                    "type": "string",
                    "description": "The URL to request"
                },
                "headers": {
                    "type": "array",
                    "description": "Optional headers as a list of {name, value} objects",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" },
                            "value": { "type": "string" }
                        },
                        "required": ["name", "value"],
                        "additionalProperties": false
                    }
                },
                "body": {
                    "description": "Request body (for POST/PUT/PATCH). Can be a JSON object, array, string, or other value."
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Request timeout in seconds (default: 30)"
                }
            },
            "required": ["method", "url"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let method = require_str(&params, "method")?;

        let url = require_str(&params, "url")?;
        let mut parsed_url = validate_url(url, &self.url_allowlist)?;

        // Parse headers
        let mut headers_vec = parse_headers_param(params.get("headers"))?;

        // Build request
        let mut request = match method.to_uppercase().as_str() {
            "GET" => self.client.get(parsed_url.clone()),
            "POST" => self.client.post(parsed_url.clone()),
            "PUT" => self.client.put(parsed_url.clone()),
            "DELETE" => self.client.delete(parsed_url.clone()),
            "PATCH" => self.client.patch(parsed_url.clone()),
            _ => {
                return Err(ToolError::InvalidParameters(format!(
                    "unsupported method: {}",
                    method
                )));
            }
        };

        // Add headers
        for (key, value) in &headers_vec {
            request = request.header(key.as_str(), value.as_str());
        }

        // Add body if present
        let body_bytes = if let Some(body) = params.get("body") {
            if let Some(body_str) = body.as_str() {
                if let Ok(json_body) = serde_json::from_str::<serde_json::Value>(body_str) {
                    let bytes = serde_json::to_vec(&json_body).map_err(|e| {
                        ToolError::InvalidParameters(format!("invalid body JSON: {}", e))
                    })?;
                    request = request.json(&json_body);
                    Some(bytes)
                } else {
                    let bytes = body_str.as_bytes().to_vec();
                    request = request.body(body_str.to_string());
                    Some(bytes)
                }
            } else {
                let bytes = serde_json::to_vec(body).map_err(|e| {
                    ToolError::InvalidParameters(format!("invalid body JSON: {}", e))
                })?;
                request = request.json(body);
                Some(bytes)
            }
        } else {
            None
        };

        // Credential injection from shared registry
        if let (Some(registry), Some(store)) = (
            self.credential_registry.as_ref(),
            self.secrets_store.as_ref(),
        ) {
            let host = parsed_url.host_str().unwrap_or("").to_string();
            let path = parsed_url.path().to_string();
            let matched: Vec<crate::secrets::CredentialMapping> = registry.find_for_host(&host);
            for mapping in &matched {
                match store
                    .get_for_injection(
                        &_ctx.user_id,
                        &mapping.secret_name,
                        crate::secrets::SecretAccessContext::new(
                            "builtin.http",
                            "http_credential_injection",
                        )
                        .target(host.clone(), path.clone()),
                    )
                    .await
                {
                    Ok(secret) => {
                        let mut injected = InjectedCredentials::empty();
                        inject_credential(&mut injected, &mapping.location, &secret);
                        for (name, value) in &injected.headers {
                            request = request.header(name.as_str(), value.as_str());
                            headers_vec.push((name.clone(), value.clone()));
                        }
                        for (name, value) in &injected.query_params {
                            parsed_url.query_pairs_mut().append_pair(name, value);
                            request = request.query(&[(name.as_str(), value.as_str())]);
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            secret = %mapping.secret_name,
                            error = %e,
                            "Failed to inject credential for HTTP tool"
                        );
                    }
                }
            }
        }

        // Leak detection on outbound request (url/headers/body)
        let detector = LeakDetector::new();
        detector
            .scan_http_request(parsed_url.as_str(), &headers_vec, body_bytes.as_deref())
            .map_err(|e| ToolError::NotAuthorized(format!("{}", e)))?;

        // Execute request
        let response = request.send().await.map_err(|e| {
            if e.is_timeout() {
                ToolError::Timeout(Duration::from_secs(30))
            } else {
                ToolError::ExternalService(e.to_string())
            }
        })?;

        let status = response.status().as_u16();

        let headers: HashMap<String, String> = response
            .headers()
            .iter()
            .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string())))
            .collect();

        // Pre-check Content-Length header to reject obviously oversized responses
        // before downloading anything, preventing OOM from malicious servers.
        if let Some(content_length) = response.headers().get(reqwest::header::CONTENT_LENGTH)
            && let Ok(s) = content_length.to_str()
            && let Ok(len) = s.parse::<usize>()
            && len > MAX_RESPONSE_SIZE
        {
            tracing::warn!(
                url = %parsed_url,
                content_length = len,
                max = MAX_RESPONSE_SIZE,
                "Rejected HTTP response: Content-Length exceeds limit"
            );
            return Err(ToolError::ExecutionFailed(format!(
                "Response Content-Length ({} bytes) exceeds maximum allowed size ({} bytes)",
                len, MAX_RESPONSE_SIZE
            )));
        }

        // Stream the response body with a hard size cap. Even if Content-Length was
        // absent or lied about the size, we stop reading once we exceed the limit.
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = StreamExt::next(&mut stream).await {
            let chunk = chunk.map_err(|e| {
                ToolError::ExternalService(format!("failed to read response body: {}", e))
            })?;
            if body.len() + chunk.len() > MAX_RESPONSE_SIZE {
                return Err(ToolError::ExecutionFailed(format!(
                    "Response body exceeds maximum allowed size ({} bytes)",
                    MAX_RESPONSE_SIZE
                )));
            }
            body.extend_from_slice(&chunk);
        }
        let body_bytes = bytes::Bytes::from(body);

        let body_text = String::from_utf8_lossy(&body_bytes).into_owned();

        #[cfg(feature = "html-to-markdown")]
        let body_text = if is_html_response(&headers) {
            match convert_html_to_markdown(&body_text, parsed_url.as_str()) {
                Ok(md) => md,
                Err(e) => {
                    tracing::warn!(url = %parsed_url, error = %e, "HTML-to-markdown conversion failed, returning raw HTML");
                    body_text
                }
            }
        } else {
            body_text
        };

        // Try to parse as JSON, fall back to string
        let body: serde_json::Value = serde_json::from_str(&body_text)
            .unwrap_or_else(|_| serde_json::Value::String(body_text.clone()));

        let result = serde_json::json!({
            "status": status,
            "headers": headers,
            "body": body
        });

        Ok(ToolOutput::success(result, start.elapsed()).with_raw(body_text))
    }

    fn estimated_duration(&self, _params: &serde_json::Value) -> Option<Duration> {
        Some(Duration::from_secs(5)) // Average HTTP request time
    }

    fn requires_sanitization(&self) -> bool {
        true // External data always needs sanitization
    }

    fn requires_approval(&self, params: &serde_json::Value) -> ApprovalRequirement {
        // 1. Manual auth headers/query params in LLM params
        if crate::safety::params_contain_manual_credentials(params) {
            return ApprovalRequirement::Always;
        }
        // 2. Target host has credential mappings (will be auto-injected)
        if let Some(ref registry) = self.credential_registry
            && let Some(host) = extract_host_from_params(params)
            && registry.has_credentials_for_host(&host)
        {
            return ApprovalRequirement::Always;
        }
        // Default: outbound HTTP still needs approval unless auto-approved
        ApprovalRequirement::UnlessAutoApproved
    }

    fn rate_limit_config(&self) -> Option<crate::tools::tool::ToolRateLimitConfig> {
        Some(crate::tools::tool::ToolRateLimitConfig::new(30, 500))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_tool_schema_headers_is_array() {
        let tool = HttpTool::new();
        let schema = tool.parameters_schema();
        assert_eq!(schema["properties"]["headers"]["type"], "array");
    }

    #[test]
    fn test_validate_url_upgrades_http_to_https() {
        // validate_url silently upgrades http:// → https:// since the LLM
        // frequently generates http:// URLs even for HTTPS-capable sites.
        let url = validate_url("http://example.com", &[]).unwrap();
        assert_eq!(url.scheme(), "https");
    }

    #[test]
    fn test_validate_url_rejects_localhost() {
        let err = validate_url("https://localhost:8080", &[]).unwrap_err();
        assert!(err.to_string().contains("localhost"));
    }

    #[test]
    fn test_validate_url_accepts_https_public() {
        let url = validate_url("https://example.com", &[]).unwrap();
        assert_eq!(url.host_str(), Some("example.com"));
    }

    #[test]
    fn test_validate_url_rejects_private_ip_literal() {
        let err = validate_url("https://192.168.1.1/api", &[]).unwrap_err();
        assert!(err.to_string().contains("not allowed"));
    }

    #[test]
    fn test_validate_url_rejects_loopback_ip() {
        let err = validate_url("https://127.0.0.1/api", &[]).unwrap_err();
        assert!(err.to_string().contains("not allowed"));
    }

    #[test]
    fn test_validate_url_rejects_link_local() {
        let err = validate_url("https://169.254.169.254/latest/meta-data/", &[]).unwrap_err();
        assert!(err.to_string().contains("not allowed"));
    }

    #[test]
    fn test_is_disallowed_ip_covers_ranges() {
        // Private ranges
        assert!(validate_url("https://10.0.0.1/test", &[]).is_err());
        assert!(validate_url("https://172.16.0.1/test", &[]).is_err());
        assert!(validate_url("https://192.168.0.1/test", &[]).is_err());
        // Loopback
        assert!(validate_url("https://127.0.0.1/test", &[]).is_err());
        // Cloud metadata
        assert!(validate_url("https://169.254.169.254/latest/meta-data/", &[]).is_err());
        // Public
        assert!(validate_url("https://8.8.8.8/test", &[]).is_ok());
    }

    #[test]
    fn test_max_response_size_is_reasonable() {
        // MAX_RESPONSE_SIZE should be 5 MB to prevent OOM while allowing typical API responses.
        assert_eq!(MAX_RESPONSE_SIZE, 5 * 1024 * 1024);
    }

    #[test]
    fn test_parse_headers_param_accepts_object_legacy_shape() {
        let headers = serde_json::json!({"Authorization": "Bearer token"});
        let parsed = parse_headers_param(Some(&headers)).unwrap();
        assert_eq!(
            parsed,
            vec![("Authorization".to_string(), "Bearer token".to_string())]
        );
    }

    #[test]
    fn test_parse_headers_param_accepts_array_shape() {
        let headers = serde_json::json!([
            {"name": "Authorization", "value": "Bearer token"},
            {"name": "X-Test", "value": "1"}
        ]);
        let parsed = parse_headers_param(Some(&headers)).unwrap();
        assert_eq!(
            parsed,
            vec![
                ("Authorization".to_string(), "Bearer token".to_string()),
                ("X-Test".to_string(), "1".to_string())
            ]
        );
    }

    #[test]
    fn test_http_tool_schema_body_is_freeform() {
        let schema = HttpTool::new().parameters_schema();
        let body = schema
            .get("properties")
            .and_then(|p| p.get("body"))
            .expect("body schema missing");

        // Body is intentionally freeform (no "type" constraint) for OpenAI
        // compatibility. OpenAI rejects union types containing "array" unless
        // "items" is also specified, and body accepts any JSON value.
        assert!(
            body.get("type").is_none(),
            "body schema should not have a 'type' to be freeform for OpenAI compatibility"
        );
    }

    // ── Approval requirement tests ──────────────────────────────────────

    #[test]
    fn test_no_auth_headers_returns_unless_auto_approved() {
        let tool = HttpTool::new();
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com/data"
        });
        assert_eq!(
            tool.requires_approval(&params),
            ApprovalRequirement::UnlessAutoApproved
        );
    }

    #[test]
    fn test_auth_header_object_format_returns_always() {
        let tool = HttpTool::new();
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com/data",
            "headers": {"Authorization": "Bearer token123"}
        });
        assert_eq!(tool.requires_approval(&params), ApprovalRequirement::Always);
    }

    #[test]
    fn test_auth_header_array_format_returns_always() {
        let tool = HttpTool::new();
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com/data",
            "headers": [{"name": "Authorization", "value": "Bearer token123"}]
        });
        assert_eq!(tool.requires_approval(&params), ApprovalRequirement::Always);
    }

    #[test]
    fn test_auth_header_case_insensitive() {
        let tool = HttpTool::new();

        // Object format with mixed case
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": {"AUTHORIZATION": "Bearer x"}
        });
        assert_eq!(tool.requires_approval(&params), ApprovalRequirement::Always);

        // Array format with mixed case
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": [{"name": "X-Api-Key", "value": "key123"}]
        });
        assert_eq!(tool.requires_approval(&params), ApprovalRequirement::Always);
    }

    #[test]
    fn test_all_auth_header_names_detected() {
        let tool = HttpTool::new();
        for header_name in [
            "authorization",
            "x-api-key",
            "cookie",
            "proxy-authorization",
            "x-auth-token",
            "api-key",
            "x-token",
            "x-access-token",
            "x-session-token",
            "x-csrf-token",
            "x-secret",
            "x-api-secret",
        ] {
            let mut headers = serde_json::Map::new();
            headers.insert(header_name.to_string(), serde_json::json!("value"));
            let params = serde_json::json!({
                "method": "GET",
                "url": "https://example.com",
                "headers": headers
            });
            assert_eq!(
                tool.requires_approval(&params),
                ApprovalRequirement::Always,
                "Header '{}' should trigger Always approval",
                header_name
            );
        }
    }

    #[test]
    fn test_non_auth_headers_return_unless_auto_approved() {
        let tool = HttpTool::new();
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": {"Content-Type": "application/json", "Accept": "text/html"}
        });
        assert_eq!(
            tool.requires_approval(&params),
            ApprovalRequirement::UnlessAutoApproved
        );
    }

    #[test]
    fn test_empty_headers_return_unless_auto_approved() {
        let tool = HttpTool::new();

        // Empty object
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": {}
        });
        assert_eq!(
            tool.requires_approval(&params),
            ApprovalRequirement::UnlessAutoApproved
        );

        // Empty array
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": []
        });
        assert_eq!(
            tool.requires_approval(&params),
            ApprovalRequirement::UnlessAutoApproved
        );
    }

    // ── Credential registry approval tests ─────────────────────────────

    #[test]
    fn test_host_with_credential_mapping_returns_always() {
        use crate::secrets::CredentialMapping;
        use crate::tools::wasm::SharedCredentialRegistry;

        let registry = Arc::new(SharedCredentialRegistry::new());
        registry.add_mappings(vec![CredentialMapping::bearer(
            "openai_key",
            "api.openai.com",
        )]);

        let tool = HttpTool::new().with_credentials(
            registry,
            // secrets_store is not used in requires_approval, just needs to be present
            Arc::new(crate::secrets::InMemorySecretsStore::new(Arc::new(
                crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(
                    "0123456789abcdef0123456789abcdef".to_string(),
                ))
                .unwrap(),
            ))),
        );

        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.openai.com/v1/models"
        });
        assert_eq!(tool.requires_approval(&params), ApprovalRequirement::Always);
    }

    #[test]
    fn test_host_without_credential_mapping_returns_unless_auto_approved() {
        use crate::tools::wasm::SharedCredentialRegistry;

        let registry = Arc::new(SharedCredentialRegistry::new());
        // Empty registry - no credential mappings

        let tool = HttpTool::new().with_credentials(
            registry,
            Arc::new(crate::secrets::InMemorySecretsStore::new(Arc::new(
                crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(
                    "0123456789abcdef0123456789abcdef".to_string(),
                ))
                .unwrap(),
            ))),
        );

        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com/data"
        });
        assert_eq!(
            tool.requires_approval(&params),
            ApprovalRequirement::UnlessAutoApproved
        );
    }

    #[test]
    fn test_url_query_param_credential_returns_always() {
        let tool = HttpTool::new();
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com/data?api_key=secret123"
        });
        assert_eq!(tool.requires_approval(&params), ApprovalRequirement::Always);
    }

    #[test]
    fn test_bearer_value_in_custom_header_returns_always() {
        let tool = HttpTool::new();
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": {"X-Custom": "Bearer sk-test123"}
        });
        assert_eq!(tool.requires_approval(&params), ApprovalRequirement::Always);
    }

    #[test]
    fn test_extract_host_from_params_valid() {
        let params = serde_json::json!({
            "url": "https://api.example.com/path"
        });
        assert_eq!(
            extract_host_from_params(&params),
            Some("api.example.com".to_string())
        );
    }

    #[test]
    fn test_extract_host_from_params_missing_url() {
        let params = serde_json::json!({"method": "GET"});
        assert_eq!(extract_host_from_params(&params), None);
    }

    // ── URL Allowlist tests ────────────────────────────────────────────

    #[test]
    fn test_allowlist_blocks_non_listed_host() {
        let allowlist = vec!["api.openai.com".to_string()];
        let err = validate_url("https://evil.com/data", &allowlist).unwrap_err();
        assert!(err.to_string().contains("not in the URL allowlist"));
    }

    #[test]
    fn test_allowlist_allows_listed_host() {
        let allowlist = vec!["api.openai.com".to_string()];
        let url = validate_url("https://api.openai.com/v1/models", &allowlist).unwrap();
        assert_eq!(url.host_str(), Some("api.openai.com"));
    }

    #[test]
    fn test_allowlist_glob_matches_subdomain() {
        let allowlist = vec!["*.example.com".to_string()];
        let url = validate_url("https://api.example.com/path", &allowlist).unwrap();
        assert_eq!(url.host_str(), Some("api.example.com"));

        // Root domain also matches
        let url2 = validate_url("https://example.com/path", &allowlist).unwrap();
        assert_eq!(url2.host_str(), Some("example.com"));
    }

    #[test]
    fn test_allowlist_glob_rejects_different_domain() {
        let allowlist = vec!["*.openai.com".to_string()];
        let err = validate_url("https://evil-openai.com/phish", &allowlist).unwrap_err();
        assert!(err.to_string().contains("not in the URL allowlist"));
    }

    #[test]
    fn test_empty_allowlist_allows_all() {
        let url = validate_url("https://anything.com/path", &[]).unwrap();
        assert_eq!(url.host_str(), Some("anything.com"));
    }

    // ── IPv4-mapped IPv6 bypass blocking tests ─────────────────────────

    #[test]
    fn test_ipv4_mapped_v6_loopback_blocked() {
        // ::ffff:127.0.0.1 — IPv4-mapped loopback
        assert!(
            validate_url("https://[::ffff:127.0.0.1]/data", &[]).is_err(),
            "IPv4-mapped loopback should be blocked"
        );
    }

    #[test]
    fn test_ipv4_mapped_v6_private_blocked() {
        // ::ffff:192.168.1.1 — IPv4-mapped private
        assert!(
            validate_url("https://[::ffff:192.168.1.1]/data", &[]).is_err(),
            "IPv4-mapped private should be blocked"
        );
    }

    #[test]
    fn test_ipv4_mapped_v6_public_allowed() {
        // ::ffff:8.8.8.8 — IPv4-mapped public
        assert!(
            validate_url("https://[::ffff:8.8.8.8]/data", &[]).is_ok(),
            "IPv4-mapped public should be allowed"
        );
    }

    #[test]
    fn test_ipv4_mapped_v6_metadata_blocked() {
        // ::ffff:169.254.169.254 — IPv4-mapped cloud metadata
        assert!(
            validate_url("https://[::ffff:169.254.169.254]/data", &[]).is_err(),
            "IPv4-mapped cloud metadata should be blocked"
        );
    }
}
