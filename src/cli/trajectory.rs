//! Trajectory archive CLI helpers.
//!
//! This module provides a lightweight export/stats surface over the
//! JSONL trajectory archive managed by `agent::learning::TrajectoryLogger`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use clap::Subcommand;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::agent::learning::{
    TrajectoryLogger, TrajectoryOutcome, TrajectoryTurnRecord, TrajectoryTurnStatus,
};
use crate::agent::{AgentRunArtifact, AgentRunArtifactLogger};

/// Minimal trajectory archive commands.
#[derive(Subcommand, Debug, Clone)]
pub enum TrajectoryCommand {
    /// Export trajectory records.
    Export {
        /// Export format (`jsonl`, `json`, `sft`, or `dpo`).
        #[arg(short, long, default_value = "jsonl")]
        format: String,

        /// Optional output file. If omitted, writes to stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Minimum assessment score for SFT examples.
        #[arg(long, default_value_t = 0.7)]
        min_score: f64,

        /// Maximum exported records/examples.
        #[arg(long)]
        max_records: Option<usize>,

        /// Write a sibling `<output>.manifest.json` export manifest.
        #[arg(long, default_value_t = false)]
        with_manifest: bool,
    },

    /// Show archive statistics.
    Stats,
}

/// Run a trajectory command.
pub async fn run_trajectory_command(cmd: TrajectoryCommand) -> anyhow::Result<()> {
    let logger = TrajectoryLogger::new();
    let artifact_logger = AgentRunArtifactLogger::new();

    match cmd {
        TrajectoryCommand::Export {
            format,
            output,
            min_score,
            max_records,
            with_manifest,
        } => {
            let records = sort_records(load_archive_records(&logger, &artifact_logger)?)?;
            let options = ExportOptions {
                min_score,
                max_records,
            };
            let rendered = render_export_with_options(&records, &format, options)?;
            if with_manifest && output.is_none() {
                anyhow::bail!("--with-manifest requires --output");
            }
            if let Some(output) = output {
                if let Some(parent) = output.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                tokio::fs::write(&output, &rendered.payload).await?;
                if with_manifest {
                    let manifest = rendered.manifest();
                    tokio::fs::write(
                        manifest_path_for_output(&output),
                        serde_json::to_string_pretty(&manifest)?,
                    )
                    .await?;
                }
                println!("Wrote trajectory export to {}", output.display());
            } else {
                print!("{}", rendered.payload);
                if !rendered.payload.ends_with('\n') {
                    println!();
                }
            }
        }
        TrajectoryCommand::Stats => {
            let stats = summarize_records(
                logger.log_root().to_path_buf(),
                &load_archive_records(&logger, &artifact_logger)?,
            );
            print_stats(&stats);
        }
    }

    Ok(())
}

fn load_archive_records(
    legacy_logger: &TrajectoryLogger,
    artifact_logger: &AgentRunArtifactLogger,
) -> anyhow::Result<Vec<TrajectoryTurnRecord>> {
    let mut records = legacy_logger.load_records()?;
    records.extend(
        artifact_logger
            .load_records()?
            .into_iter()
            .filter_map(trajectory_record_from_artifact),
    );
    Ok(records)
}

