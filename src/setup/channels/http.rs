//! HTTP webhook channel setup.

use secrecy::SecretString;
use thinclaw_channels::setup as channel_setup;

use crate::setup::prompts::{confirm, optional_input, print_blank_line, print_info, print_success};

use super::{ChannelSetupError, SecretsContext};

/// Result of HTTP webhook setup.
#[derive(Debug, Clone)]
pub struct HttpSetupResult {
    pub enabled: bool,
    pub port: u16,
    pub host: String,
}

/// Set up HTTP webhook channel.
pub async fn setup_http(secrets: &SecretsContext) -> Result<HttpSetupResult, ChannelSetupError> {
    print_info("HTTP webhook setup");
    print_blank_line();
    print_info("The HTTP webhook allows external services to send messages to the agent.");
    print_blank_line();

    let port_str = optional_input("Port", Some("default: 8080"))?;
    let port: u16 = port_str
        .as_deref()
        .unwrap_or("8080")
        .parse()
        .map_err(|e| ChannelSetupError::Validation(format!("Invalid port: {}", e)))?;

    if port < 1024 {
        print_info("Note: Ports below 1024 may require root privileges");
    }

    let host =
        optional_input("Host", Some("default: 0.0.0.0"))?.unwrap_or_else(|| "0.0.0.0".to_string());

    // Generate a webhook secret
    if confirm("Generate a webhook secret for authentication?", true)? {
        let secret = channel_setup::generate_webhook_secret();
        secrets
            .save_secret("http_webhook_secret", &SecretString::from(secret))
            .await?;
        print_success("Webhook secret generated and saved to database");
        print_info("Retrieve it later with: thinclaw secret get http_webhook_secret");
    }

    print_success(&format!("HTTP webhook will listen on {}:{}", host, port));

    Ok(HttpSetupResult {
        enabled: true,
        port,
        host,
    })
}
