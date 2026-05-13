//! Root database adapter for the extracted agent learning/outcome port.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use thinclaw_agent::ports::{
    LearningEventQuery, LearningEventRecord, LearningOutcomesPort, OutcomeContractQuery,
    OutcomeContractRecord, OutcomeObservationRecord,
};
use uuid::Uuid;

use crate::db::Database;
use crate::error::DatabaseError;
use crate::history::{LearningEvent, OutcomeContract, OutcomeObservation};

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

fn learning_event_from_record(record: &LearningEventRecord) -> LearningEvent {
    LearningEvent {
        id: record.id.unwrap_or_else(Uuid::new_v4),
        user_id: record.user_id.clone(),
        actor_id: record.actor_id.clone(),
        channel: record.channel.clone(),
        thread_id: record.thread_id.clone(),
        conversation_id: uuid_field(&record.payload, "conversation_id"),
        message_id: uuid_field(&record.payload, "message_id"),
        job_id: uuid_field(&record.payload, "job_id"),
        event_type: record.event_type.clone(),
        source: string_field(&record.payload, "source", "agent"),
        payload: record.payload.clone(),
        metadata: Some(record.payload.get("metadata").cloned().unwrap_or_default()),
        created_at: record.created_at,
    }
}

fn learning_event_to_record(event: LearningEvent) -> LearningEventRecord {
    let mut payload = event.payload;
    insert_optional_uuid(&mut payload, "conversation_id", event.conversation_id);
    insert_optional_uuid(&mut payload, "message_id", event.message_id);
    insert_optional_uuid(&mut payload, "job_id", event.job_id);
    payload["source"] = serde_json::Value::String(event.source);
    if let Some(metadata) = event.metadata {
        payload["metadata"] = metadata;
    }

    LearningEventRecord {
        id: Some(event.id),
        user_id: event.user_id,
        actor_id: event.actor_id,
        channel: event.channel,
        thread_id: event.thread_id,
        event_type: event.event_type,
        payload,
        created_at: event.created_at,
    }
}

fn outcome_contract_from_record(record: &OutcomeContractRecord) -> OutcomeContract {
    let due_at = record.due_at.unwrap_or_else(Utc::now);
    OutcomeContract {
        id: record.id,
        user_id: record.user_id.clone(),
        actor_id: record.actor_id.clone(),
        channel: record.channel.clone(),
        thread_id: record.thread_id.clone(),
        source_kind: record.source_kind.clone(),
        source_id: record.source_id.clone(),
        contract_type: string_field(&record.payload, "contract_type", "generic"),
        status: record.status.clone(),
        summary: optional_string_field(&record.payload, "summary"),
        due_at,
        expires_at: datetime_field(&record.payload, "expires_at")
            .unwrap_or_else(|| due_at + ChronoDuration::hours(24)),
        final_verdict: optional_string_field(&record.payload, "final_verdict"),
        final_score: record
            .payload
            .get("final_score")
            .and_then(serde_json::Value::as_f64),
        evaluation_details: record
            .payload
            .get("evaluation_details")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        metadata: record.metadata.clone(),
        dedupe_key: optional_string_field(&record.payload, "dedupe_key").unwrap_or_else(|| {
            format!("{}:{}:{}", record.source_kind, record.source_id, record.id)
        }),
        claimed_at: datetime_field(&record.payload, "claimed_at"),
        evaluated_at: datetime_field(&record.payload, "evaluated_at"),
        created_at: record.created_at,
        updated_at: record.updated_at,
    }
}

