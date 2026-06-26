//! Routine proxy methods: list, trigger, history, create, toggle, delete, and
//! run-history clearing.

use super::core::RemoteGatewayProxy;

impl RemoteGatewayProxy {
    /// List all routines.
    pub async fn list_routines(&self) -> Result<serde_json::Value, String> {
        self.get_json("/api/routines").await
    }

    /// Trigger a routine manually.
    pub async fn trigger_routine(&self, routine_id: &str) -> Result<serde_json::Value, String> {
        self.post_json(
            &format!("/api/routines/{}/trigger", urlencoding::encode(routine_id)),
            &serde_json::json!({}),
        )
        .await
    }

    /// Get routine run history.
    pub async fn get_routine_history(
        &self,
        routine_id: &str,
        limit: u32,
    ) -> Result<serde_json::Value, String> {
        self.get_json(&format!(
            "/api/routines/{}/runs?limit={}",
            urlencoding::encode(routine_id),
            limit
        ))
        .await
    }

    /// Create a new routine.
    pub async fn create_routine(
        &self,
        name: &str,
        description: &str,
        schedule: &str,
        task: &str,
    ) -> Result<serde_json::Value, String> {
        self.post_json(
            "/api/routines",
            &serde_json::json!({
                "name": name,
                "description": description,
                "schedule": schedule,
                "task": task,
            }),
        )
        .await
    }

    /// Toggle a routine enabled/disabled.
    pub async fn toggle_routine(
        &self,
        routine_id: &str,
        enabled: bool,
    ) -> Result<serde_json::Value, String> {
        self.post_json(
            &format!("/api/routines/{}/toggle", urlencoding::encode(routine_id)),
            &serde_json::json!({ "enabled": enabled }),
        )
        .await
    }

    /// Delete a routine.
    pub async fn delete_routine(&self, routine_id: &str) -> Result<serde_json::Value, String> {
        self.delete_json(&format!(
            "/api/routines/{}",
            urlencoding::encode(routine_id)
        ))
        .await?;
        Ok(serde_json::json!({ "ok": true, "deleted_id": routine_id }))
    }

    /// Clear routine run history. If `routine_id` is absent, clears runs for
    /// all routines visible to the authenticated remote principal.
    pub async fn clear_routine_runs(
        &self,
        routine_id: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        self.delete_json_body(
            "/api/routines/runs",
            &serde_json::json!({ "routine_id": routine_id }),
        )
        .await
    }
}
