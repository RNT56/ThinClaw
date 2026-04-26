use super::*;

/// Outcome classification for a terminal turn trajectory record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrajectoryOutcome {
    Success,
    Failure,
    Neutral,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrajectoryTurnStatus {
    Completed,
    Failed,
    Interrupted,
    Processing,
}

fn default_trajectory_turn_status() -> TrajectoryTurnStatus {
    TrajectoryTurnStatus::Completed
}

/// Optional user feedback attached to a turn record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryFeedback {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
}

/// Assessment metadata used to classify a turn for training exports.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryAssessment {
    pub outcome: TrajectoryOutcome,
    pub score: f64,
    pub source: String,
    pub reasoning: String,
}

/// Structured turn record written to the trajectory JSONL archive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryTurnRecord {
    pub session_id: Uuid,
    pub thread_id: Uuid,
    pub user_id: String,
    pub actor_id: String,
    pub channel: String,
    pub conversation_scope_id: Uuid,
    pub conversation_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_thread_id: Option<String>,
    pub turn_number: usize,
    pub user_message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_response: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<crate::agent::session::TurnToolCall>,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default = "default_trajectory_turn_status")]
    pub turn_status: TrajectoryTurnStatus,
    pub outcome: TrajectoryOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_snapshot_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ephemeral_overlay_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_context_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_feedback: Option<TrajectoryFeedback>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assessment: Option<TrajectoryAssessment>,
}

impl TrajectoryTurnRecord {
    /// Build a trajectory record from a terminal thread turn snapshot.
    pub fn from_turn(
        session: &crate::agent::session::Session,
        thread_id: Uuid,
        _thread: &crate::agent::session::Thread,
        incoming: &crate::channels::IncomingMessage,
        turn: &crate::agent::session::Turn,
    ) -> Self {
        let identity = incoming.resolved_identity();
        Self {
            session_id: session.id,
            thread_id,
            user_id: incoming.user_id.clone(),
            actor_id: identity.actor_id,
            channel: incoming.channel.clone(),
            conversation_scope_id: session.conversation_scope_id,
            conversation_kind: session.conversation_kind.as_str().to_string(),
            external_thread_id: incoming.thread_id.clone(),
            turn_number: turn.turn_number,
            user_message: turn.user_input.clone(),
            assistant_response: turn.response.clone(),
            tool_calls: turn.tool_calls.clone(),
            started_at: turn.started_at,
            completed_at: turn.completed_at,
            turn_status: Self::turn_status(turn),
            outcome: Self::classify_turn(turn),
            failure_reason: turn.error.clone(),
            execution_backend: None,
            llm_provider: None,
            llm_model: None,
            prompt_snapshot_hash: None,
            ephemeral_overlay_hash: None,
            provider_context_refs: Vec::new(),
            user_feedback: None,
            assessment: Some(Self::heuristic_assessment(turn)),
        }
    }

    /// Stable target identifier for this turn, used by learning feedback and
    /// dataset exports.
    pub fn target_id(&self) -> String {
        format!(
            "{}:{}:{}",
            self.session_id, self.thread_id, self.turn_number
        )
    }

    /// Classify the recorded turn using the local thread state.
    pub fn classify_turn(turn: &crate::agent::session::Turn) -> TrajectoryOutcome {
        match turn.state {
            crate::agent::session::TurnState::Completed => TrajectoryOutcome::Success,
            crate::agent::session::TurnState::Failed => TrajectoryOutcome::Failure,
            crate::agent::session::TurnState::Interrupted => TrajectoryOutcome::Neutral,
            crate::agent::session::TurnState::Processing => TrajectoryOutcome::Neutral,
        }
    }

    pub fn turn_status(turn: &crate::agent::session::Turn) -> TrajectoryTurnStatus {
        match turn.state {
            crate::agent::session::TurnState::Completed => TrajectoryTurnStatus::Completed,
            crate::agent::session::TurnState::Failed => TrajectoryTurnStatus::Failed,
            crate::agent::session::TurnState::Interrupted => TrajectoryTurnStatus::Interrupted,
            crate::agent::session::TurnState::Processing => TrajectoryTurnStatus::Processing,
        }
    }

