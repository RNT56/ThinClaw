//! Message sending CLI command.
//!
//! Allows injecting a message into the agent via the gateway HTTP API.
//!
//! Usage: `thinclaw message send --text "hello world"`

use clap::Subcommand;

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
pub async fn run_message_command(cmd: MessageCommand) -> anyhow::Result<()> {
    match cmd {
        MessageCommand::Send {
            text,
            user_id,
            gateway_url,
        } => send_message(text, user_id, gateway_url).await,
    }
}

/// Send a message to the agent via the gateway's REST API.
async fn send_message(
    text: String,
    user_id: String,
    gateway_url: Option<String>,
) -> anyhow::Result<()> {
    let base_url = gateway_url.unwrap_or_else(|| {
        // Check env for gateway port
        let port = std::env::var("GATEWAY_PORT").unwrap_or_else(|_| "3000".to_string());
        let host = std::env::var("GATEWAY_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        format!("http://{}:{}", host, port)
    });

    let url = format!("{}/api/chat", base_url);

    // Check for auth token
    let auth_token = std::env::var("GATEWAY_AUTH_TOKEN").ok();

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let body = serde_json::json!({
        "message": text,
        "user_id": user_id,
    });

    let mut request = client.post(&url).json(&body);

    if let Some(ref token) = auth_token {
        request = request.bearer_auth(token);
    }

    println!("📤 Sending to {}...", url);

    let response = request.send().await.map_err(|e| {
        if e.is_connect() {
            anyhow::anyhow!(
                "Could not connect to gateway at {}. Is the agent running?\n\
                 Start with: thinclaw run\n\
                 Or specify --gateway-url",
                base_url
            )
        } else {
            anyhow::anyhow!("Request failed: {}", e)
        }
    })?;

    let status = response.status();
    let body_text = response.text().await.unwrap_or_default();

    if status.is_success() {
        // Try to parse as JSON for pretty output
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body_text) {
            if let Some(reply) = json.get("response").and_then(|v| v.as_str()) {
                println!("💬 Response:\n{}", reply);
            } else {
                println!(
                    "✅ Sent. Response:\n{}",
                    serde_json::to_string_pretty(&json)?
                );
            }
        } else {
            println!("✅ Sent. Raw response:\n{}", body_text);
        }
    } else {
        anyhow::bail!("Gateway returned HTTP {}: {}", status.as_u16(), body_text);
    }

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
