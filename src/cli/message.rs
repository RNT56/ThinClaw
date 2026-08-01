//! Message sending CLI command.
//!
//! Allows injecting a message into the agent via the gateway HTTP API.
//!
//! Usage: `thinclaw send "hello world"`

use clap::Subcommand;

use crate::cli::{CliContext, GatewayClient};

#[derive(Subcommand, Debug, Clone)]
pub enum MessageCommand {
    /// Send a message to the agent via the gateway
    Send {
        /// Message text to send
        #[arg(short, long)]
        text: String,

        /// User ID (default: "cli")
        #[arg(short, long, default_value = "cli")]
        user_id: String,

        /// Gateway URL (default: http://127.0.0.1:3000)
        #[arg(long)]
        gateway_url: Option<String>,
    },
}

/// Run a message command.
pub async fn run_message_command(cmd: MessageCommand, context: &CliContext) -> anyhow::Result<()> {
    match cmd {
        MessageCommand::Send {
            text,
            user_id,
            gateway_url,
        } => send_message(text, user_id, gateway_url, context).await,
    }
}

/// Send a message to the agent via the gateway's REST API.
async fn send_message(
    text: String,
    user_id: String,
    gateway_url: Option<String>,
    context: &CliContext,
) -> anyhow::Result<()> {
    #[derive(serde::Serialize)]
    struct SendRequest {
        content: String,
        user_id: String,
        client_message_id: uuid::Uuid,
    }
    #[derive(Debug, serde::Deserialize, serde::Serialize)]
    struct SendResult {
        message_id: uuid::Uuid,
        status: String,
    }

    let settings = crate::settings::Settings::load();
    let client = GatewayClient::resolve(gateway_url.as_deref(), None, Some(&settings))?;
    context.output().progress(format!(
        "Sending through {}...",
        client.credential_free_origin()
    ))?;
    let result: SendResult = client
        .post_json(
            "/api/chat/send",
            &SendRequest {
                content: text,
                user_id,
                client_message_id: uuid::Uuid::new_v4(),
            },
        )
        .await?;
    context.output().write_record("send", &result, |result| {
        format!(
            "Message accepted: {} ({})",
            result.message_id, result.status
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn test_message_command_parse() {
        #[derive(clap::Parser)]
        struct TestCli {
            #[command(subcommand)]
            cmd: MessageCommand,
        }
        TestCli::command().debug_assert();
    }
}
