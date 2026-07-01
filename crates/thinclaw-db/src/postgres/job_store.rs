//! postgres: job_store.

use super::*;

#[async_trait]
impl JobStore for PgBackend {
    async fn save_job(&self, ctx: &JobContext) -> Result<(), DatabaseError> {
        self.store.save_job(ctx).await
    }

    async fn get_job(&self, id: Uuid) -> Result<Option<JobContext>, DatabaseError> {
        self.store.get_job(id).await
    }

    async fn list_jobs_for_user(&self, user_id: &str) -> Result<Vec<JobContext>, DatabaseError> {
        self.store.list_jobs_for_user(user_id).await
    }

    async fn update_job_status(
        &self,
        id: Uuid,
        status: JobState,
        failure_reason: Option<&str>,
    ) -> Result<(), DatabaseError> {
        self.store
            .update_job_status(id, status, failure_reason)
            .await
    }

    async fn abandon_active_direct_jobs(&self, reason: &str) -> Result<u64, DatabaseError> {
        self.store.abandon_active_direct_jobs(reason).await
    }

    async fn mark_job_stuck(&self, id: Uuid) -> Result<(), DatabaseError> {
        self.store.mark_job_stuck(id).await
    }

    async fn get_stuck_jobs(&self) -> Result<Vec<Uuid>, DatabaseError> {
        self.store.get_stuck_jobs().await
    }

    async fn save_action(&self, job_id: Uuid, action: &ActionRecord) -> Result<(), DatabaseError> {
        self.store.save_action(job_id, action).await
    }

    async fn get_job_actions(&self, job_id: Uuid) -> Result<Vec<ActionRecord>, DatabaseError> {
        self.store.get_job_actions(job_id).await
    }

    async fn record_llm_call(&self, record: &LlmCallRecord<'_>) -> Result<Uuid, DatabaseError> {
        self.store.record_llm_call(record).await
    }

    async fn save_estimation_snapshot(
        &self,
        job_id: Uuid,
        category: &str,
        tool_names: &[String],
        estimated_cost: Decimal,
        estimated_time_secs: i32,
        estimated_value: Decimal,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .save_estimation_snapshot(
                job_id,
                category,
                tool_names,
                estimated_cost,
                estimated_time_secs,
                estimated_value,
            )
            .await
    }

    async fn update_estimation_actuals(
        &self,
        id: Uuid,
        actual_cost: Decimal,
        actual_time_secs: i32,
        actual_value: Option<Decimal>,
    ) -> Result<(), DatabaseError> {
        self.store
            .update_estimation_actuals(id, actual_cost, actual_time_secs, actual_value)
            .await
    }
}

// ==================== SandboxStore ====================
