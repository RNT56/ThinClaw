//! Trajectory compatibility facade.
//!
//! Portable trajectory record/logging types live in `thinclaw-agent`. The
//! store-backed hydration adapter stays in the root crate because it depends on
//! the concrete database/history records.

use std::sync::Arc;

pub use thinclaw_agent::trajectory::{
    TrajectoryAssessment, TrajectoryFeedback, TrajectoryLogger, TrajectoryOutcome, TrajectoryStats,
    TrajectoryTurnRecord, TrajectoryTurnStatus,
};

use super::{Database, DbLearningEvaluation, DbLearningEvent, DbLearningFeedbackRecord};

fn feedback_outcome(verdict: &str) -> Option<TrajectoryOutcome> {
    match verdict.trim().to_ascii_lowercase().as_str() {
        "helpful" | "approve" | "approved" | "accept" | "accepted" | "useful" | "good"
        | "positive" | "success" | "like" => Some(TrajectoryOutcome::Success),
        "harmful" | "reject" | "rejected" | "dont_learn" | "bad" | "negative" | "failure"
        | "dislike" => Some(TrajectoryOutcome::Failure),
        "neutral" | "mixed" | "needs_review" | "unclear" => Some(TrajectoryOutcome::Neutral),
        _ => None,
    }
}

fn feedback_score(outcome: TrajectoryOutcome, fallback_score: f64) -> f64 {
    match outcome {
        TrajectoryOutcome::Success => fallback_score.max(0.95),
        TrajectoryOutcome::Failure => fallback_score.min(0.05),
        TrajectoryOutcome::Neutral => 0.5,
    }
}

fn feedback_matches_turn(
    feedback: &DbLearningFeedbackRecord,
    record: &TrajectoryTurnRecord,
    target_id: &str,
) -> bool {
    if feedback.target_id == target_id {
        return true;
    }

    let metadata = &feedback.metadata;
    metadata
        .get("trajectory_target_id")
        .and_then(|value| value.as_str())
        .is_some_and(|value| value == target_id)
        || metadata
            .get("thread_id")
            .and_then(|value| value.as_str())
            .is_some_and(|value| value == record.thread_id.to_string())
            && metadata
                .get("turn_number")
                .and_then(|value| value.as_u64())
                .is_some_and(|value| value as usize == record.turn_number)
        || metadata
            .get("session_id")
            .and_then(|value| value.as_str())
            .is_some_and(|value| value == record.session_id.to_string())
            && metadata
                .get("turn_number")
                .and_then(|value| value.as_u64())
                .is_some_and(|value| value as usize == record.turn_number)
}

fn metadata_matches_turn(
    metadata: &serde_json::Value,
    record: &TrajectoryTurnRecord,
    target_id: &str,
) -> bool {
    metadata
        .get("trajectory_target_id")
        .and_then(|value| value.as_str())
        .is_some_and(|value| value == target_id)
        || metadata
            .get("target_id")
            .and_then(|value| value.as_str())
            .is_some_and(|value| value == target_id)
        || metadata
            .get("thread_id")
            .and_then(|value| value.as_str())
            .is_some_and(|value| value == record.thread_id.to_string())
            && metadata
                .get("turn_number")
                .and_then(|value| value.as_u64())
                .is_some_and(|value| value as usize == record.turn_number)
        || metadata
            .get("session_id")
            .and_then(|value| value.as_str())
            .is_some_and(|value| value == record.session_id.to_string())
            && metadata
                .get("turn_number")
                .and_then(|value| value.as_u64())
                .is_some_and(|value| value as usize == record.turn_number)
}

fn event_matches_turn(
    event: &DbLearningEvent,
    record: &TrajectoryTurnRecord,
    target_id: &str,
) -> bool {
    metadata_matches_turn(&event.payload, record, target_id)
        || event
            .metadata
            .as_ref()
            .is_some_and(|metadata| metadata_matches_turn(metadata, record, target_id))
        || event
            .payload
            .get("target")
            .and_then(|value| value.as_str())
            .is_some_and(|value| value == format!("trajectory_turn:{target_id}"))
        || event
            .payload
            .get("target")
            .and_then(|value| value.as_str())
            .is_some_and(|value| value == format!("thread_turn:{target_id}"))
}