fn trajectory_record_from_artifact(artifact: AgentRunArtifact) -> Option<TrajectoryTurnRecord> {
    let session_id = artifact.session_id?;
    let thread_id = artifact.thread_id?;
    let user_id = artifact.user_id?;
    let actor_id = artifact.actor_id?;
    let channel = artifact.channel?;
    let conversation_scope_id = artifact.conversation_scope_id?;
    let conversation_kind = artifact.conversation_kind?;
    let turn_number = artifact.turn_number?;
    let user_message = artifact.user_message?;

    let turn_status = match artifact.status {
        crate::agent::AgentRunStatus::Completed => TrajectoryTurnStatus::Completed,
        crate::agent::AgentRunStatus::Failed => TrajectoryTurnStatus::Failed,
        crate::agent::AgentRunStatus::Interrupted => TrajectoryTurnStatus::Interrupted,
    };
    let outcome = match turn_status {
        TrajectoryTurnStatus::Completed => TrajectoryOutcome::Success,
        TrajectoryTurnStatus::Failed => TrajectoryOutcome::Failure,
        TrajectoryTurnStatus::Interrupted | TrajectoryTurnStatus::Processing => {
            TrajectoryOutcome::Neutral
        }
    };

    let execution_backend = artifact
        .execution_backend
        .clone()
        .or_else(|| metadata_string(&artifact.metadata, "execution_backend"));
    let llm_provider = metadata_string(&artifact.metadata, "llm_provider")
        .or_else(|| metadata_string(&artifact.metadata, "provider"));
    let llm_model = metadata_string(&artifact.metadata, "llm_model")
        .or_else(|| metadata_string(&artifact.metadata, "model"));
    let prompt_snapshot_hash = artifact
        .prompt_snapshot_hash
        .clone()
        .or_else(|| metadata_string(&artifact.metadata, "prompt_snapshot_hash"));
    let ephemeral_overlay_hash = artifact
        .ephemeral_overlay_hash
        .clone()
        .or_else(|| metadata_string(&artifact.metadata, "ephemeral_overlay_hash"));
    let provider_context_refs = if artifact.provider_context_refs.is_empty() {
        metadata_string_array(&artifact.metadata, "provider_context_refs")
    } else {
        artifact.provider_context_refs.clone()
    };

    Some(TrajectoryTurnRecord {
        session_id,
        thread_id,
        user_id,
        actor_id,
        channel,
        conversation_scope_id,
        conversation_kind,
        external_thread_id: artifact.external_thread_id,
        turn_number,
        user_message,
        assistant_response: artifact.assistant_response,
        tool_calls: artifact.tool_calls,
        started_at: artifact.started_at,
        completed_at: artifact.completed_at,
        turn_status,
        outcome,
        failure_reason: artifact.failure_reason,
        execution_backend,
        llm_provider,
        llm_model,
        prompt_snapshot_hash,
        ephemeral_overlay_hash,
        provider_context_refs,
        user_feedback: None,
        assessment: None,
    })
}

fn metadata_string(metadata: &serde_json::Value, key: &str) -> Option<String> {
    metadata
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn metadata_string_array(metadata: &serde_json::Value, key: &str) -> Vec<String> {
    let Some(values) = metadata.get(key).and_then(|value| value.as_array()) else {
        return Vec::new();
    };
    let mut refs = values
        .iter()
        .filter_map(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    refs.sort();
    refs.dedup();
    refs
}

fn summarize_records(
    log_root: std::path::PathBuf,
    records: &[TrajectoryTurnRecord],
) -> crate::agent::learning::TrajectoryStats {
    let mut session_ids = std::collections::BTreeSet::new();
    let mut first_seen: Option<chrono::DateTime<chrono::Utc>> = None;
    let mut last_seen: Option<chrono::DateTime<chrono::Utc>> = None;
    let mut success_count = 0usize;
    let mut failure_count = 0usize;
    let mut neutral_count = 0usize;

    for record in records {
        session_ids.insert(record.session_id);
        let effective_ts = record.completed_at.unwrap_or(record.started_at);
        first_seen = Some(
            first_seen.map_or(effective_ts, |current: chrono::DateTime<chrono::Utc>| {
                current.min(effective_ts)
            }),
        );
        last_seen = Some(
            last_seen.map_or(effective_ts, |current: chrono::DateTime<chrono::Utc>| {
                current.max(effective_ts)
            }),
        );
        match record.effective_assessment().outcome {
            TrajectoryOutcome::Success => success_count += 1,
            TrajectoryOutcome::Failure => failure_count += 1,
            TrajectoryOutcome::Neutral => neutral_count += 1,
        }
    }

    crate::agent::learning::TrajectoryStats {
        log_root,
        file_count: 0,
        record_count: records.len(),
        session_count: session_ids.len(),
        first_seen,
        last_seen,
        success_count,
        failure_count,
        neutral_count,
    }
}

#[derive(Debug, Clone, Copy)]
struct ExportOptions {
    min_score: f64,
    max_records: Option<usize>,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            min_score: 0.7,
            max_records: None,
        }
    }
}

#[derive(Debug, Clone)]
struct RenderedExport {
    payload: String,
    format: String,
    source_record_count: usize,
    exported_record_count: usize,
    skipped_counts: BTreeMap<String, usize>,
    thresholds: ExportThresholds,
}

#[derive(Debug, Clone, Serialize)]
struct ExportThresholds {
    sft_min_score: f64,
}

#[derive(Debug, Clone, Serialize)]
struct ExportManifest {
    schema_version: u32,
    format: String,
    timestamp: chrono::DateTime<chrono::Utc>,
    source_counts: ExportSourceCounts,
    skipped_counts: BTreeMap<String, usize>,
    thresholds: ExportThresholds,
    content_hash: String,
}

#[derive(Debug, Clone, Serialize)]
struct ExportSourceCounts {
    input_records: usize,
    exported_records: usize,
}

