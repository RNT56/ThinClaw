//! WASM channel setup.

use secrecy::{ExposeSecret, SecretString};
use thinclaw_channels::setup as channel_setup;

use crate::setup::prompts::{
    confirm, optional_input, print_blank_line, print_error, print_info, print_success, secret_input,
};
use crate::setup::validation::{
    ValidationCredential, ValidationEndpointGrant, validate_extension_credential,
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
    capabilities: &crate::channels::wasm::ChannelCapabilitiesFile,
) -> Result<WasmChannelSetupResult, ChannelSetupError> {
    let setup = &capabilities.setup;
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

    // Validate only through the channel's declared HTTP grant. Credentials are
    // injected in headers and never interpolated into URLs.
    if let Some(ref validation_endpoint) = setup.validation_endpoint {
        print_info("Validating credentials...");
        let request = validation_endpoint.request();
        let method = request.map_or("GET", |request| request.method.as_str());
        let expected_status = request.map_or(200, |request| request.success_status);
        let secret = if let Some(name) = request.and_then(|request| request.secret_name.as_deref())
        {
            if !setup
                .required_secrets
                .iter()
                .any(|required| required.name == name)
            {
                print_error("Validation credential is not a declared setup secret");
                None
            } else {
                match secrets.get_secret(name).await {
                    Ok(value) => Some(value),
                    Err(_) => {
                        print_info(&format!(
                            "Skipping validation: secret '{}' is not available",
                            name
                        ));
                        None
                    }
                }
            }
        } else {
            None
        };
        let credential = match (
            request.and_then(|request| request.credential.as_ref()),
            secret.as_ref(),
        ) {
            (None, None) => Some(ValidationCredential::None),
            (
                None | Some(crate::channels::wasm::CredentialLocationSchema::Bearer),
                Some(secret),
            ) => Some(ValidationCredential::Bearer(secret.expose_secret())),
            (
                Some(crate::channels::wasm::CredentialLocationSchema::Basic { username }),
                Some(secret),
            ) => Some(ValidationCredential::Basic {
                username,
                password: secret.expose_secret(),
            }),
            (
                Some(crate::channels::wasm::CredentialLocationSchema::Header { name, prefix }),
                Some(secret),
            ) => Some(ValidationCredential::Header {
                name,
                prefix: prefix.as_deref(),
                value: secret.expose_secret(),
            }),
            _ => None,
        };
        let grants = capabilities
            .to_capabilities()
            .tool_capabilities
            .http
            .map(|http| {
                http.allowlist
                    .into_iter()
                    .map(|grant| ValidationEndpointGrant {
                        host: grant.host,
                        path_prefix: grant.path_prefix,
                        methods: grant.methods,
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let result = if let Some(credential) = credential {
            validate_extension_credential(
                validation_endpoint.url(),
                method,
                expected_status,
                &grants,
                credential,
            )
            .await
        } else {
            Err(crate::setup::validation::ValidationRequestError::Invalid(
                "credential setup is missing or uses an unsafe URL location".to_string(),
            ))
        };
        match result {
            Ok(()) => print_success("Credentials validated successfully"),
            Err(error) => {
                print_error(&format!("Credential validation failed: {error}"));
                print_info("The channel will still be configured, but credentials may be invalid.");
            }
        }
    }

    print_success(&format!("{} channel configured", channel_name));

    Ok(WasmChannelSetupResult {
        enabled: true,
        channel_name: channel_name.to_string(),
    })
}
