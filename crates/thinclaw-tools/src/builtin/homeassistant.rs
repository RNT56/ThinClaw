//! Home Assistant integration tool.
//!
//! Provides smart home control via the Home Assistant REST API.
//! Requires `HASS_URL` and `HASS_TOKEN` environment variables.
//! Gated: only registered when both env vars are present.

use std::time::Duration;

use async_trait::async_trait;

use thinclaw_tools_core::{
    ApprovalRequirement, Tool, ToolApprovalClass, ToolError, ToolMetadata, ToolOutput,
    ToolRateLimitConfig, ToolRouteIntent, ToolSideEffectLevel, require_str,
};
use thinclaw_types::JobContext;

/// Home Assistant REST API client.
#[derive(Clone)]
pub struct HassClient {
    base_url: String,
    token: String,
}

const MAX_HASS_RESPONSE_BYTES: usize = 10 * 1024 * 1024;
const MAX_HASS_BASE_URL_BYTES: usize = 16 * 1024;
const MAX_HASS_TOKEN_BYTES: usize = 64 * 1024;
const MAX_HASS_DNS_ADDRESSES: usize = 64;
const MAX_HASS_IDENTIFIER_BYTES: usize = 128;
const MAX_HASS_SERVICE_DATA_BYTES: usize = 64 * 1024;
const MAX_HASS_OUTPUT_ITEMS: usize = 100;
const MAX_HASS_OUTPUT_STRING_CHARS: usize = 4096;
const MAX_HASS_OUTPUT_DEPTH: usize = 8;

impl std::fmt::Debug for HassClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HassClient")
            .field("base_url", &redacted_hass_url(&self.base_url))
            .field("token", &"[REDACTED]")
            .finish()
    }
}

impl HassClient {
    /// Create from explicit URL and token.
    pub fn new(base_url: String, token: String) -> Self {
        // Strip trailing slash
        let base_url = base_url.trim_end_matches('/').to_string();

        Self { base_url, token }
    }

    /// Try to create from environment variables.
    pub fn from_env() -> Option<Self> {
        let url = std::env::var("HASS_URL").ok()?;
        let token = std::env::var("HASS_TOKEN").ok()?;
        if url.is_empty() || token.is_empty() {
            return None;
        }
        Some(Self::new(url, token))
    }