fn evaluation_outcome(
    evaluation: &DbLearningEvaluation,
    base_assessment: &TrajectoryAssessment,
) -> TrajectoryAssessment {
    let status = evaluation.status.trim().to_ascii_lowercase();
    let raw_score = evaluation
        .score
        .or_else(|| {
            evaluation
                .details
                .get("quality_score")
                .and_then(|value| value.as_f64())
        })
        .unwrap_or(base_assessment.score);
    let normalized_score = if raw_score > 1.0 {
        (raw_score / 100.0).clamp(0.0, 1.0)
    } else {
        raw_score.clamp(0.0, 1.0)
    };

    let outcome = match status.as_str() {
        "accepted" | "approve" | "approved" | "good" | "pass" | "passed" => {
            TrajectoryOutcome::Success
        }
        "poor" | "reject" | "rejected" | "bad" | "fail" | "failed" => TrajectoryOutcome::Failure,
        "review" | "needs_review" | "mixed" | "neutral" => TrajectoryOutcome::Neutral,
        _ if normalized_score >= 0.7 => TrajectoryOutcome::Success,
        _ if normalized_score <= 0.3 => TrajectoryOutcome::Failure,
        _ => TrajectoryOutcome::Neutral,
    };

    TrajectoryAssessment {
        outcome,
        score: normalized_score,
        source: format!("learning_evaluation:{}", evaluation.evaluator),
        reasoning: format!(
            "Turn label derived from learning evaluation status '{}' with score {:.2}.",
            evaluation.status, normalized_score
        ),
    }
}

pub async fn hydrate_trajectory_record(
    record: &mut TrajectoryTurnRecord,
    store: Option<&Arc<dyn Database>>,
) {
    let Some(store) = store else {
        let assessment = record
            .assessment
            .clone()
            .unwrap_or_else(|| TrajectoryAssessment {
                outcome: record.outcome,
                score: record.preference_score(),
                source: "legacy_archive".to_string(),
                reasoning: "Archive record was logged without store-backed feedback.".to_string(),
            });
        record.outcome = assessment.outcome;
        record.assessment = Some(assessment);
        return;
    };

    let target_id = record.target_id();
    let mut matched_feedback: Option<DbLearningFeedbackRecord> = None;
    let mut matched_evaluation: Option<DbLearningEvaluation> = None;

    for target_type in ["trajectory_turn", "thread_turn"] {
        match store
            .list_learning_feedback(&record.user_id, Some(target_type), Some(&target_id), 10)
            .await
        {
            Ok(entries) => {
                if let Some(entry) = entries.into_iter().next() {
                    matched_feedback = Some(entry);
                    break;
                }
            }
            Err(err) => {
                tracing::debug!(
                    user_id = %record.user_id,
                    target_type,
                    error = %err,
                    "Failed to load targeted trajectory feedback"
                );
            }
        }
    }

    if matched_feedback.is_none() {
        match store
            .list_learning_feedback(&record.user_id, None, None, 100)
            .await
        {
            Ok(entries) => {
                matched_feedback = entries
                    .into_iter()
                    .find(|feedback| feedback_matches_turn(feedback, record, &target_id));
            }
            Err(err) => {
                tracing::debug!(
                    user_id = %record.user_id,
                    error = %err,
                    "Failed to load recent trajectory feedback"
                );
            }
        }
    }

    if matched_feedback.is_none() {
        match (
            store
                .list_learning_events(&record.user_id, None, None, None, 200)
                .await,
            store.list_learning_evaluations(&record.user_id, 200).await,
        ) {
            (Ok(events), Ok(evaluations)) => {
                let matched_event_ids: std::collections::HashSet<_> = events
                    .iter()
                    .filter(|event| event_matches_turn(event, record, &target_id))
                    .map(|event| event.id)
                    .collect();
                matched_evaluation = evaluations
                    .into_iter()
                    .find(|evaluation| matched_event_ids.contains(&evaluation.learning_event_id));
            }
            (Err(err), _) => {
                tracing::debug!(
                    user_id = %record.user_id,
                    error = %err,
                    "Failed to load recent trajectory learning events"
                );
            }
            (_, Err(err)) => {
                tracing::debug!(
                    user_id = %record.user_id,
                    error = %err,
                    "Failed to load recent trajectory learning evaluations"
                );
            }
        }
    }

    let base_assessment = record
        .assessment
        .clone()
        .unwrap_or_else(|| TrajectoryAssessment {
            outcome: record.outcome,
            score: record.preference_score(),
            source: "legacy_archive".to_string(),
            reasoning: "Archive record predates structured trajectory assessment.".to_string(),
        });

    if let Some(feedback) = matched_feedback {
        let verdict_outcome =
            feedback_outcome(&feedback.verdict).unwrap_or(base_assessment.outcome);
        let score = feedback_score(verdict_outcome, base_assessment.score);
        record.user_feedback = Some(TrajectoryFeedback {
            label: feedback.verdict.clone(),
            notes: feedback.note.clone(),
            source: Some(feedback.target_type.clone()),
            created_at: Some(feedback.created_at),
        });
        record.assessment = Some(TrajectoryAssessment {
            outcome: verdict_outcome,
            score,
            source: "learning_feedback".to_string(),
            reasoning: format!(
                "Turn label derived from explicit learning feedback verdict '{}'.",
                feedback.verdict
            ),
        });
        record.outcome = verdict_outcome;
    } else if let Some(evaluation) = matched_evaluation {
        let assessment = evaluation_outcome(&evaluation, &base_assessment);
        record.assessment = Some(assessment.clone());
        record.outcome = assessment.outcome;
    } else {
        record.assessment = Some(base_assessment.clone());
        record.outcome = base_assessment.outcome;
    }
}
