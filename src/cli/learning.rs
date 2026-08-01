//! Durable learning-ledger administration through the running gateway.

use std::io::{IsTerminal, Write};
use std::path::PathBuf;

use clap::{Subcommand, ValueEnum};
use serde::Serialize;
use uuid::Uuid;

use super::{CliContext, CliError, GatewayClient};

#[derive(Subcommand, Debug, Clone)]
pub enum LearningCommand {
    /// Summarize learning configuration and recent durable activity
    Status {
        #[arg(long, default_value_t = 25)]
        recent: usize,
        /// Include bounded live provider probes
        #[arg(long)]
        live: bool,
    },
    /// Read one durable learning ledger
    History {
        #[arg(value_enum)]
        kind: LearningHistoryKind,
        #[arg(long)]
        channel: Option<String>,
        #[arg(long)]
        thread_id: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    #[command(subcommand)]
    Outcomes(LearningOutcomesCommand),
    #[command(subcommand)]
    Feedback(LearningFeedbackCommand),
    #[command(subcommand)]
    Proposals(LearningProposalsCommand),
    #[command(subcommand)]
    Rollbacks(LearningRollbacksCommand),
    #[command(subcommand)]
    ExternalMemory(ExternalMemoryCommand),
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum LearningHistoryKind {
    Events,
    Candidates,
    Versions,
    Evaluations,
    Feedback,
    Proposals,
    Rollbacks,
    All,
}

#[derive(Subcommand, Debug, Clone)]
pub enum LearningOutcomesCommand {
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        contract_type: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    Show {
        id: Uuid,
    },
    Review {
        id: Uuid,
        #[arg(long, value_enum)]
        decision: OutcomeDecision,
        #[arg(long)]
        verdict: Option<String>,
        #[arg(long)]
        yes: bool,
    },
    EvaluateNow {
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeDecision {
    Confirm,
    Dismiss,
    Requeue,
}

#[derive(Subcommand, Debug, Clone)]
pub enum LearningFeedbackCommand {
    Submit {
        target_type: String,
        target_id: String,
        #[arg(long)]
        verdict: String,
        #[arg(long)]
        note: Option<String>,
        #[arg(long)]
        metadata_file: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum LearningProposalsCommand {
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    Show {
        id: Uuid,
    },
    Review {
        id: Uuid,
        #[arg(long, value_enum)]
        decision: ProposalDecision,
        #[arg(long)]
        note: Option<String>,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalDecision {
    Approve,
    Reject,
}

#[derive(Subcommand, Debug, Clone)]
pub enum LearningRollbacksCommand {
    Record {
        artifact_type: String,
        artifact_name: String,
        #[arg(long)]
        reason: String,
        #[arg(long)]
        version: Option<Uuid>,
        #[arg(long)]
        metadata_file: Option<PathBuf>,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ExternalMemoryCommand {
    /// Show provider configuration and optionally live health
    Status {
        #[arg(long)]
        live: bool,
    },
}

pub async fn run_learning_command(
    command: LearningCommand,
    context: &CliContext,
) -> Result<(), CliError> {
    let config = context.config().await?;
    let client = GatewayClient::resolve_from_config(None, None, config)
        .map_err(|error| CliError::operational(error.to_string()))?;
    match command {
        LearningCommand::Status { recent, live } => {
            validate_limit(recent)?;
            let mut value: serde_json::Value = client
                .get_json("/api/learning/status", &ListQuery::limit(recent))
                .await
                .map_err(gateway_error)?;
            if live {
                let health: serde_json::Value = client
                    .get_json("/api/learning/provider-health", &ListQuery::limit(recent))
                    .await
                    .map_err(gateway_error)?;
                value["live_provider_health"] = health;
            }
            write_value(context, "data.learning.status", value)
        }
        LearningCommand::History {
            kind,
            channel,
            thread_id,
            limit,
        } => history(kind, channel, thread_id, limit, context, &client).await,
        LearningCommand::Outcomes(command) => outcomes(command, context, &client).await,
        LearningCommand::Feedback(command) => feedback(command, context, &client).await,
        LearningCommand::Proposals(command) => proposals(command, context, &client).await,
        LearningCommand::Rollbacks(command) => rollbacks(command, context, &client).await,
        LearningCommand::ExternalMemory(ExternalMemoryCommand::Status { live }) => {
            let path = if live {
                "/api/learning/provider-health"
            } else {
                "/api/learning/status"
            };
            let value: serde_json::Value = client
                .get_json(path, &ListQuery::limit(25))
                .await
                .map_err(gateway_error)?;
            write_value(context, "data.learning.external-memory.status", value)
        }
    }
}

async fn history(
    kind: LearningHistoryKind,
    channel: Option<String>,
    thread_id: Option<String>,
    limit: usize,
    context: &CliContext,
    client: &GatewayClient,
) -> Result<(), CliError> {
    validate_limit(limit)?;
    let query = ListQuery {
        limit,
        channel: channel.as_deref(),
        thread_id: thread_id.as_deref(),
        status: None,
        contract_type: None,
    };
    let path = match kind {
        LearningHistoryKind::Events | LearningHistoryKind::Evaluations => "/api/learning/history",
        LearningHistoryKind::Candidates => "/api/learning/candidates",
        LearningHistoryKind::Versions => "/api/learning/artifact-versions",
        LearningHistoryKind::Feedback => "/api/learning/feedback",
        LearningHistoryKind::Proposals => "/api/learning/code-proposals",
        LearningHistoryKind::Rollbacks => "/api/learning/rollbacks",
        LearningHistoryKind::All => {
            let paths = [
                ("events", "/api/learning/history"),
                ("candidates", "/api/learning/candidates"),
                ("versions", "/api/learning/artifact-versions"),
                ("feedback", "/api/learning/feedback"),
                ("proposals", "/api/learning/code-proposals"),
                ("rollbacks", "/api/learning/rollbacks"),
            ];
            let mut ledgers = serde_json::Map::new();
            for (name, path) in paths {
                let value: serde_json::Value =
                    client.get_json(path, &query).await.map_err(gateway_error)?;
                ledgers.insert(name.to_string(), value);
            }
            return write_value(
                context,
                "data.learning.history",
                serde_json::Value::Object(ledgers),
            );
        }
    };
    let mut value: serde_json::Value =
        client.get_json(path, &query).await.map_err(gateway_error)?;
    if matches!(kind, LearningHistoryKind::Events) {
        value
            .as_object_mut()
            .map(|object| object.remove("evaluations"));
    } else if matches!(kind, LearningHistoryKind::Evaluations) {
        value.as_object_mut().map(|object| object.remove("events"));
    }
    write_value(context, "data.learning.history", value)
}

async fn outcomes(
    command: LearningOutcomesCommand,
    context: &CliContext,
    client: &GatewayClient,
) -> Result<(), CliError> {
    match command {
        LearningOutcomesCommand::List {
            status,
            contract_type,
            limit,
        } => {
            validate_limit(limit)?;
            let value: serde_json::Value = client
                .get_json(
                    "/api/learning/outcomes",
                    &ListQuery {
                        limit,
                        channel: None,
                        thread_id: None,
                        status: status.as_deref(),
                        contract_type: contract_type.as_deref(),
                    },
                )
                .await
                .map_err(gateway_error)?;
            write_value(context, "data.learning.outcomes.list", value)
        }
        LearningOutcomesCommand::Show { id } => {
            let value: serde_json::Value = client
                .get_json(&format!("/api/learning/outcomes/{id}"), &EmptyQuery {})
                .await
                .map_err(gateway_error)?;
            write_value(context, "data.learning.outcomes.show", value)
        }
        LearningOutcomesCommand::Review {
            id,
            decision,
            verdict,
            yes,
        } => {
            if matches!(decision, OutcomeDecision::Confirm) && verdict.is_none() {
                return Err(CliError::usage(
                    "--verdict is required with --decision confirm",
                ));
            }
            confirm("review learning outcome", &id.to_string(), yes)?;
            let value: serde_json::Value = client
                .post_json(
                    &format!("/api/learning/outcomes/{id}/review"),
                    &serde_json::json!({"decision": decision, "verdict": verdict}),
                )
                .await
                .map_err(gateway_error)?;
            write_value(context, "data.learning.outcomes.review", value)
        }
        LearningOutcomesCommand::EvaluateNow { yes } => {
            confirm("run billable learning evaluation", "due outcomes", yes)?;
            let value: serde_json::Value = client
                .post_json(
                    "/api/learning/outcomes/evaluate-now",
                    &serde_json::json!({}),
                )
                .await
                .map_err(gateway_error)?;
            write_value(context, "data.learning.outcomes.evaluate-now", value)
        }
    }
}

async fn feedback(
    command: LearningFeedbackCommand,
    context: &CliContext,
    client: &GatewayClient,
) -> Result<(), CliError> {
    let LearningFeedbackCommand::Submit {
        target_type,
        target_id,
        verdict,
        note,
        metadata_file,
    } = command;
    let metadata = metadata_file.as_deref().map(read_json_object).transpose()?;
    let value: serde_json::Value = client
        .post_json(
            "/api/learning/feedback",
            &serde_json::json!({
                "target_type": target_type,
                "target_id": target_id,
                "verdict": verdict,
                "note": note,
                "metadata": metadata
            }),
        )
        .await
        .map_err(gateway_error)?;
    write_value(context, "data.learning.feedback.submit", value)
}

async fn proposals(
    command: LearningProposalsCommand,
    context: &CliContext,
    client: &GatewayClient,
) -> Result<(), CliError> {
    match command {
        LearningProposalsCommand::List { status, limit } => {
            validate_limit(limit)?;
            let value: serde_json::Value = client
                .get_json(
                    "/api/learning/code-proposals",
                    &ListQuery {
                        limit,
                        channel: None,
                        thread_id: None,
                        status: status.as_deref(),
                        contract_type: None,
                    },
                )
                .await
                .map_err(gateway_error)?;
            write_value(context, "data.learning.proposals.list", value)
        }
        LearningProposalsCommand::Show { id } => {
            let id_string = id.to_string();
            let value: serde_json::Value = client
                .get_json("/api/learning/code-proposals", &ListQuery::limit(500))
                .await
                .map_err(gateway_error)?;
            let proposal = value
                .get("proposals")
                .and_then(serde_json::Value::as_array)
                .and_then(|proposals| {
                    proposals.iter().find(|proposal| {
                        proposal.get("id").and_then(serde_json::Value::as_str)
                            == Some(id_string.as_str())
                    })
                })
                .cloned()
                .ok_or_else(|| CliError::operational(format!("proposal {id} not found")))?;
            write_value(context, "data.learning.proposals.show", proposal)
        }
        LearningProposalsCommand::Review {
            id,
            decision,
            note,
            yes,
        } => {
            confirm("review code proposal", &id.to_string(), yes)?;
            let value: serde_json::Value = client
                .post_json(
                    &format!("/api/learning/code-proposals/{id}/review"),
                    &serde_json::json!({"decision": decision, "note": note}),
                )
                .await
                .map_err(gateway_error)?;
            write_value(context, "data.learning.proposals.review", value)
        }
    }
}

async fn rollbacks(
    command: LearningRollbacksCommand,
    context: &CliContext,
    client: &GatewayClient,
) -> Result<(), CliError> {
    let LearningRollbacksCommand::Record {
        artifact_type,
        artifact_name,
        reason,
        version,
        metadata_file,
        yes,
    } = command;
    confirm("record rollback observation", &artifact_name, yes)?;
    let metadata = metadata_file.as_deref().map(read_json_object).transpose()?;
    let mut value: serde_json::Value = client
        .post_json(
            "/api/learning/rollbacks",
            &serde_json::json!({
                "artifact_type": artifact_type,
                "artifact_name": artifact_name,
                "artifact_version_id": version,
                "reason": reason,
                "metadata": metadata
            }),
        )
        .await
        .map_err(gateway_error)?;
    value["artifact_restored"] = serde_json::Value::Bool(false);
    write_value(context, "data.learning.rollbacks.record", value)
}

#[derive(Serialize)]
struct ListQuery<'a> {
    limit: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    channel: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    contract_type: Option<&'a str>,
}

impl ListQuery<'static> {
    fn limit(limit: usize) -> Self {
        Self {
            limit,
            channel: None,
            thread_id: None,
            status: None,
            contract_type: None,
        }
    }
}

#[derive(Serialize)]
struct EmptyQuery {}

fn validate_limit(limit: usize) -> Result<(), CliError> {
    if (1..=500).contains(&limit) {
        Ok(())
    } else {
        Err(CliError::usage("learning limit must be between 1 and 500"))
    }
}

fn read_json_object(path: &std::path::Path) -> Result<serde_json::Value, CliError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        CliError::operational(format!("cannot inspect {}: {error}", path.display()))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > 64 * 1024 {
        return Err(CliError::usage(
            "metadata must be a regular non-symlink JSON file no larger than 64 KiB",
        ));
    }
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).map_err(|error| {
            CliError::operational(format!("failed to read {}: {error}", path.display()))
        })?)
        .map_err(|error| CliError::usage(format!("metadata is not valid JSON: {error}")))?;
    if !value.is_object() {
        return Err(CliError::usage("metadata JSON must be an object"));
    }
    Ok(value)
}

fn confirm(action: &str, target: &str, yes: bool) -> Result<(), CliError> {
    if yes {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        return Err(CliError::usage(format!(
            "{action} requires --yes in noninteractive mode"
        )));
    }
    eprint!("{action} '{target}'? [y/N] ");
    std::io::stderr()
        .flush()
        .map_err(|error| CliError::operational(error.to_string()))?;
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .map_err(|error| CliError::operational(error.to_string()))?;
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        Err(CliError::operational(format!("{action} cancelled")))
    }
}

fn gateway_error(error: super::GatewayClientError) -> CliError {
    CliError::operational(error.to_string())
}

fn write_value(
    context: &CliContext,
    command: &'static str,
    value: serde_json::Value,
) -> Result<(), CliError> {
    context.output().write_record(command, &value, |value| {
        serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
    })
}