    /// Heuristic fallback assessment used when no explicit learning feedback
    /// exists for the turn.
    pub fn heuristic_assessment(turn: &crate::agent::session::Turn) -> TrajectoryAssessment {
        let has_response = turn
            .response
            .as_deref()
            .map(str::trim)
            .is_some_and(|response| !response.is_empty());
        let has_error = turn
            .error
            .as_deref()
            .map(str::trim)
            .is_some_and(|error| !error.is_empty());
        let tool_count = turn.tool_calls.len() as f64;

        match turn.state {
            crate::agent::session::TurnState::Failed => TrajectoryAssessment {
                outcome: TrajectoryOutcome::Failure,
                score: 0.05,
                source: "turn_state".to_string(),
                reasoning: "Turn failed before producing a complete response.".to_string(),
            },
            crate::agent::session::TurnState::Interrupted => TrajectoryAssessment {
                outcome: TrajectoryOutcome::Neutral,
                score: 0.35,
                source: "turn_state".to_string(),
                reasoning: "Turn was interrupted before it could be evaluated.".to_string(),
            },
            crate::agent::session::TurnState::Processing => TrajectoryAssessment {
                outcome: TrajectoryOutcome::Neutral,
                score: 0.4,
                source: "turn_state".to_string(),
                reasoning: "Turn was still processing when archived.".to_string(),
            },
            crate::agent::session::TurnState::Completed => {
                if !has_response && has_error {
                    TrajectoryAssessment {
                        outcome: TrajectoryOutcome::Failure,
                        score: 0.15,
                        source: "turn_state".to_string(),
                        reasoning: "Turn completed with an error and no assistant response."
                            .to_string(),
                    }
                } else if !has_response {
                    TrajectoryAssessment {
                        outcome: TrajectoryOutcome::Neutral,
                        score: 0.45,
                        source: "turn_state".to_string(),
                        reasoning: "Turn completed without a durable assistant response."
                            .to_string(),
                    }
                } else {
                    let mut score = 0.72;
                    score += (tool_count.min(3.0)) * 0.05;
                    if has_error {
                        score -= 0.35;
                    }
                    let score = score.clamp(0.1, 0.95);
                    let outcome = if score >= 0.6 {
                        TrajectoryOutcome::Success
                    } else if score <= 0.25 {
                        TrajectoryOutcome::Failure
                    } else {
                        TrajectoryOutcome::Neutral
                    };
                    TrajectoryAssessment {
                        outcome,
                        score,
                        source: "heuristic_turn_eval_v1".to_string(),
                        reasoning: if has_error {
                            "Turn produced a response, but errors reduced confidence in its quality."
                                .to_string()
                        } else {
                            "Turn completed with a usable agent response.".to_string()
                        },
                    }
                }
            }
        }
    }

    pub fn effective_assessment(&self) -> TrajectoryAssessment {
        self.assessment
            .clone()
            .unwrap_or_else(|| TrajectoryAssessment {
                outcome: self.outcome,
                score: match self.outcome {
                    TrajectoryOutcome::Success => 0.75,
                    TrajectoryOutcome::Failure => 0.1,
                    TrajectoryOutcome::Neutral => 0.45,
                },
                source: "legacy_archive".to_string(),
                reasoning: "Archive record predates structured trajectory assessment.".to_string(),
            })
    }

    pub fn preference_score(&self) -> f64 {
        self.effective_assessment().score
    }
}

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

/// Basic stats summary for the trajectory archive.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TrajectoryStats {
    pub log_root: PathBuf,
    pub file_count: usize,
    pub record_count: usize,
    pub session_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_seen: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<DateTime<Utc>>,
    pub success_count: usize,
    pub failure_count: usize,
    pub neutral_count: usize,
}

