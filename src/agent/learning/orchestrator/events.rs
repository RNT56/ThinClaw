use super::*;
impl LearningOrchestrator {
    pub async fn handle_event(
        &self,
        trigger: &str,
        event: &DbLearningEvent,
    ) -> Result<LearningOutcome, String> {
        let settings = self.load_settings_for_user(&event.user_id).await;
        let mut outcome = LearningOutcome {
            trigger: trigger.to_string(),
            event_id: event.id,
            evaluation_id: None,
            candidate_id: None,
            auto_applied: false,
            code_proposal_id: None,
            notes: Vec::new(),
        };

        if event.source == "learning::explicit_feedback" {
            outcome
                .notes
                .push("explicit feedback event recorded".to_string());
            return Ok(outcome);
        }

        if !settings.enabled {
            outcome
                .notes
                .push("learning disabled; event persisted only".to_string());
            return Ok(outcome);
        }

        if self.is_duplicate_or_cooldown(event).await {
            outcome
                .notes
                .push("duplicate/cooldown hit; skipped candidate generation".to_string());
            return Ok(outcome);
        }

        let (quality_score, evaluator_status, class, risk, confidence) =
            self.evaluate_event(event).await;

        let evaluation = DbLearningEvaluation {
            id: Uuid::new_v4(),
            learning_event_id: event.id,
            user_id: event.user_id.clone(),
            evaluator: "learning_orchestrator_v1".to_string(),
            status: evaluator_status,
            score: Some(quality_score as f64),
            details: serde_json::json!({
                "quality_score": quality_score,
                "class": class.as_str(),
                "risk_tier": risk.as_str(),
                "confidence": confidence,
            }),
            created_at: Utc::now(),
        };
        match self.store.insert_learning_evaluation(&evaluation).await {
            Ok(id) => outcome.evaluation_id = Some(id),
            Err(err) => {
                outcome
                    .notes
                    .push(format!("failed to persist evaluation: {err}"));
            }
        }

        let candidate = DbLearningCandidate {
            id: Uuid::new_v4(),
            learning_event_id: Some(event.id),
            user_id: event.user_id.clone(),
            candidate_type: class.as_str().to_string(),
            risk_tier: risk.as_str().to_string(),
            confidence: Some(confidence as f64),
            target_type: event
                .payload
                .get("target_type")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            target_name: event
                .payload
                .get("target")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            summary: Some(
                event
                    .payload
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Auto-distilled learning candidate")
                    .to_string(),
            ),
            proposal: event.payload.clone(),
            created_at: Utc::now(),
        };

        let candidate_id = self
            .store
            .insert_learning_candidate(&candidate)
            .await
            .map_err(|e| e.to_string())?;
        outcome.candidate_id = Some(candidate_id);

        let safe_mode_active = self.safe_mode_tripped(&settings, &event.user_id).await;
        let route = route_learning_candidate(
            class,
            risk,
            LearningRoutePolicy {
                learning_enabled: settings.enabled,
                safe_mode_active,
                auto_apply_allowed: auto_apply_allowed_for_class(
                    class,
                    &settings.auto_apply_classes,
                    settings.prompt_mutation.enabled,
                ),
                code_auto_apply_without_review: settings.code_proposals.auto_apply_without_review,
            },
        );

        if let LearningRouteAction::CodeProposal { auto_approve } = route {
            match self.create_code_proposal(event, &candidate).await {
                Ok(proposal_id) => {
                    outcome.code_proposal_id = Some(proposal_id);
                    if auto_approve {
                        match self
                            .approve_code_proposal(
                                &event.user_id,
                                proposal_id,
                                Some("auto-approved in reckless_desktop mode"),
                            )
                            .await
                        {
                            Ok(Some(updated)) => {
                                outcome.auto_applied = updated.status == "applied";
                                outcome.notes.push(format!(
                                    "code proposal auto-approved in reckless desktop mode ({})",
                                    updated.status
                                ));
                            }
                            Ok(None) => outcome
                                .notes
                                .push("code proposal disappeared before auto-approval".to_string()),
                            Err(err) => outcome
                                .notes
                                .push(format!("code auto-approval failed: {err}")),
                        }
                    } else {
                        outcome.notes.push(
                            "high-risk candidate routed to approval-gated code proposal"
                                .to_string(),
                        );
                    }
                }
                Err(err) => {
                    outcome
                        .notes
                        .push(format!("high-risk proposal suppressed: {err}"));
                }
            }
            return Ok(outcome);
        }

        match route {
            LearningRouteAction::AutoApply => {
                if self
                    .auto_apply_candidate(&settings, class, &candidate)
                    .await
                    .unwrap_or(false)
                {
                    outcome.auto_applied = true;
                    outcome
                        .notes
                        .push("candidate auto-applied in Tier A".to_string());
                } else {
                    outcome
                        .notes
                        .push("candidate queued for manual review".to_string());
                }
            }
            LearningRouteAction::ManualReview => {
                outcome
                    .notes
                    .push("candidate queued for manual review".to_string());
            }
            LearningRouteAction::PersistedOnly => outcome
                .notes
                .push("learning disabled; event persisted only".to_string()),
            LearningRouteAction::HeldForReview => outcome
                .notes
                .push("safe mode is active; candidate held for review".to_string()),
            LearningRouteAction::CodeProposal { .. } => {}
        }

        Ok(outcome)
    }

