use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thinclaw_channels_core::IncomingMessage;
use uuid::Uuid;

use crate::session::{Session, Thread, Turn, TurnState, TurnToolCall};

const MAX_TRAJECTORY_RECORD_BYTES: usize = 8 * 1024 * 1024;
const MAX_TRAJECTORY_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TRAJECTORY_TOTAL_READ_BYTES: u64 = 256 * 1024 * 1024;
const MAX_TRAJECTORY_FILES: usize = 4096;
const MAX_TRAJECTORY_ENTRIES: usize = 8192;
const MAX_TRAJECTORY_DEPTH: usize = 4;

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
    pub tool_calls: Vec<TurnToolCall>,
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
        session: &Session,
        thread_id: Uuid,
        _thread: &Thread,
        incoming: &IncomingMessage,
        turn: &Turn,
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
    pub fn classify_turn(turn: &Turn) -> TrajectoryOutcome {
        match turn.state {
            TurnState::Completed => TrajectoryOutcome::Success,
            TurnState::Failed => TrajectoryOutcome::Failure,
            TurnState::Interrupted => TrajectoryOutcome::Neutral,
            TurnState::Processing => TrajectoryOutcome::Neutral,
        }
    }

    pub fn turn_status(turn: &Turn) -> TrajectoryTurnStatus {
        match turn.state {
            TurnState::Completed => TrajectoryTurnStatus::Completed,
            TurnState::Failed => TrajectoryTurnStatus::Failed,
            TurnState::Interrupted => TrajectoryTurnStatus::Interrupted,
            TurnState::Processing => TrajectoryTurnStatus::Processing,
        }
    }

    /// Heuristic fallback assessment used when no explicit learning feedback
    /// exists for the turn.
    pub fn heuristic_assessment(turn: &Turn) -> TrajectoryAssessment {
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
            TurnState::Failed => TrajectoryAssessment {
                outcome: TrajectoryOutcome::Failure,
                score: 0.05,
                source: "turn_state".to_string(),
                reasoning: "Turn failed before producing a complete response.".to_string(),
            },
            TurnState::Interrupted => TrajectoryAssessment {
                outcome: TrajectoryOutcome::Neutral,
                score: 0.35,
                source: "turn_state".to_string(),
                reasoning: "Turn was interrupted before it could be evaluated.".to_string(),
            },
            TurnState::Processing => TrajectoryAssessment {
                outcome: TrajectoryOutcome::Neutral,
                score: 0.4,
                source: "turn_state".to_string(),
                reasoning: "Turn was still processing when archived.".to_string(),
            },
            TurnState::Completed => {
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
        ensure_real_directory(&self.log_root).await?;
        ensure_real_directory(&dir).await?;

        let path = dir.join(format!("{}.jsonl", record.session_id));
        let mut line = serde_json::to_vec(record)?;
        line.push(b'\n');
        if line.len() > MAX_TRAJECTORY_RECORD_BYTES {
            anyhow::bail!("trajectory record exceeds the archive limit");
        }
        thinclaw_platform::append_private_file_locked_async(
            path.clone(),
            line,
            MAX_TRAJECTORY_FILE_BYTES,
        )
        .await?;

        Ok(path)
    }

    /// Load every record found under the trajectory root.
    pub fn load_records(&self) -> anyhow::Result<Vec<TrajectoryTurnRecord>> {
        if !self.log_root.exists() {
            return Ok(Vec::new());
        }

        let mut records = Vec::new();
        let mut total_bytes = 0_u64;
        for path in collect_jsonl_files(&self.log_root)? {
            let metadata = std::fs::symlink_metadata(&path)?;
            total_bytes = total_bytes
                .checked_add(metadata.len())
                .ok_or_else(|| anyhow::anyhow!("trajectory archive size overflow"))?;
            if total_bytes > MAX_TRAJECTORY_TOTAL_READ_BYTES {
                anyhow::bail!("trajectory archive exceeds the total read limit");
            }
            let bytes =
                thinclaw_platform::read_regular_file_bounded(&path, MAX_TRAJECTORY_FILE_BYTES)?;
            let content = String::from_utf8(bytes)?;
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
    thinclaw_platform::resolve_data_dir("trajectories")
}

fn collect_jsonl_files(root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    fn visit(
        dir: &Path,
        depth: usize,
        output: &mut Vec<PathBuf>,
        entries_seen: &mut usize,
    ) -> anyhow::Result<()> {
        if depth > MAX_TRAJECTORY_DEPTH {
            return Ok(());
        }
        let metadata = std::fs::symlink_metadata(dir)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            anyhow::bail!("trajectory archive path is not a real directory");
        }
        for entry in std::fs::read_dir(dir)? {
            *entries_seen = entries_seen.saturating_add(1);
            if *entries_seen > MAX_TRAJECTORY_ENTRIES {
                anyhow::bail!("trajectory archive contains too many entries");
            }
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                visit(&path, depth + 1, output, entries_seen)?;
            } else if file_type.is_file() && path.extension().is_some_and(|ext| ext == "jsonl") {
                output.push(path);
                if output.len() > MAX_TRAJECTORY_FILES {
                    anyhow::bail!("trajectory archive contains too many files");
                }
            }
        }
        Ok(())
    }

    let mut files = Vec::new();
    let mut entries_seen = 0_usize;
    match std::fs::symlink_metadata(root) {
        Ok(_) => visit(root, 0, &mut files, &mut entries_seen)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(files),
        Err(error) => return Err(error.into()),
    }
    files.sort();
    Ok(files)
}

async fn ensure_real_directory(path: &Path) -> anyhow::Result<()> {
    tokio::fs::create_dir_all(path).await?;
    let metadata = tokio::fs::symlink_metadata(path).await?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        anyhow::bail!("trajectory archive path is not a real directory");
    }
    Ok(())
}