impl RenderedExport {
    fn manifest(&self) -> ExportManifest {
        ExportManifest {
            schema_version: 1,
            format: self.format.clone(),
            timestamp: chrono::Utc::now(),
            source_counts: ExportSourceCounts {
                input_records: self.source_record_count,
                exported_records: self.exported_record_count,
            },
            skipped_counts: self.skipped_counts.clone(),
            thresholds: self.thresholds.clone(),
            content_hash: content_hash(&self.payload),
        }
    }
}

#[cfg(test)]
fn render_export(records: &[TrajectoryTurnRecord], format: &str) -> anyhow::Result<String> {
    Ok(render_export_with_options(records, format, ExportOptions::default())?.payload)
}

fn render_export_with_options(
    records: &[TrajectoryTurnRecord],
    format: &str,
    options: ExportOptions,
) -> anyhow::Result<RenderedExport> {
    let normalized_format = format.to_ascii_lowercase();
    let thresholds = ExportThresholds {
        sft_min_score: options.min_score,
    };
    let mut skipped_counts = BTreeMap::new();
    let payload = match normalized_format.as_str() {
        "jsonl" | "ndjson" => {
            let items = limited_records(records, options.max_records)?;
            count_truncated(&mut skipped_counts, records.len(), items.len());
            render_jsonl_payload(&items)?
        }
        "json" => {
            let items = limited_records(records, options.max_records)?;
            count_truncated(&mut skipped_counts, records.len(), items.len());
            serde_json::to_string_pretty(&items)?
        }
        "sft" => {
            let validated = build_sft_examples(records, options.min_score);
            skipped_counts = validated.skipped_counts;
            let original_len = validated.items.len();
            let items = limit_items(validated.items, options.max_records);
            count_truncated(&mut skipped_counts, original_len, items.len());
            render_jsonl_payload(&items)?
        }
        "dpo" => {
            let validated = build_dpo_examples(records);
            skipped_counts = validated.skipped_counts;
            let original_len = validated.items.len();
            let items = limit_items(validated.items, options.max_records);
            count_truncated(&mut skipped_counts, original_len, items.len());
            render_jsonl_payload(&items)?
        }
        other => anyhow::bail!("unsupported trajectory export format: {}", other),
    };
    let exported_record_count = match normalized_format.as_str() {
        "jsonl" | "ndjson" | "json" => limited_len(records.len(), options.max_records),
        "sft" | "dpo" => payload
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count(),
        _ => 0,
    };

    Ok(RenderedExport {
        payload,
        format: normalized_format,
        source_record_count: records.len(),
        exported_record_count,
        skipped_counts,
        thresholds,
    })
}

fn limited_records(
    records: &[TrajectoryTurnRecord],
    max_records: Option<usize>,
) -> anyhow::Result<Vec<TrajectoryTurnRecord>> {
    Ok(records
        .iter()
        .take(max_records.unwrap_or(usize::MAX))
        .cloned()
        .collect())
}

fn limit_items<T>(items: Vec<T>, max_records: Option<usize>) -> Vec<T> {
    items
        .into_iter()
        .take(max_records.unwrap_or(usize::MAX))
        .collect()
}

fn limited_len(len: usize, max_records: Option<usize>) -> usize {
    max_records.map_or(len, |max| len.min(max))
}