    pub async fn route_existing_candidate(
        &self,
        trigger: &str,
        candidate: &DbLearningCandidate,
    ) -> Result<LearningOutcome, String> {
        let settings = self.load_settings_for_user(&candidate.user_id).await;
        let class = ImprovementClass::from_str(&candidate.candidate_type);
        let risk = RiskTier::from_str(&candidate.risk_tier);
        let event_id = candidate.learning_event_id.unwrap_or(candidate.id);
        let mut outcome = LearningOutcome {
            trigger: trigger.to_string(),
            event_id,
            evaluation_id: None,
            candidate_id: Some(candidate.id),
            auto_applied: false,
            code_proposal_id: None,
            notes: Vec::new(),
        };

        if !settings.enabled {
            outcome
                .notes
                .push("learning disabled; outcome candidate persisted only".to_string());
            return Ok(outcome);
        }

        let safe_mode_active = self.safe_mode_tripped(&settings, &candidate.user_id).await;
        let route = route_learning_candidate(
            class,
            risk,
            LearningRoutePolicy {
                learning_enabled: settings.enabled,
                safe_mode_active,
                auto_apply_allowed: auto_apply_allowed_for_class(
                    class,
                    &settings.auto_apply_classes,
                    settings.prompt_mutation.enabled,
                ),
                code_auto_apply_without_review: settings.code_proposals.auto_apply_without_review,
            },
        );

        if let LearningRouteAction::CodeProposal { auto_approve } = route {
            match self.create_code_proposal_from_candidate(candidate).await {
                Ok(proposal_id) => {
                    outcome.code_proposal_id = Some(proposal_id);
                    if auto_approve {
                        match self
                            .approve_code_proposal(
                                &candidate.user_id,
                                proposal_id,
                                Some("auto-approved in reckless_desktop mode"),
                            )
                            .await
                        {
                            Ok(Some(updated)) => {
                                outcome.auto_applied = updated.status == "applied";
                                outcome.notes.push(format!(
                                    "outcome code proposal auto-approved in reckless desktop mode ({})",
                                    updated.status
                                ));
                            }
                            Ok(None) => outcome.notes.push(
                                "outcome code proposal disappeared before auto-approval"
                                    .to_string(),
                            ),
                            Err(err) => outcome
                                .notes
                                .push(format!("outcome code auto-approval failed: {err}")),
                        }
                    } else {
                        outcome.notes.push(
                            "outcome candidate routed to approval-gated code proposal".to_string(),
                        );
                    }
                }
                Err(err) => {
                    outcome
                        .notes
                        .push(format!("outcome code proposal suppressed: {err}"));
                }
            }
            return Ok(outcome);
        }

        match route {
            LearningRouteAction::AutoApply => {
                if self
                    .auto_apply_candidate(&settings, class, candidate)
                    .await
                    .unwrap_or(false)
                {
                    outcome.auto_applied = true;
                    outcome
                        .notes
                        .push("outcome candidate auto-applied in Tier A".to_string());
                } else {
                    outcome
                        .notes
                        .push("outcome candidate queued for manual review".to_string());
                }
            }
            LearningRouteAction::ManualReview => outcome
                .notes
                .push("outcome candidate queued for manual review".to_string()),
            LearningRouteAction::PersistedOnly => outcome
                .notes
                .push("learning disabled; outcome candidate persisted only".to_string()),
            LearningRouteAction::HeldForReview => outcome
                .notes
                .push("safe mode is active; outcome candidate held for review".to_string()),
            LearningRouteAction::CodeProposal { .. } => {}
        }

        Ok(outcome)
    }

    pub(in crate::agent::learning) async fn is_duplicate_or_cooldown(
        &self,
        event: &DbLearningEvent,
    ) -> bool {
        let Ok(recent) = self
            .store
            .list_learning_events(
                &event.user_id,
                event.actor_id.as_deref(),
                event.channel.as_deref(),
                event.thread_id.as_deref(),
                30,
            )
            .await
        else {
            return false;
        };

        let event_hash = stable_json_hash(&event.payload);
        for prior in recent {
            if prior.id == event.id {
                continue;
            }
            if prior.event_type != event.event_type || prior.source != event.source {
                continue;
            }
            if stable_json_hash(&prior.payload) != event_hash {
                continue;
            }
            let age_secs = (event.created_at - prior.created_at).num_seconds().abs();
            if age_secs <= 900 {
                return true;
            }
        }
        false
    }

    pub(in crate::agent::learning) async fn evaluate_event(
        &self,
        event: &DbLearningEvent,
    ) -> (u32, String, ImprovementClass, RiskTier, f32) {
        let evaluation = evaluate_learning_event(&event.event_type, &event.payload);
        (
            evaluation.quality_score,
            evaluation.evaluator_status,
            evaluation.class,
            evaluation.risk_tier,
            evaluation.confidence,
        )
    }

    pub(in crate::agent::learning) async fn safe_mode_tripped(
        &self,
        settings: &LearningSettings,
        user_id: &str,
    ) -> bool {
        let feedback = match self
            .store
            .list_learning_feedback(user_id, None, None, 100)
            .await
        {
            Ok(feedback) => feedback,
            Err(_) => return false,
        };

        let rollbacks = self
            .store
            .list_learning_rollbacks(user_id, None, None, 100)
            .await
            .unwrap_or_default();

        let negative_feedback = feedback
            .iter()
            .filter(|entry| is_negative_learning_feedback_verdict(&entry.verdict))
            .count();
        let outcome_stats = self
            .store
            .outcome_summary_stats(user_id)
            .await
            .unwrap_or_default();

        safe_mode_should_trip(
            settings.safe_mode.enabled,
            settings.safe_mode.thresholds.min_samples,
            settings.safe_mode.thresholds.negative_feedback_ratio,
            settings.safe_mode.thresholds.rollback_ratio,
            feedback.len(),
            negative_feedback,
            rollbacks.len(),
            outcome_stats.evaluated_last_7d,
            outcome_stats.negative_ratio_last_7d,
        )
    }
}