    /// Check if HA requirements are met.
    pub fn is_available() -> bool {
        std::env::var("HASS_URL").is_ok() && std::env::var("HASS_TOKEN").is_ok()
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.token)
    }

    fn endpoint(&self, path: &str) -> Result<reqwest::Url, ToolError> {
        if self.base_url.is_empty() || self.base_url.len() > MAX_HASS_BASE_URL_BYTES {
            return Err(ToolError::InvalidParameters(
                "Home Assistant base URL is empty or oversized".to_string(),
            ));
        }
        let mut base = reqwest::Url::parse(&self.base_url).map_err(|error| {
            ToolError::InvalidParameters(format!("invalid Home Assistant base URL: {error}"))
        })?;
        if !matches!(base.scheme(), "http" | "https")
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(ToolError::NotAuthorized(
                "Home Assistant base URL must be an HTTP(S) URL without credentials, query, or fragment"
                    .to_string(),
            ));
        }
        if path.is_empty()
            || path.len() > 1024
            || path.split('/').any(|segment| {
                segment.is_empty()
                    || matches!(segment, "." | "..")
                    || !segment.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
                    })
            })
        {
            return Err(ToolError::InvalidParameters(
                "Home Assistant API path is malformed".to_string(),
            ));
        }
        {
            let mut segments = base.path_segments_mut().map_err(|_| {
                ToolError::InvalidParameters(
                    "Home Assistant URL cannot be used as an API base".to_string(),
                )
            })?;
            segments.pop_if_empty();
            segments.push("api");
            segments.extend(path.split('/'));
        }
        Ok(base)
    }

    async fn http_client(&self, endpoint: &reqwest::Url) -> Result<reqwest::Client, ToolError> {
        if self.token.trim().is_empty()
            || self.token.len() > MAX_HASS_TOKEN_BYTES
            || self.token.chars().any(char::is_control)
        {
            return Err(ToolError::NotAuthorized(
                "Home Assistant token is malformed or exceeds its size limit".to_string(),
            ));
        }
        let host = endpoint.host_str().ok_or_else(|| {
            ToolError::InvalidParameters("Home Assistant URL has no host".to_string())
        })?;
        let port = endpoint.port_or_known_default().ok_or_else(|| {
            ToolError::InvalidParameters("Home Assistant URL has no usable port".to_string())
        })?;
        let addresses = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::net::lookup_host((host, port)),
        )
        .await
        .map_err(|_| {
            ToolError::ExternalService("Home Assistant hostname resolution timed out".to_string())
        })?
        .map_err(|_| {
            ToolError::ExternalService("Home Assistant hostname resolution failed".to_string())
        })?;
        let mut addresses = addresses.collect::<Vec<_>>();
        addresses.sort_unstable();
        addresses.dedup();
        if addresses.is_empty()
            || addresses.len() > MAX_HASS_DNS_ADDRESSES
            || addresses.iter().any(|address| {
                let ip = address.ip();
                !is_usable_hass_ip(ip)
                    || endpoint.scheme() == "http" && thinclaw_tools_core::is_public_outbound_ip(ip)
            })
        {
            return Err(ToolError::NotAuthorized(
                "Home Assistant hostname resolved outside its permitted network boundary"
                    .to_string(),
            ));
        }

        reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .resolve_to_addrs(host, &addresses)
            .build()
            .map_err(|error| {
                ToolError::ExternalService(format!(
                    "Home Assistant HTTP client is unavailable: {error}"
                ))
            })
    }

    async fn response_bytes(
        response: reqwest::Response,
        limit: usize,
    ) -> Result<Vec<u8>, ToolError> {
        if response
            .content_length()
            .is_some_and(|length| usize::try_from(length).map_or(true, |length| length > limit))
        {
            return Err(ToolError::ExternalService(
                "Home Assistant response is oversized".to_string(),
            ));
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        use futures::StreamExt as _;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| {
                ToolError::ExternalService(format!(
                    "Home Assistant response failed: {}",
                    error.without_url()
                ))
            })?;
            if bytes.len().saturating_add(chunk.len()) > limit {
                return Err(ToolError::ExternalService(
                    "Home Assistant response is oversized".to_string(),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }

    async fn get(&self, path: &str) -> Result<serde_json::Value, ToolError> {
        let url = self.endpoint(path)?;
        let client = self.http_client(&url).await?;
        let response = self
            .request(&client, reqwest::Method::GET, url)
            .send()
            .await
            .map_err(|e| {
                ToolError::ExternalService(format!("HA request failed: {}", e.without_url()))
            })?;

        if !response.status().is_success() {
            return Err(ToolError::ExternalService(format!(
                "HA returned HTTP {}",
                response.status()
            )));
        }

        let body = Self::response_bytes(response, MAX_HASS_RESPONSE_BYTES).await?;
        serde_json::from_slice(&body)
            .map_err(|e| ToolError::ExternalService(format!("HA response parse error: {}", e)))
    }

    async fn post(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let url = self.endpoint(path)?;
        let client = self.http_client(&url).await?;
        let response = self
            .request(&client, reqwest::Method::POST, url)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                ToolError::ExternalService(format!("HA request failed: {}", e.without_url()))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            return Err(ToolError::ExternalService(format!(
                "HA returned HTTP {}",
                status
            )));
        }

        let body = Self::response_bytes(response, MAX_HASS_RESPONSE_BYTES).await?;
        serde_json::from_slice(&body)
            .map_err(|e| ToolError::ExternalService(format!("HA response parse error: {}", e)))
    }

    fn request(
        &self,
        client: &reqwest::Client,
        method: reqwest::Method,
        url: reqwest::Url,
    ) -> reqwest::RequestBuilder {
        client
            .request(method, url)
            .header("Authorization", self.auth_header())
            .header("Content-Type", "application/json")
    }
}

fn redacted_hass_url(value: &str) -> String {
    let Ok(url) = reqwest::Url::parse(value) else {
        return "<invalid-url>".to_string();
    };
    let Some(host) = url.host_str() else {
        return "<invalid-url>".to_string();
    };
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    }
}

fn is_usable_hass_ip(ip: std::net::IpAddr) -> bool {
    thinclaw_tools_core::is_public_outbound_ip(ip)
        || match ip {
            std::net::IpAddr::V4(ip) => ip.is_private() || ip.is_loopback(),
            std::net::IpAddr::V6(ip) => ip.is_unique_local() || ip.is_loopback(),
        }
}

