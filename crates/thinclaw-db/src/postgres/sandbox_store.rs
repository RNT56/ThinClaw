//! postgres: sandbox_store.

use super::*;

#[async_trait]
impl SandboxStore for PgBackend {
    async fn save_sandbox_job(&self, job: &SandboxJobRecord) -> Result<(), DatabaseError> {
        self.store.save_sandbox_job(job).await
    }

    async fn get_sandbox_job(&self, id: Uuid) -> Result<Option<SandboxJobRecord>, DatabaseError> {
        self.store.get_sandbox_job(id).await
    }

    async fn list_sandbox_jobs(&self) -> Result<Vec<SandboxJobRecord>, DatabaseError> {
        self.store.list_sandbox_jobs().await
    }

    async fn update_sandbox_job_status(
        &self,
        id: Uuid,
        status: &str,
        success: Option<bool>,
        message: Option<&str>,
        started_at: Option<DateTime<Utc>>,
        completed_at: Option<DateTime<Utc>>,
    ) -> Result<(), DatabaseError> {
        self.store
            .update_sandbox_job_status(id, status, success, message, started_at, completed_at)
            .await
    }

    async fn cleanup_stale_sandbox_jobs(&self) -> Result<u64, DatabaseError> {
        self.store.cleanup_stale_sandbox_jobs().await
    }

    async fn sandbox_job_summary(&self) -> Result<SandboxJobSummary, DatabaseError> {
        self.store.sandbox_job_summary().await
    }

    async fn list_sandbox_jobs_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<SandboxJobRecord>, DatabaseError> {
        self.store.list_sandbox_jobs_for_user(user_id).await
    }

    async fn sandbox_job_summary_for_user(
        &self,
        user_id: &str,
    ) -> Result<SandboxJobSummary, DatabaseError> {
        self.store.sandbox_job_summary_for_user(user_id).await
    }

    async fn sandbox_job_belongs_to_user(
        &self,
        job_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError> {
        self.store
            .sandbox_job_belongs_to_user(job_id, user_id)
            .await
    }

    async fn update_sandbox_job_mode(&self, id: Uuid, mode: &str) -> Result<(), DatabaseError> {
        self.store.update_sandbox_job_mode(id, mode).await
    }

    async fn get_sandbox_job_mode(&self, id: Uuid) -> Result<Option<String>, DatabaseError> {
        self.store.get_sandbox_job_mode(id).await
    }

    async fn save_job_event(
        &self,
        job_id: Uuid,
        event_type: &str,
        data: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.store.save_job_event(job_id, event_type, data).await
    }

    async fn list_job_events(
        &self,
        job_id: Uuid,
        limit: Option<i64>,
    ) -> Result<Vec<JobEventRecord>, DatabaseError> {
        self.store.list_job_events(job_id, limit).await
    }
}

// ==================== RoutineStore ====================
