//! Compatibility adapters for extracted routine tools.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use thinclaw_agent::ports::RoutineStorePort;
use thinclaw_agent::routine::{
    Routine, RoutineEvent, RoutineEventEvaluation, RoutineRun, RoutineTrigger,
    RoutineTriggerDecision, RunStatus,
};
pub use thinclaw_agent::routine_tools::{
    RoutineCreateTool, RoutineDeleteTool, RoutineEngineControlPort, RoutineHistoryTool,
    RoutineListTool, RoutineOutcomeObserver, RoutineUpdateTool,
};
use thinclaw_types::error::DatabaseError;

use crate::agent::routine_engine::RoutineEngine;
use crate::db::Database;

pub struct RootRoutineStorePort {
    inner: Arc<dyn Database>,
}

impl RootRoutineStorePort {
    pub fn shared(inner: Arc<dyn Database>) -> Arc<dyn RoutineStorePort> {
        Arc::new(Self { inner })
    }
}

#[async_trait]
impl RoutineStorePort for RootRoutineStorePort {
    async fn create_routine(&self, routine: &Routine) -> Result<(), DatabaseError> {
        self.inner.create_routine(routine).await
    }

    async fn get_routine(&self, id: Uuid) -> Result<Option<Routine>, DatabaseError> {
        self.inner.get_routine(id).await
    }

