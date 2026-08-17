//! Authenticated control of the running runtime's transient presence registry.

use clap::{Subcommand, ValueEnum};
use serde::Serialize;
use thinclaw_gateway::web::types::{
    PresenceClearResponse, PresencePublishRequest, PresencePublishResponse,
    PresenceSnapshotResponse, PresenceState, PresenceSurface,
};
use uuid::Uuid;

use crate::cli::{CliContext, CliError, GatewayClient};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum PresenceStateArg {
    Online,
    Away,
    Busy,
    Typing,
}

impl From<PresenceStateArg> for PresenceState {
    fn from(value: PresenceStateArg) -> Self {
        match value {
            PresenceStateArg::Online => Self::Online,
            PresenceStateArg::Away => Self::Away,
            PresenceStateArg::Busy => Self::Busy,
            PresenceStateArg::Typing => Self::Typing,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum PresenceSurfaceArg {
    Desktop,
    Web,
    Ios,
    Watchos,
    #[default]
    Cli,
    Channel,
}

impl From<PresenceSurfaceArg> for PresenceSurface {
    fn from(value: PresenceSurfaceArg) -> Self {
        match value {
            PresenceSurfaceArg::Desktop => Self::Desktop,
            PresenceSurfaceArg::Web => Self::Web,
            PresenceSurfaceArg::Ios => Self::Ios,
            PresenceSurfaceArg::Watchos => Self::Watchos,
            PresenceSurfaceArg::Cli => Self::Cli,
            PresenceSurfaceArg::Channel => Self::Channel,
        }
    }
}

#[derive(Subcommand, Debug, Clone)]
pub enum PresenceCommand {
    /// Publish or renew a bounded presence session
    Publish {
        #[arg(value_enum)]
        state: PresenceStateArg,
        /// Reuse an existing session ID to renew it; otherwise one is generated
        #[arg(long)]
        session_id: Option<Uuid>,
        /// Optional owned conversation scope (required for `typing`)
        #[arg(long)]
        thread_id: Option<Uuid>,
        /// Server-enforced lease TTL (5-120 seconds)
        #[arg(long, default_value_t = 45)]
        ttl_seconds: u64,
        #[arg(long, value_enum, default_value_t = PresenceSurfaceArg::Cli)]
        surface: PresenceSurfaceArg,
        /// Explicit running-runtime URL (otherwise config/environment is used)
        #[arg(long)]
        gateway_url: Option<String>,
    },
    /// List the current aggregate in principal or owned-thread scope
    List {
        #[arg(long)]
        thread_id: Option<Uuid>,
        #[arg(long)]
        gateway_url: Option<String>,
    },
    /// Idempotently clear one owned presence session
    Clear {
        session_id: Uuid,
        #[arg(long)]
        gateway_url: Option<String>,
    },
}

#[derive(Debug, Serialize)]
struct SnapshotQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
struct PublishOutput {
    session_id: Uuid,
    #[serde(flatten)]
    response: PresencePublishResponse,
}

#[derive(Debug, Serialize)]
struct ClearOutput {
    session_id: Uuid,
    #[serde(flatten)]
    response: PresenceClearResponse,
}

async fn gateway_client(
    context: &CliContext,
    gateway_url: Option<&str>,
) -> Result<GatewayClient, CliError> {
    let config = context.config().await?;
    GatewayClient::resolve_from_config(gateway_url, None, config)
        .map_err(|error| CliError::operational(error.to_string()))
}

pub async fn run_presence_command(
    command: PresenceCommand,
    context: &CliContext,
) -> Result<(), CliError> {
    match command {
        PresenceCommand::Publish {
            state,
            session_id,
            thread_id,
            ttl_seconds,
            surface,
            gateway_url,
        } => {
            if !(5..=120).contains(&ttl_seconds) {
                return Err(CliError::usage(
                    "presence TTL must be between 5 and 120 seconds",
                ));
            }
            if matches!(state, PresenceStateArg::Typing) && thread_id.is_none() {
                return Err(CliError::usage("typing presence requires --thread-id"));
            }
            let session_id = session_id.unwrap_or_else(Uuid::new_v4);
            let client = gateway_client(context, gateway_url.as_deref()).await?;
            let response: PresencePublishResponse = client
                .put_json_confirmed(
                    &format!("/api/presence/{session_id}"),
                    &PresencePublishRequest {
                        state: state.into(),
                        surface: surface.into(),
                        thread_id,
                        ttl_seconds: Some(ttl_seconds),
                    },
                )
                .await
                .map_err(gateway_error)?;
            let output = PublishOutput {
                session_id,
                response,
            };
            context
                .output()
                .write_record("presence.publish", &output, |output| {
                    format!(
                        "{}\t{:?}\t{} session(s)\texpires {}",
                        output.session_id,
                        output.response.presence.state,
                        output.response.presence.session_count,
                        output.response.presence.expires_at
                    )
                })
        }
        PresenceCommand::List {
            thread_id,
            gateway_url,
        } => {
            let client = gateway_client(context, gateway_url.as_deref()).await?;
            let response: PresenceSnapshotResponse = client
                .get_json("/api/presence", &SnapshotQuery { thread_id })
                .await
                .map_err(gateway_error)?;
            context
                .output()
                .write_record("presence.list", &response, |response| {
                    if response.presences.is_empty() {
                        return "No active presence".to_string();
                    }
                    response
                        .presences
                        .iter()
                        .map(|presence| {
                            format!(
                                "{}\t{:?}\t{} session(s)\texpires {}",
                                presence.actor_id,
                                presence.state,
                                presence.session_count,
                                presence.expires_at
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                })
        }
        PresenceCommand::Clear {
            session_id,
            gateway_url,
        } => {
            let client = gateway_client(context, gateway_url.as_deref()).await?;
            let response: PresenceClearResponse = client
                .delete_json_confirmed(&format!("/api/presence/{session_id}"))
                .await
                .map_err(gateway_error)?;
            let output = ClearOutput {
                session_id,
                response,
            };
            context
                .output()
                .write_record("presence.clear", &output, |output| {
                    format!(
                        "{}\t{}",
                        output.session_id,
                        if output.response.cleared {
                            "cleared"
                        } else {
                            "already absent"
                        }
                    )
                })
        }
    }
}

fn gateway_error(error: impl std::fmt::Display) -> CliError {
    CliError::operational(format!(
        "running runtime presence operation failed: {error}"
    ))
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;
    use crate::cli::{Cli, Command};

    #[test]
    fn parser_exposes_publish_list_and_clear() {
        for args in [
            vec!["thinclaw", "presence", "publish", "online"],
            vec!["thinclaw", "presence", "list"],
            vec![
                "thinclaw",
                "presence",
                "clear",
                "00000000-0000-0000-0000-000000000001",
            ],
        ] {
            assert!(Cli::try_parse_from(args).is_ok());
        }
    }

    #[test]
    fn typing_without_thread_is_rejected_by_command_contract() {
        let cli = Cli::try_parse_from(["thinclaw", "presence", "publish", "typing"])
            .expect("parse command");
        assert!(matches!(
            cli.command,
            Some(Command::Presence(PresenceCommand::Publish {
                state: PresenceStateArg::Typing,
                thread_id: None,
                ..
            }))
        ));
    }
}
