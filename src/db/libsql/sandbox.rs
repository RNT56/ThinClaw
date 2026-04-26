//! Sandbox-related SandboxStore implementation for LibSqlBackend.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use libsql::params;
use uuid::Uuid;

use super::{
    LibSqlBackend, fmt_opt_ts, fmt_ts, get_i64, get_json, get_opt_bool, get_opt_text, get_opt_ts,
    get_text, get_ts, opt_text,
};
use crate::db::SandboxStore;
use crate::error::DatabaseError;
use crate::history::{JobEventRecord, SandboxJobRecord, SandboxJobSummary};
use crate::sandbox_jobs::SandboxJobSpec;

fn parse_job_mode(raw: &str) -> crate::sandbox_types::JobMode {
    match raw {
        "claude_code" => crate::sandbox_types::JobMode::ClaudeCode,
        "codex_code" => crate::sandbox_types::JobMode::CodexCode,
        _ => crate::sandbox_types::JobMode::Worker,
    }
}

#[async_trait]
impl SandboxStore for LibSqlBackend {
    async fn save_sandbox_job(&self, job: &SandboxJobRecord) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
                INSERT INTO agent_jobs (
                    id, title, description, status, source, user_id, principal_id, actor_id,
                    project_dir, job_mode, metadata, success, failure_reason,
                    created_at, started_at, completed_at, credential_grants
                ) VALUES (?1, ?2, ?3, ?4, 'sandbox', ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
                ON CONFLICT (id) DO UPDATE SET
                    title = excluded.title,
                    description = excluded.description,
                    status = excluded.status,
                    principal_id = excluded.principal_id,
                    success = excluded.success,
                    failure_reason = excluded.failure_reason,
                    actor_id = excluded.actor_id,
                    project_dir = excluded.project_dir,
                    job_mode = excluded.job_mode,
                    metadata = excluded.metadata,
                    started_at = excluded.started_at,
                    completed_at = excluded.completed_at,
                    credential_grants = excluded.credential_grants
                "#,
            params![
                job.id.to_string(),
                job.spec.title.as_str(),
                job.spec.description.as_str(),
                job.status.as_str(),
                job.spec.principal_id.as_str(),
                job.spec.principal_id.as_str(),
                job.spec.actor_id.as_str(),
                opt_text(job.spec.project_dir.as_deref()),
                job.spec.mode.as_str(),
                job.spec.persisted_metadata().to_string(),
                job.success.map(|b| b as i64),
                opt_text(job.failure_reason.as_deref()),
                fmt_ts(&job.created_at),
                fmt_opt_ts(&job.started_at),
                fmt_opt_ts(&job.completed_at),
                job.credential_grants_json.as_str(),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn get_sandbox_job(&self, id: Uuid) -> Result<Option<SandboxJobRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, title, description, status, user_id, principal_id, actor_id, project_dir,
                       job_mode, metadata, success, failure_reason, created_at, started_at,
                       completed_at, COALESCE(credential_grants, '[]')
                FROM agent_jobs WHERE id = ?1 AND source = 'sandbox'
                "#,
                params![id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(SandboxJobRecord {
                id: get_text(&row, 0).parse().unwrap_or_default(),
                spec: SandboxJobSpec::from_persisted(
                    get_text(&row, 1),
                    get_text(&row, 2),
                    get_text(&row, 5),
                    get_text(&row, 6),
                    get_opt_text(&row, 7),
                    parse_job_mode(&get_text(&row, 8)),
                    get_json(&row, 9),
                ),
                status: get_text(&row, 3),
                success: get_opt_bool(&row, 10),
                failure_reason: get_opt_text(&row, 11),
                created_at: get_ts(&row, 12),
                started_at: get_opt_ts(&row, 13),
                completed_at: get_opt_ts(&row, 14),
                credential_grants_json: get_text(&row, 15),
            })),
            None => Ok(None),
        }
    }

