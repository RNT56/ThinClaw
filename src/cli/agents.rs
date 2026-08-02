//! Durable agent workspace administration.

use clap::Subcommand;
use serde::Serialize;

use crate::agent::agent_registry::AgentRegistry;
use crate::cli::{CliContext, CliError};
use thinclaw_types::AgentWorkspaceRecord;

const DEFAULT_PRINCIPAL: &str = "default";

#[derive(Subcommand, Debug, Clone)]
pub enum AgentCommand {
    /// List registered agent workspaces
    List {
        /// Deprecated per-command output selector; use global --output-format
        #[arg(long, hide = true)]
        format: Option<String>,
    },
    /// Register a new agent workspace
    Add {
        #[arg(long)]
        id: String,
        #[arg(long)]
        display_name: Option<String>,
        #[arg(long)]
        system_prompt: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long, value_delimiter = ',')]
        channels: Vec<String>,
        #[arg(long, value_delimiter = ',')]
        keywords: Vec<String>,
        #[arg(long)]
        default: bool,
    },
    /// Update an existing agent workspace
    Update {
        id: String,
        #[arg(long)]
        display_name: Option<String>,
        #[arg(long, conflicts_with = "clear_system_prompt")]
        system_prompt: Option<String>,
        #[arg(long)]
        clear_system_prompt: bool,
        #[arg(long, conflicts_with = "clear_model")]
        model: Option<String>,
        #[arg(long)]
        clear_model: bool,
        #[arg(long, value_delimiter = ',')]
        channels: Option<Vec<String>>,
        #[arg(long, value_delimiter = ',')]
        keywords: Option<Vec<String>>,
    },
    /// Unregister an agent workspace
    Remove { id: String },
    /// Show one registered agent workspace
    Show { id: String },
    /// Set the default agent workspace
    SetDefault { id: String },
}

#[derive(Debug, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
enum AgentCommandResult {
    List { agents: Vec<AgentWorkspaceRecord> },
    Show { agent: AgentWorkspaceRecord },
    Added { agent: AgentWorkspaceRecord },
    Updated { agent: AgentWorkspaceRecord },
    Removed { agent_id: String },
    DefaultSet { agent: AgentWorkspaceRecord },
}

/// Run an agent command against the same durable registry used at runtime.
pub async fn run_agents_command(cmd: AgentCommand, context: &CliContext) -> Result<(), CliError> {
    let registry = context.agent_registry().await?;
    let result = execute(cmd, &registry).await?;
    context
        .output()
        .write_record("agents", &result, render_human)
}

