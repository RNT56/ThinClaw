//! Authenticated control of the running runtime's shared sub-agent executor.

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand, ValueEnum};
use serde::Serialize;
use thinclaw_gateway::web::types::{
    SubagentCancelApiResponse, SubagentRunApiResponse, SubagentRunsApiResponse,
    SubagentSpawnApiRequest, SubagentSpawnApiResponse,
};
use thinclaw_types::{
    SubagentMemoryMode, SubagentSkillMode, SubagentTaskPacket, SubagentToolMode, ToolProfile,
};
use uuid::Uuid;

use crate::cli::{CliContext, CliError, GatewayClient};

const MAX_TASK_PACKET_BYTES: u64 = 1024 * 1024;

#[derive(Args, Debug, Clone, Default)]
pub struct SubagentGatewayArgs {
    /// Explicit running-runtime URL (otherwise config/environment is used)
    #[arg(long)]
    pub gateway_url: Option<String>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum SubagentMemoryModeArg {
    ProvidedContextOnly,
    GrantedToolsOnly,
}

impl From<SubagentMemoryModeArg> for SubagentMemoryMode {
    fn from(value: SubagentMemoryModeArg) -> Self {
        match value {
            SubagentMemoryModeArg::ProvidedContextOnly => Self::ProvidedContextOnly,
            SubagentMemoryModeArg::GrantedToolsOnly => Self::GrantedToolsOnly,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum SubagentToolModeArg {
    ExplicitOnly,
}

impl From<SubagentToolModeArg> for SubagentToolMode {
    fn from(_value: SubagentToolModeArg) -> Self {
        Self::ExplicitOnly
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum SubagentSkillModeArg {
    ExplicitOnly,
}

impl From<SubagentSkillModeArg> for SubagentSkillMode {
    fn from(_value: SubagentSkillModeArg) -> Self {
        Self::ExplicitOnly
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum SubagentToolProfileArg {
    Standard,
    Restricted,
    ExplicitOnly,
}

impl From<SubagentToolProfileArg> for ToolProfile {
    fn from(value: SubagentToolProfileArg) -> Self {
        match value {
            SubagentToolProfileArg::Standard => Self::Standard,
            SubagentToolProfileArg::Restricted => Self::Restricted,
            SubagentToolProfileArg::ExplicitOnly => Self::ExplicitOnly,
        }
    }
}

#[derive(Args, Debug, Clone)]
#[group(required = true, multiple = true, args = ["task", "task_packet"])]
pub struct SubagentSpawnArgs {
    /// Short display name for the sub-agent
    #[arg(long)]
    pub name: String,
    /// Task objective (optional when --task-packet supplies one)
    #[arg(long)]
    pub task: Option<String>,
    /// UTF-8 JSON file containing the canonical SubagentTaskPacket
    #[arg(long, value_name = "PATH")]
    pub task_packet: Option<PathBuf>,
    /// Explicit tool grant (repeat for multiple tools)
    #[arg(long = "tool")]
    pub tools: Vec<String>,
    /// Explicit skill grant (repeat for multiple skills)
    #[arg(long = "skill")]
    pub skills: Vec<String>,
    #[arg(long, value_enum)]
    pub memory_mode: Option<SubagentMemoryModeArg>,
    #[arg(long, value_enum)]
    pub tool_mode: Option<SubagentToolModeArg>,
    #[arg(long, value_enum)]
    pub skill_mode: Option<SubagentSkillModeArg>,
    #[arg(long, value_enum)]
    pub tool_profile: Option<SubagentToolProfileArg>,
    /// Optional custom system prompt
    #[arg(long)]
    pub system_prompt: Option<String>,
    /// Optional model override
    #[arg(long)]
    pub model: Option<String>,
    /// Runtime timeout in seconds
    #[arg(long)]
    pub timeout_secs: Option<u64>,
    /// Wait for completion and include the final response
    #[arg(long)]
    pub wait: bool,
    /// Durable parent thread identifier for correlation
    #[arg(long)]
    pub parent_thread_id: Option<String>,
    #[command(flatten)]
    pub gateway: SubagentGatewayArgs,
}

#[derive(Subcommand, Debug, Clone)]
pub enum SubagentCommand {
    /// Spawn through the running runtime's shared executor
    Spawn(SubagentSpawnArgs),
    /// List durable runs, including completed runs
    List {
        /// Filter by running, completed, failed, timed_out, or cancelled
        #[arg(long)]
        status: Option<String>,
        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[command(flatten)]
        gateway: SubagentGatewayArgs,
    },
    /// Show one durable run
    Status {
        id: Uuid,
        #[command(flatten)]
        gateway: SubagentGatewayArgs,
    },
    /// Cancel a running sub-agent
    Cancel {
        id: Uuid,
        #[command(flatten)]
        gateway: SubagentGatewayArgs,
    },
}

#[derive(Debug, Serialize)]
struct EmptyQuery {}

#[derive(Debug, Serialize)]
struct ListQuery<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<&'a str>,
    limit: usize,
}

fn read_task_packet(path: &Path) -> Result<SubagentTaskPacket, CliError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        CliError::usage(format!(
            "task packet '{}' is unavailable: {error}",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(CliError::usage(format!(
            "task packet '{}' must be a regular non-symlink file",
            path.display()
        )));
    }
    if metadata.len() > MAX_TASK_PACKET_BYTES {
        return Err(CliError::usage(format!(
            "task packet exceeds the {MAX_TASK_PACKET_BYTES}-byte limit"
        )));
    }
    let bytes = std::fs::read(path).map_err(|error| {
        CliError::usage(format!(
            "failed to read task packet '{}': {error}",
            path.display()
        ))
    })?;
    serde_json::from_slice(&bytes)
        .map_err(|error| CliError::usage(format!("invalid task packet JSON: {error}")))
}

async fn gateway_client(
    context: &CliContext,
    gateway_url: Option<&str>,
) -> Result<GatewayClient, CliError> {
    let config = context.config().await?;
    GatewayClient::resolve_from_config(gateway_url, None, config)
        .map_err(|error| CliError::operational(error.to_string()))
}

pub async fn run_subagents_command(
    command: SubagentCommand,
    context: &CliContext,
) -> Result<(), CliError> {
    match command {
        SubagentCommand::Spawn(args) => {
            let packet = args
                .task_packet
                .as_deref()
                .map(read_task_packet)
                .transpose()?;
            let task = args
                .task
                .or_else(|| packet.as_ref().map(|packet| packet.objective.clone()))
                .unwrap_or_default();
            let request = SubagentSpawnApiRequest {
                name: args.name,
                task,
                system_prompt: args.system_prompt,
                model: args.model,
                task_packet: packet,
                memory_mode: args.memory_mode.map(Into::into),
                tool_mode: args.tool_mode.map(Into::into),
                skill_mode: args.skill_mode.map(Into::into),
                tool_profile: args.tool_profile.map(Into::into),
                allowed_tools: (!args.tools.is_empty()).then_some(args.tools),
                allowed_skills: (!args.skills.is_empty()).then_some(args.skills),
                timeout_secs: args.timeout_secs,
                wait: args.wait,
                parent_thread_id: args.parent_thread_id,
            };
            let client = gateway_client(context, args.gateway.gateway_url.as_deref()).await?;
            let response: SubagentSpawnApiResponse = client
                .post_json("/api/subagents", &request)
                .await
                .map_err(gateway_error)?;
            context
                .output()
                .write_record("subagents.spawn", &response, |response| {
                    if let Some(result) = response.result.as_ref() {
                        format!(
                            "{}\t{}\t{}\n{}",
                            response.run.id,
                            response.run.status,
                            response.run.name,
                            result.response
                        )
                    } else {
                        format!(
                            "{}\t{}\t{}",
                            response.run.id, response.run.status, response.run.name
                        )
                    }
                })
        }
        SubagentCommand::List {
            status,
            limit,
            gateway,
        } => {
            if !(1..=500).contains(&limit) {
                return Err(CliError::usage(
                    "sub-agent list limit must be between 1 and 500",
                ));
            }
            let client = gateway_client(context, gateway.gateway_url.as_deref()).await?;
            let response: SubagentRunsApiResponse = client
                .get_json(
                    "/api/subagents",
                    &ListQuery {
                        status: status.as_deref(),
                        limit,
                    },
                )
                .await
                .map_err(gateway_error)?;
            context
                .output()
                .write_record("subagents.list", &response, |response| {
                    if response.runs.is_empty() {
                        "No sub-agent runs found.".to_string()
                    } else {
                        response
                            .runs
                            .iter()
                            .map(|run| {
                                format!(
                                    "{}\t{}\t{}\t{}",
                                    run.id,
                                    run.status,
                                    run.name,
                                    run.spawned_at.to_rfc3339()
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    }
                })
        }
        SubagentCommand::Status { id, gateway } => {
            let client = gateway_client(context, gateway.gateway_url.as_deref()).await?;
            let response: SubagentRunApiResponse = client
                .get_json(&format!("/api/subagents/{id}"), &EmptyQuery {})
                .await
                .map_err(gateway_error)?;
            context
                .output()
                .write_record("subagents.status", &response, |response| {
                    let run = &response.run;
                    format!(
                        "ID: {}\nName: {}\nStatus: {}\nTask: {}",
                        run.id, run.name, run.status, run.task
                    )
                })
        }
        SubagentCommand::Cancel { id, gateway } => {
            let client = gateway_client(context, gateway.gateway_url.as_deref()).await?;
            let response: SubagentCancelApiResponse = client
                .post_json(
                    &format!("/api/subagents/{id}/cancel"),
                    &serde_json::json!({}),
                )
                .await
                .map_err(gateway_error)?;
            context
                .output()
                .write_record("subagents.cancel", &response, |response| {
                    format!("{}\t{}", response.agent_id, response.status)
                })
        }
    }
}

fn gateway_error(error: impl std::fmt::Display) -> CliError {
    CliError::operational(format!(
        "running runtime sub-agent operation failed: {error}"
    ))
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;
    use crate::cli::{Cli, Command};
    use crate::cli::{ColorChoice, OutputFormat, OutputPolicy};
    use thinclaw_types::subagent::SubagentRunRecord;

    #[test]
    fn parser_preserves_task_packet_and_capability_constraints() {
        let cli = Cli::try_parse_from([
            "thinclaw",
            "subagents",
            "spawn",
            "--name",
            "reviewer",
            "--task",
            "Review the change",
            "--tool",
            "memory_read",
            "--skill",
            "rust-review",
            "--tool-profile",
            "explicit-only",
            "--wait",
        ])
        .expect("parse sub-agent spawn");
        let Some(Command::Subagents(SubagentCommand::Spawn(args))) = cli.command else {
            panic!("wrong command");
        };
        assert_eq!(args.name, "reviewer");
        assert_eq!(args.tools, ["memory_read"]);
        assert_eq!(args.skills, ["rust-review"]);
        assert!(args.wait);
    }

    #[test]
    fn parser_exposes_list_status_and_cancel() {
        for args in [
            vec!["thinclaw", "subagents", "list"],
            vec![
                "thinclaw",
                "subagents",
                "status",
                "00000000-0000-0000-0000-000000000001",
            ],
            vec![
                "thinclaw",
                "subagents",
                "cancel",
                "00000000-0000-0000-0000-000000000001",
            ],
        ] {
            assert!(Cli::try_parse_from(args).is_ok());
        }
    }

    #[test]
    fn json_output_contract_is_versioned_and_keeps_completed_runs() {
        let mut run = SubagentRunRecord::new_running(
            Uuid::nil(),
            "reviewer",
            "review",
            "principal-a",
            "actor-a",
            None,
            None,
            chrono::Utc::now(),
        );
        run.status = thinclaw_types::subagent::SUBAGENT_RUN_STATUS_COMPLETED.to_string();
        run.completed_at = Some(chrono::Utc::now());
        let response = SubagentRunsApiResponse { runs: vec![run] };
        let output = OutputPolicy::resolve(
            OutputFormat::Json,
            ColorChoice::Never,
            false,
            false,
            false,
            false,
            false,
        )
        .unwrap()
        .render_record("subagents.list", &response, |_| String::new())
        .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["command"], "subagents.list");
        assert_eq!(value["data"]["runs"][0]["principal_id"], "principal-a");
        assert_eq!(value["data"]["runs"][0]["status"], "completed");
    }
}
