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

        if self.safe_mode_tripped(&settings, &event.user_id).await {
            outcome
                .notes
                .push("safe mode is active; candidate held for review".to_string());
            return Ok(outcome);
        }

        if risk.rank() >= RiskTier::High.rank() || class == ImprovementClass::Code {
            match self.create_code_proposal(event, &candidate).await {
                Ok(proposal_id) => {
                    outcome.code_proposal_id = Some(proposal_id);
                    if class == ImprovementClass::Code
                        && settings.code_proposals.auto_apply_without_review
                    {
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

        let auto_apply_allowed = settings
            .auto_apply_classes
            .iter()
            .any(|entry| entry.eq_ignore_ascii_case(class.as_str()));
        if auto_apply_allowed
            && self
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

        if self.safe_mode_tripped(&settings, &candidate.user_id).await {
            outcome
                .notes
                .push("safe mode is active; outcome candidate held for review".to_string());
            return Ok(outcome);
        }

        if risk.rank() >= RiskTier::High.rank() || class == ImprovementClass::Code {
            match self.create_code_proposal_from_candidate(candidate).await {
                Ok(proposal_id) => {
                    outcome.code_proposal_id = Some(proposal_id);
                    if class == ImprovementClass::Code
                        && settings.code_proposals.auto_apply_without_review
                    {
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

        let auto_apply_allowed = settings
            .auto_apply_classes
            .iter()
            .any(|entry| entry.eq_ignore_ascii_case(class.as_str()));
        if auto_apply_allowed
            && self
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
        let success = event
            .payload
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let wasted_tool_calls = event
            .payload
            .get("wasted_tool_calls")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let repeated_failures = event
            .payload
            .get("repeated_failures")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let correction_count = event
            .payload
            .get("correction_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let safety_incident = event
            .payload
            .get("safety_incident")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let class = classify_event(event);
        let mut risk = match class {
            ImprovementClass::Code => RiskTier::Critical,
            ImprovementClass::Prompt => RiskTier::Medium,
            ImprovementClass::Routine => RiskTier::Medium,
            ImprovementClass::Skill => RiskTier::Low,
            ImprovementClass::Memory => RiskTier::Low,
            ImprovementClass::Unknown => RiskTier::Medium,
        };
        if safety_incident {
            risk = RiskTier::Critical;
        }

        let mut score: i32 = if success { 82 } else { 45 };
        score -= (wasted_tool_calls as i32) * 4;
        score -= (repeated_failures as i32) * 7;
        score -= (correction_count as i32) * 5;
        if safety_incident {
            score -= 35;
        }
        score = score.clamp(0, 100);

        let confidence = ((score as f32 / 100.0)
            + if correction_count > 0 { 0.15 } else { 0.0 }
            + if repeated_failures > 0 { 0.1 } else { 0.0 })
        .clamp(0.0, 1.0);

        let status = if score >= 70 {
            "accepted"
        } else if score >= 45 {
            "review"
        } else {
            "poor"
        }
        .to_string();

        (score as u32, status, class, risk, confidence)
    }

    pub(in crate::agent::learning) async fn safe_mode_tripped(
        &self,
        settings: &LearningSettings,
        user_id: &str,
    ) -> bool {
        if !settings.safe_mode.enabled {
            return false;
        }

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

        let sample = feedback.len().max(rollbacks.len()) as u32;
        if sample < settings.safe_mode.thresholds.min_samples {
            return false;
        }

        let negative_feedback = feedback
            .iter()
            .filter(|entry| {
                matches!(
                    entry.verdict.to_ascii_lowercase().as_str(),
                    "harmful" | "revert" | "dont_learn" | "reject"
                )
            })
            .count() as f64;

        let feedback_ratio = negative_feedback / sample as f64;
        let rollback_ratio = rollbacks.len() as f64 / sample as f64;
        let outcome_stats = self
            .store
            .outcome_summary_stats(user_id)
            .await
            .unwrap_or_default();
        let outcome_ratio = outcome_stats.negative_ratio_last_7d;

        feedback_ratio >= settings.safe_mode.thresholds.negative_feedback_ratio
            || rollback_ratio >= settings.safe_mode.thresholds.rollback_ratio
            || (outcome_stats.evaluated_last_7d >= settings.safe_mode.thresholds.min_samples as u64
                && outcome_ratio >= settings.safe_mode.thresholds.negative_feedback_ratio)
    }
}
