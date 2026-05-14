use super::*;
pub use thinclaw_types::sandbox::{SandboxJobRecord, SandboxJobSummary};

// ==================== Sandbox Jobs ====================

#[cfg(feature = "postgres")]
impl Store {
    /// Insert a new sandbox job into `agent_jobs`.
    pub async fn save_sandbox_job(&self, job: &SandboxJobRecord) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            INSERT INTO agent_jobs (
                id, title, description, status, source, user_id, principal_id, actor_id,
                project_dir, job_mode, metadata, success, failure_reason,
                created_at, started_at, completed_at, credential_grants
            ) VALUES ($1, $2, $3, $4, 'sandbox', $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
            ON CONFLICT (id) DO UPDATE SET
                title = EXCLUDED.title,
                description = EXCLUDED.description,
                status = EXCLUDED.status,
                principal_id = EXCLUDED.principal_id,
                success = EXCLUDED.success,
                failure_reason = EXCLUDED.failure_reason,
                actor_id = EXCLUDED.actor_id,
                project_dir = EXCLUDED.project_dir,
                job_mode = EXCLUDED.job_mode,
                metadata = EXCLUDED.metadata,
                started_at = EXCLUDED.started_at,
                completed_at = EXCLUDED.completed_at,
                credential_grants = EXCLUDED.credential_grants
            "#,
            &[
                &job.id,
                &job.spec.title,
                &job.spec.description,
                &job.status,
                &job.spec.principal_id,
                &job.spec.principal_id,
                &job.spec.actor_id,
                &job.spec.project_dir,
                &job.spec.mode.as_str(),
                &job.spec.persisted_metadata(),
                &job.success,
                &job.failure_reason,
                &job.created_at,
                &job.started_at,
                &job.completed_at,
                &job.credential_grants_json,
            ],
        )
        .await?;
        Ok(())
    }

    /// Get a sandbox job by ID.
    pub async fn get_sandbox_job(
        &self,
        id: Uuid,
    ) -> Result<Option<SandboxJobRecord>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt(
                r#"
                SELECT id, title, description, status, user_id, principal_id, actor_id, project_dir,
                       job_mode, metadata, success, failure_reason, created_at, started_at,
                       completed_at, COALESCE(credential_grants, '[]') AS credential_grants
                FROM agent_jobs WHERE id = $1 AND source = 'sandbox'
                "#,
                &[&id],
            )
            .await?;

        Ok(row.map(|r| SandboxJobRecord {
            id: r.get("id"),
            spec: SandboxJobSpec::from_persisted(
                r.get::<_, String>("title"),
                r.get::<_, String>("description"),
                r.get::<_, String>("principal_id"),
                r.get::<_, String>("actor_id"),
                r.get::<_, Option<String>>("project_dir"),
                match r.get::<_, String>("job_mode").as_str() {
                    "claude_code" => crate::sandbox_types::JobMode::ClaudeCode,
                    "codex_code" => crate::sandbox_types::JobMode::CodexCode,
                    _ => crate::sandbox_types::JobMode::Worker,
                },
                r.get::<_, serde_json::Value>("metadata"),
            ),
            status: r.get("status"),
            success: r.get("success"),
            failure_reason: r.get("failure_reason"),
            created_at: r.get("created_at"),
            started_at: r.get("started_at"),
            completed_at: r.get("completed_at"),
            credential_grants_json: r.get::<_, String>("credential_grants"),
        }))
    }

    /// List all sandbox jobs, most recent first.
    pub async fn list_sandbox_jobs(&self) -> Result<Vec<SandboxJobRecord>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT id, title, description, status, user_id, principal_id, actor_id, project_dir,
                       job_mode, metadata, success, failure_reason, created_at, started_at,
                       completed_at, COALESCE(credential_grants, '[]') AS credential_grants
                FROM agent_jobs WHERE source = 'sandbox'
                ORDER BY created_at DESC
                "#,
                &[],
            )
            .await?;

        Ok(rows
            .iter()
            .map(|r| SandboxJobRecord {
                id: r.get("id"),
                spec: SandboxJobSpec::from_persisted(
                    r.get::<_, String>("title"),
                    r.get::<_, String>("description"),
                    r.get::<_, String>("principal_id"),
                    r.get::<_, String>("actor_id"),
                    r.get::<_, Option<String>>("project_dir"),
                    match r.get::<_, String>("job_mode").as_str() {
                        "claude_code" => crate::sandbox_types::JobMode::ClaudeCode,
                        "codex_code" => crate::sandbox_types::JobMode::CodexCode,
                        _ => crate::sandbox_types::JobMode::Worker,
                    },
                    r.get::<_, serde_json::Value>("metadata"),
                ),
                status: r.get("status"),
                success: r.get("success"),
                failure_reason: r.get("failure_reason"),
                created_at: r.get("created_at"),
                started_at: r.get("started_at"),
                completed_at: r.get("completed_at"),
                credential_grants_json: r.get::<_, String>("credential_grants"),
            })
            .collect())
    }

    /// List sandbox jobs for a specific user, most recent first.
    pub async fn list_sandbox_jobs_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<SandboxJobRecord>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT id, title, description, status, user_id, principal_id, actor_id, project_dir,
                       job_mode, metadata, success, failure_reason, created_at, started_at,
                       completed_at, COALESCE(credential_grants, '[]') AS credential_grants
                FROM agent_jobs WHERE source = 'sandbox' AND user_id = $1
                ORDER BY created_at DESC
                "#,
                &[&user_id],
            )
            .await?;

        Ok(rows
            .iter()
            .map(|r| SandboxJobRecord {
                id: r.get("id"),
                spec: SandboxJobSpec::from_persisted(
                    r.get::<_, String>("title"),
                    r.get::<_, String>("description"),
                    r.get::<_, String>("principal_id"),
                    r.get::<_, String>("actor_id"),
                    r.get::<_, Option<String>>("project_dir"),
                    match r.get::<_, String>("job_mode").as_str() {
                        "claude_code" => crate::sandbox_types::JobMode::ClaudeCode,
                        "codex_code" => crate::sandbox_types::JobMode::CodexCode,
                        _ => crate::sandbox_types::JobMode::Worker,
                    },
                    r.get::<_, serde_json::Value>("metadata"),
                ),
                status: r.get("status"),
                success: r.get("success"),
                failure_reason: r.get("failure_reason"),
                created_at: r.get("created_at"),
                started_at: r.get("started_at"),
                completed_at: r.get("completed_at"),
                credential_grants_json: r.get::<_, String>("credential_grants"),
            })
            .collect())
    }

    /// Get a summary of sandbox job counts by status for a specific user.
    pub async fn sandbox_job_summary_for_user(
        &self,
        user_id: &str,
    ) -> Result<SandboxJobSummary, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                "SELECT status, COUNT(*) as cnt FROM agent_jobs WHERE source = 'sandbox' AND user_id = $1 GROUP BY status",
                &[&user_id],
            )
            .await?;

        let mut summary = SandboxJobSummary::default();
        for row in &rows {
            let status: String = row.get("status");
            let count: i64 = row.get("cnt");
            let c = count as usize;
            summary.total += c;
            match status.as_str() {
                "creating" => summary.creating += c,
                "running" => summary.running += c,
                "completed" => summary.completed += c,
                "failed" => summary.failed += c,
                "cancelled" => summary.cancelled += c,
                "interrupted" => summary.interrupted += c,
                "stuck" => summary.stuck += c,
                _ => {}
            }
        }
        Ok(summary)
    }

    /// Check if a sandbox job belongs to a specific user.
    pub async fn sandbox_job_belongs_to_user(
        &self,
        job_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt(
                "SELECT 1 FROM agent_jobs WHERE id = $1 AND user_id = $2 AND source = 'sandbox'",
                &[&job_id, &user_id],
            )
            .await?;
        Ok(row.is_some())
    }

    /// Update sandbox job status and optional timestamps/result.
    pub async fn update_sandbox_job_status(
        &self,
        id: Uuid,
        status: &str,
        success: Option<bool>,
        message: Option<&str>,
        started_at: Option<DateTime<Utc>>,
        completed_at: Option<DateTime<Utc>>,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            UPDATE agent_jobs SET
                status = $2,
                success = COALESCE($3, success),
                failure_reason = COALESCE($4, failure_reason),
                started_at = COALESCE($5, started_at),
                completed_at = COALESCE($6, completed_at)
            WHERE id = $1 AND source = 'sandbox'
            "#,
            &[&id, &status, &success, &message, &started_at, &completed_at],
        )
        .await?;
        Ok(())
    }

    /// Mark any sandbox jobs left in "running" or "creating" as "interrupted".
    ///
    /// Called on startup to handle jobs that were running when the process died.
    pub async fn cleanup_stale_sandbox_jobs(&self) -> Result<u64, DatabaseError> {
        let conn = self.conn().await?;
        let count = conn
            .execute(
                r#"
                UPDATE agent_jobs SET
                    status = 'interrupted',
                    failure_reason = 'Process restarted',
                    completed_at = NOW()
                WHERE source = 'sandbox' AND status IN ('running', 'creating')
                "#,
                &[],
            )
            .await?;
        if count > 0 {
            tracing::info!("Marked {} stale sandbox jobs as interrupted", count);
        }
        Ok(count)
    }

    /// Get a summary of sandbox job counts by status.
    pub async fn sandbox_job_summary(&self) -> Result<SandboxJobSummary, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                "SELECT status, COUNT(*) as cnt FROM agent_jobs WHERE source = 'sandbox' GROUP BY status",
                &[],
            )
            .await?;

        let mut summary = SandboxJobSummary::default();
        for row in &rows {
            let status: String = row.get("status");
            let count: i64 = row.get("cnt");
            let c = count as usize;
            summary.total += c;
            match status.as_str() {
                "creating" => summary.creating += c,
                "running" => summary.running += c,
                "completed" => summary.completed += c,
                "failed" => summary.failed += c,
                "cancelled" => summary.cancelled += c,
                "interrupted" => summary.interrupted += c,
                "stuck" => summary.stuck += c,
                _ => {}
            }
        }
        Ok(summary)
    }
}
