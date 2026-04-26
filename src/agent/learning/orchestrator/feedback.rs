use super::*;
impl LearningOrchestrator {
    pub async fn submit_feedback(
        &self,
        user_id: &str,
        target_type: &str,
        target_id: &str,
        verdict: &str,
        note: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<Uuid, String> {
        let record = DbLearningFeedbackRecord {
            id: Uuid::new_v4(),
            user_id: user_id.to_string(),
            target_type: target_type.to_string(),
            target_id: target_id.to_string(),
            verdict: verdict.to_string(),
            note: note.map(str::to_string),
            metadata: metadata.cloned().unwrap_or_else(|| serde_json::json!({})),
            created_at: Utc::now(),
        };
        let id = self
            .store
            .insert_learning_feedback(&record)
            .await
            .map_err(|e| e.to_string())?;
        if let Err(err) = outcomes::observe_feedback(&self.store, &record).await {
            tracing::debug!(user_id = %user_id, error = %err, "Outcome feedback hook skipped");
        }
        if let Err(err) = self
            .apply_generated_skill_feedback(user_id, target_type, target_id, verdict, note)
            .await
        {
            tracing::debug!(
                user_id = %user_id,
                target_type = %target_type,
                target_id = %target_id,
                error = %err,
                "Generated skill feedback hook skipped"
            );
        }

        let feedback_event = LearningEvent::new(
            "learning::explicit_feedback",
            ImprovementClass::Unknown,
            RiskTier::Medium,
            "Explicit user learning feedback received",
        )
        .with_target(format!("{target_type}:{target_id}"))
        .with_metadata(serde_json::json!({
            "target_type": target_type,
            "target_id": target_id,
            "verdict": verdict,
            "note": note,
            "feedback_id": id,
            "source": "learning_feedback_tool",
        }))
        .into_persisted(user_id.to_string(), None, None, None, None, None, None);
        if self
            .store
            .insert_learning_event(&feedback_event)
            .await
            .is_ok()
        {
            let _ = self
                .handle_event("explicit_user_feedback", &feedback_event)
                .await;
        }

        Ok(id)
    }
}
