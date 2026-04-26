#![cfg_attr(not(feature = "postgres"), allow(unused_imports))]

use super::*;
#[cfg(feature = "postgres")]
impl Store {
    /// Save a job context to the database.
    pub async fn save_job(&self, ctx: &JobContext) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;

        let status = ctx.state.to_string();
        let estimated_time_secs = ctx.estimated_duration.map(|d| d.as_secs() as i32);
        let total_tokens_used = ctx.total_tokens_used.min(i64::MAX as u64) as i64;
        let max_tokens = ctx.max_tokens.min(i64::MAX as u64) as i64;
        let transitions = serde_json::to_value(&ctx.transitions)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let failure_reason = job_failure_reason(ctx);

        conn.execute(
            r#"
            INSERT INTO agent_jobs (
                id, conversation_id, title, description, category, status, source, user_id, principal_id, actor_id,
                budget_amount, budget_token, bid_amount, estimated_cost, estimated_time_secs,
                actual_cost, total_tokens_used, max_tokens, metadata, transitions, failure_reason,
                repair_attempts, created_at, started_at, completed_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25)
            ON CONFLICT (id) DO UPDATE SET
                title = EXCLUDED.title,
                description = EXCLUDED.description,
                category = EXCLUDED.category,
                status = EXCLUDED.status,
                user_id = EXCLUDED.user_id,
                principal_id = EXCLUDED.principal_id,
                actor_id = EXCLUDED.actor_id,
                estimated_cost = EXCLUDED.estimated_cost,
                estimated_time_secs = EXCLUDED.estimated_time_secs,
                actual_cost = EXCLUDED.actual_cost,
                total_tokens_used = EXCLUDED.total_tokens_used,
                max_tokens = EXCLUDED.max_tokens,
                metadata = EXCLUDED.metadata,
                transitions = EXCLUDED.transitions,
                failure_reason = EXCLUDED.failure_reason,
                repair_attempts = EXCLUDED.repair_attempts,
                started_at = EXCLUDED.started_at,
                completed_at = EXCLUDED.completed_at
            "#,
            &[
                &ctx.job_id,
                &ctx.conversation_id,
                &ctx.title,
                &ctx.description,
                &ctx.category,
                &status,
                &"direct", // source
                &ctx.user_id,
                &ctx.principal_id,
                &ctx.actor_id,
                &ctx.budget,
                &ctx.budget_token,
                &ctx.bid_amount,
                &ctx.estimated_cost,
                &estimated_time_secs,
                &ctx.actual_cost,
                &total_tokens_used,
                &max_tokens,
                &ctx.metadata,
                &transitions,
                &failure_reason,
                &(ctx.repair_attempts as i32),
                &ctx.created_at,
                &ctx.started_at,
                &ctx.completed_at,
            ],
        )
        .await?;

