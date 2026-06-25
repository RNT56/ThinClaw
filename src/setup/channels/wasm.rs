//! WASM channel setup.

use reqwest::Client;
use secrecy::{ExposeSecret, SecretString};
use thinclaw_channels::setup as channel_setup;

use crate::setup::prompts::{
    confirm, optional_input, print_blank_line, print_error, print_info, print_success, secret_input,
};

use super::{ChannelSetupError, SecretsContext};

/// Result of WASM channel setup.
#[derive(Debug, Clone)]
pub struct WasmChannelSetupResult {
    pub enabled: bool,
    pub channel_name: String,
}

/// prompts the user for each required secret.
pub async fn setup_wasm_channel(
    secrets: &SecretsContext,
    channel_name: &str,
    setup: &crate::channels::wasm::SetupSchema,
) -> Result<WasmChannelSetupResult, ChannelSetupError> {
    print_info(&format!("{channel_name} setup"));
    print_blank_line();

    for secret_config in &setup.required_secrets {
        // Check if this secret already exists
        if secrets.secret_exists(&secret_config.name).await {
            print_info(&format!(
                "Existing {} found in database.",
                secret_config.name
            ));
            if !confirm("Replace existing value?", false)? {
                continue;
            }
        }

        // Get the value from user or auto-generate
        let value = if secret_config.optional {
            let input_value =
                optional_input(&secret_config.prompt, Some("leave empty to auto-generate"))?;

            if let Some(v) = input_value {
                if !v.is_empty() {
                    SecretString::from(v)
                } else if let Some(ref auto_gen) = secret_config.auto_generate {
                    let generated = channel_setup::generate_secret_with_length(auto_gen.length);
                    print_info(&format!(
                        "Auto-generated {} ({} bytes)",
                        secret_config.name, auto_gen.length
                    ));
                    SecretString::from(generated)
                } else {
                    continue; // Skip optional secret with no auto-generate
                }
            } else if let Some(ref auto_gen) = secret_config.auto_generate {
                let generated = channel_setup::generate_secret_with_length(auto_gen.length);
                print_info(&format!(
                    "Auto-generated {} ({} bytes)",
                    secret_config.name, auto_gen.length
                ));
                SecretString::from(generated)
            } else {
                continue; // Skip optional secret with no auto-generate
            }
        } else {
            // Required secret
            let input_value = secret_input(&secret_config.prompt)?;

            // Validate if pattern is provided
            if let Some(ref pattern) = secret_config.validation {
                let re = regex::Regex::new(pattern).map_err(|e| {
                    ChannelSetupError::Validation(format!("Invalid validation pattern: {}", e))
                })?;
                if !re.is_match(input_value.expose_secret()) {
                    print_error(&format!(
                        "Value does not match expected format: {}",
                        pattern
                    ));
                    return Err(ChannelSetupError::Validation(
                        "Validation failed".to_string(),
                    ));
                }
            }

            input_value
        };

        // Save the secret
        secrets.save_secret(&secret_config.name, &value).await?;
        print_success(&format!("{} saved to database", secret_config.name));
    }

    // Validate configured credentials by substituting secrets into the
    // validation URL and making a GET request to verify they work.
    if let Some(ref validation_endpoint) = setup.validation_endpoint {
        let mut url = validation_endpoint.clone();

        // Substitute secret placeholders: {{secret_name}} → actual value
        for secret_config in &setup.required_secrets {
            let placeholder = format!("{{{{{}}}}}", secret_config.name);
            if url.contains(&placeholder) {
                match secrets.get_secret(&secret_config.name).await {
                    Ok(value) => {
                        url = url.replace(&placeholder, value.expose_secret());
                    }
                    Err(_) => {
                        // Secret not found — skip validation
                        print_info(&format!(
                            "Skipping validation: secret '{}' not available",
                            secret_config.name
                        ));
                        url.clear();
                        break;
                    }
                }
            }
        }

        if !url.is_empty() {
            print_info("Validating credentials...");
            let client = Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .ok();

            if let Some(client) = client {
                match client.get(&url).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        print_success("Credentials validated successfully");
                    }
                    Ok(resp) => {
                        let status = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        print_error(&format!(
                            "Credential validation failed (HTTP {}): {}",
                            status,
                            body.chars().take(200).collect::<String>()
                        ));
                        print_info(
                            "The channel will still be configured, but credentials may be invalid.",
                        );
                    }
                    Err(e) => {
                        print_info(&format!(
                            "Could not reach validation endpoint: {} (channel configured anyway)",
                            e
                        ));
                    }
                }
            }
        }
    }

    print_success(&format!("{} channel configured", channel_name));

    Ok(WasmChannelSetupResult {
        enabled: true,
        channel_name: channel_name.to_string(),
    })
}
