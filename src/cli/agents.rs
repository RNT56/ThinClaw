//! Agent management CLI commands.
//!
//! Subcommands:
//! - `agents list`         — list all registered agent workspaces
//! - `agents add`          — register a new agent workspace
//! - `agents remove <id>`  — unregister an agent workspace
//! - `agents show <id>`    — show agent workspace details
//! - `agents set-default`  — set the default agent

use clap::Subcommand;

use crate::agent::{AgentRouter, AgentWorkspace};
use crate::terminal_branding::TerminalBranding;

#[derive(Subcommand, Debug, Clone)]
pub enum AgentCommand {
    /// List all registered agent workspaces
    List {
        /// Output format: table (default) or json
        #[arg(long, default_value = "table")]
        format: String,
    },

    /// Register a new agent workspace
    Add {
        /// Unique agent identifier
        #[arg(long)]
        id: String,

        /// Display name for the agent
        #[arg(long)]
        display_name: Option<String>,

        /// System prompt override
        #[arg(long)]
        system_prompt: Option<String>,

        /// Model override (e.g., "claude-sonnet-4-20250514")
        #[arg(long)]
        model: Option<String>,

        /// Channels this agent is bound to (comma-separated)
        #[arg(long, value_delimiter = ',')]
        channels: Vec<String>,

        /// Keywords/phrases that trigger routing to this agent (comma-separated)
        #[arg(long, value_delimiter = ',')]
        keywords: Vec<String>,

        /// Mark this agent as the default (receives unrouted messages)
        #[arg(long)]
        default: bool,
    },

    /// Unregister an agent workspace
    Remove {
        /// Agent ID to remove
        id: String,
    },

    /// Show details for a specific agent workspace
    Show {
        /// Agent ID to inspect
        id: String,
    },

    /// Set the default agent (receives unrouted messages)
    SetDefault {
        /// Agent ID to make the default
        id: String,
    },
}

/// Run an agents CLI command against the given router.
pub async fn run_agents_command(cmd: AgentCommand, router: &AgentRouter) {
    let branding = TerminalBranding::current();
    match cmd {
        AgentCommand::List { format } => list_agents(router, &format).await,
        AgentCommand::Add {
            id,
            display_name,
            system_prompt,
            model,
            channels,
            keywords,
            default,
        } => {
            let ws = AgentWorkspace {
                workspace_id: None,
                agent_id: id.clone(),
                display_name: display_name.unwrap_or_else(|| id.clone()),
                system_prompt,
                bound_channels: channels,
                trigger_keywords: keywords,
                allowed_tools: None,
                allowed_skills: None,
                tool_profile: None,
                is_default: default,
                model,
            };
            router.register_agent(ws).await;
            branding.print_banner("Agents", Some("Manage routed workspaces"));
            println!("  {}", branding.good(format!("Agent '{}' registered.", id)));
            if default {
                println!("  {}", branding.muted("Also set as the default agent."));
            }
        }
        AgentCommand::Remove { id } => {
            router.unregister_agent(&id).await;
            branding.print_banner("Agents", Some("Manage routed workspaces"));
            println!("  {}", branding.good(format!("Agent '{}' removed.", id)));
        }
        AgentCommand::Show { id } => show_agent(router, &id).await,
        AgentCommand::SetDefault { id } => {
            // Re-register with is_default = true
            if let Some(mut ws) = router.get_agent(&id).await {
                ws.is_default = true;
                router.register_agent(ws).await;
                branding.print_banner("Agents", Some("Manage routed workspaces"));
                println!(
                    "  {}",
                    branding.good(format!("Agent '{}' set as default.", id))
                );
            } else {
                eprintln!("  {}", branding.bad(format!("Agent '{}' not found.", id)));
            }
        }
    }
}

