//! Extensions, hooks, tools, settings/config, diagnostics, and the
//! intentionally-disabled raw secret injection.

use super::core::RemoteGatewayProxy;

impl RemoteGatewayProxy {
    // ── Extensions ───────────────────────────────────────────────────────────

    /// List all extensions.
    pub async fn list_extensions(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/extensions").await
    }

    pub async fn activate_extension(&self, name: &str) -> Result<serde_json::Value, String> {
        self.post_json(
            &format!("/api/extensions/{}/activate", urlencoding::encode(name)),
            &serde_json::json!({}),
        )
        .await
    }

    pub async fn remove_extension(&self, name: &str) -> Result<serde_json::Value, String> {
        self.post_json(
            &format!("/api/extensions/{}/remove", urlencoding::encode(name)),
            &serde_json::json!({}),
        )
        .await
    }

    pub async fn list_hooks(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/hooks").await
    }

    pub async fn register_hooks(
        &self,
        bundle_json: &str,
        source: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        self.post_json(
            "/api/hooks",
            &serde_json::json!({ "bundle_json": bundle_json, "source": source }),
        )
        .await
    }

    pub async fn unregister_hook(&self, name: &str) -> Result<serde_json::Value, String> {
        self.delete_json(&format!("/api/hooks/{}", urlencoding::encode(name)))
            .await
    }

    pub async fn list_tools(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/extensions/tools").await
    }

    // ── Settings / Config ────────────────────────────────────────────────────

    /// Get a config setting from the remote agent.
    pub async fn get_setting(&self, key: &str) -> Result<serde_json::Value, String> {
        self.get_json(&format!("/api/settings/{}", urlencoding::encode(key)))
            .await
    }

    /// List all non-sensitive config settings from the remote agent.
    pub async fn list_settings(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/settings").await
    }

    /// Set a config setting on the remote agent.
    pub async fn set_setting(&self, key: &str, value: &serde_json::Value) -> Result<(), String> {
        let url = format!("/api/settings/{}", urlencoding::encode(key));
        let body = serde_json::json!({ "value": value });
        self.put_json(&url, &body).await.map(|_| ())
    }

    /// Legacy raw-secret injection is intentionally unavailable in remote mode.
    ///
    /// Remote credentials must move through the Provider Vault save/delete
    /// endpoints so the gateway stores them in its own secrets backend and only
    /// returns sanitized status metadata to Desktop.
    pub async fn inject_secrets(
        &self,
        _secrets: &std::collections::HashMap<String, String>,
    ) -> Result<u32, String> {
        Err(
            "unavailable: remote raw secret injection is disabled; use provider vault save/delete"
                .to_string(),
        )
    }

    // ── Diagnostics / Logs ───────────────────────────────────────────────────

    /// Get full diagnostics from the remote gateway.
    pub async fn get_diagnostics(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/gateway/status").await
    }
}
