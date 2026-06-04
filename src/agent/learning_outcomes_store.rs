//! Root database adapter for the extracted agent learning/outcome port.

use std::sync::Arc;

use async_trait::async_trait;
use thinclaw_agent::learning_outcome_records::{
    learning_event_from_record, learning_event_to_record, outcome_contract_from_record,
    outcome_contract_to_record, outcome_observation_from_record, outcome_observation_to_record,
};
use thinclaw_agent::ports::{
    LearningEventQuery, LearningEventRecord, LearningOutcomesPort, OutcomeContractQuery,
    OutcomeContractRecord, OutcomeObservationRecord,
};
use uuid::Uuid;

use crate::db::Database;
use crate::error::DatabaseError;

pub struct RootLearningOutcomesPort {
    store: Arc<dyn Database>,
}

impl RootLearningOutcomesPort {
    pub fn shared(store: Arc<dyn Database>) -> Arc<dyn LearningOutcomesPort> {
        Arc::new(Self { store })
    }
}

#[async_trait]
impl LearningOutcomesPort for RootLearningOutcomesPort {
    async fn record_action(
        &self,
        job_id: Uuid,
        action: &thinclaw_types::ActionRecord,
    ) -> Result<(), DatabaseError> {
        self.store.save_action(job_id, action).await
    }

    async fn record_learning_event(
        &self,
        event: &LearningEventRecord,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .insert_learning_event(&learning_event_from_record(event))
            .await
    }

    async fn list_learning_events(
        &self,
        query: &LearningEventQuery,
    ) -> Result<Vec<LearningEventRecord>, DatabaseError> {
        let events = self
            .store
            .list_learning_events(
                &query.user_id,
                query.actor_id.as_deref(),
                query.channel.as_deref(),
                query.thread_id.as_deref(),
                query.limit,
            )
            .await?;
        Ok(events.into_iter().map(learning_event_to_record).collect())
    }

    async fn insert_outcome_contract(
        &self,
        contract: &OutcomeContractRecord,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .insert_outcome_contract(&outcome_contract_from_record(contract))
            .await
    }

    async fn list_outcome_contracts(
        &self,
        query: &OutcomeContractQuery,
    ) -> Result<Vec<OutcomeContractRecord>, DatabaseError> {
        let contracts = self
            .store
            .list_outcome_contracts(&crate::history::OutcomeContractQuery {
                user_id: query.user_id.clone(),
                actor_id: query.actor_id.clone(),
                status: query.status.clone(),
                contract_type: None,
                source_kind: None,
                source_id: None,
                thread_id: query.thread_id.clone(),
                limit: query.limit,
            })
            .await?;
        Ok(contracts
            .into_iter()
            .map(outcome_contract_to_record)
            .collect())
    }

    async fn update_outcome_contract(
        &self,
        contract: &OutcomeContractRecord,
    ) -> Result<(), DatabaseError> {
        self.store
            .update_outcome_contract(&outcome_contract_from_record(contract))
            .await
    }

    async fn insert_outcome_observation(
        &self,
        observation: &OutcomeObservationRecord,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .insert_outcome_observation(&outcome_observation_from_record(observation))
            .await
    }

    async fn list_outcome_observations(
        &self,
        contract_id: Uuid,
    ) -> Result<Vec<OutcomeObservationRecord>, DatabaseError> {
        let observations = self.store.list_outcome_observations(contract_id).await?;
        Ok(observations
            .into_iter()
            .map(outcome_observation_to_record)
            .collect())
    }
}
