//! postgres: conversation_store.

use super::*;

#[async_trait]
impl ConversationStore for PgBackend {
    async fn create_conversation(
        &self,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .create_conversation(channel, user_id, thread_id)
            .await
    }

    async fn touch_conversation(&self, id: Uuid) -> Result<(), DatabaseError> {
        self.store.touch_conversation(id).await
    }

    async fn add_conversation_message(
        &self,
        conversation_id: Uuid,
        role: &str,
        content: &str,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .add_conversation_message(conversation_id, role, content)
            .await
    }

    async fn add_conversation_message_with_attribution(
        &self,
        conversation_id: Uuid,
        role: &str,
        content: &str,
        actor_id: Option<&str>,
        actor_display_name: Option<&str>,
        raw_sender_id: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .add_conversation_message_with_attribution(
                conversation_id,
                role,
                content,
                actor_id,
                actor_display_name,
                raw_sender_id,
                metadata,
            )
            .await
    }

    async fn ensure_conversation(
        &self,
        id: Uuid,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
    ) -> Result<(), DatabaseError> {
        self.store
            .ensure_conversation(id, channel, user_id, thread_id)
            .await
    }

    async fn list_conversations_with_preview(
        &self,
        user_id: &str,
        channel: &str,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError> {
        self.store
            .list_conversations_with_preview(user_id, channel, limit)
            .await
    }

    async fn infer_primary_user_id_for_channel(
        &self,
        channel: &str,
    ) -> Result<Option<String>, DatabaseError> {
        self.store.infer_primary_user_id_for_channel(channel).await
    }

    async fn get_or_create_assistant_conversation(
        &self,
        user_id: &str,
        channel: &str,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .get_or_create_assistant_conversation(user_id, channel)
            .await
    }

    async fn create_conversation_with_metadata(
        &self,
        channel: &str,
        user_id: &str,
        metadata: &serde_json::Value,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .create_conversation_with_metadata(channel, user_id, metadata)
            .await
    }

    async fn update_conversation_identity(
        &self,
        id: Uuid,
        principal_id: Option<&str>,
        actor_id: Option<&str>,
        conversation_scope_id: Option<Uuid>,
        conversation_kind: thinclaw_history::ConversationKind,
        stable_external_conversation_key: Option<&str>,
    ) -> Result<(), DatabaseError> {
        self.store
            .update_conversation_identity(
                id,
                principal_id,
                actor_id,
                conversation_scope_id,
                conversation_kind,
                stable_external_conversation_key,
            )
            .await
    }

    async fn set_conversation_handoff_metadata(
        &self,
        id: Uuid,
        handoff: &thinclaw_history::ConversationHandoffMetadata,
    ) -> Result<(), DatabaseError> {
        self.store
            .set_conversation_handoff_metadata(id, handoff)
            .await
    }

    async fn list_actor_conversations_for_recall(
        &self,
        principal_id: &str,
        actor_id: &str,
        include_group_history: bool,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError> {
        self.store
            .list_actor_conversations_for_recall(
                principal_id,
                actor_id,
                include_group_history,
                limit,
            )
            .await
    }

    async fn list_conversation_messages_paginated(
        &self,
        conversation_id: Uuid,
        before: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<(Vec<ConversationMessage>, bool), DatabaseError> {
        self.store
            .list_conversation_messages_paginated(conversation_id, before, limit)
            .await
    }

    async fn update_conversation_metadata_field(
        &self,
        id: Uuid,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.store
            .update_conversation_metadata_field(id, key, value)
            .await
    }

    async fn get_conversation_metadata(
        &self,
        id: Uuid,
    ) -> Result<Option<serde_json::Value>, DatabaseError> {
        self.store.get_conversation_metadata(id).await
    }

    async fn list_conversation_messages(
        &self,
        conversation_id: Uuid,
    ) -> Result<Vec<ConversationMessage>, DatabaseError> {
        self.store.list_conversation_messages(conversation_id).await
    }

    async fn search_conversation_messages(
        &self,
        user_id: &str,
        query: &str,
        actor_id: Option<&str>,
        channel: Option<&str>,
        thread_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<SessionSearchHit>, DatabaseError> {
        self.store
            .search_conversation_messages(user_id, query, actor_id, channel, thread_id, limit)
            .await
    }

    async fn list_conversation_messages_for_learning(
        &self,
        user_id: &str,
        actor_id: Option<&str>,
        channel: Option<&str>,
        thread_id: Option<&str>,
        role: Option<&str>,
        limit: i64,
    ) -> Result<Vec<SessionSearchHit>, DatabaseError> {
        self.store
            .list_conversation_messages_for_learning(
                user_id, actor_id, channel, thread_id, role, limit,
            )
            .await
    }

    async fn insert_learning_event(&self, event: &LearningEvent) -> Result<Uuid, DatabaseError> {
        self.store.insert_learning_event(event).await
    }

    async fn list_learning_events(
        &self,
        user_id: &str,
        actor_id: Option<&str>,
        channel: Option<&str>,
        thread_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningEvent>, DatabaseError> {
        self.store
            .list_learning_events(user_id, actor_id, channel, thread_id, limit)
            .await
    }

    async fn insert_learning_evaluation(
        &self,
        evaluation: &LearningEvaluation,
    ) -> Result<Uuid, DatabaseError> {
        self.store.insert_learning_evaluation(evaluation).await
    }

    async fn list_learning_evaluations(
        &self,
        user_id: &str,
        limit: i64,
    ) -> Result<Vec<LearningEvaluation>, DatabaseError> {
        self.store.list_learning_evaluations(user_id, limit).await
    }

    async fn insert_learning_candidate(
        &self,
        candidate: &LearningCandidate,
    ) -> Result<Uuid, DatabaseError> {
        self.store.insert_learning_candidate(candidate).await
    }

    async fn list_learning_candidates(
        &self,
        user_id: &str,
        candidate_type: Option<&str>,
        risk_tier: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningCandidate>, DatabaseError> {
        self.store
            .list_learning_candidates(user_id, candidate_type, risk_tier, limit)
            .await
    }

    async fn update_learning_candidate_proposal(
        &self,
        candidate_id: Uuid,
        proposal: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.store
            .update_learning_candidate_proposal(candidate_id, proposal)
            .await
    }

    async fn insert_learning_artifact_version(
        &self,
        version: &LearningArtifactVersion,
    ) -> Result<Uuid, DatabaseError> {
        self.store.insert_learning_artifact_version(version).await
    }

    async fn list_learning_artifact_versions(
        &self,
        user_id: &str,
        artifact_type: Option<&str>,
        artifact_name: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningArtifactVersion>, DatabaseError> {
        self.store
            .list_learning_artifact_versions(user_id, artifact_type, artifact_name, limit)
            .await
    }

    async fn insert_learning_feedback(
        &self,
        feedback: &LearningFeedbackRecord,
    ) -> Result<Uuid, DatabaseError> {
        self.store.insert_learning_feedback(feedback).await
    }

    async fn list_learning_feedback(
        &self,
        user_id: &str,
        target_type: Option<&str>,
        target_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningFeedbackRecord>, DatabaseError> {
        self.store
            .list_learning_feedback(user_id, target_type, target_id, limit)
            .await
    }

    async fn insert_learning_rollback(
        &self,
        rollback: &LearningRollbackRecord,
    ) -> Result<Uuid, DatabaseError> {
        self.store.insert_learning_rollback(rollback).await
    }

    async fn list_learning_rollbacks(
        &self,
        user_id: &str,
        artifact_type: Option<&str>,
        artifact_name: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningRollbackRecord>, DatabaseError> {
        self.store
            .list_learning_rollbacks(user_id, artifact_type, artifact_name, limit)
            .await
    }

    async fn insert_learning_code_proposal(
        &self,
        proposal: &LearningCodeProposal,
    ) -> Result<Uuid, DatabaseError> {
        self.store.insert_learning_code_proposal(proposal).await
    }

    async fn get_learning_code_proposal(
        &self,
        user_id: &str,
        proposal_id: Uuid,
    ) -> Result<Option<LearningCodeProposal>, DatabaseError> {
        self.store
            .get_learning_code_proposal(user_id, proposal_id)
            .await
    }

    async fn list_learning_code_proposals(
        &self,
        user_id: &str,
        status: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningCodeProposal>, DatabaseError> {
        self.store
            .list_learning_code_proposals(user_id, status, limit)
            .await
    }

    async fn update_learning_code_proposal(
        &self,
        proposal_id: Uuid,
        status: &str,
        branch_name: Option<&str>,
        pr_url: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<(), DatabaseError> {
        self.store
            .update_learning_code_proposal(proposal_id, status, branch_name, pr_url, metadata)
            .await
    }

    async fn insert_outcome_contract(
        &self,
        contract: &OutcomeContract,
    ) -> Result<Uuid, DatabaseError> {
        self.store.insert_outcome_contract(contract).await
    }

    async fn get_outcome_contract(
        &self,
        user_id: &str,
        contract_id: Uuid,
    ) -> Result<Option<OutcomeContract>, DatabaseError> {
        self.store.get_outcome_contract(user_id, contract_id).await
    }

    async fn list_outcome_contracts(
        &self,
        query: &OutcomeContractQuery,
    ) -> Result<Vec<OutcomeContract>, DatabaseError> {
        self.store.list_outcome_contracts(query).await
    }

    async fn claim_due_outcome_contracts(
        &self,
        limit: i64,
        now: DateTime<Utc>,
    ) -> Result<Vec<OutcomeContract>, DatabaseError> {
        self.store.claim_due_outcome_contracts(limit, now).await
    }

    async fn claim_due_outcome_contracts_for_user(
        &self,
        user_id: &str,
        limit: i64,
        now: DateTime<Utc>,
    ) -> Result<Vec<OutcomeContract>, DatabaseError> {
        self.store
            .claim_due_outcome_contracts_for_user(Some(user_id), limit, now)
            .await
    }

    async fn update_outcome_contract(
        &self,
        contract: &OutcomeContract,
    ) -> Result<(), DatabaseError> {
        self.store.update_outcome_contract(contract).await
    }

    async fn outcome_summary_stats(
        &self,
        user_id: &str,
    ) -> Result<OutcomeSummaryStats, DatabaseError> {
        self.store.outcome_summary_stats(user_id).await
    }

    async fn list_users_with_pending_outcome_work(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<OutcomePendingUser>, DatabaseError> {
        self.store.list_users_with_pending_outcome_work(now).await
    }

    async fn outcome_evaluator_health(
        &self,
        user_id: &str,
        now: DateTime<Utc>,
    ) -> Result<OutcomeEvaluatorHealth, DatabaseError> {
        self.store.outcome_evaluator_health(user_id, now).await
    }

    async fn insert_outcome_observation(
        &self,
        observation: &OutcomeObservation,
    ) -> Result<Uuid, DatabaseError> {
        self.store.insert_outcome_observation(observation).await
    }

    async fn list_outcome_observations(
        &self,
        contract_id: Uuid,
    ) -> Result<Vec<OutcomeObservation>, DatabaseError> {
        self.store.list_outcome_observations(contract_id).await
    }

    async fn conversation_belongs_to_user(
        &self,
        conversation_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError> {
        self.store
            .conversation_belongs_to_user(conversation_id, user_id)
            .await
    }

    async fn conversation_belongs_to_actor(
        &self,
        conversation_id: Uuid,
        principal_id: &str,
        actor_id: &str,
    ) -> Result<bool, DatabaseError> {
        self.store
            .conversation_belongs_to_actor(conversation_id, principal_id, actor_id)
            .await
    }

    async fn delete_conversation(&self, id: Uuid) -> Result<bool, DatabaseError> {
        self.store.delete_conversation(id).await
    }

    async fn delete_conversation_messages(
        &self,
        conversation_id: Uuid,
    ) -> Result<u64, DatabaseError> {
        self.store
            .delete_conversation_messages(conversation_id)
            .await
    }
}

// ==================== JobStore ====================
