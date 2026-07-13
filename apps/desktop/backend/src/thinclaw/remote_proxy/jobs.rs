//! Jobs, autonomy, learning, experiments, and MCP proxy methods.

use super::core::RemoteGatewayProxy;

impl RemoteGatewayProxy {
    pub async fn get_jobs(
        &self,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.get_json("/api/jobs").await
    }

    pub async fn get_jobs_summary(
        &self,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.get_json("/api/jobs/summary").await
    }

    pub async fn get_job_detail(
        &self,
        job_id: &str,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.get_json(&format!("/api/jobs/{}", urlencoding::encode(job_id)))
            .await
    }

    pub async fn cancel_job(
        &self,
        job_id: &str,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.post_json(
            &format!("/api/jobs/{}/cancel", urlencoding::encode(job_id)),
            &serde_json::json!({}),
        )
        .await
    }

    pub async fn restart_job(
        &self,
        job_id: &str,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.post_json(
            &format!("/api/jobs/{}/restart", urlencoding::encode(job_id)),
            &serde_json::json!({}),
        )
        .await
    }

    pub async fn prompt_job(
        &self,
        job_id: &str,
        content: Option<String>,
        done: bool,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        let mut body = serde_json::Map::new();
        if let Some(content) = content {
            body.insert("content".to_string(), serde_json::Value::String(content));
        }
        body.insert("done".to_string(), serde_json::Value::Bool(done));
        self.post_json(
            &format!("/api/jobs/{}/prompt", urlencoding::encode(job_id)),
            &serde_json::Value::Object(body),
        )
        .await
    }

    pub async fn get_job_events(
        &self,
        job_id: &str,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.get_json(&format!("/api/jobs/{}/events", urlencoding::encode(job_id)))
            .await
    }

    pub async fn list_job_files(
        &self,
        job_id: &str,
        path: Option<&str>,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        let suffix = path
            .filter(|p| !p.is_empty())
            .map(|p| format!("?path={}", urlencoding::encode(p)))
            .unwrap_or_default();
        self.get_json(&format!(
            "/api/jobs/{}/files/list{}",
            urlencoding::encode(job_id),
            suffix
        ))
        .await
    }

    pub async fn read_job_file(
        &self,
        job_id: &str,
        path: &str,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.get_json(&format!(
            "/api/jobs/{}/files/read?path={}",
            urlencoding::encode(job_id),
            urlencoding::encode(path)
        ))
        .await
    }

    pub async fn get_autonomy_status(
        &self,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.get_json("/api/autonomy/status").await
    }

    pub async fn bootstrap_autonomy(
        &self,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.post_json("/api/autonomy/bootstrap", &serde_json::json!({}))
            .await
    }

    pub async fn pause_autonomy(
        &self,
        reason: Option<String>,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.post_json(
            "/api/autonomy/pause",
            &serde_json::json!({ "reason": reason }),
        )
        .await
    }

    pub async fn resume_autonomy(
        &self,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.post_json("/api/autonomy/resume", &serde_json::json!({}))
            .await
    }

    pub async fn get_autonomy_permissions(
        &self,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.get_json("/api/autonomy/permissions").await
    }

    pub async fn rollback_autonomy(
        &self,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.post_json("/api/autonomy/rollback", &serde_json::json!({}))
            .await
    }

    pub async fn get_autonomy_rollouts(
        &self,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.get_json("/api/autonomy/rollouts").await
    }

    pub async fn get_autonomy_checks(
        &self,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.get_json("/api/autonomy/checks").await
    }

    pub async fn get_autonomy_evidence(
        &self,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.get_json("/api/autonomy/evidence").await
    }

    pub async fn get_learning_status(
        &self,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.get_json("/api/learning/status").await
    }

    pub async fn get_experiment_projects(
        &self,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.get_json("/api/experiments/projects").await
    }

    pub async fn get_mcp_servers(
        &self,
    ) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
        self.get_json("/api/mcp/servers").await
    }
}
