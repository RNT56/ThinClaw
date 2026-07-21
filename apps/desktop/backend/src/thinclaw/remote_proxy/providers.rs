//! Channel-pairing, skills, provider/routing config, cost, cache, and log
//! proxy methods.

use super::core::RemoteGatewayProxy;

impl RemoteGatewayProxy {
    // ── Channels / Pairing ─────────────────────────────────────────────────

    /// List pending and approved channel pairings.
    pub async fn list_pairings(
        &self,
        channel: &str,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.get_json(&format!("/api/pairing/{}", urlencoding::encode(channel)))
            .await
    }

    /// Approve a channel pairing code.
    pub async fn approve_pairing(
        &self,
        channel: &str,
        code: &str,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.post_json(
            &format!("/api/pairing/{}/approve", urlencoding::encode(channel)),
            &serde_json::json!({ "code": code }),
        )
        .await
    }

    // ── Skills ───────────────────────────────────────────────────────────────

    /// List all installed skills.
    pub async fn list_skills(
        &self,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.get_json("/api/skills").await
    }

    // ── Providers / Routing ─────────────────────────────────────────────────

    /// Get remote provider and routing configuration.
    pub async fn get_providers_config(
        &self,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.get_json("/api/providers/config").await
    }

    /// Replace remote provider and routing configuration.
    pub async fn set_providers_config(
        &self,
        config: &serde_json::Value,
    ) -> Result<(), crate::thinclaw::bridge::BridgeError> {
        self.put_json("/api/providers/config", config)
            .await
            .map(|_| ())
    }

    /// Get remote model options for one provider.
    pub async fn get_provider_models(
        &self,
        slug: &str,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.get_json(&format!(
            "/api/providers/{}/models",
            urlencoding::encode(slug)
        ))
        .await
    }

    /// List remote providers with sanitized credential status only.
    pub async fn list_provider_status(
        &self,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.get_json("/api/providers").await
    }

    /// Simulate a remote route decision through ThinClaw's provider planner.
    pub async fn simulate_route(
        &self,
        request: &serde_json::Value,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.post_json("/api/providers/route/simulate", request)
            .await
    }

    /// Save a remote provider API key through the provider vault endpoint.
    pub async fn save_provider_key(
        &self,
        slug: &str,
        api_key: &str,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.post_json(
            &format!("/api/providers/{}/key", urlencoding::encode(slug)),
            &serde_json::json!({ "api_key": api_key }),
        )
        .await
    }

    /// Delete a remote provider API key through the provider vault endpoint.
    pub async fn delete_provider_key(
        &self,
        slug: &str,
    ) -> Result<(), crate::thinclaw::bridge::BridgeError> {
        self.delete_json(&format!("/api/providers/{}/key", urlencoding::encode(slug)))
            .await
            .map(|_| ())
    }

    // ── Costs ────────────────────────────────────────────────────────────────

    /// Get remote LLM cost summary.
    pub async fn get_cost_summary(
        &self,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.get_json("/api/costs/summary").await
    }

    /// Export remote cost data as CSV.
    pub async fn export_cost_csv(&self) -> Result<String, crate::thinclaw::bridge::BridgeError> {
        self.get_text("/api/costs/export").await
    }

    /// Reset remote cost tracking data.
    pub async fn reset_costs(&self) -> Result<(), crate::thinclaw::bridge::BridgeError> {
        self.post_json("/api/costs/reset", &serde_json::json!({}))
            .await
            .map(|_| ())
    }

    pub async fn cache_stats(
        &self,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.get_json("/api/cache/stats").await
    }

    pub async fn logs_recent(
        &self,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.get_json("/api/logs/recent").await
    }
}
