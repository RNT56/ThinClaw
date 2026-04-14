//! Trajectory archive CLI helpers.
//!
//! This module provides a lightweight export/stats surface over the
//! JSONL trajectory archive managed by `agent::learning::TrajectoryLogger`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use clap::Subcommand;
use serde::Serialize;

use crate::agent::learning::{TrajectoryLogger, TrajectoryOutcome, TrajectoryTurnRecord};

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
    },

    /// Show archive statistics.
    Stats,
}

/// Run a trajectory command.
pub async fn run_trajectory_command(cmd: TrajectoryCommand) -> anyhow::Result<()> {
    let logger = TrajectoryLogger::new();

    match cmd {
        TrajectoryCommand::Export { format, output } => {
            let records = sort_records(logger.load_records()?)?;
            let payload = render_export(&records, &format)?;
            if let Some(output) = output {
                if let Some(parent) = output.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                tokio::fs::write(&output, payload).await?;
                println!("Wrote trajectory export to {}", output.display());
            } else {
                print!("{}", payload);
                if !payload.ends_with('\n') {
                    println!();
                }
            }
        }
        TrajectoryCommand::Stats => {
            let stats = logger.stats()?;
            print_stats(&stats);
        }
    }

    Ok(())
}

fn render_export(records: &[TrajectoryTurnRecord], format: &str) -> anyhow::Result<String> {
    match format.to_ascii_lowercase().as_str() {
        "jsonl" | "ndjson" => {
            let mut out = String::new();
            for record in records {
                out.push_str(&serde_json::to_string(record)?);
                out.push('\n');
            }
            Ok(out)
        }
        "json" => Ok(serde_json::to_string_pretty(records)?),
        "sft" => render_jsonl_payload(&build_sft_examples(records)?),
        "dpo" => render_jsonl_payload(&build_dpo_examples(records)?),
        other => anyhow::bail!("unsupported trajectory export format: {}", other),
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
        "outcome": assessment.outcome,
        "score": assessment.score,
        "assessment_source": assessment.source,
        "feedback": record.user_feedback,
    })
}

fn build_sft_examples(records: &[TrajectoryTurnRecord]) -> anyhow::Result<Vec<SftExample>> {
    let mut examples = Vec::new();

    for record in records {
        let response = match record_response(record) {
            Some(response) => response,
            None => continue,
        };
        let assessment = record.effective_assessment();
        if assessment.outcome == TrajectoryOutcome::Failure || assessment.score < 0.6 {
            continue;
        }
        examples.push(SftExample {
            messages: vec![
                ExportMessage {
                    role: "user",
                    content: record.user_message.clone(),
                },
                ExportMessage {
                    role: "assistant",
                    content: response,
                },
            ],
            metadata: record_export_metadata(record),
        });
    }

    Ok(examples)
}

fn build_dpo_examples(records: &[TrajectoryTurnRecord]) -> anyhow::Result<Vec<DpoExample>> {
    let mut groups: BTreeMap<String, Vec<&TrajectoryTurnRecord>> = BTreeMap::new();
    for record in records {
        if record_response(record).is_none() {
            continue;
        }
        groups
            .entry(normalized_prompt_key(&record.user_message))
            .or_default()
            .push(record);
    }

    let mut pairs = Vec::new();
    for candidates in groups.values_mut() {
        if candidates.len() < 2 {
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
        if score_gap < 0.3
            || chosen_assessment.outcome == TrajectoryOutcome::Failure
            || rejected_assessment.outcome == TrajectoryOutcome::Success
        {
            continue;
        }

        let Some(chosen_response) = record_response(chosen) else {
            continue;
        };
        let Some(rejected_response) = record_response(rejected) else {
            continue;
        };
        if chosen_response == rejected_response {
            continue;
        }

        pairs.push(DpoExample {
            prompt: vec![ExportMessage {
                role: "user",
                content: chosen.user_message.clone(),
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

    Ok(pairs)
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
    use crate::agent::learning::{TrajectoryAssessment, TrajectoryOutcome};
    use uuid::Uuid;

    #[test]
    fn render_export_rejects_unknown_format() {
        let err = render_export(&[], "bogus").unwrap_err();
        assert!(
            err.to_string()
                .contains("unsupported trajectory export format")
        );
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
                outcome: TrajectoryOutcome::Success,
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
                outcome: TrajectoryOutcome::Success,
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
                outcome: TrajectoryOutcome::Success,
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
                    outcome: TrajectoryOutcome::Success,
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
                    outcome: TrajectoryOutcome::Failure,
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
}
