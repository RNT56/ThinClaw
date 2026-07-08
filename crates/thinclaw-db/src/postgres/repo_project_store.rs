//! postgres: repo_project_store.

use super::*;

#[async_trait]
impl RepoProjectStore for PgBackend {
    async fn create_repo_project(&self, project: &RepoProject) -> Result<(), DatabaseError> {
        self.store.create_repo_project(project).await
    }

    async fn get_repo_project(&self, id: Uuid) -> Result<Option<RepoProject>, DatabaseError> {
        self.store.get_repo_project(id).await
    }

    async fn list_repo_projects(&self) -> Result<Vec<RepoProject>, DatabaseError> {
        self.store.list_repo_projects().await
    }

    async fn update_repo_project(&self, project: &RepoProject) -> Result<(), DatabaseError> {
        self.store.update_repo_project(project).await
    }

    async fn delete_repo_project(&self, id: Uuid) -> Result<bool, DatabaseError> {
        self.store.delete_repo_project(id).await
    }

    async fn upsert_repo_project_repo(&self, repo: &RepoProjectRepo) -> Result<(), DatabaseError> {
        self.store.upsert_repo_project_repo(repo).await
    }

    async fn list_repo_project_repos(
        &self,
        project_id: Uuid,
    ) -> Result<Vec<RepoProjectRepo>, DatabaseError> {
        self.store.list_repo_project_repos(project_id).await
    }

    async fn upsert_repo_project_task(&self, task: &RepoProjectTask) -> Result<(), DatabaseError> {
        self.store.upsert_repo_project_task(task).await
    }

    async fn get_repo_project_task(
        &self,
        id: Uuid,
    ) -> Result<Option<RepoProjectTask>, DatabaseError> {
        self.store.get_repo_project_task(id).await
    }

    async fn list_repo_project_tasks(
        &self,
        project_id: Uuid,
    ) -> Result<Vec<RepoProjectTask>, DatabaseError> {
        self.store.list_repo_project_tasks(project_id).await
    }

    async fn upsert_repo_worker_run(&self, run: &RepoWorkerRun) -> Result<(), DatabaseError> {
        self.store.upsert_repo_worker_run(run).await
    }

    async fn list_repo_worker_runs(
        &self,
        project_id: Uuid,
    ) -> Result<Vec<RepoWorkerRun>, DatabaseError> {
        self.store.list_repo_worker_runs(project_id).await
    }

    async fn append_repo_project_event(
        &self,
        event: &RepoProjectEvent,
    ) -> Result<(), DatabaseError> {
        self.store.append_repo_project_event(event).await
    }

    async fn list_repo_project_events(
        &self,
        project_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RepoProjectEvent>, DatabaseError> {
        self.store.list_repo_project_events(project_id, limit).await
    }

    async fn upsert_repo_merge_gate_decision(
        &self,
        project_id: Uuid,
        task_id: Uuid,
        decision: &MergeGateDecision,
    ) -> Result<(), DatabaseError> {
        self.store
            .upsert_repo_merge_gate_decision(project_id, task_id, decision)
            .await
    }

    async fn list_repo_merge_gate_decisions(
        &self,
        project_id: Uuid,
    ) -> Result<Vec<(Uuid, MergeGateDecision)>, DatabaseError> {
        self.store.list_repo_merge_gate_decisions(project_id).await
    }

    async fn record_repo_webhook_delivery(
        &self,
        delivery: &RepoWebhookDelivery,
    ) -> Result<bool, DatabaseError> {
        self.store.record_repo_webhook_delivery(delivery).await
    }

    async fn get_repo_webhook_delivery(
        &self,
        delivery_id: &str,
    ) -> Result<Option<RepoWebhookDelivery>, DatabaseError> {
        self.store.get_repo_webhook_delivery(delivery_id).await
    }

    async fn list_repo_webhook_deliveries(
        &self,
        limit: i64,
    ) -> Result<Vec<RepoWebhookDelivery>, DatabaseError> {
        self.store.list_repo_webhook_deliveries(limit).await
    }

    async fn upsert_repo_project_run(&self, run: &RepoProjectRun) -> Result<(), DatabaseError> {
        self.store.upsert_repo_project_run(run).await
    }

    async fn get_repo_project_run(
        &self,
        id: Uuid,
    ) -> Result<Option<RepoProjectRun>, DatabaseError> {
        self.store.get_repo_project_run(id).await
    }

    async fn list_repo_project_runs(
        &self,
        project_id: Uuid,
    ) -> Result<Vec<RepoProjectRun>, DatabaseError> {
        self.store.list_repo_project_runs(project_id).await
    }
}

// ==================== ToolFailureStore ====================