/// Appends terminal turns to `~/.thinclaw/trajectories/` as JSONL.
#[derive(Debug, Clone)]
pub struct TrajectoryLogger {
    log_root: PathBuf,
}

impl Default for TrajectoryLogger {
    fn default() -> Self {
        Self::new()
    }
}

impl TrajectoryLogger {
    /// Create a logger rooted at the default ThinClaw trajectory directory.
    pub fn new() -> Self {
        Self::with_root(default_trajectory_root())
    }

    /// Create a logger with an explicit root directory.
    pub fn with_root(log_root: impl Into<PathBuf>) -> Self {
        Self {
            log_root: log_root.into(),
        }
    }

    /// Get the configured trajectory root.
    pub fn log_root(&self) -> &Path {
        &self.log_root
    }

    /// Append a single record to the JSONL archive.
    pub async fn append_turn(&self, record: &TrajectoryTurnRecord) -> anyhow::Result<PathBuf> {
        let effective_ts = record.completed_at.unwrap_or(record.started_at);
        let day = effective_ts.format("%Y-%m-%d").to_string();
        let dir = self.log_root.join(day);
        tokio::fs::create_dir_all(&dir).await?;

        let path = dir.join(format!("{}.jsonl", record.session_id));
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await?;

        use tokio::io::AsyncWriteExt;
        let line = serde_json::to_string(record)?;
        file.write_all(line.as_bytes()).await?;
        file.write_all(b"\n").await?;

        Ok(path)
    }

    /// Load every record found under the trajectory root.
    pub fn load_records(&self) -> anyhow::Result<Vec<TrajectoryTurnRecord>> {
        if !self.log_root.exists() {
            return Ok(Vec::new());
        }

        let mut records = Vec::new();
        for path in collect_jsonl_files(&self.log_root)? {
            let content = std::fs::read_to_string(&path)?;
            for line in content
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
            {
                let record: TrajectoryTurnRecord = serde_json::from_str(line)?;
                records.push(record);
            }
        }
        Ok(records)
    }

    /// Summarize the archive for CLI stats output.
    pub fn stats(&self) -> anyhow::Result<TrajectoryStats> {
        let files = if self.log_root.exists() {
            collect_jsonl_files(&self.log_root)?
        } else {
            Vec::new()
        };
        let records = self.load_records()?;
        let mut session_ids = std::collections::BTreeSet::new();
        let mut first_seen: Option<DateTime<Utc>> = None;
        let mut last_seen: Option<DateTime<Utc>> = None;
        let mut success_count = 0;
        let mut failure_count = 0;
        let mut neutral_count = 0;

        for record in &records {
            session_ids.insert(record.session_id);
            let ts = record.completed_at.unwrap_or(record.started_at);
            first_seen = Some(first_seen.map_or(ts, |current| current.min(ts)));
            last_seen = Some(last_seen.map_or(ts, |current| current.max(ts)));
            match record.outcome {
                TrajectoryOutcome::Success => success_count += 1,
                TrajectoryOutcome::Failure => failure_count += 1,
                TrajectoryOutcome::Neutral => neutral_count += 1,
            }
        }

        Ok(TrajectoryStats {
            log_root: self.log_root.clone(),
            file_count: files.len(),
            record_count: records.len(),
            session_count: session_ids.len(),
            first_seen,
            last_seen,
            success_count,
            failure_count,
            neutral_count,
        })
    }
}

fn default_trajectory_root() -> PathBuf {
    crate::platform::resolve_data_dir("trajectories")
}

fn collect_jsonl_files(root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    fn visit(dir: &Path, output: &mut Vec<PathBuf>) -> anyhow::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                visit(&path, output)?;
            } else if path.extension().is_some_and(|ext| ext == "jsonl") {
                output.push(path);
            }
        }
        Ok(())
    }

    let mut files = Vec::new();
    if root.exists() {
        visit(root, &mut files)?;
    }
    files.sort();
    Ok(files)
}