fn valid_hass_identifier(value: &str, allow_dot: bool) -> bool {
    !value.is_empty()
        && value.len() <= MAX_HASS_IDENTIFIER_BYTES
        && !matches!(value, "." | "..")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_') || allow_dot && byte == b'.'
        })
        && (!allow_dot
            || value.split_once('.').is_some_and(|(domain, name)| {
                !domain.is_empty() && !name.is_empty() && !name.contains('.')
            }))
}

fn bounded_hass_label(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(512)
        .collect()
}

fn bounded_hass_json(value: serde_json::Value, depth: usize) -> serde_json::Value {
    if depth >= MAX_HASS_OUTPUT_DEPTH {
        return serde_json::Value::String("[depth limit reached]".to_string());
    }
    match value {
        serde_json::Value::String(value) => {
            serde_json::Value::String(value.chars().take(MAX_HASS_OUTPUT_STRING_CHARS).collect())
        }
        serde_json::Value::Array(values) => serde_json::Value::Array(
            values
                .into_iter()
                .take(MAX_HASS_OUTPUT_ITEMS)
                .map(|value| bounded_hass_json(value, depth + 1))
                .collect(),
        ),
        serde_json::Value::Object(values) => serde_json::Value::Object(
            values
                .into_iter()
                .take(MAX_HASS_OUTPUT_ITEMS)
                .map(|(key, value)| {
                    (
                        key.chars().take(MAX_HASS_IDENTIFIER_BYTES).collect(),
                        bounded_hass_json(value, depth + 1),
                    )
                })
                .collect(),
        ),
        value => value,
    }
}

fn validate_hass_params(params: &serde_json::Value) -> Result<(), ToolError> {
    let object = params.as_object().ok_or_else(|| {
        ToolError::InvalidParameters("Home Assistant parameters must be an object".to_string())
    })?;
    const ALLOWED: &[&str] = &["action", "entity_id", "domain", "service", "service_data"];
    if object.len() > ALLOWED.len() || object.keys().any(|key| !ALLOWED.contains(&key.as_str())) {
        return Err(ToolError::InvalidParameters(
            "Home Assistant parameters contain unsupported fields".to_string(),
        ));
    }
    if !object
        .get("action")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|action| {
            matches!(
                action,
                "list_entities" | "get_state" | "list_services" | "call_service"
            )
        })
    {
        return Err(ToolError::InvalidParameters(
            "Unknown or missing Home Assistant action".to_string(),
        ));
    }
    for (key, allow_dot) in [("entity_id", true), ("domain", false), ("service", false)] {
        if let Some(value) = object.get(key) {
            let value = value
                .as_str()
                .ok_or_else(|| ToolError::InvalidParameters(format!("{key} must be a string")))?;
            if !valid_hass_identifier(value, allow_dot) {
                return Err(ToolError::InvalidParameters(format!(
                    "{key} is malformed or exceeds its size limit"
                )));
            }
        }
    }
    if let Some(service_data) = object.get("service_data")
        && (!service_data.is_object()
            || serde_json::to_vec(service_data)
                .map_or(true, |encoded| encoded.len() > MAX_HASS_SERVICE_DATA_BYTES))
    {
        return Err(ToolError::InvalidParameters(
            "service_data must be a bounded object".to_string(),
        ));
    }
    Ok(())
}

/// Home Assistant integration tool.
pub struct HomeAssistantTool {
    client: HassClient,
}

impl HomeAssistantTool {
    pub fn new(client: HassClient) -> Self {
        Self { client }
    }

    /// Create from environment variables, if available.
    pub fn from_env() -> Option<Self> {
        HassClient::from_env().map(Self::new)
    }
}

#[async_trait]
impl Tool for HomeAssistantTool {
    fn name(&self) -> &str {
        "homeassistant"
    }