async fn list_agents(router: &AgentRouter, format: &str) {
    let branding = TerminalBranding::current();
    let agents = router.list_agents().await;

    if agents.is_empty() {
        branding.print_banner("Agents", Some("Manage routed workspaces"));
        println!(
            "{}",
            branding.warn("No agents registered. Use `thinclaw agents add` to register one.")
        );
        return;
    }

    if format == "json" {
        let json: Vec<serde_json::Value> = agents
            .iter()
            .map(|a| {
                serde_json::json!({
                    "id": a.agent_id,
                    "display_name": a.display_name,
                    "is_default": a.is_default,
                    "channels": a.bound_channels,
                    "keywords": a.trigger_keywords,
                    "model": a.model,
                    "has_system_prompt": a.system_prompt.is_some(),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json).unwrap_or_default()
        );
        return;
    }

    branding.print_banner("Agents", Some("Manage routed workspaces"));
    println!(
        "{}",
        branding.body_bold(format!(
            "{:<18}  {:<20}  {:<8}  {:<16}  MODEL",
            "AGENT ID", "DISPLAY NAME", "DEFAULT", "CHANNELS"
        ))
    );
    println!("{}", branding.separator(85));

    for a in &agents {
        let channels_str = if a.bound_channels.is_empty() {
            "(all)".to_string()
        } else {
            a.bound_channels.join(", ")
        };
        let default_str = if a.is_default { "✓" } else { "" };
        let model_str = a.model.as_deref().unwrap_or("—");

        println!(
            "{:<18}  {:<20}  {:<8}  {:<16}  {}",
            a.agent_id, a.display_name, default_str, channels_str, model_str
        );
    }

    println!();
    println!(
        "{}",
        branding.muted(format!("{} agent(s) registered.", agents.len()))
    );
}

async fn show_agent(router: &AgentRouter, id: &str) {
    let branding = TerminalBranding::current();
    match router.get_agent(id).await {
        Some(a) => {
            branding.print_banner("Agents", Some("Inspect a routed workspace"));
            println!("{}", branding.key_value("Agent", &a.agent_id));
            println!("{}", branding.key_value("Display Name", &a.display_name));
            println!(
                "{}",
                branding.key_value("Default", if a.is_default { "yes" } else { "no" })
            );
            println!(
                "{}",
                branding.key_value(
                    "Model",
                    a.model.as_deref().unwrap_or("(inherit from agent)")
                )
            );
            println!(
                "{}",
                branding.key_value(
                    "Bound Channels",
                    if a.bound_channels.is_empty() {
                        "(all)".to_string()
                    } else {
                        a.bound_channels.join(", ")
                    }
                )
            );
            println!(
                "{}",
                branding.key_value(
                    "Trigger Keywords",
                    if a.trigger_keywords.is_empty() {
                        "(none)".to_string()
                    } else {
                        a.trigger_keywords.join(", ")
                    }
                )
            );
            if let Some(ref prompt) = a.system_prompt {
                let preview = if prompt.len() > 120 {
                    let end = prompt
                        .char_indices()
                        .map(|(i, _)| i)
                        .take_while(|&i| i < 120)
                        .last()
                        .unwrap_or(0);
                    format!("{}...", &prompt[..end])
                } else {
                    prompt.clone()
                };
                println!("{}", branding.key_value("System Prompt", preview));
            }
        }
        None => {
            eprintln!("  {}", branding.bad(format!("Agent '{}' not found.", id)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_add_and_list_agent() {
        let router = AgentRouter::new();
        let cmd = AgentCommand::Add {
            id: "test-bot".to_string(),
            display_name: Some("Test Bot".to_string()),
            system_prompt: None,
            model: Some("claude-sonnet".to_string()),
            channels: vec!["telegram".to_string()],
            keywords: vec!["help".to_string()],
            default: true,
        };
        run_agents_command(cmd, &router).await;
        assert_eq!(router.agent_count().await, 1);

        let agent = router.get_agent("test-bot").await;
        assert!(agent.is_some());
        let agent = agent.unwrap();
        assert_eq!(agent.display_name, "Test Bot");
        assert!(agent.is_default);
    }

    #[tokio::test]
    async fn test_remove_agent() {
        let router = AgentRouter::new();
        let ws = AgentWorkspace {
            workspace_id: None,
            agent_id: "temp".to_string(),
            display_name: "Temp".to_string(),
            system_prompt: None,
            bound_channels: vec![],
            trigger_keywords: vec![],
            allowed_tools: None,
            allowed_skills: None,
            tool_profile: None,
            is_default: false,
            model: None,
        };
        router.register_agent(ws).await;
        assert_eq!(router.agent_count().await, 1);

        let cmd = AgentCommand::Remove {
            id: "temp".to_string(),
        };
        run_agents_command(cmd, &router).await;
        assert_eq!(router.agent_count().await, 0);
    }
}