fn outcome_contract_to_record(contract: OutcomeContract) -> OutcomeContractRecord {
    let payload = serde_json::json!({
        "contract_type": contract.contract_type,
        "summary": contract.summary,
        "expires_at": contract.expires_at,
        "final_verdict": contract.final_verdict,
        "final_score": contract.final_score,
        "evaluation_details": contract.evaluation_details,
        "dedupe_key": contract.dedupe_key,
        "claimed_at": contract.claimed_at,
        "evaluated_at": contract.evaluated_at,
    });

    OutcomeContractRecord {
        id: contract.id,
        user_id: contract.user_id,
        actor_id: contract.actor_id,
        channel: contract.channel,
        thread_id: contract.thread_id,
        source_kind: contract.source_kind,
        source_id: contract.source_id,
        status: contract.status,
        due_at: Some(contract.due_at),
        payload,
        metadata: contract.metadata,
        created_at: contract.created_at,
        updated_at: contract.updated_at,
    }
}

fn outcome_observation_from_record(record: &OutcomeObservationRecord) -> OutcomeObservation {
    OutcomeObservation {
        id: record.id,
        contract_id: record.contract_id,
        observation_kind: string_field(&record.result, "observation_kind", "generic"),
        polarity: string_field(&record.result, "polarity", "neutral"),
        weight: record
            .result
            .get("weight")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(1.0),
        summary: optional_string_field(&record.result, "summary"),
        evidence: record.result.clone(),
        fingerprint: record
            .fingerprint
            .clone()
            .unwrap_or_else(|| format!("{}:{}", record.contract_id, record.id)),
        observed_at: record.observed_at,
        created_at: record.observed_at,
    }
}

fn outcome_observation_to_record(observation: OutcomeObservation) -> OutcomeObservationRecord {
    OutcomeObservationRecord {
        id: observation.id,
        contract_id: observation.contract_id,
        observed_at: observation.observed_at,
        evaluator: "outcome".to_string(),
        result: serde_json::json!({
            "observation_kind": observation.observation_kind,
            "polarity": observation.polarity,
            "weight": observation.weight,
            "summary": observation.summary,
            "evidence": observation.evidence,
            "created_at": observation.created_at,
        }),
        fingerprint: Some(observation.fingerprint),
    }
}

fn string_field(payload: &serde_json::Value, key: &str, default: &str) -> String {
    payload
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or(default)
        .to_string()
}

fn optional_string_field(payload: &serde_json::Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string)
}

fn uuid_field(payload: &serde_json::Value, key: &str) -> Option<Uuid> {
    payload
        .get(key)
        .and_then(serde_json::Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
}

fn insert_optional_uuid(payload: &mut serde_json::Value, key: &str, value: Option<Uuid>) {
    if let Some(value) = value
        && let Some(object) = payload.as_object_mut()
    {
        object.insert(
            key.to_string(),
            serde_json::Value::String(value.to_string()),
        );
    }
}

fn datetime_field(payload: &serde_json::Value, key: &str) -> Option<DateTime<Utc>> {
    payload
        .get(key)
        .and_then(serde_json::Value::as_str)
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_contract_round_trip_preserves_required_payload_fields() {
        let now = Utc::now();
        let record = OutcomeContractRecord {
            id: Uuid::new_v4(),
            user_id: "user-1".to_string(),
            actor_id: Some("actor-1".to_string()),
            channel: Some("web".to_string()),
            thread_id: Some("thread-1".to_string()),
            source_kind: "turn".to_string(),
            source_id: "turn-1".to_string(),
            status: "open".to_string(),
            due_at: Some(now),
            payload: serde_json::json!({
                "contract_type": "turn",
                "summary": "check outcome",
                "dedupe_key": "turn:1",
            }),
            metadata: serde_json::json!({"k": "v"}),
            created_at: now,
            updated_at: now,
        };

        let history = outcome_contract_from_record(&record);
        assert_eq!(history.contract_type, "turn");
        assert_eq!(history.summary.as_deref(), Some("check outcome"));
        assert_eq!(history.dedupe_key, "turn:1");

        let restored = outcome_contract_to_record(history);
        assert_eq!(restored.payload["contract_type"], "turn");
        assert_eq!(restored.metadata["k"], "v");
    }
}