    async fn list_sandbox_jobs(&self) -> Result<Vec<SandboxJobRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, title, description, status, user_id, principal_id, actor_id, project_dir,
                       job_mode, metadata, success, failure_reason, created_at, started_at,
                       completed_at, COALESCE(credential_grants, '[]')
                FROM agent_jobs WHERE source = 'sandbox'
                ORDER BY created_at DESC
                "#,
                (),
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut jobs = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            jobs.push(SandboxJobRecord {
                id: get_text(&row, 0).parse().unwrap_or_default(),
                spec: SandboxJobSpec::from_persisted(
                    get_text(&row, 1),
                    get_text(&row, 2),
                    get_text(&row, 5),
                    get_text(&row, 6),
                    get_opt_text(&row, 7),
                    parse_job_mode(&get_text(&row, 8)),
                    get_json(&row, 9),
                ),
                status: get_text(&row, 3),
                success: get_opt_bool(&row, 10),
                failure_reason: get_opt_text(&row, 11),
                created_at: get_ts(&row, 12),
                started_at: get_opt_ts(&row, 13),
                completed_at: get_opt_ts(&row, 14),
                credential_grants_json: get_text(&row, 15),
            });
        }
        Ok(jobs)
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
        let conn = self.connect().await?;
        conn.execute(
            r#"
                UPDATE agent_jobs SET
                    status = ?2,
                    success = COALESCE(?3, success),
                    failure_reason = COALESCE(?4, failure_reason),
                    started_at = COALESCE(?5, started_at),
                    completed_at = COALESCE(?6, completed_at)
                WHERE id = ?1 AND source = 'sandbox'
                "#,
            params![
                id.to_string(),
                status,
                success.map(|b| b as i64),
                message,
                fmt_opt_ts(&started_at),
                fmt_opt_ts(&completed_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn cleanup_stale_sandbox_jobs(&self) -> Result<u64, DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        let count = conn
            .execute(
                r#"
                UPDATE agent_jobs SET
                    status = 'interrupted',
                    failure_reason = 'Process restarted',
                    completed_at = ?1
                WHERE source = 'sandbox' AND status IN ('running', 'creating')
                "#,
                params![now],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        if count > 0 {
            tracing::info!("Marked {} stale sandbox jobs as interrupted", count);
        }
        Ok(count)
    }

    async fn sandbox_job_summary(&self) -> Result<SandboxJobSummary, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT status, COUNT(*) as cnt FROM agent_jobs WHERE source = 'sandbox' GROUP BY status",
                (),
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut summary = SandboxJobSummary::default();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let status = get_text(&row, 0);
            let count = get_i64(&row, 1) as usize;
            summary.total += count;
            match status.as_str() {
                "creating" => summary.creating += count,
                "running" => summary.running += count,
                "completed" => summary.completed += count,
                "failed" => summary.failed += count,
                "cancelled" => summary.cancelled += count,
                "interrupted" => summary.interrupted += count,
                "stuck" => summary.stuck += count,
                _ => {}
            }
        }
        Ok(summary)
    }

    async fn list_sandbox_jobs_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<SandboxJobRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, title, description, status, user_id, principal_id, actor_id, project_dir,
                       job_mode, metadata, success, failure_reason, created_at, started_at,
                       completed_at, COALESCE(credential_grants, '[]')
                FROM agent_jobs WHERE source = 'sandbox' AND user_id = ?1
                ORDER BY created_at DESC
                "#,
                libsql::params![user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut jobs = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            jobs.push(SandboxJobRecord {
                id: get_text(&row, 0).parse().unwrap_or_default(),
                spec: SandboxJobSpec::from_persisted(
                    get_text(&row, 1),
                    get_text(&row, 2),
                    get_text(&row, 5),
                    get_text(&row, 6),
                    get_opt_text(&row, 7),
                    parse_job_mode(&get_text(&row, 8)),
                    get_json(&row, 9),
                ),
                status: get_text(&row, 3),
                success: get_opt_bool(&row, 10),
                failure_reason: get_opt_text(&row, 11),
                created_at: get_ts(&row, 12),
                started_at: get_opt_ts(&row, 13),
                completed_at: get_opt_ts(&row, 14),
                credential_grants_json: get_text(&row, 15),
            });
        }
        Ok(jobs)
    }

    async fn sandbox_job_summary_for_user(
        &self,
        user_id: &str,
    ) -> Result<SandboxJobSummary, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT status, COUNT(*) as cnt FROM agent_jobs WHERE source = 'sandbox' AND user_id = ?1 GROUP BY status",
                libsql::params![user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut summary = SandboxJobSummary::default();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let status = get_text(&row, 0);
            let count = get_i64(&row, 1) as usize;
            summary.total += count;
            match status.as_str() {
                "creating" => summary.creating += count,
                "running" => summary.running += count,
                "completed" => summary.completed += count,
                "failed" => summary.failed += count,
                "cancelled" => summary.cancelled += count,
                "interrupted" => summary.interrupted += count,
                "stuck" => summary.stuck += count,
                _ => {}
            }
        }
        Ok(summary)
    }

    async fn sandbox_job_belongs_to_user(
        &self,
        job_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT 1 FROM agent_jobs WHERE id = ?1 AND user_id = ?2 AND source = 'sandbox'",
                libsql::params![job_id.to_string(), user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let found = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(found.is_some())
    }

    async fn update_sandbox_job_mode(&self, id: Uuid, mode: &str) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            "UPDATE agent_jobs SET job_mode = ?2 WHERE id = ?1",
            params![id.to_string(), mode],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn get_sandbox_job_mode(&self, id: Uuid) -> Result<Option<String>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT job_mode FROM agent_jobs WHERE id = ?1",
                params![id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(get_text(&row, 0))),
            None => Ok(None),
        }
    }

    async fn save_job_event(
        &self,
        job_id: Uuid,
        event_type: &str,
        data: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            "INSERT INTO job_events (job_id, event_type, data) VALUES (?1, ?2, ?3)",
            params![job_id.to_string(), event_type, data.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn list_job_events(
        &self,
        job_id: Uuid,
        limit: Option<i64>,
    ) -> Result<Vec<JobEventRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = if let Some(n) = limit {
            conn.query(
                r#"
                SELECT id, job_id, event_type, data, created_at
                FROM (
                    SELECT id, job_id, event_type, data, created_at
                    FROM job_events WHERE job_id = ?1
                    ORDER BY id DESC
                    LIMIT ?2
                )
                ORDER BY id ASC
                "#,
                params![job_id.to_string(), n],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        } else {
            conn.query(
                r#"
                SELECT id, job_id, event_type, data, created_at
                FROM job_events WHERE job_id = ?1 ORDER BY id ASC
                "#,
                params![job_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        };

        let mut events = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            events.push(JobEventRecord {
                id: get_i64(&row, 0),
                job_id: get_text(&row, 1).parse().unwrap_or_default(),
                event_type: get_text(&row, 2),
                data: get_json(&row, 3),
                created_at: get_ts(&row, 4),
            });
        }
        Ok(events)
    }
}