        Ok(())
    }

    /// Get a job by ID.
    pub async fn get_job(&self, id: Uuid) -> Result<Option<JobContext>, DatabaseError> {
        let conn = self.conn().await?;

        let row = conn
            .query_opt(
                r#"
                SELECT id, conversation_id, title, description, category, status, user_id, principal_id, actor_id,
                       budget_amount, budget_token, bid_amount, estimated_cost, estimated_time_secs,
                       actual_cost, total_tokens_used, max_tokens, metadata, transitions,
                       repair_attempts, created_at, started_at, completed_at
                FROM agent_jobs WHERE id = $1 AND source = 'direct'
                "#,
                &[&id],
            )
            .await?;

        match row {
            Some(row) => {
                let status_str: String = row.get("status");
                let state = parse_job_state(&status_str);
                let estimated_time_secs: Option<i32> = row.get("estimated_time_secs");
                let transitions_json: serde_json::Value = row.get("transitions");
                let transitions = serde_json::from_value::<Vec<StateTransition>>(transitions_json)
                    .unwrap_or_default();
                let metadata: serde_json::Value = row.get("metadata");

                Ok(Some(JobContext {
                    job_id: row.get("id"),
                    state,
                    user_id: row.get::<_, String>("user_id"),
                    principal_id: row.get::<_, String>("principal_id"),
                    actor_id: row.get("actor_id"),
                    conversation_id: row.get("conversation_id"),
                    title: row.get("title"),
                    description: row.get("description"),
                    category: row.get("category"),
                    budget: row.get("budget_amount"),
                    budget_token: row.get("budget_token"),
                    bid_amount: row.get("bid_amount"),
                    estimated_cost: row.get("estimated_cost"),
                    estimated_duration: estimated_time_secs
                        .map(|s| std::time::Duration::from_secs(s as u64)),
                    actual_cost: row
                        .get::<_, Option<Decimal>>("actual_cost")
                        .unwrap_or_default(),
                    total_tokens_used: row.get::<_, i64>("total_tokens_used").max(0) as u64,
                    max_tokens: row.get::<_, i64>("max_tokens").max(0) as u64,
                    repair_attempts: row.get::<_, i32>("repair_attempts") as u32,
                    created_at: row.get("created_at"),
                    started_at: row.get("started_at"),
                    completed_at: row.get("completed_at"),
                    transitions,
                    metadata,
                    extra_env: std::sync::Arc::new(std::collections::HashMap::new()),
                }))
            }
            None => Ok(None),
        }
    }

    /// Update job status.
    pub async fn update_job_status(
        &self,
        id: Uuid,
        status: JobState,
        failure_reason: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        let status_str = status.to_string();

        conn.execute(
            "UPDATE agent_jobs SET status = $2, failure_reason = $3 WHERE id = $1",
            &[&id, &status_str, &failure_reason],
        )
        .await?;

        Ok(())
    }

    /// Mark any in-flight direct jobs from a previous process as abandoned.
    pub async fn abandon_active_direct_jobs(&self, reason: &str) -> Result<u64, DatabaseError> {
        let conn = self.conn().await?;
        let count = conn
            .execute(
                r#"
                UPDATE agent_jobs
                SET status = 'abandoned',
                    failure_reason = COALESCE(NULLIF(failure_reason, ''), $1),
                    completed_at = COALESCE(completed_at, NOW())
                WHERE source = 'direct'
                  AND status IN ('pending', 'in_progress', 'stuck')
                "#,
                &[&reason],
            )
            .await?;
        Ok(count)
    }

    /// List all direct jobs for a principal.
    pub async fn list_jobs_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<JobContext>, DatabaseError> {
        let conn = self.conn().await?;

        let rows = conn
            .query(
                r#"
                SELECT id, conversation_id, title, description, category, status, user_id, principal_id, actor_id,
                       budget_amount, budget_token, bid_amount, estimated_cost, estimated_time_secs,
                       actual_cost, total_tokens_used, max_tokens, metadata, transitions,
                       repair_attempts, created_at, started_at, completed_at
                FROM agent_jobs
                WHERE user_id = $1 AND source = 'direct'
                ORDER BY created_at DESC
                "#,
                &[&user_id],
            )
            .await?;

        let mut jobs = Vec::with_capacity(rows.len());
        for row in rows {
            let status_str: String = row.get("status");
            let state = parse_job_state(&status_str);
            let estimated_time_secs: Option<i32> = row.get("estimated_time_secs");
            let transitions_json: serde_json::Value = row.get("transitions");
            let transitions = serde_json::from_value::<Vec<StateTransition>>(transitions_json)
                .unwrap_or_default();
            let metadata: serde_json::Value = row.get("metadata");

            jobs.push(JobContext {
                job_id: row.get("id"),
                state,
                user_id: row.get::<_, String>("user_id"),
                principal_id: row.get::<_, String>("principal_id"),
                actor_id: row.get("actor_id"),
                conversation_id: row.get("conversation_id"),
                title: row.get("title"),
                description: row.get("description"),
                category: row.get("category"),
                budget: row.get("budget_amount"),
                budget_token: row.get("budget_token"),
                bid_amount: row.get("bid_amount"),
                estimated_cost: row.get("estimated_cost"),
                estimated_duration: estimated_time_secs
                    .map(|seconds| std::time::Duration::from_secs(seconds as u64)),
                actual_cost: row
                    .get::<_, Option<Decimal>>("actual_cost")
                    .unwrap_or_default(),
                total_tokens_used: row.get::<_, i64>("total_tokens_used").max(0) as u64,
                max_tokens: row.get::<_, i64>("max_tokens").max(0) as u64,
                repair_attempts: row.get::<_, i32>("repair_attempts") as u32,
                created_at: row.get("created_at"),
                started_at: row.get("started_at"),
                completed_at: row.get("completed_at"),
                transitions,
                metadata,
                extra_env: std::sync::Arc::new(std::collections::HashMap::new()),
            });
        }

        Ok(jobs)
    }

    /// Mark job as stuck.
    pub async fn mark_job_stuck(&self, id: Uuid) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;

        conn.execute(
            "UPDATE agent_jobs SET status = 'stuck', stuck_since = NOW() WHERE id = $1",
            &[&id],
        )
        .await?;

        Ok(())
    }

    /// Get stuck jobs.
    pub async fn get_stuck_jobs(&self) -> Result<Vec<Uuid>, DatabaseError> {
        let conn = self.conn().await?;

        let rows = conn
            .query("SELECT id FROM agent_jobs WHERE status = 'stuck'", &[])
            .await?;

        Ok(rows.iter().map(|r| r.get("id")).collect())
    }

    // ==================== Actions ====================

    /// Save a job action.
    pub async fn save_action(
        &self,
        job_id: Uuid,
        action: &ActionRecord,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;

        let duration_ms = action.duration.as_millis() as i32;
        let warnings_json = serde_json::to_value(&action.sanitization_warnings)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

        conn.execute(
            r#"
            INSERT INTO job_actions (
                id, job_id, sequence_num, tool_name, input, output_raw, output_sanitized,
                sanitization_warnings, cost, duration_ms, success, error_message, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            "#,
            &[
                &action.id,
                &job_id,
                &(action.sequence as i32),
                &action.tool_name,
                &action.input,
                &action.output_raw,
                &action.output_sanitized,
                &warnings_json,
                &action.cost,
                &duration_ms,
                &action.success,
                &action.error,
                &action.executed_at,
            ],
        )
        .await?;

        Ok(())
    }

    /// Get actions for a job.
    pub async fn get_job_actions(&self, job_id: Uuid) -> Result<Vec<ActionRecord>, DatabaseError> {
        let conn = self.conn().await?;

        let rows = conn
            .query(
                r#"
                SELECT id, sequence_num, tool_name, input, output_raw, output_sanitized,
                       sanitization_warnings, cost, duration_ms, success, error_message, created_at
                FROM job_actions WHERE job_id = $1 ORDER BY sequence_num
                "#,
                &[&job_id],
            )
            .await?;

        let mut actions = Vec::new();
        for row in rows {
            let duration_ms: i32 = row.get("duration_ms");
            let warnings_json: serde_json::Value = row.get("sanitization_warnings");
            let warnings: Vec<String> = serde_json::from_value(warnings_json).unwrap_or_default();

            actions.push(ActionRecord {
                id: row.get("id"),
                sequence: row.get::<_, i32>("sequence_num") as u32,
                tool_name: row.get("tool_name"),
                input: row.get("input"),
                output_raw: row.get("output_raw"),
                output_sanitized: row.get("output_sanitized"),
                sanitization_warnings: warnings,
                cost: row.get("cost"),
                duration: std::time::Duration::from_millis(duration_ms as u64),
                success: row.get("success"),
                error: row.get("error_message"),
                executed_at: row.get("created_at"),
            });
        }

        Ok(actions)
    }

    // ==================== LLM Calls ====================

    /// Record an LLM call.
    pub async fn record_llm_call(&self, record: &LlmCallRecord<'_>) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;
        let id = Uuid::new_v4();

        conn.execute(
            r#"
            INSERT INTO llm_calls (id, job_id, conversation_id, provider, model, input_tokens, output_tokens, cost, purpose)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
            &[
                &id,
                &record.job_id,
                &record.conversation_id,
                &record.provider,
                &record.model,
                &(record.input_tokens as i32),
                &(record.output_tokens as i32),
                &record.cost,
                &record.purpose,
            ],
        )
        .await?;

        Ok(id)
    }

    // ==================== Estimation Snapshots ====================

    /// Save an estimation snapshot for learning.
    pub async fn save_estimation_snapshot(
        &self,
        job_id: Uuid,
        category: &str,
        tool_names: &[String],
        estimated_cost: Decimal,
        estimated_time_secs: i32,
        estimated_value: Decimal,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;
        let id = Uuid::new_v4();

        conn.execute(
            r#"
            INSERT INTO estimation_snapshots (id, job_id, category, tool_names, estimated_cost, estimated_time_secs, estimated_value)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
            &[
                &id,
                &job_id,
                &category,
                &tool_names,
                &estimated_cost,
                &estimated_time_secs,
                &estimated_value,
            ],
        )
        .await?;

        Ok(id)
    }

    /// Update estimation snapshot with actual values.
    pub async fn update_estimation_actuals(
        &self,
        id: Uuid,
        actual_cost: Decimal,
        actual_time_secs: i32,
        actual_value: Option<Decimal>,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;

        conn.execute(
            "UPDATE estimation_snapshots SET actual_cost = $2, actual_time_secs = $3, actual_value = $4 WHERE id = $1",
            &[&id, &actual_cost, &actual_time_secs, &actual_value],
        )
        .await?;

        Ok(())
    }
}
