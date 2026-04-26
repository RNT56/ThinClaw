use super::*;

// ==================== Job Events ====================

/// A persisted job streaming event (from worker or Claude Code bridge).
#[derive(Debug, Clone)]
pub struct JobEventRecord {
    pub id: i64,
    pub job_id: Uuid,
    pub event_type: String,
    pub data: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[cfg(feature = "postgres")]
impl Store {
    /// Persist a job event (fire-and-forget from orchestrator handler).
    pub async fn save_job_event(
        &self,
        job_id: Uuid,
        event_type: &str,
        data: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            INSERT INTO job_events (job_id, event_type, data)
            VALUES ($1, $2, $3)
            "#,
            &[&job_id, &event_type, data],
        )
        .await?;
        Ok(())
    }

    /// Load job events for a job, ordered by id.
    ///
    /// When `limit` is `Some(n)`, returns the **most recent** `n` events
    /// (ordered ascending by id). When `None`, returns all events.
    pub async fn list_job_events(
        &self,
        job_id: Uuid,
        limit: Option<i64>,
    ) -> Result<Vec<JobEventRecord>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = if let Some(n) = limit {
            // Sub-select the last N rows by id DESC, then re-sort ASC.
            conn.query(
                r#"
                SELECT id, job_id, event_type, data, created_at
                FROM (
                    SELECT id, job_id, event_type, data, created_at
                    FROM job_events
                    WHERE job_id = $1
                    ORDER BY id DESC
                    LIMIT $2
                ) sub
                ORDER BY id ASC
                "#,
                &[&job_id, &n],
            )
            .await?
        } else {
            conn.query(
                r#"
                SELECT id, job_id, event_type, data, created_at
                FROM job_events
                WHERE job_id = $1
                ORDER BY id ASC
                "#,
                &[&job_id],
            )
            .await?
        };
        Ok(rows
            .iter()
            .map(|r| JobEventRecord {
                id: r.get("id"),
                job_id: r.get("job_id"),
                event_type: r.get("event_type"),
                data: r.get("data"),
                created_at: r.get("created_at"),
            })
            .collect())
    }

    /// Update the job_mode column for a sandbox job.
    pub async fn update_sandbox_job_mode(&self, id: Uuid, mode: &str) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            "UPDATE agent_jobs SET job_mode = $2 WHERE id = $1",
            &[&id, &mode],
        )
        .await?;
        Ok(())
    }

    /// Get the job_mode for a sandbox job.
    pub async fn get_sandbox_job_mode(&self, id: Uuid) -> Result<Option<String>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt("SELECT job_mode FROM agent_jobs WHERE id = $1", &[&id])
            .await?;
        Ok(row.map(|r| r.get("job_mode")))
    }
}
