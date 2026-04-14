//! Home Assistant integration tool.
//!
//! Provides smart home control via the Home Assistant REST API.
//! Requires `HASS_URL` and `HASS_TOKEN` environment variables.
//! Gated: only registered when both env vars are present.

use std::time::Duration;

use async_trait::async_trait;

use crate::context::JobContext;
use crate::tools::tool::{
    ApprovalRequirement, Tool, ToolError, ToolOutput, ToolRateLimitConfig, require_str,
};

/// Home Assistant REST API client.
#[derive(Debug, Clone)]
pub struct HassClient {
    base_url: String,
    token: String,
    client: reqwest::Client,
}

impl HassClient {
    /// Create from explicit URL and token.
    pub fn new(base_url: String, token: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_default();

        // Strip trailing slash
        let base_url = base_url.trim_end_matches('/').to_string();

        Self {
            base_url,
            token,
            client,
        }
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

    async fn get(&self, path: &str) -> Result<serde_json::Value, ToolError> {
        let url = format!("{}/api/{}", self.base_url, path);
        let response = self
            .client
            .get(&url)
            .header("Authorization", self.auth_header())
            .header("Content-Type", "application/json")
            .send()
            .await
            .map_err(|e| ToolError::ExternalService(format!("HA request failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(ToolError::ExternalService(format!(
                "HA returned HTTP {}",
                response.status()
            )));
        }

        response
            .json()
            .await
            .map_err(|e| ToolError::ExternalService(format!("HA response parse error: {}", e)))
    }

    async fn post(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let url = format!("{}/api/{}", self.base_url, path);
        let response = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .json(&body)
            .send()
            .await
            .map_err(|e| ToolError::ExternalService(format!("HA request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body_text = response.text().await.unwrap_or_default();
            return Err(ToolError::ExternalService(format!(
                "HA returned HTTP {}: {}",
                status,
                body_text.chars().take(200).collect::<String>()
            )));
        }

        response
            .json()
            .await
            .map_err(|e| ToolError::ExternalService(format!("HA response parse error: {}", e)))
    }
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
        "Control your Home Assistant smart home. Actions: \
         'list_entities' (list devices, optionally filter by domain), \
         'get_state' (get detailed state of an entity), \
         'list_services' (list available services by domain), \
         'call_service' (invoke a service like turning on lights)."
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
                    "description": "Entity ID (for get_state and call_service, e.g. 'light.living_room')"
                },
                "domain": {
                    "type": "string",
                    "description": "Filter by domain (for list_entities: 'light', 'switch', 'sensor', etc.)"
                },
                "service": {
                    "type": "string",
                    "description": "Service to call (for call_service, e.g. 'turn_on', 'turn_off')"
                },
                "service_data": {
                    "type": "object",
                    "description": "Additional data for the service call (e.g. {\"brightness\": 255})"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
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
                        let entity_id = s["entity_id"].as_str().unwrap_or("");
                        let state = s["state"].as_str().unwrap_or("unknown");
                        let friendly_name = s
                            .get("attributes")
                            .and_then(|a| a.get("friendly_name"))
                            .and_then(|n| n.as_str())
                            .unwrap_or(entity_id);

                        serde_json::json!({
                            "entity_id": entity_id,
                            "state": state,
                            "name": friendly_name,
                        })
                    })
                    .collect();

                // Truncate for LLM context budget
                let truncated = entities.len() > 100;
                entities.truncate(100);

                Ok(ToolOutput::success(
                    serde_json::json!({
                        "entities": entities,
                        "total": entities.len(),
                        "truncated": truncated,
                        "domain_filter": domain_filter,
                    }),
                    start.elapsed(),
                ))
            }

            "get_state" => {
                let entity_id = require_str(&params, "entity_id")?;

                let state = self.client.get(&format!("states/{}", entity_id)).await?;

                Ok(ToolOutput::success(state, start.elapsed()))
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
                        serde_json::json!(matching)
                    } else {
                        services
                    }
                } else {
                    services
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
                        "result": result,
                    }),
                    start.elapsed(),
                ))
            }

            _ => Err(ToolError::InvalidParameters(format!(
                "Unknown action: '{}'. Use: list_entities, get_state, list_services, call_service",
                action
            ))),
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
    use crate::config::helpers::lock_env;

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
