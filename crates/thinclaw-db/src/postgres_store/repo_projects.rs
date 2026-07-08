#[cfg(feature = "postgres")]
use uuid::Uuid;

#[cfg(feature = "postgres")]
use thinclaw_repo_projects::{
    MergeGateDecision, RepoProject, RepoProjectEvent, RepoProjectRepo, RepoProjectRun,
    RepoProjectTask, RepoWebhookDelivery, RepoWorkerRun,
};
#[cfg(feature = "postgres")]
use thinclaw_types::error::DatabaseError;

#[cfg(feature = "postgres")]
use super::Store;

#[cfg(feature = "postgres")]
impl Store {
    pub async fn create_repo_project(&self, project: &RepoProject) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            INSERT INTO repo_projects (id, state, data, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5)
            "#,
            &[
                &project.id,
                &json_label(&project.state, "draft"),
                &json_value(project)?,
                &project.created_at,
                &project.updated_at,
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn get_repo_project(&self, id: Uuid) -> Result<Option<RepoProject>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt("SELECT data FROM repo_projects WHERE id = $1", &[&id])
            .await?;
        row.map(|row| json_from_row(&row, "data")).transpose()
    }

    pub async fn list_repo_projects(&self) -> Result<Vec<RepoProject>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                "SELECT data FROM repo_projects ORDER BY updated_at DESC, created_at DESC",
                &[],
            )
            .await?;
        rows.iter().map(|row| json_from_row(row, "data")).collect()
    }

    pub async fn update_repo_project(&self, project: &RepoProject) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            UPDATE repo_projects
               SET state = $2, data = $3, updated_at = $4
             WHERE id = $1
            "#,
            &[
                &project.id,
                &json_label(&project.state, "draft"),
                &json_value(project)?,
                &project.updated_at,
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn delete_repo_project(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let conn = self.conn().await?;
        let count = conn
            .execute("DELETE FROM repo_projects WHERE id = $1", &[&id])
            .await?;
        Ok(count > 0)
    }

    pub async fn upsert_repo_project_repo(
        &self,
        repo: &RepoProjectRepo,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            INSERT INTO repo_project_repos (
                id, project_id, owner, repo, enrolled, data, created_at, updated_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT(id) DO UPDATE SET
                project_id = excluded.project_id,
                owner = excluded.owner,
                repo = excluded.repo,
                enrolled = excluded.enrolled,
                data = excluded.data,
                updated_at = excluded.updated_at
            "#,
            &[
                &repo.id,
                &repo.project_id,
                &repo.owner,
                &repo.repo,
                &repo.enrolled,
                &json_value(repo)?,
                &repo.created_at,
                &repo.updated_at,
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn list_repo_project_repos(
        &self,
        project_id: Uuid,
    ) -> Result<Vec<RepoProjectRepo>, DatabaseError> {
        self.list_json_by_project("repo_project_repos", project_id, "owner ASC, repo ASC")
            .await
    }

    pub async fn upsert_repo_project_task(
        &self,
        task: &RepoProjectTask,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            INSERT INTO repo_project_tasks (
                id, project_id, repo_id, state, priority, data, created_at, updated_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT(id) DO UPDATE SET
                project_id = excluded.project_id,
                repo_id = excluded.repo_id,
                state = excluded.state,
                priority = excluded.priority,
                data = excluded.data,
                updated_at = excluded.updated_at
            "#,
            &[
                &task.id,
                &task.project_id,
                &task.repo_id,
                &json_label(&task.state, "queued"),
                &task.priority,
                &json_value(task)?,
                &task.created_at,
                &task.updated_at,
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn get_repo_project_task(
        &self,
        id: Uuid,
    ) -> Result<Option<RepoProjectTask>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt("SELECT data FROM repo_project_tasks WHERE id = $1", &[&id])
            .await?;
        row.map(|row| json_from_row(&row, "data")).transpose()
    }

    pub async fn list_repo_project_tasks(
        &self,
        project_id: Uuid,
    ) -> Result<Vec<RepoProjectTask>, DatabaseError> {
        self.list_json_by_project(
            "repo_project_tasks",
            project_id,
            "priority DESC, updated_at DESC",
        )
        .await
    }

    pub async fn upsert_repo_worker_run(&self, run: &RepoWorkerRun) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            INSERT INTO repo_worker_runs (
                id, project_id, task_id, state, data, created_at, updated_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT(id) DO UPDATE SET
                project_id = excluded.project_id,
                task_id = excluded.task_id,
                state = excluded.state,
                data = excluded.data,
                updated_at = excluded.updated_at
            "#,
            &[
                &run.id,
                &run.project_id,
                &run.task_id,
                &json_label(&run.state, "queued"),
                &json_value(run)?,
                &run.created_at,
                &run.updated_at,
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn list_repo_worker_runs(
        &self,
        project_id: Uuid,
    ) -> Result<Vec<RepoWorkerRun>, DatabaseError> {
        self.list_json_by_project("repo_worker_runs", project_id, "updated_at DESC")
            .await
    }

    pub async fn append_repo_project_event(
        &self,
        event: &RepoProjectEvent,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            INSERT INTO repo_project_events (
                id, project_id, task_id, event_kind, data, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT(id) DO NOTHING
            "#,
            &[
                &event.id,
                &event.project_id,
                &event.task_id,
                &json_label(&event.kind, "project_created"),
                &json_value(event)?,
                &event.created_at,
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn list_repo_project_events(
        &self,
        project_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RepoProjectEvent>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                "SELECT data FROM repo_project_events WHERE project_id = $1 ORDER BY created_at DESC LIMIT $2",
                &[&project_id, &limit.clamp(1, 500)],
            )
            .await?;
        rows.iter().map(|row| json_from_row(row, "data")).collect()
    }

    pub async fn upsert_repo_merge_gate_decision(
        &self,
        project_id: Uuid,
        task_id: Uuid,
        decision: &MergeGateDecision,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            INSERT INTO repo_merge_gate_decisions (project_id, task_id, data, updated_at)
            VALUES ($1, $2, $3, now())
            ON CONFLICT(project_id, task_id) DO UPDATE SET
                data = excluded.data,
                updated_at = excluded.updated_at
            "#,
            &[&project_id, &task_id, &json_value(decision)?],
        )
        .await?;
        Ok(())
    }

    pub async fn list_repo_merge_gate_decisions(
        &self,
        project_id: Uuid,
    ) -> Result<Vec<(Uuid, MergeGateDecision)>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                "SELECT task_id, data FROM repo_merge_gate_decisions WHERE project_id = $1 ORDER BY updated_at DESC",
                &[&project_id],
            )
            .await?;
        rows.iter()
            .map(|row| Ok((row.get("task_id"), json_from_row(row, "data")?)))
            .collect()
    }

    pub async fn record_repo_webhook_delivery(
        &self,
        delivery: &RepoWebhookDelivery,
    ) -> Result<bool, DatabaseError> {
        let conn = self.conn().await?;
        let count = conn
            .execute(
                r#"
                INSERT INTO repo_webhook_deliveries (delivery_id, event, data, received_at)
                VALUES ($1, $2, $3, $4)
                ON CONFLICT(delivery_id) DO NOTHING
                "#,
                &[
                    &delivery.delivery_id,
                    &delivery.event,
                    &json_value(delivery)?,
                    &delivery.received_at,
                ],
            )
            .await?;
        Ok(count > 0)
    }

    pub async fn get_repo_webhook_delivery(
        &self,
        delivery_id: &str,
    ) -> Result<Option<RepoWebhookDelivery>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt(
                "SELECT data FROM repo_webhook_deliveries WHERE delivery_id = $1 LIMIT 1",
                &[&delivery_id],
            )
            .await?;
        row.map(|row| json_from_row(&row, "data")).transpose()
    }

    pub async fn list_repo_webhook_deliveries(
        &self,
        limit: i64,
    ) -> Result<Vec<RepoWebhookDelivery>, DatabaseError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                "SELECT data FROM repo_webhook_deliveries ORDER BY received_at DESC LIMIT $1",
                &[&limit.clamp(1, 1000)],
            )
            .await?;
        rows.iter().map(|row| json_from_row(row, "data")).collect()
    }

    pub async fn upsert_repo_project_run(&self, run: &RepoProjectRun) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            INSERT INTO repo_project_runs (id, project_id, state, data, created_at)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT(id) DO UPDATE SET
                state = excluded.state,
                data = excluded.data
            "#,
            &[
                &run.id,
                &run.project_id,
                &json_label(&run.state, "queued"),
                &json_value(run)?,
                &run.created_at,
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn get_repo_project_run(
        &self,
        id: Uuid,
    ) -> Result<Option<RepoProjectRun>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt("SELECT data FROM repo_project_runs WHERE id = $1", &[&id])
            .await?;
        row.map(|row| json_from_row(&row, "data")).transpose()
    }

    pub async fn list_repo_project_runs(
        &self,
        project_id: Uuid,
    ) -> Result<Vec<RepoProjectRun>, DatabaseError> {
        self.list_json_by_project("repo_project_runs", project_id, "created_at DESC")
            .await
    }

    async fn list_json_by_project<T>(
        &self,
        table: &str,
        project_id: Uuid,
        order_by: &str,
    ) -> Result<Vec<T>, DatabaseError>
    where
        T: serde::de::DeserializeOwned,
    {
        let conn = self.conn().await?;
        let sql = format!("SELECT data FROM {table} WHERE project_id = $1 ORDER BY {order_by}");
        let rows = conn.query(&sql, &[&project_id]).await?;
        rows.iter().map(|row| json_from_row(row, "data")).collect()
    }
}

#[cfg(feature = "postgres")]
fn json_value<T: serde::Serialize>(value: &T) -> Result<serde_json::Value, DatabaseError> {
    serde_json::to_value(value).map_err(|e| DatabaseError::Serialization(e.to_string()))
}

#[cfg(feature = "postgres")]
fn json_from_row<T: serde::de::DeserializeOwned>(
    row: &tokio_postgres::Row,
    column: &str,
) -> Result<T, DatabaseError> {
    let value: serde_json::Value = row.get(column);
    serde_json::from_value(value).map_err(|e| DatabaseError::Serialization(e.to_string()))
}

#[cfg(feature = "postgres")]
fn json_label<T: serde::Serialize>(value: &T, fallback: &str) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| fallback.to_string())
}
