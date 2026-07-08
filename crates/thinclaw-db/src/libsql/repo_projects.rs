//! Repo project store implementation for LibSqlBackend.

use async_trait::async_trait;
use libsql::params;
use uuid::Uuid;

use super::{LibSqlBackend, fmt_ts, opt_text_owned, row_json_to};
use crate::RepoProjectStore;
use thinclaw_repo_projects::{
    MergeGateDecision, RepoProject, RepoProjectEvent, RepoProjectRepo, RepoProjectRun,
    RepoProjectTask, RepoWebhookDelivery, RepoWorkerRun,
};
use thinclaw_types::error::DatabaseError;

#[async_trait]
impl RepoProjectStore for LibSqlBackend {
    async fn create_repo_project(&self, project: &RepoProject) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
            INSERT INTO repo_projects (id, state, data, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            params![
                project.id.to_string(),
                json_label(&project.state, "draft"),
                to_json(project)?,
                fmt_ts(&project.created_at),
                fmt_ts(&project.updated_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn get_repo_project(&self, id: Uuid) -> Result<Option<RepoProject>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT data FROM repo_projects WHERE id = ?1",
                params![id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(row_json_to(&row, 0)?)),
            None => Ok(None),
        }
    }

    async fn list_repo_projects(&self) -> Result<Vec<RepoProject>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT data FROM repo_projects ORDER BY updated_at DESC, created_at DESC",
                (),
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut items = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            items.push(row_json_to(&row, 0)?);
        }
        Ok(items)
    }

    async fn update_repo_project(&self, project: &RepoProject) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
            UPDATE repo_projects
               SET state = ?2, data = ?3, updated_at = ?4
             WHERE id = ?1
            "#,
            params![
                project.id.to_string(),
                json_label(&project.state, "draft"),
                to_json(project)?,
                fmt_ts(&project.updated_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn delete_repo_project(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let count = conn
            .execute(
                "DELETE FROM repo_projects WHERE id = ?1",
                params![id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(count > 0)
    }

    async fn upsert_repo_project_repo(&self, repo: &RepoProjectRepo) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
            INSERT INTO repo_project_repos (
                id, project_id, owner, repo, enrolled, data, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ON CONFLICT(id) DO UPDATE SET
                project_id = excluded.project_id,
                owner = excluded.owner,
                repo = excluded.repo,
                enrolled = excluded.enrolled,
                data = excluded.data,
                updated_at = excluded.updated_at
            "#,
            params![
                repo.id.to_string(),
                repo.project_id.to_string(),
                repo.owner.as_str(),
                repo.repo.as_str(),
                if repo.enrolled { 1_i64 } else { 0_i64 },
                to_json(repo)?,
                fmt_ts(&repo.created_at),
                fmt_ts(&repo.updated_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn list_repo_project_repos(
        &self,
        project_id: Uuid,
    ) -> Result<Vec<RepoProjectRepo>, DatabaseError> {
        list_json_by_project(
            self,
            "repo_project_repos",
            project_id,
            "owner ASC, repo ASC",
        )
        .await
    }

    async fn upsert_repo_project_task(&self, task: &RepoProjectTask) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
            INSERT INTO repo_project_tasks (
                id, project_id, repo_id, state, priority, data, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ON CONFLICT(id) DO UPDATE SET
                project_id = excluded.project_id,
                repo_id = excluded.repo_id,
                state = excluded.state,
                priority = excluded.priority,
                data = excluded.data,
                updated_at = excluded.updated_at
            "#,
            params![
                task.id.to_string(),
                task.project_id.to_string(),
                task.repo_id.to_string(),
                json_label(&task.state, "queued"),
                task.priority as i64,
                to_json(task)?,
                fmt_ts(&task.created_at),
                fmt_ts(&task.updated_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn get_repo_project_task(
        &self,
        id: Uuid,
    ) -> Result<Option<RepoProjectTask>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT data FROM repo_project_tasks WHERE id = ?1",
                params![id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(row_json_to(&row, 0)?)),
            None => Ok(None),
        }
    }

    async fn list_repo_project_tasks(
        &self,
        project_id: Uuid,
    ) -> Result<Vec<RepoProjectTask>, DatabaseError> {
        list_json_by_project(
            self,
            "repo_project_tasks",
            project_id,
            "priority DESC, updated_at DESC",
        )
        .await
    }

    async fn upsert_repo_worker_run(&self, run: &RepoWorkerRun) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
            INSERT INTO repo_worker_runs (
                id, project_id, task_id, state, data, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(id) DO UPDATE SET
                project_id = excluded.project_id,
                task_id = excluded.task_id,
                state = excluded.state,
                data = excluded.data,
                updated_at = excluded.updated_at
            "#,
            params![
                run.id.to_string(),
                run.project_id.to_string(),
                run.task_id.to_string(),
                json_label(&run.state, "queued"),
                to_json(run)?,
                fmt_ts(&run.created_at),
                fmt_ts(&run.updated_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn list_repo_worker_runs(
        &self,
        project_id: Uuid,
    ) -> Result<Vec<RepoWorkerRun>, DatabaseError> {
        list_json_by_project(self, "repo_worker_runs", project_id, "updated_at DESC").await
    }

    async fn append_repo_project_event(
        &self,
        event: &RepoProjectEvent,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
            INSERT OR IGNORE INTO repo_project_events (
                id, project_id, task_id, event_kind, data, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            params![
                event.id.to_string(),
                event.project_id.to_string(),
                opt_text_owned(event.task_id.map(|id| id.to_string())),
                json_label(&event.kind, "project_created"),
                to_json(event)?,
                fmt_ts(&event.created_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn list_repo_project_events(
        &self,
        project_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RepoProjectEvent>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT data FROM repo_project_events WHERE project_id = ?1 ORDER BY created_at DESC LIMIT ?2",
                params![project_id.to_string(), limit.clamp(1, 500)],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut items = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            items.push(row_json_to(&row, 0)?);
        }
        Ok(items)
    }

    async fn upsert_repo_merge_gate_decision(
        &self,
        project_id: Uuid,
        task_id: Uuid,
        decision: &MergeGateDecision,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
            INSERT INTO repo_merge_gate_decisions (
                project_id, task_id, data, updated_at
            ) VALUES (?1, ?2, ?3, datetime('now'))
            ON CONFLICT(project_id, task_id) DO UPDATE SET
                data = excluded.data,
                updated_at = excluded.updated_at
            "#,
            params![
                project_id.to_string(),
                task_id.to_string(),
                to_json(decision)?
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn list_repo_merge_gate_decisions(
        &self,
        project_id: Uuid,
    ) -> Result<Vec<(Uuid, MergeGateDecision)>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT task_id, data FROM repo_merge_gate_decisions WHERE project_id = ?1 ORDER BY updated_at DESC",
                params![project_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut items = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let task_id = row
                .get::<String>(0)
                .map_err(|e| DatabaseError::Serialization(e.to_string()))?
                .parse()
                .map_err(|e| DatabaseError::Serialization(format!("invalid task_id: {e}")))?;
            items.push((task_id, row_json_to(&row, 1)?));
        }
        Ok(items)
    }

    async fn record_repo_webhook_delivery(
        &self,
        delivery: &RepoWebhookDelivery,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let count = conn
            .execute(
                r#"
                INSERT OR IGNORE INTO repo_webhook_deliveries (
                    delivery_id, event, data, received_at
                ) VALUES (?1, ?2, ?3, ?4)
                "#,
                params![
                    delivery.delivery_id.as_str(),
                    delivery.event.as_str(),
                    to_json(delivery)?,
                    fmt_ts(&delivery.received_at),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(count > 0)
    }

    async fn get_repo_webhook_delivery(
        &self,
        delivery_id: &str,
    ) -> Result<Option<RepoWebhookDelivery>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT data FROM repo_webhook_deliveries WHERE delivery_id = ?1 LIMIT 1",
                params![delivery_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        else {
            return Ok(None);
        };
        row_json_to(&row, 0).map(Some)
    }

    async fn list_repo_webhook_deliveries(
        &self,
        limit: i64,
    ) -> Result<Vec<RepoWebhookDelivery>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT data FROM repo_webhook_deliveries ORDER BY received_at DESC LIMIT ?1",
                params![limit.clamp(1, 1000)],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut items = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            items.push(row_json_to(&row, 0)?);
        }
        Ok(items)
    }

    async fn upsert_repo_project_run(&self, run: &RepoProjectRun) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
            INSERT INTO repo_project_runs (id, project_id, state, data, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(id) DO UPDATE SET
                state = excluded.state,
                data = excluded.data
            "#,
            params![
                run.id.to_string(),
                run.project_id.to_string(),
                json_label(&run.state, "queued"),
                to_json(run)?,
                fmt_ts(&run.created_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn get_repo_project_run(
        &self,
        id: Uuid,
    ) -> Result<Option<RepoProjectRun>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT data FROM repo_project_runs WHERE id = ?1",
                params![id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(row_json_to(&row, 0)?)),
            None => Ok(None),
        }
    }

    async fn list_repo_project_runs(
        &self,
        project_id: Uuid,
    ) -> Result<Vec<RepoProjectRun>, DatabaseError> {
        list_json_by_project(self, "repo_project_runs", project_id, "created_at DESC").await
    }
}

async fn list_json_by_project<T>(
    backend: &LibSqlBackend,
    table: &str,
    project_id: Uuid,
    order_by: &str,
) -> Result<Vec<T>, DatabaseError>
where
    T: serde::de::DeserializeOwned,
{
    let sql = format!("SELECT data FROM {table} WHERE project_id = ?1 ORDER BY {order_by}");
    let conn = backend.connect().await?;
    let mut rows = conn
        .query(&sql, params![project_id.to_string()])
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
    let mut items = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
    {
        items.push(row_json_to(&row, 0)?);
    }
    Ok(items)
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<String, DatabaseError> {
    serde_json::to_string(value).map_err(|e| DatabaseError::Serialization(e.to_string()))
}

fn json_label<T: serde::Serialize>(value: &T, fallback: &str) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| fallback.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;
    use chrono::{TimeZone, Utc};
    use thinclaw_repo_projects::{
        CodingBackend, MergeGateDenialReason, MergeMethod, ProjectPolicy, RepoProjectEventKind,
        RepoProjectRunState, RepoProjectState, RepoProjectTaskState, RepoWorkerRunState,
    };

    fn ts(offset_secs: i64) -> chrono::DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + offset_secs, 0)
            .single()
            .expect("test timestamp should be valid")
    }

    #[tokio::test]
    async fn file_backed_repo_project_state_survives_restart() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("repo-project-restart.db");
        let project_id = Uuid::new_v4();
        let repo_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();
        let run_id = Uuid::new_v4();
        let event_id = Uuid::new_v4();

        {
            let backend = LibSqlBackend::new_local(&db_path).await.expect("backend");
            backend.run_migrations().await.expect("migrations");

            let project = RepoProject {
                id: project_id,
                slug: "restart-smoke".to_string(),
                name: "Restart smoke".to_string(),
                state: RepoProjectState::Active,
                policy: ProjectPolicy {
                    auto_merge: true,
                    merge_method: MergeMethod::Squash,
                    default_coding_backend: CodingBackend::Worker,
                    ..ProjectPolicy::default()
                },
                description: Some("durable restart fixture".to_string()),
                current_run_id: Some(run_id),
                created_at: ts(0),
                updated_at: ts(10),
                started_at: Some(ts(5)),
                completed_at: None,
            };
            backend
                .create_repo_project(&project)
                .await
                .expect("create project");

            let repo = RepoProjectRepo {
                id: repo_id,
                project_id,
                owner: "owner".to_string(),
                repo: "repo".to_string(),
                github_repo_id: Some(42),
                installation_id: Some(99),
                default_branch: "main".to_string(),
                base_branch: Some("main".to_string()),
                enrolled: true,
                local_path: Some("owner__repo".to_string()),
                auth_mode: thinclaw_repo_projects::GitHubAuthMode::GitHubApp,
                metadata: serde_json::json!({ "fixture": true }),
                created_at: ts(1),
                updated_at: ts(11),
            };
            backend
                .upsert_repo_project_repo(&repo)
                .await
                .expect("upsert repo");

            let task = RepoProjectTask {
                id: task_id,
                project_id,
                repo_id,
                title: "Wait for CI".to_string(),
                body: Some("verify restart recovery".to_string()),
                state: RepoProjectTaskState::WaitingCi,
                coding_backend: CodingBackend::Worker,
                base_branch: "main".to_string(),
                branch_name: "thinclaw/restart-smoke/abcdef123456".to_string(),
                head_sha: Some("abc123".to_string()),
                pull_request_number: Some(7),
                pull_request_url: Some("https://github.com/owner/repo/pull/7".to_string()),
                github_issue_number: None,
                assigned_worker_id: Some("worker-1".to_string()),
                priority: 50,
                labels: vec!["smoke".to_string()],
                metadata: serde_json::json!({ "phase": "ci" }),
                created_at: ts(2),
                updated_at: ts(12),
                queued_at: Some(ts(2)),
                started_at: Some(ts(3)),
                completed_at: None,
            };
            backend
                .upsert_repo_project_task(&task)
                .await
                .expect("upsert task");

            let worker_run = RepoWorkerRun {
                id: run_id,
                project_id,
                project_run_id: run_id,
                repo_id,
                task_id,
                state: RepoWorkerRunState::Running,
                coding_backend: CodingBackend::Worker,
                worker_id: "worker-1".to_string(),
                branch_name: "thinclaw/restart-smoke/abcdef123456".to_string(),
                job_id: Some("job-1".to_string()),
                commit_sha: Some("abc123".to_string()),
                exit_code: None,
                summary: Some("waiting for CI".to_string()),
                metadata: serde_json::json!({ "restart": true }),
                created_at: ts(3),
                updated_at: ts(13),
                started_at: Some(ts(4)),
                completed_at: None,
            };
            backend
                .upsert_repo_worker_run(&worker_run)
                .await
                .expect("upsert worker run");

            let event = RepoProjectEvent {
                id: event_id,
                project_id,
                repo_id: Some(repo_id),
                task_id: Some(task_id),
                project_run_id: Some(run_id),
                worker_run_id: Some(run_id),
                kind: RepoProjectEventKind::WorkerRunStarted,
                message: "Worker started before restart".to_string(),
                details: serde_json::json!({ "job_id": "job-1" }),
                created_at: ts(4),
            };
            backend
                .append_repo_project_event(&event)
                .await
                .expect("append event");

            let decision = MergeGateDecision::denied(
                vec![MergeGateDenialReason::ChecksNotGreen],
                MergeMethod::Squash,
            );
            backend
                .upsert_repo_merge_gate_decision(project_id, task_id, &decision)
                .await
                .expect("upsert gate");
        }

        let restarted = LibSqlBackend::new_local(&db_path)
            .await
            .expect("restart backend");
        restarted
            .run_migrations()
            .await
            .expect("restart migrations");

        let project = restarted
            .get_repo_project(project_id)
            .await
            .expect("get project")
            .expect("project should survive restart");
        assert_eq!(project.state, RepoProjectState::Active);
        assert_eq!(project.current_run_id, Some(run_id));

        let repos = restarted
            .list_repo_project_repos(project_id)
            .await
            .expect("list repos");
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].installation_id, Some(99));

        let task = restarted
            .get_repo_project_task(task_id)
            .await
            .expect("get task")
            .expect("task should survive restart");
        assert_eq!(task.state, RepoProjectTaskState::WaitingCi);
        assert_eq!(task.pull_request_number, Some(7));

        let runs = restarted
            .list_repo_worker_runs(project_id)
            .await
            .expect("list worker runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].state, RepoWorkerRunState::Running);
        assert_eq!(runs[0].job_id.as_deref(), Some("job-1"));

        let events = restarted
            .list_repo_project_events(project_id, 10)
            .await
            .expect("list events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, event_id);
        assert_eq!(events[0].kind, RepoProjectEventKind::WorkerRunStarted);

        let gates = restarted
            .list_repo_merge_gate_decisions(project_id)
            .await
            .expect("list gates");
        assert_eq!(gates.len(), 1);
        assert_eq!(gates[0].0, task_id);
        assert!(!gates[0].1.approved);
        assert_eq!(
            gates[0].1.reasons,
            vec![MergeGateDenialReason::ChecksNotGreen]
        );
    }

    #[tokio::test]
    async fn webhook_delivery_dedup_and_project_run_round_trip() {
        let dir = tempfile::tempdir().expect("temp dir");
        let backend = LibSqlBackend::new_local(&dir.path().join("rp-extra.db"))
            .await
            .expect("backend");
        backend.run_migrations().await.expect("migrations");

        // Webhook delivery idempotency survives by primary key.
        let delivery = thinclaw_repo_projects::RepoWebhookDelivery {
            delivery_id: "delivery-1".to_string(),
            event: "pull_request".to_string(),
            action: Some("opened".to_string()),
            repository_full_name: Some("owner/repo".to_string()),
            installation_id: Some(42),
            raw_payload_base64: None,
            signature_header: None,
            received_at: ts(0),
        };
        assert!(
            backend
                .record_repo_webhook_delivery(&delivery)
                .await
                .expect("record delivery"),
            "first delivery is new"
        );
        assert!(
            !backend
                .record_repo_webhook_delivery(&delivery)
                .await
                .expect("record delivery again"),
            "redelivery is a duplicate"
        );
        let deliveries = backend
            .list_repo_webhook_deliveries(10)
            .await
            .expect("list deliveries");
        assert_eq!(deliveries.len(), 1);
        assert_eq!(deliveries[0].event, "pull_request");
        assert_eq!(deliveries[0].installation_id, Some(42));
        let loaded = backend
            .get_repo_webhook_delivery("delivery-1")
            .await
            .expect("get delivery")
            .expect("delivery should exist");
        assert_eq!(loaded, delivery);
        assert!(
            backend
                .get_repo_webhook_delivery("missing-delivery")
                .await
                .expect("get missing delivery")
                .is_none()
        );

        // Project run upsert + status update round trip.
        let project_id = Uuid::new_v4();
        let project = RepoProject {
            id: project_id,
            slug: "runs".to_string(),
            name: "Runs".to_string(),
            state: RepoProjectState::Active,
            policy: ProjectPolicy::default(),
            description: None,
            current_run_id: None,
            created_at: ts(0),
            updated_at: ts(0),
            started_at: None,
            completed_at: None,
        };
        backend
            .create_repo_project(&project)
            .await
            .expect("project");

        let run_id = Uuid::new_v4();
        let mut run = RepoProjectRun {
            id: run_id,
            project_id,
            state: RepoProjectRunState::Running,
            trigger: "supervisor".to_string(),
            summary: None,
            tasks_seen: 0,
            tasks_queued: 0,
            tasks_completed: 0,
            tasks_failed: 0,
            metadata: serde_json::json!({}),
            created_at: ts(1),
            started_at: Some(ts(1)),
            completed_at: None,
        };
        backend
            .upsert_repo_project_run(&run)
            .await
            .expect("insert run");
        run.state = RepoProjectRunState::Completed;
        run.tasks_seen = 4;
        run.tasks_completed = 3;
        run.completed_at = Some(ts(2));
        backend
            .upsert_repo_project_run(&run)
            .await
            .expect("update run");

        let fetched = backend
            .get_repo_project_run(run_id)
            .await
            .expect("get run")
            .expect("run exists");
        assert_eq!(fetched.state, RepoProjectRunState::Completed);
        assert_eq!(fetched.tasks_completed, 3);
        assert_eq!(fetched.tasks_seen, 4);

        let runs = backend
            .list_repo_project_runs(project_id)
            .await
            .expect("list runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].id, run_id);
    }
}
