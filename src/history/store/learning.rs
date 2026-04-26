#![cfg_attr(not(feature = "postgres"), allow(unused_imports))]

use super::*;
#[cfg(feature = "postgres")]
impl Store {
    /// Persist a learning event for later retrieval and evaluation.
    pub async fn insert_learning_event(
        &self,
        event: &LearningEvent,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;
        let id = if event.id.is_nil() {
            Uuid::new_v4()
        } else {
            event.id
        };
        conn.execute(
            r#"
            INSERT INTO learning_events (
                id, user_id, actor_id, channel, thread_id, conversation_id,
                message_id, job_id, event_type, source, payload, metadata, created_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6,
                $7, $8, $9, $10, $11::jsonb, $12::jsonb, $13
            )
            "#,
            &[
                &id,
                &event.user_id,
                &event.actor_id,
                &event.channel,
                &event.thread_id,
                &event.conversation_id,
                &event.message_id,
                &event.job_id,
                &event.event_type,
                &event.source,
                &event.payload,
                &event
                    .metadata
                    .clone()
                    .unwrap_or_else(|| serde_json::json!({})),
                &event.created_at,
            ],
        )
        .await?;
        Ok(id)
    }

    /// List learning events for a user, optionally scoped to actor/channel/thread.
    pub async fn list_learning_events(
        &self,
        user_id: &str,
        actor_id: Option<&str>,
        channel: Option<&str>,
        thread_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningEvent>, DatabaseError> {
        if limit <= 0 {
            return Ok(Vec::new());
        }

        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT
                    id,
                    user_id,
                    actor_id,
                    channel,
                    thread_id,
                    conversation_id,
                    message_id,
                    job_id,
                    event_type,
                    source,
                    payload,
                    metadata,
                    created_at
                FROM learning_events
                WHERE user_id = $1
                  AND ($2::text IS NULL OR COALESCE(NULLIF(actor_id, ''), user_id) = $2)
                  AND ($3::text IS NULL OR channel = $3)
                  AND ($4::text IS NULL OR thread_id = $4)
                ORDER BY created_at DESC, id DESC
                LIMIT $5
                "#,
                &[&user_id, &actor_id, &channel, &thread_id, &limit],
            )
            .await?;

        Ok(rows.iter().map(learning_event_from_row).collect())
    }

    /// Persist an evaluator record linked to a learning event.
    pub async fn insert_learning_evaluation(
        &self,
        evaluation: &LearningEvaluation,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;
        let id = if evaluation.id.is_nil() {
            Uuid::new_v4()
        } else {
            evaluation.id
        };
        conn.execute(
            r#"
            INSERT INTO learning_evaluations (
                id, learning_event_id, user_id, evaluator, status, score, details, created_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7::jsonb, $8
            )
            "#,
            &[
                &id,
                &evaluation.learning_event_id,
                &evaluation.user_id,
                &evaluation.evaluator,
                &evaluation.status,
                &evaluation.score,
                &evaluation.details,
                &evaluation.created_at,
            ],
        )
        .await?;
        Ok(id)
    }