fn content_hash(payload: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(payload.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

fn manifest_path_for_output(output: &Path) -> PathBuf {
    let mut manifest_path = output.as_os_str().to_os_string();
    manifest_path.push(".manifest.json");
    PathBuf::from(manifest_path)
}

#[derive(Debug, Clone)]
struct ValidatedExport<T> {
    items: Vec<T>,
    skipped_counts: BTreeMap<String, usize>,
}

fn count_skip(skipped_counts: &mut BTreeMap<String, usize>, reason: &'static str) {
    *skipped_counts.entry(reason.to_string()).or_insert(0) += 1;
}

fn count_truncated(skipped_counts: &mut BTreeMap<String, usize>, original_len: usize, len: usize) {
    if original_len > len {
        skipped_counts.insert("max_records_limit".to_string(), original_len - len);
    }
}

#[derive(Debug, Clone, Serialize)]
struct ExportMessage {
    role: &'static str,
    content: String,
}

#[derive(Debug, Clone, Serialize)]
struct SftExample {
    messages: Vec<ExportMessage>,
    metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
struct DpoExample {
    prompt: Vec<ExportMessage>,
    chosen: Vec<ExportMessage>,
    rejected: Vec<ExportMessage>,
    metadata: serde_json::Value,
}

fn render_jsonl_payload<T: Serialize>(items: &[T]) -> anyhow::Result<String> {
    let mut out = String::new();
    for item in items {
        out.push_str(&serde_json::to_string(item)?);
        out.push('\n');
    }
    Ok(out)
}

fn normalized_prompt_key(prompt: &str) -> String {
    prompt.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn record_response(record: &TrajectoryTurnRecord) -> Option<String> {
    record
        .assistant_response
        .as_deref()
        .map(str::trim)
        .filter(|response| !response.is_empty())
        .map(ToOwned::to_owned)
}

fn record_export_metadata(record: &TrajectoryTurnRecord) -> serde_json::Value {
    let assessment = record.effective_assessment();
    serde_json::json!({
        "target_id": record.target_id(),
        "session_id": record.session_id,
        "thread_id": record.thread_id,
        "turn_number": record.turn_number,
        "channel": record.channel,
        "turn_status": record.turn_status,
        "outcome": assessment.outcome,
        "score": assessment.score,
        "assessment_source": assessment.source,
        "feedback": record.user_feedback,
        "failure_reason": record.failure_reason,
        "execution_backend": record.execution_backend,
        "llm_provider": record.llm_provider,
        "llm_model": record.llm_model,
        "prompt_snapshot_hash": record.prompt_snapshot_hash,
        "ephemeral_overlay_hash": record.ephemeral_overlay_hash,
        "provider_context_refs": record.provider_context_refs,
    })
}

fn record_user_message(record: &TrajectoryTurnRecord) -> Option<String> {
    let user_message = record.user_message.trim();
    if user_message.is_empty() {
        None
    } else {
        Some(user_message.to_string())
    }
}

fn build_sft_examples(
    records: &[TrajectoryTurnRecord],
    min_score: f64,
) -> ValidatedExport<SftExample> {
    let mut examples = Vec::new();
    let mut skipped_counts = BTreeMap::new();

    for record in records {
        let user_message = match record_user_message(record) {
            Some(user_message) => user_message,
            None => {
                count_skip(&mut skipped_counts, "missing_user_message");
                continue;
            }
        };
        let response = match record_response(record) {
            Some(response) => response,
            None => {
                count_skip(&mut skipped_counts, "missing_assistant_response");
                continue;
            }
        };
        let assessment = record.effective_assessment();
        if record.turn_status == TrajectoryTurnStatus::Failed {
            count_skip(&mut skipped_counts, "failed_turn");
            continue;
        }
        if assessment.outcome != TrajectoryOutcome::Success {
            count_skip(&mut skipped_counts, "non_successful_assessment");
            continue;
        }
        if assessment.score < min_score {
            count_skip(&mut skipped_counts, "below_min_score");
            continue;
        }
        examples.push(SftExample {
            messages: vec![
                ExportMessage {
                    role: "user",
                    content: user_message,
                },
                ExportMessage {
                    role: "assistant",
                    content: response,
                },
            ],
            metadata: record_export_metadata(record),
        });
    }

    ValidatedExport {
        items: examples,
        skipped_counts,
    }
}

fn build_dpo_examples(records: &[TrajectoryTurnRecord]) -> ValidatedExport<DpoExample> {
    let mut groups: BTreeMap<String, Vec<&TrajectoryTurnRecord>> = BTreeMap::new();
    let mut skipped_counts = BTreeMap::new();
    for record in records {
        if record_response(record).is_none() {
            count_skip(&mut skipped_counts, "missing_assistant_response");
            continue;
        }
        let Some(user_message) = record_user_message(record) else {
            count_skip(&mut skipped_counts, "missing_user_message");
            continue;
        };
        groups
            .entry(normalized_prompt_key(&user_message))
            .or_default()
            .push(record);
    }

    let mut pairs = Vec::new();
    for candidates in groups.values_mut() {
        if candidates.len() < 2 {
            count_skip(&mut skipped_counts, "no_matched_pair");
            continue;
        }
        candidates.sort_by(|left, right| {
            right
                .preference_score()
                .partial_cmp(&left.preference_score())
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let chosen = candidates[0];
        let rejected = candidates[candidates.len() - 1];
        let chosen_assessment = chosen.effective_assessment();
        let rejected_assessment = rejected.effective_assessment();
        let score_gap = chosen_assessment.score - rejected_assessment.score;
        if score_gap <= 0.0 || chosen_assessment.score == rejected_assessment.score {
            count_skip(&mut skipped_counts, "scores_not_distinct");
            continue;
        }
        if chosen_assessment.outcome == TrajectoryOutcome::Failure {
            count_skip(&mut skipped_counts, "chosen_failed_assessment");
            continue;
        }
        if rejected_assessment.outcome == TrajectoryOutcome::Success {
            count_skip(&mut skipped_counts, "rejected_successful_assessment");
            continue;
        }

        let Some(chosen_response) = record_response(chosen) else {
            count_skip(&mut skipped_counts, "missing_chosen_response");
            continue;
        };
        let Some(rejected_response) = record_response(rejected) else {
            count_skip(&mut skipped_counts, "missing_rejected_response");
            continue;
        };
        if chosen_response == rejected_response {
            count_skip(&mut skipped_counts, "duplicate_response");
            continue;
        }

        pairs.push(DpoExample {
            prompt: vec![ExportMessage {
                role: "user",
                content: record_user_message(chosen).unwrap_or_default(),
            }],
            chosen: vec![ExportMessage {
                role: "assistant",
                content: chosen_response,
            }],
            rejected: vec![ExportMessage {
                role: "assistant",
                content: rejected_response,
            }],
            metadata: serde_json::json!({
                "chosen": record_export_metadata(chosen),
                "rejected": record_export_metadata(rejected),
                "score_gap": score_gap,
            }),
        });
    }

    ValidatedExport {
        items: pairs,
        skipped_counts,
    }
}

fn print_stats(stats: &crate::agent::learning::TrajectoryStats) {
    println!("Trajectory Archive");
    println!("  Root:     {}", stats.log_root.display());
    println!("  Files:    {}", stats.file_count);
    println!("  Records:  {}", stats.record_count);
    println!("  Sessions: {}", stats.session_count);

    if let Some(first) = stats.first_seen {
        println!("  First:    {}", first.to_rfc3339());
    }
    if let Some(last) = stats.last_seen {
        println!("  Last:     {}", last.to_rfc3339());
    }

    println!(
        "  Outcome:   success={}, failure={}, neutral={}",
        stats.success_count, stats.failure_count, stats.neutral_count
    );
}

fn sort_records(
    mut records: Vec<TrajectoryTurnRecord>,
) -> anyhow::Result<Vec<TrajectoryTurnRecord>> {
    records.sort_by_key(|record| {
        (
            record.completed_at.unwrap_or(record.started_at),
            record.session_id,
            record.turn_number,
        )
    });

    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::learning::{TrajectoryAssessment, TrajectoryOutcome, TrajectoryTurnStatus};
    use crate::cli::{Cli, Command};
    use clap::Parser;
    use uuid::Uuid;

    fn test_record(
        prompt: &str,
        response: Option<&str>,
        status: TrajectoryTurnStatus,
        outcome: TrajectoryOutcome,
        score: f64,
    ) -> TrajectoryTurnRecord {
        let now = chrono::Utc::now();
        TrajectoryTurnRecord {
            session_id: Uuid::new_v4(),
            thread_id: Uuid::new_v4(),
            user_id: "u".into(),
            actor_id: "a".into(),
            channel: "cli".into(),
            conversation_scope_id: Uuid::new_v4(),
            conversation_kind: "direct".into(),
            external_thread_id: None,
            turn_number: 1,
            user_message: prompt.into(),
            assistant_response: response.map(str::to_string),
            tool_calls: vec![],
            started_at: now,
            completed_at: Some(now),
            turn_status: status,
            outcome,
            failure_reason: if status == TrajectoryTurnStatus::Failed {
                Some("synthetic failure".into())
            } else {
                None
            },
            execution_backend: Some("interactive_chat".into()),
            llm_provider: Some("test-provider".into()),
            llm_model: Some("test-model".into()),
            prompt_snapshot_hash: Some("sha256:prompt".into()),
            ephemeral_overlay_hash: Some("sha256:overlay".into()),
            provider_context_refs: vec!["ctx-1".into()],
            user_feedback: None,
            assessment: Some(TrajectoryAssessment {
                outcome,
                score,
                source: "test".into(),
                reasoning: "synthetic".into(),
            }),
        }
    }

    #[test]
    fn render_export_rejects_unknown_format() {
        let err = render_export(&[], "bogus").unwrap_err();
        assert!(
            err.to_string()
                .contains("unsupported trajectory export format")
        );
    }

    #[test]
    fn export_flags_parse_defaults_and_overrides() {
        let cli = Cli::try_parse_from([
            "thinclaw",
            "trajectory",
            "export",
            "--format",
            "sft",
            "--output",
            "out.jsonl",
            "--max-records",
            "5",
            "--with-manifest",
        ])
        .expect("parse trajectory export flags");

        let Some(Command::Trajectory(TrajectoryCommand::Export {
            format,
            output,
            min_score,
            max_records,
            with_manifest,
        })) = cli.command
        else {
            panic!("expected trajectory export command");
        };

        assert_eq!(format, "sft");
        assert_eq!(output, Some(PathBuf::from("out.jsonl")));
        assert_eq!(min_score, 0.7);
        assert_eq!(max_records, Some(5));
        assert!(with_manifest);

        let cli = Cli::try_parse_from([
            "thinclaw",
            "trajectory",
            "export",
            "--format",
            "sft",
            "--min-score",
            "0.9",
        ])
        .expect("parse min score override");
        let Some(Command::Trajectory(TrajectoryCommand::Export { min_score, .. })) = cli.command
        else {
            panic!("expected trajectory export command");
        };
        assert_eq!(min_score, 0.9);
    }

    #[test]
    fn sort_records_orders_by_time_then_session() {
        let now = chrono::Utc::now();
        let mut records = vec![
            TrajectoryTurnRecord {
                session_id: Uuid::from_u128(2),
                thread_id: Uuid::new_v4(),
                user_id: "u".into(),
                actor_id: "a".into(),
                channel: "cli".into(),
                conversation_scope_id: Uuid::new_v4(),
                conversation_kind: "direct".into(),
                external_thread_id: None,
                turn_number: 1,
                user_message: "b".into(),
                assistant_response: Some("ok".into()),
                tool_calls: vec![],
                started_at: now,
                completed_at: Some(now),
                turn_status: TrajectoryTurnStatus::Completed,
                outcome: TrajectoryOutcome::Success,
                failure_reason: None,
                execution_backend: Some("interactive_chat".into()),
                llm_provider: None,
                llm_model: None,
                prompt_snapshot_hash: None,
                ephemeral_overlay_hash: None,
                provider_context_refs: Vec::new(),
                user_feedback: None,
                assessment: Some(TrajectoryAssessment {
                    outcome: TrajectoryOutcome::Success,
                    score: 0.95,
                    source: "test".into(),
                    reasoning: "positive".into(),
                }),
            },
            TrajectoryTurnRecord {
                session_id: Uuid::from_u128(1),
                thread_id: Uuid::new_v4(),
                user_id: "u".into(),
                actor_id: "a".into(),
                channel: "cli".into(),
                conversation_scope_id: Uuid::new_v4(),
                conversation_kind: "direct".into(),
                external_thread_id: None,
                turn_number: 0,
                user_message: "a".into(),
                assistant_response: Some("ok".into()),
                tool_calls: vec![],
                started_at: now,
                completed_at: Some(now),
                turn_status: TrajectoryTurnStatus::Completed,
                outcome: TrajectoryOutcome::Success,
                failure_reason: None,
                execution_backend: Some("interactive_chat".into()),
                llm_provider: None,
                llm_model: None,
                prompt_snapshot_hash: None,
                ephemeral_overlay_hash: None,
                provider_context_refs: Vec::new(),
                user_feedback: None,
                assessment: Some(TrajectoryAssessment {
                    outcome: TrajectoryOutcome::Success,
                    score: 0.8,
                    source: "test".into(),
                    reasoning: "positive".into(),
                }),
            },
        ];

        let sorted = sort_records(records.clone()).unwrap();
        assert_eq!(sorted[0].session_id, Uuid::from_u128(1));
        assert_eq!(sorted[1].session_id, Uuid::from_u128(2));
        records.reverse();
        assert_eq!(sort_records(records).unwrap().len(), 2);
    }

    #[test]
    fn render_sft_export_filters_to_positive_examples() {
        let now = chrono::Utc::now();
        let payload = render_export(
            &[TrajectoryTurnRecord {
                session_id: Uuid::new_v4(),
                thread_id: Uuid::new_v4(),
                user_id: "u".into(),
                actor_id: "a".into(),
                channel: "cli".into(),
                conversation_scope_id: Uuid::new_v4(),
                conversation_kind: "direct".into(),
                external_thread_id: None,
                turn_number: 1,
                user_message: "hello".into(),
                assistant_response: Some("hi".into()),
                tool_calls: vec![],
                started_at: now,
                completed_at: Some(now),
                turn_status: TrajectoryTurnStatus::Completed,
                outcome: TrajectoryOutcome::Success,
                failure_reason: None,
                execution_backend: Some("interactive_chat".into()),
                llm_provider: None,
                llm_model: None,
                prompt_snapshot_hash: None,
                ephemeral_overlay_hash: None,
                provider_context_refs: Vec::new(),
                user_feedback: None,
                assessment: Some(TrajectoryAssessment {
                    outcome: TrajectoryOutcome::Success,
                    score: 0.95,
                    source: "test".into(),
                    reasoning: "positive".into(),
                }),
            }],
            "sft",
        )
        .unwrap();

        assert!(payload.contains("\"messages\""));
        assert!(payload.contains("\"hello\""));
        assert!(payload.contains("\"hi\""));
    }

    #[test]
    fn sft_validation_counts_invalid_records() {
        let records = vec![
            test_record(
                "good",
                Some("useful"),
                TrajectoryTurnStatus::Completed,
                TrajectoryOutcome::Success,
                0.95,
            ),
            test_record(
                "",
                Some("missing prompt"),
                TrajectoryTurnStatus::Completed,
                TrajectoryOutcome::Success,
                0.95,
            ),
            test_record(
                "missing response",
                None,
                TrajectoryTurnStatus::Completed,
                TrajectoryOutcome::Success,
                0.95,
            ),
            test_record(
                "failed",
                Some("bad"),
                TrajectoryTurnStatus::Failed,
                TrajectoryOutcome::Failure,
                0.05,
            ),
            test_record(
                "low",
                Some("ok"),
                TrajectoryTurnStatus::Completed,
                TrajectoryOutcome::Success,
                0.65,
            ),
        ];

        let rendered = render_export_with_options(
            &records,
            "sft",
            ExportOptions {
                min_score: 0.7,
                max_records: None,
            },
        )
        .expect("render sft");

        assert_eq!(rendered.exported_record_count, 1);
        assert_eq!(rendered.skipped_counts["missing_user_message"], 1);
        assert_eq!(rendered.skipped_counts["missing_assistant_response"], 1);
        assert_eq!(rendered.skipped_counts["failed_turn"], 1);
        assert_eq!(rendered.skipped_counts["below_min_score"], 1);
        assert!(rendered.payload.contains("useful"));
        assert!(!rendered.payload.contains("missing prompt"));
    }

    #[test]
    fn render_dpo_export_pairs_high_and_low_quality_responses() {
        let now = chrono::Utc::now();
        let prompt = "Explain closures";
        let payload = render_export(
            &[
                TrajectoryTurnRecord {
                    session_id: Uuid::new_v4(),
                    thread_id: Uuid::new_v4(),
                    user_id: "u".into(),
                    actor_id: "a".into(),
                    channel: "cli".into(),
                    conversation_scope_id: Uuid::new_v4(),
                    conversation_kind: "direct".into(),
                    external_thread_id: None,
                    turn_number: 1,
                    user_message: prompt.into(),
                    assistant_response: Some("A closure captures its environment.".into()),
                    tool_calls: vec![],
                    started_at: now,
                    completed_at: Some(now),
                    turn_status: TrajectoryTurnStatus::Completed,
                    outcome: TrajectoryOutcome::Success,
                    failure_reason: None,
                    execution_backend: Some("interactive_chat".into()),
                    llm_provider: None,
                    llm_model: None,
                    prompt_snapshot_hash: None,
                    ephemeral_overlay_hash: None,
                    provider_context_refs: Vec::new(),
                    user_feedback: None,
                    assessment: Some(TrajectoryAssessment {
                        outcome: TrajectoryOutcome::Success,
                        score: 0.95,
                        source: "test".into(),
                        reasoning: "positive".into(),
                    }),
                },
                TrajectoryTurnRecord {
                    session_id: Uuid::new_v4(),
                    thread_id: Uuid::new_v4(),
                    user_id: "u".into(),
                    actor_id: "a".into(),
                    channel: "cli".into(),
                    conversation_scope_id: Uuid::new_v4(),
                    conversation_kind: "direct".into(),
                    external_thread_id: None,
                    turn_number: 2,
                    user_message: prompt.into(),
                    assistant_response: Some("Closures are things.".into()),
                    tool_calls: vec![],
                    started_at: now,
                    completed_at: Some(now),
                    turn_status: TrajectoryTurnStatus::Failed,
                    outcome: TrajectoryOutcome::Failure,
                    failure_reason: Some("synthetic failure".into()),
                    execution_backend: Some("interactive_chat".into()),
                    llm_provider: None,
                    llm_model: None,
                    prompt_snapshot_hash: None,
                    ephemeral_overlay_hash: None,
                    provider_context_refs: Vec::new(),
                    user_feedback: None,
                    assessment: Some(TrajectoryAssessment {
                        outcome: TrajectoryOutcome::Failure,
                        score: 0.05,
                        source: "test".into(),
                        reasoning: "negative".into(),
                    }),
                },
            ],
            "dpo",
        )
        .unwrap();

        assert!(payload.contains("\"chosen\""));
        assert!(payload.contains("\"rejected\""));
        assert!(payload.contains("captures its environment"));
        assert!(payload.contains("Closures are things"));
    }

    #[test]
    fn dpo_validation_requires_distinct_scores() {
        let prompt = "same prompt";
        let records = vec![
            test_record(
                prompt,
                Some("chosen-ish"),
                TrajectoryTurnStatus::Completed,
                TrajectoryOutcome::Success,
                0.8,
            ),
            test_record(
                prompt,
                Some("rejected-ish"),
                TrajectoryTurnStatus::Completed,
                TrajectoryOutcome::Neutral,
                0.8,
            ),
        ];

        let rendered = render_export_with_options(&records, "dpo", ExportOptions::default())
            .expect("render dpo");

        assert_eq!(rendered.exported_record_count, 0);
        assert_eq!(rendered.skipped_counts["scores_not_distinct"], 1);
        assert!(rendered.payload.is_empty());
    }

    #[test]
    fn manifest_reports_counts_thresholds_and_content_hash() {
        let records = vec![
            test_record(
                "one",
                Some("first"),
                TrajectoryTurnStatus::Completed,
                TrajectoryOutcome::Success,
                0.95,
            ),
            test_record(
                "two",
                Some("second"),
                TrajectoryTurnStatus::Completed,
                TrajectoryOutcome::Success,
                0.9,
            ),
        ];
        let rendered = render_export_with_options(
            &records,
            "sft",
            ExportOptions {
                min_score: 0.7,
                max_records: Some(1),
            },
        )
        .expect("render sft with limit");
        let manifest = rendered.manifest();

        assert_eq!(manifest.schema_version, 1);
        assert_eq!(manifest.format, "sft");
        assert_eq!(manifest.source_counts.input_records, 2);
        assert_eq!(manifest.source_counts.exported_records, 1);
        assert_eq!(manifest.skipped_counts["max_records_limit"], 1);
        assert_eq!(manifest.thresholds.sft_min_score, 0.7);
        assert_eq!(manifest.content_hash, content_hash(&rendered.payload));
    }

    #[test]
    fn artifact_conversion_uses_metadata_fallbacks() {
        let now = chrono::Utc::now();
        let session_id = Uuid::new_v4();
        let thread_id = Uuid::new_v4();
        let scope_id = Uuid::new_v4();
        let artifact = AgentRunArtifact::new(
            "chat",
            crate::agent::AgentRunStatus::Completed,
            now,
            Some(now),
        )
        .with_metadata(serde_json::json!({
            "execution_backend": "metadata_backend",
            "llm_provider": "metadata_provider",
            "llm_model": "metadata_model",
            "prompt_snapshot_hash": "sha256:metadata_prompt",
            "ephemeral_overlay_hash": "sha256:metadata_overlay",
            "provider_context_refs": ["ctx-b", "ctx-a", "ctx-a"]
        }));
        let artifact = AgentRunArtifact {
            session_id: Some(session_id),
            thread_id: Some(thread_id),
            user_id: Some("u".into()),
            actor_id: Some("a".into()),
            channel: Some("cli".into()),
            conversation_scope_id: Some(scope_id),
            conversation_kind: Some("direct".into()),
            turn_number: Some(1),
            user_message: Some("prompt".into()),
            assistant_response: Some("response".into()),
            ..artifact
        };

        let record = trajectory_record_from_artifact(artifact).expect("artifact record");

        assert_eq!(
            record.execution_backend.as_deref(),
            Some("metadata_backend")
        );
        assert_eq!(record.llm_provider.as_deref(), Some("metadata_provider"));
        assert_eq!(record.llm_model.as_deref(), Some("metadata_model"));
        assert_eq!(
            record.prompt_snapshot_hash.as_deref(),
            Some("sha256:metadata_prompt")
        );
        assert_eq!(
            record.ephemeral_overlay_hash.as_deref(),
            Some("sha256:metadata_overlay")
        );
        assert_eq!(record.provider_context_refs, vec!["ctx-a", "ctx-b"]);
    }
}