async fn execute(
    cmd: AgentCommand,
    registry: &AgentRegistry,
) -> Result<AgentCommandResult, CliError> {
    match cmd {
        AgentCommand::List { format: _ } => {
            let workspaces = registry.list_agents().await;
            let mut agents = Vec::with_capacity(workspaces.len());
            for workspace in workspaces {
                let record = registry
                    .get_agent_record(&workspace.agent_id)
                    .await
                    .map_err(registry_error)?
                    .ok_or_else(|| {
                        CliError::operational(format!(
                            "agent '{}' disappeared while reading the durable registry",
                            workspace.agent_id
                        ))
                    })?;
                agents.push(record);
            }
            agents.sort_by(|left, right| left.agent_id.cmp(&right.agent_id));
            Ok(AgentCommandResult::List { agents })
        }
        AgentCommand::Show { id } => {
            let agent = registry
                .get_agent_record(&id)
                .await
                .map_err(registry_error)?
                .ok_or_else(|| CliError::operational(format!("agent '{id}' not found")))?;
            Ok(AgentCommandResult::Show { agent })
        }
        AgentCommand::Add {
            id,
            display_name,
            system_prompt,
            model,
            channels,
            keywords,
            default,
        } => {
            let display_name = display_name.unwrap_or_else(|| id.clone());
            let agent = registry
                .create_agent(
                    DEFAULT_PRINCIPAL,
                    &id,
                    &display_name,
                    system_prompt.as_deref(),
                    model.as_deref(),
                    channels,
                    keywords,
                    default,
                    None,
                    None,
                    None,
                )
                .await
                .map_err(registry_error)?;
            Ok(AgentCommandResult::Added { agent })
        }
        AgentCommand::Update {
            id,
            display_name,
            system_prompt,
            clear_system_prompt,
            model,
            clear_model,
            channels,
            keywords,
        } => {
            let system_prompt = optional_clear(system_prompt.as_deref(), clear_system_prompt);
            let model = optional_clear(model.as_deref(), clear_model);
            let agent = registry
                .update_agent(
                    &id,
                    display_name.as_deref(),
                    system_prompt,
                    model,
                    channels,
                    keywords,
                    None,
                    None,
                    None,
                    None,
                )
                .await
                .map_err(registry_error)?;
            Ok(AgentCommandResult::Updated { agent })
        }
        AgentCommand::Remove { id } => {
            registry
                .remove_agent(&id, false)
                .await
                .map_err(registry_error)?;
            Ok(AgentCommandResult::Removed { agent_id: id })
        }
        AgentCommand::SetDefault { id } => {
            let target = registry
                .get_agent_record(&id)
                .await
                .map_err(registry_error)?
                .ok_or_else(|| CliError::operational(format!("agent '{id}' not found")))?;

            // Keep the persisted invariant explicit: at most one record is the
            // default. Clear old defaults before promoting the selected record.
            for workspace in registry.list_agents().await {
                if workspace.is_default && workspace.agent_id != id {
                    registry
                        .update_agent(
                            &workspace.agent_id,
                            None,
                            None,
                            None,
                            None,
                            None,
                            Some(false),
                            None,
                            None,
                            None,
                        )
                        .await
                        .map_err(registry_error)?;
                }
            }
            let agent = if target.is_default {
                target
            } else {
                registry
                    .update_agent(
                        &id,
                        None,
                        None,
                        None,
                        None,
                        None,
                        Some(true),
                        None,
                        None,
                        None,
                    )
                    .await
                    .map_err(registry_error)?
            };
            Ok(AgentCommandResult::DefaultSet { agent })
        }
    }
}

fn optional_clear(value: Option<&str>, clear: bool) -> Option<Option<&str>> {
    if clear { Some(None) } else { value.map(Some) }
}

fn registry_error(error: impl std::fmt::Display) -> CliError {
    CliError::operational(error.to_string())
}

fn render_human(result: &AgentCommandResult) -> String {
    match result {
        AgentCommandResult::List { agents } if agents.is_empty() => {
            "No agent workspaces are registered.".to_string()
        }
        AgentCommandResult::List { agents } => agents
            .iter()
            .map(|agent| {
                format!(
                    "{}\t{}\t{}\t{}",
                    agent.agent_id,
                    agent.display_name,
                    if agent.is_default { "default" } else { "" },
                    agent.model.as_deref().unwrap_or("inherit")
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        AgentCommandResult::Show { agent } => format_agent(agent),
        AgentCommandResult::Added { agent } => {
            format!(
                "Agent '{}' was saved while the runtime is stopped.",
                agent.agent_id
            )
        }
        AgentCommandResult::Updated { agent } => {
            format!(
                "Agent '{}' was updated while the runtime is stopped.",
                agent.agent_id
            )
        }
        AgentCommandResult::Removed { agent_id } => {
            format!("Agent '{agent_id}' was removed while the runtime is stopped.")
        }
        AgentCommandResult::DefaultSet { agent } => format!(
            "Agent '{}' is now the durable default while the runtime is stopped.",
            agent.agent_id
        ),
    }
}

fn format_agent(agent: &AgentWorkspaceRecord) -> String {
    format!(
        "Agent: {}\nDisplay name: {}\nDefault: {}\nModel: {}\nChannels: {}\nKeywords: {}",
        agent.agent_id,
        agent.display_name,
        agent.is_default,
        agent.model.as_deref().unwrap_or("inherit"),
        if agent.bound_channels.is_empty() {
            "all".to_string()
        } else {
            agent.bound_channels.join(", ")
        },
        if agent.trigger_keywords.is_empty() {
            "none".to_string()
        } else {
            agent.trigger_keywords.join(", ")
        }
    )
}

#[cfg(test)]
mod tests {
    use super::optional_clear;

    #[test]
    fn clear_flags_are_distinct_from_omitted_updates() {
        assert_eq!(optional_clear(None, false), None);
        assert_eq!(optional_clear(None, true), Some(None));
        assert_eq!(optional_clear(Some("value"), false), Some(Some("value")));
    }
}