    /// List evaluator records for a user.
    pub async fn list_learning_evaluations(
        &self,
        user_id: &str,
        limit: i64,
    ) -> Result<Vec<LearningEvaluation>, DatabaseError> {
        if limit <= 0 {
            return Ok(Vec::new());
        }
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT id, learning_event_id, user_id, evaluator, status, score, details, created_at
                FROM learning_evaluations
                WHERE user_id = $1
                ORDER BY created_at DESC, id DESC
                LIMIT $2
                "#,
                &[&user_id, &limit],
            )
            .await?;
        Ok(rows.iter().map(learning_evaluation_from_row).collect())
    }

    /// Persist a distilled learning candidate.
    pub async fn insert_learning_candidate(
        &self,
        candidate: &LearningCandidate,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;
        let id = if candidate.id.is_nil() {
            Uuid::new_v4()
        } else {
            candidate.id
        };
        conn.execute(
            r#"
            INSERT INTO learning_candidates (
                id, learning_event_id, user_id, candidate_type, risk_tier, confidence,
                target_type, target_name, summary, proposal, created_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6,
                $7, $8, $9, $10::jsonb, $11
            )
            "#,
            &[
                &id,
                &candidate.learning_event_id,
                &candidate.user_id,
                &candidate.candidate_type,
                &candidate.risk_tier,
                &candidate.confidence,
                &candidate.target_type,
                &candidate.target_name,
                &candidate.summary,
                &candidate.proposal,
                &candidate.created_at,
            ],
        )
        .await?;
        Ok(id)
    }

    /// List candidates with optional type/tier filters.
    pub async fn list_learning_candidates(
        &self,
        user_id: &str,
        candidate_type: Option<&str>,
        risk_tier: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningCandidate>, DatabaseError> {
        if limit <= 0 {
            return Ok(Vec::new());
        }
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT
                    id, learning_event_id, user_id, candidate_type, risk_tier, confidence,
                    target_type, target_name, summary, proposal, created_at
                FROM learning_candidates
                WHERE user_id = $1
                  AND ($2::text IS NULL OR candidate_type = $2)
                  AND ($3::text IS NULL OR risk_tier = $3)
                ORDER BY created_at DESC, id DESC
                LIMIT $4
                "#,
                &[&user_id, &candidate_type, &risk_tier, &limit],
            )
            .await?;
        Ok(rows.iter().map(learning_candidate_from_row).collect())
    }

    /// Update the canonical proposal payload for an existing learning candidate.
    pub async fn update_learning_candidate_proposal(
        &self,
        candidate_id: Uuid,
        proposal: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            UPDATE learning_candidates
            SET proposal = $2::jsonb
            WHERE id = $1
            "#,
            &[&candidate_id, proposal],
        )
        .await?;
        Ok(())
    }

    /// Persist a versioned artifact mutation.
    pub async fn insert_learning_artifact_version(
        &self,
        version: &LearningArtifactVersion,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;
        let id = if version.id.is_nil() {
            Uuid::new_v4()
        } else {
            version.id
        };
        conn.execute(
            r#"
            INSERT INTO learning_artifact_versions (
                id, candidate_id, user_id, artifact_type, artifact_name, version_label,
                status, diff_summary, before_content, after_content, provenance, created_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6,
                $7, $8, $9, $10, $11::jsonb, $12
            )
            "#,
            &[
                &id,
                &version.candidate_id,
                &version.user_id,
                &version.artifact_type,
                &version.artifact_name,
                &version.version_label,
                &version.status,
                &version.diff_summary,
                &version.before_content,
                &version.after_content,
                &version.provenance,
                &version.created_at,
            ],
        )
        .await?;
        Ok(id)
    }

    /// List versioned artifacts with optional artifact filters.
    pub async fn list_learning_artifact_versions(
        &self,
        user_id: &str,
        artifact_type: Option<&str>,
        artifact_name: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningArtifactVersion>, DatabaseError> {
        if limit <= 0 {
            return Ok(Vec::new());
        }
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT
                    id, candidate_id, user_id, artifact_type, artifact_name, version_label,
                    status, diff_summary, before_content, after_content, provenance, created_at
                FROM learning_artifact_versions
                WHERE user_id = $1
                  AND ($2::text IS NULL OR artifact_type = $2)
                  AND ($3::text IS NULL OR artifact_name = $3)
                ORDER BY created_at DESC, id DESC
                LIMIT $4
                "#,
                &[&user_id, &artifact_type, &artifact_name, &limit],
            )
            .await?;
        Ok(rows
            .iter()
            .map(learning_artifact_version_from_row)
            .collect())
    }

    /// Persist explicit feedback on a learned target.
    pub async fn insert_learning_feedback(
        &self,
        feedback: &LearningFeedbackRecord,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;
        let id = if feedback.id.is_nil() {
            Uuid::new_v4()
        } else {
            feedback.id
        };
        conn.execute(
            r#"
            INSERT INTO learning_feedback (
                id, user_id, target_type, target_id, verdict, note, metadata, created_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7::jsonb, $8
            )
            "#,
            &[
                &id,
                &feedback.user_id,
                &feedback.target_type,
                &feedback.target_id,
                &feedback.verdict,
                &feedback.note,
                &feedback.metadata,
                &feedback.created_at,
            ],
        )
        .await?;
        Ok(id)
    }

    /// List feedback entries, optionally scoped to a target.
    pub async fn list_learning_feedback(
        &self,
        user_id: &str,
        target_type: Option<&str>,
        target_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningFeedbackRecord>, DatabaseError> {
        if limit <= 0 {
            return Ok(Vec::new());
        }
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT id, user_id, target_type, target_id, verdict, note, metadata, created_at
                FROM learning_feedback
                WHERE user_id = $1
                  AND ($2::text IS NULL OR target_type = $2)
                  AND ($3::text IS NULL OR target_id = $3)
                ORDER BY created_at DESC, id DESC
                LIMIT $4
                "#,
                &[&user_id, &target_type, &target_id, &limit],
            )
            .await?;
        Ok(rows.iter().map(learning_feedback_from_row).collect())
    }

    /// Persist rollback metadata for a learned artifact.
    pub async fn insert_learning_rollback(
        &self,
        rollback: &LearningRollbackRecord,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;
        let id = if rollback.id.is_nil() {
            Uuid::new_v4()
        } else {
            rollback.id
        };
        conn.execute(
            r#"
            INSERT INTO learning_rollbacks (
                id, user_id, artifact_type, artifact_name, artifact_version_id, reason, metadata, created_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7::jsonb, $8
            )
            "#,
            &[
                &id,
                &rollback.user_id,
                &rollback.artifact_type,
                &rollback.artifact_name,
                &rollback.artifact_version_id,
                &rollback.reason,
                &rollback.metadata,
                &rollback.created_at,
            ],
        )
        .await?;
        Ok(id)
    }

    /// List rollback records for a user.
    pub async fn list_learning_rollbacks(
        &self,
        user_id: &str,
        artifact_type: Option<&str>,
        artifact_name: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningRollbackRecord>, DatabaseError> {
        if limit <= 0 {
            return Ok(Vec::new());
        }
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT
                    id, user_id, artifact_type, artifact_name, artifact_version_id, reason, metadata, created_at
                FROM learning_rollbacks
                WHERE user_id = $1
                  AND ($2::text IS NULL OR artifact_type = $2)
                  AND ($3::text IS NULL OR artifact_name = $3)
                ORDER BY created_at DESC, id DESC
                LIMIT $4
                "#,
                &[&user_id, &artifact_type, &artifact_name, &limit],
            )
            .await?;
        Ok(rows.iter().map(learning_rollback_from_row).collect())
    }

    /// Persist a code-change proposal package.
    pub async fn insert_learning_code_proposal(
        &self,
        proposal: &LearningCodeProposal,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.conn().await?;
        let id = if proposal.id.is_nil() {
            Uuid::new_v4()
        } else {
            proposal.id
        };
        let target_files = serde_json::to_value(&proposal.target_files)
            .map_err(|e| DatabaseError::Serialization(format!("serialize target_files: {e}")))?;
        conn.execute(
            r#"
            INSERT INTO learning_code_proposals (
                id, learning_event_id, user_id, status, title, rationale, target_files, diff,
                validation_results, rollback_note, confidence, branch_name, pr_url, metadata, created_at, updated_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7::jsonb, $8,
                $9::jsonb, $10, $11, $12, $13, $14::jsonb, $15, $16
            )
            "#,
            &[
                &id,
                &proposal.learning_event_id,
                &proposal.user_id,
                &proposal.status,
                &proposal.title,
                &proposal.rationale,
                &target_files,
                &proposal.diff,
                &proposal.validation_results,
                &proposal.rollback_note,
                &proposal.confidence,
                &proposal.branch_name,
                &proposal.pr_url,
                &proposal.metadata,
                &proposal.created_at,
                &proposal.updated_at,
            ],
        )
        .await?;
        Ok(id)
    }

    /// Retrieve a single code proposal belonging to a user.
    pub async fn get_learning_code_proposal(
        &self,
        user_id: &str,
        proposal_id: Uuid,
    ) -> Result<Option<LearningCodeProposal>, DatabaseError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt(
                r#"
                SELECT
                    id, learning_event_id, user_id, status, title, rationale, target_files, diff,
                    validation_results, rollback_note, confidence, branch_name, pr_url, metadata, created_at, updated_at
                FROM learning_code_proposals
                WHERE id = $1 AND user_id = $2
                "#,
                &[&proposal_id, &user_id],
            )
            .await?;
        Ok(row.as_ref().map(learning_code_proposal_from_row))
    }

    /// List code proposals for a user with optional status filtering.
    pub async fn list_learning_code_proposals(
        &self,
        user_id: &str,
        status: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LearningCodeProposal>, DatabaseError> {
        if limit <= 0 {
            return Ok(Vec::new());
        }
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT
                    id, learning_event_id, user_id, status, title, rationale, target_files, diff,
                    validation_results, rollback_note, confidence, branch_name, pr_url, metadata, created_at, updated_at
                FROM learning_code_proposals
                WHERE user_id = $1
                  AND ($2::text IS NULL OR status = $2)
                ORDER BY created_at DESC, id DESC
                LIMIT $3
                "#,
                &[&user_id, &status, &limit],
            )
            .await?;
        Ok(rows.iter().map(learning_code_proposal_from_row).collect())
    }

    /// Update proposal review status and publish metadata.
    pub async fn update_learning_code_proposal(
        &self,
        proposal_id: Uuid,
        status: &str,
        branch_name: Option<&str>,
        pr_url: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;
        conn.execute(
            r#"
            UPDATE learning_code_proposals
            SET status = $2,
                branch_name = COALESCE($3, branch_name),
                pr_url = COALESCE($4, pr_url),
                metadata = CASE
                    WHEN $5::jsonb IS NULL THEN metadata
                    ELSE coalesce(metadata, '{}'::jsonb) || $5::jsonb
                END,
                updated_at = NOW()
            WHERE id = $1
            "#,
            &[&proposal_id, &status, &branch_name, &pr_url, &metadata],
        )
        .await?;
        Ok(())
    }
}