    async fn get_routine_by_name(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<Option<Routine>, DatabaseError> {
        self.inner.get_routine_by_name(user_id, name).await
    }

    async fn list_routines(&self, user_id: &str) -> Result<Vec<Routine>, DatabaseError> {
        self.inner.list_routines(user_id).await
    }

    async fn list_event_routines(&self) -> Result<Vec<Routine>, DatabaseError> {
        self.inner.list_event_routines().await
    }

    async fn get_routine_event_cache_version(&self) -> Result<i64, DatabaseError> {
        self.inner.get_routine_event_cache_version().await
    }

    async fn list_due_cron_routines(&self) -> Result<Vec<Routine>, DatabaseError> {
        self.inner.list_due_cron_routines().await
    }

    async fn update_routine(&self, routine: &Routine) -> Result<(), DatabaseError> {
        self.inner.update_routine(routine).await
    }

    async fn update_routine_runtime(
        &self,
        id: Uuid,
        last_run_at: DateTime<Utc>,
        next_fire_at: Option<DateTime<Utc>>,
        run_count: u64,
        consecutive_failures: u32,
        state: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.inner
            .update_routine_runtime(
                id,
                last_run_at,
                next_fire_at,
                run_count,
                consecutive_failures,
                state,
            )
            .await
    }

    async fn delete_routine(&self, id: Uuid) -> Result<bool, DatabaseError> {
        self.inner.delete_routine(id).await
    }

    async fn create_routine_run(&self, run: &RoutineRun) -> Result<(), DatabaseError> {
        self.inner.create_routine_run(run).await
    }

    async fn complete_routine_run(
        &self,
        id: Uuid,
        status: RunStatus,
        result_summary: Option<&str>,
        tokens_used: Option<i32>,
    ) -> Result<(), DatabaseError> {
        self.inner
            .complete_routine_run(id, status, result_summary, tokens_used)
            .await
    }

    async fn list_routine_runs(
        &self,
        routine_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RoutineRun>, DatabaseError> {
        self.inner.list_routine_runs(routine_id, limit).await
    }

    async fn count_running_routine_runs(&self, routine_id: Uuid) -> Result<i64, DatabaseError> {
        self.inner.count_running_routine_runs(routine_id).await
    }

    async fn count_all_running_routine_runs(&self) -> Result<i64, DatabaseError> {
        self.inner.count_all_running_routine_runs().await
    }

    async fn link_routine_run_to_job(
        &self,
        run_id: Uuid,
        job_id: Uuid,
    ) -> Result<(), DatabaseError> {
        self.inner.link_routine_run_to_job(run_id, job_id).await
    }

    async fn cleanup_stale_routine_runs(&self) -> Result<u64, DatabaseError> {
        self.inner.cleanup_stale_routine_runs().await
    }

    async fn delete_routine_runs(&self, routine_id: Uuid) -> Result<u64, DatabaseError> {
        self.inner.delete_routine_runs(routine_id).await
    }

    async fn delete_all_routine_runs(&self) -> Result<u64, DatabaseError> {
        self.inner.delete_all_routine_runs().await
    }

    async fn create_routine_event(
        &self,
        event: &RoutineEvent,
    ) -> Result<RoutineEvent, DatabaseError> {
        self.inner.create_routine_event(event).await
    }

    async fn claim_routine_event(
        &self,
        id: Uuid,
        worker_id: &str,
        stale_before: DateTime<Utc>,
    ) -> Result<Option<RoutineEvent>, DatabaseError> {
        self.inner
            .claim_routine_event(id, worker_id, stale_before)
            .await
    }

    async fn release_routine_event(
        &self,
        id: Uuid,
        diagnostics: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.inner.release_routine_event(id, diagnostics).await
    }

    async fn list_pending_routine_events(
        &self,
        stale_before: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<RoutineEvent>, DatabaseError> {
        self.inner
            .list_pending_routine_events(stale_before, limit)
            .await
    }

    async fn complete_routine_event(
        &self,
        id: Uuid,
        processed_at: DateTime<Utc>,
        matched_routines: u32,
        fired_routines: u32,
        diagnostics: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.inner
            .complete_routine_event(
                id,
                processed_at,
                matched_routines,
                fired_routines,
                diagnostics,
            )
            .await
    }

    async fn fail_routine_event(
        &self,
        id: Uuid,
        processed_at: DateTime<Utc>,
        error_message: &str,
    ) -> Result<(), DatabaseError> {
        self.inner
            .fail_routine_event(id, processed_at, error_message)
            .await
    }

    async fn list_routine_events_for_actor(
        &self,
        user_id: &str,
        actor_id: &str,
        limit: i64,
    ) -> Result<Vec<RoutineEvent>, DatabaseError> {
        self.inner
            .list_routine_events_for_actor(user_id, actor_id, limit)
            .await
    }

    async fn upsert_routine_event_evaluation(
        &self,
        evaluation: &RoutineEventEvaluation,
    ) -> Result<(), DatabaseError> {
        self.inner.upsert_routine_event_evaluation(evaluation).await
    }

    async fn list_routine_event_evaluations_for_event(
        &self,
        event_id: Uuid,
    ) -> Result<Vec<RoutineEventEvaluation>, DatabaseError> {
        self.inner
            .list_routine_event_evaluations_for_event(event_id)
            .await
    }

    async fn list_routine_event_evaluations(
        &self,
        routine_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RoutineEventEvaluation>, DatabaseError> {
        self.inner
            .list_routine_event_evaluations(routine_id, limit)
            .await
    }

    async fn routine_run_exists_for_trigger_key(
        &self,
        routine_id: Uuid,
        trigger_key: &str,
    ) -> Result<bool, DatabaseError> {
        self.inner
            .routine_run_exists_for_trigger_key(routine_id, trigger_key)
            .await
    }

    async fn enqueue_routine_trigger(&self, trigger: &RoutineTrigger) -> Result<(), DatabaseError> {
        self.inner.enqueue_routine_trigger(trigger).await
    }

    async fn claim_routine_triggers(
        &self,
        worker_id: &str,
        stale_before: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<RoutineTrigger>, DatabaseError> {
        self.inner
            .claim_routine_triggers(worker_id, stale_before, limit)
            .await
    }

    async fn release_routine_trigger(
        &self,
        id: Uuid,
        diagnostics: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.inner.release_routine_trigger(id, diagnostics).await
    }

    async fn complete_routine_trigger(
        &self,
        id: Uuid,
        processed_at: DateTime<Utc>,
        decision: RoutineTriggerDecision,
        diagnostics: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.inner
            .complete_routine_trigger(id, processed_at, decision, diagnostics)
            .await
    }

    async fn fail_routine_trigger(
        &self,
        id: Uuid,
        processed_at: DateTime<Utc>,
        error_message: &str,
    ) -> Result<(), DatabaseError> {
        self.inner
            .fail_routine_trigger(id, processed_at, error_message)
            .await
    }

    async fn list_routine_triggers(
        &self,
        routine_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RoutineTrigger>, DatabaseError> {
        self.inner.list_routine_triggers(routine_id, limit).await
    }
}

#[async_trait]
impl RoutineEngineControlPort for RoutineEngine {
    async fn refresh_event_cache(&self) {
        RoutineEngine::refresh_event_cache(self).await;
    }
}

pub struct RootRoutineOutcomeObserver {
    store: Arc<dyn Database>,
}

impl RootRoutineOutcomeObserver {
    pub fn shared(store: Arc<dyn Database>) -> Arc<dyn RoutineOutcomeObserver> {
        Arc::new(Self { store })
    }
}

#[async_trait]
impl RoutineOutcomeObserver for RootRoutineOutcomeObserver {
    async fn observe_state_change(&self, routine: &Routine, event_type: &str) {
        let _ =
            crate::agent::outcomes::observe_routine_state_change(&self.store, routine, event_type)
                .await;
    }
}