    fn description(&self) -> &str {
        "Interact with Home Assistant to inspect device state or call smart-home \
         services. Use this for live home-automation questions and actions such as \
         checking sensors, listing entities, or turning devices on and off."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list_entities", "get_state", "list_services", "call_service"],
                    "description": "The action to perform"
                },
                "entity_id": {
                    "type": "string",
                    "maxLength": MAX_HASS_IDENTIFIER_BYTES,
                    "pattern": "^[A-Za-z0-9_-]+\\.[A-Za-z0-9_-]+$",
                    "description": "Entity ID (for get_state and call_service, e.g. 'light.living_room')"
                },
                "domain": {
                    "type": "string",
                    "maxLength": MAX_HASS_IDENTIFIER_BYTES,
                    "pattern": "^[A-Za-z0-9_-]+$",
                    "description": "Filter by domain (for list_entities: 'light', 'switch', 'sensor', etc.)"
                },
                "service": {
                    "type": "string",
                    "maxLength": MAX_HASS_IDENTIFIER_BYTES,
                    "pattern": "^[A-Za-z0-9_-]+$",
                    "description": "Service to call (for call_service, e.g. 'turn_on', 'turn_off')"
                },
                "service_data": {
                    "type": "object",
                    "description": "Additional data for the service call (e.g. {\"brightness\": 255})"
                }
            },
            "required": ["action"],
            "additionalProperties": false
        })
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            authoritative_source: true,
            live_data: true,
            side_effect_level: ToolSideEffectLevel::Write,
            approval_class: ToolApprovalClass::Conditional,
            parallel_safe: false,
            route_intents: vec![ToolRouteIntent::LocalState],
        }
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        validate_hass_params(&params)?;
        let action = require_str(&params, "action")?;

        match action {
            "list_entities" => {
                let domain_filter = params.get("domain").and_then(|v| v.as_str());

                let states: Vec<serde_json::Value> =
                    serde_json::from_value(self.client.get("states").await?)
                        .map_err(|e| ToolError::ExternalService(format!("Parse error: {}", e)))?;

                let mut entities: Vec<serde_json::Value> = states
                    .iter()
                    .filter(|s| {
                        if let Some(domain) = domain_filter {
                            s.get("entity_id")
                                .and_then(|id| id.as_str())
                                .is_some_and(|id| id.starts_with(&format!("{}.", domain)))
                        } else {
                            true
                        }
                    })
                    .map(|s| {
                        let entity_id = bounded_hass_label(s["entity_id"].as_str().unwrap_or(""));
                        let state = bounded_hass_label(s["state"].as_str().unwrap_or("unknown"));
                        let friendly_name = s
                            .get("attributes")
                            .and_then(|a| a.get("friendly_name"))
                            .and_then(|n| n.as_str())
                            .map(bounded_hass_label)
                            .unwrap_or_else(|| entity_id.clone());

                        serde_json::json!({
                            "entity_id": entity_id,
                            "state": state,
                            "name": friendly_name,
                        })
                    })
                    .collect();

                // Truncate for LLM context budget
                let total = entities.len();
                let truncated = entities.len() > MAX_HASS_OUTPUT_ITEMS;
                entities.truncate(MAX_HASS_OUTPUT_ITEMS);

                Ok(ToolOutput::success(
                    serde_json::json!({
                        "entities": entities,
                        "total": total,
                        "truncated": truncated,
                        "domain_filter": domain_filter,
                    }),
                    start.elapsed(),
                ))
            }

            "get_state" => {
                let entity_id = require_str(&params, "entity_id")?;

                let state = self.client.get(&format!("states/{}", entity_id)).await?;

                Ok(ToolOutput::success(
                    bounded_hass_json(state, 0),
                    start.elapsed(),
                ))
            }

            "list_services" => {
                let domain_filter = params.get("domain").and_then(|v| v.as_str());

                let services = self.client.get("services").await?;

                let filtered = if let Some(domain) = domain_filter {
                    if let Some(arr) = services.as_array() {
                        let matching: Vec<&serde_json::Value> = arr
                            .iter()
                            .filter(|s| {
                                s.get("domain")
                                    .and_then(|d| d.as_str())
                                    .is_some_and(|d| d == domain)
                            })
                            .collect();
                        bounded_hass_json(serde_json::json!(matching), 0)
                    } else {
                        bounded_hass_json(services, 0)
                    }
                } else {
                    bounded_hass_json(services, 0)
                };

                Ok(ToolOutput::success(
                    serde_json::json!({
                        "services": filtered,
                        "domain_filter": domain_filter,
                    }),
                    start.elapsed(),
                ))
            }

            "call_service" => {
                let entity_id = require_str(&params, "entity_id")?;
                let service = require_str(&params, "service")?;

                // Extract domain from entity_id (e.g., "light.living_room" → "light")
                let domain = entity_id.split('.').next().ok_or_else(|| {
                    ToolError::InvalidParameters(format!(
                        "Invalid entity_id format: '{}' (expected 'domain.name')",
                        entity_id
                    ))
                })?;

                let mut service_data = params
                    .get("service_data")
                    .cloned()
                    .unwrap_or(serde_json::json!({}));

                // Ensure entity_id is in the service data
                if let Some(obj) = service_data.as_object_mut() {
                    obj.insert("entity_id".to_string(), serde_json::json!(entity_id));
                }

                let result = self
                    .client
                    .post(&format!("services/{}/{}", domain, service), service_data)
                    .await?;

                Ok(ToolOutput::success(
                    serde_json::json!({
                        "success": true,
                        "entity_id": entity_id,
                        "service": format!("{}.{}", domain, service),
                        "result": bounded_hass_json(result, 0),
                    }),
                    start.elapsed(),
                ))
            }

            _ => Err(ToolError::InvalidParameters(
                "Unknown Home Assistant action".to_string(),
            )),
        }
    }

    fn requires_approval(&self, params: &serde_json::Value) -> ApprovalRequirement {
        match params.get("action").and_then(|v| v.as_str()) {
            Some("call_service") => ApprovalRequirement::UnlessAutoApproved,
            _ => ApprovalRequirement::Never,
        }
    }

    fn execution_timeout(&self) -> Duration {
        Duration::from_secs(30)
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        Some(ToolRateLimitConfig::new(20, 200))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use thinclaw_config::helpers::lock_env;

    #[test]
    fn test_hass_client_url_normalization() {
        let client = HassClient::new(
            "http://homeassistant.local:8123/".to_string(),
            "token".to_string(),
        );
        assert_eq!(client.base_url, "http://homeassistant.local:8123");
    }

    #[test]
    fn test_hass_client_auth_header() {
        let client = HassClient::new("http://ha.local".to_string(), "my_token".to_string());
        assert_eq!(client.auth_header(), "Bearer my_token");
        assert!(!format!("{client:?}").contains("my_token"));
    }

    #[test]
    fn hass_endpoint_rejects_path_traversal() {
        let client = HassClient::new("http://ha.local".to_string(), "token".to_string());
        assert!(client.endpoint("states/light.kitchen").is_ok());
        assert!(client.endpoint("states/../../config").is_err());
        assert!(
            client
                .endpoint("states/light.kitchen?token=secret")
                .is_err()
        );
    }

    #[test]
    fn hass_params_reject_malformed_identifiers_and_large_service_data() {
        assert!(
            validate_hass_params(&serde_json::json!({
                "action": "get_state",
                "entity_id": "../../config",
            }))
            .is_err()
        );
        assert!(
            validate_hass_params(&serde_json::json!({
                "action": "call_service",
                "entity_id": "light.kitchen",
                "service": "turn_on",
                "service_data": {"payload": "x".repeat(MAX_HASS_SERVICE_DATA_BYTES)},
            }))
            .is_err()
        );
    }

    #[test]
    fn test_approval_read_actions() {
        let client = HassClient::new("http://ha.local".to_string(), "token".to_string());
        let tool = HomeAssistantTool::new(client);

        assert_eq!(
            tool.requires_approval(&serde_json::json!({"action": "list_entities"})),
            ApprovalRequirement::Never
        );
        assert_eq!(
            tool.requires_approval(&serde_json::json!({"action": "get_state"})),
            ApprovalRequirement::Never
        );
    }

    #[test]
    fn test_approval_write_actions() {
        let client = HassClient::new("http://ha.local".to_string(), "token".to_string());
        let tool = HomeAssistantTool::new(client);

        assert_eq!(
            tool.requires_approval(&serde_json::json!({"action": "call_service"})),
            ApprovalRequirement::UnlessAutoApproved
        );
    }

    #[test]
    fn test_from_env_missing() {
        let _env_guard = lock_env();
        // Clear env vars to ensure from_env returns None
        unsafe {
            std::env::remove_var("HASS_URL");
            std::env::remove_var("HASS_TOKEN");
        }
        assert!(HomeAssistantTool::from_env().is_none());
    }
}
