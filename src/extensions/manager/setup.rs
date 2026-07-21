//! Setup and OAuth-callback surfaces: completing a WASM-tool OAuth callback,
//! deriving the per-extension setup schema, enriching the domain setup-status,
//! live validation, and saving setup secrets (with hot-activation).

use crate::extensions::{ExtensionError, ExtensionKind, InstalledExtension};
use crate::secrets::CreateSecretParams;
use crate::setup::validation::{
    ValidationCredential, ValidationEndpointGrant, ensure_endpoint_is_granted,
    validate_extension_credential,
};
use crate::tools::wasm::WasmToolOAuthFlow;
use thinclaw_types::{
    IntegrationSetupStatus, SetupAction, SetupAuthMode, SetupSecretDescriptor, SetupState,
};

use super::ExtensionManager;
use super::core::{
    AuthRequestContext, ExtensionSetupSchema, SetupResult, lock_wasm_package,
    setup_auth_mode_from_schema,
};
use crate::extensions::{ActivateResult, AuthResult};

impl ExtensionManager {
    pub async fn complete_oauth_callback(
        &self,
        nonce: &str,
        code: &str,
    ) -> Result<(String, Option<String>, AuthResult, ActivateResult), ExtensionError> {
        let pending = self.take_pending_auth(nonce).await.ok_or_else(|| {
            ExtensionError::AuthFailed("Invalid or expired OAuth state".to_string())
        })?;

        match pending.kind {
            ExtensionKind::WasmTool => {
                let auth = self
                    .load_wasm_tool_auth(&pending.name)
                    .await?
                    .ok_or_else(|| {
                        ExtensionError::AuthFailed("Tool has no OAuth configuration".to_string())
                    })?;
                let redirect_uri = pending.redirect_uri.clone().ok_or_else(|| {
                    ExtensionError::AuthFailed("Missing callback redirect URI".to_string())
                })?;

                let flow = WasmToolOAuthFlow::new(
                    self.secrets.as_ref(),
                    &self.user_id,
                    &self.wasm_tools_dir,
                );
                let token = flow
                    .exchange_code(&auth, &redirect_uri, code, pending.code_verifier.as_deref())
                    .await
                    .map_err(|e| ExtensionError::AuthFailed(e.to_string()))?;
                flow.store_token_exchange(&auth, &token)
                    .await
                    .map_err(|e| ExtensionError::AuthFailed(e.to_string()))?;

                let auth_check = flow
                    .check_auth_status(&auth)
                    .await
                    .map_err(|e| ExtensionError::AuthFailed(e.to_string()))?;
                let mut auth_result = self.auth_result(
                    &pending.name,
                    ExtensionKind::WasmTool,
                    auth_check.auth_mode.as_str(),
                    auth_check.auth_status.as_str(),
                );
                Self::apply_auth_check(&mut auth_result, &auth_check);

                let activate_result = match self.activate_wasm_tool(&pending.name).await {
                    Ok(result) => result,
                    Err(error) => {
                        self.broadcast_auth_completed(
                            &pending.name,
                            false,
                            format!(
                                "{} connected, but activation failed: {}",
                                pending.name, error
                            ),
                            Some(&auth_result),
                            pending.thread_id.clone(),
                        )
                        .await;
                        return Err(error);
                    }
                };
                self.broadcast_auth_completed(
                    &pending.name,
                    true,
                    format!("{} connected and activated.", pending.name),
                    Some(&auth_result),
                    pending.thread_id.clone(),
                )
                .await;

                Ok((
                    pending.name,
                    pending.thread_id,
                    auth_result,
                    activate_result,
                ))
            }
            ExtensionKind::McpServer => {
                let _operation = self.mcp_operation_lock.lock().await;
                let redirect_uri = pending.redirect_uri.as_deref().ok_or_else(|| {
                    ExtensionError::AuthFailed("Missing callback redirect URI".to_string())
                })?;
                let prepared = pending.mcp_authorization.as_ref().ok_or_else(|| {
                    ExtensionError::AuthFailed("Missing prepared MCP OAuth transaction".to_string())
                })?;
                let server = self
                    .get_mcp_server(&pending.name)
                    .await
                    .map_err(|error| ExtensionError::AuthFailed(error.to_string()))?;
                crate::tools::mcp::auth::complete_mcp_authorization(
                    &server,
                    &self.secrets,
                    &self.user_id,
                    prepared,
                    code,
                    redirect_uri,
                )
                .await
                .map_err(|error| ExtensionError::AuthFailed(error.to_string()))?;

                self.discard_mcp_client_unlocked(&pending.name).await;
                let auth_result = self.auth_result(
                    &pending.name,
                    ExtensionKind::McpServer,
                    "oauth",
                    "authenticated",
                );
                let activate_result = match self.activate_mcp_unlocked(&pending.name).await {
                    Ok(result) => result,
                    Err(error) => {
                        self.broadcast_auth_completed(
                            &pending.name,
                            false,
                            format!(
                                "{} connected, but activation failed: {}",
                                pending.name, error
                            ),
                            Some(&auth_result),
                            pending.thread_id.clone(),
                        )
                        .await;
                        return Err(error);
                    }
                };
                self.broadcast_auth_completed(
                    &pending.name,
                    true,
                    format!("{} connected and activated.", pending.name),
                    Some(&auth_result),
                    pending.thread_id.clone(),
                )
                .await;
                Ok((
                    pending.name,
                    pending.thread_id,
                    auth_result,
                    activate_result,
                ))
            }
            _ => Err(ExtensionError::AuthFailed(
                "OAuth callback is not supported for this extension kind".to_string(),
            )),
        }
    }

    /// Get the setup/auth schema for an extension.
    pub async fn get_setup_schema(
        &self,
        name: &str,
        context: AuthRequestContext,
    ) -> Result<ExtensionSetupSchema, ExtensionError> {
        let kind = self.determine_installed_kind(name).await?;
        match kind {
            ExtensionKind::WasmChannel => {
                let package = lock_wasm_package(&self.wasm_channels_dir, name).await?;
                let Some(cap_bytes) = package.capabilities_bytes.as_deref() else {
                    return Ok(ExtensionSetupSchema {
                        mode: "none".to_string(),
                        auth_status: "no_auth_required".to_string(),
                        fields: Vec::new(),
                        auth_url: None,
                        instructions: None,
                        setup_url: None,
                        validation_url: None,
                        shared_auth_provider: None,
                        missing_scopes: Vec::new(),
                    });
                };
                let cap_file =
                    crate::channels::wasm::ChannelCapabilitiesFile::from_bytes(cap_bytes)
                        .map_err(|e| ExtensionError::Other(e.to_string()))?;

                let mut fields = Vec::new();
                for secret in &cap_file.setup.required_secrets {
                    let provided = self
                        .secrets
                        .exists(&self.user_id, &secret.name)
                        .await
                        .map_err(|error| ExtensionError::AuthFailed(error.to_string()))?;
                    fields.push(crate::channels::web::types::SecretFieldInfo {
                        name: secret.name.clone(),
                        prompt: secret.prompt.clone(),
                        optional: secret.optional,
                        provided,
                        auto_generate: secret.auto_generate.is_some(),
                    });
                }
                let (authenticated, _) = self
                    .check_channel_manifest_auth_status(Some(&cap_file))
                    .await?;
                Ok(ExtensionSetupSchema {
                    mode: "secrets".to_string(),
                    auth_status: if authenticated {
                        "authenticated".to_string()
                    } else {
                        "awaiting_token".to_string()
                    },
                    fields,
                    auth_url: None,
                    instructions: None,
                    setup_url: None,
                    validation_url: cap_file
                        .setup
                        .validation_endpoint
                        .as_ref()
                        .map(|endpoint| endpoint.url().to_string()),
                    shared_auth_provider: None,
                    missing_scopes: Vec::new(),
                })
            }
            ExtensionKind::WasmTool => {
                let auth = self.load_wasm_tool_auth(name).await?;
                let auth_result = self.auth_wasm_tool(name, None, context).await?;
                let validation_url = auth
                    .as_ref()
                    .and_then(|auth| auth.validation_endpoint.as_ref())
                    .map(|endpoint| endpoint.url.clone());
                let fields = if auth_result.auth_mode == "manual_token" {
                    auth.map(|auth| {
                        vec![crate::channels::web::types::SecretFieldInfo {
                            name: auth.secret_name,
                            prompt: auth.instructions.unwrap_or_else(|| {
                                format!(
                                    "Enter the token for {}.",
                                    auth.display_name.unwrap_or_else(|| name.to_string())
                                )
                            }),
                            optional: false,
                            provided: auth_result.auth_status == "authenticated",
                            auto_generate: false,
                        }]
                    })
                    .unwrap_or_default()
                } else {
                    Vec::new()
                };
                Ok(ExtensionSetupSchema {
                    mode: auth_result.auth_mode,
                    auth_status: auth_result.auth_status,
                    fields,
                    auth_url: auth_result.auth_url,
                    instructions: auth_result.instructions,
                    setup_url: auth_result.setup_url,
                    validation_url,
                    shared_auth_provider: auth_result.shared_auth_provider,
                    missing_scopes: auth_result.missing_scopes,
                })
            }
            _ => Ok(ExtensionSetupSchema {
                mode: "none".to_string(),
                auth_status: "no_auth_required".to_string(),
                fields: Vec::new(),
                auth_url: None,
                instructions: None,
                setup_url: None,
                validation_url: None,
                shared_auth_provider: None,
                missing_scopes: Vec::new(),
            }),
        }
    }

    /// Build a descriptor-rich setup status for UI/API/onboarding surfaces.
    ///
    /// `InstalledExtension::setup_status` is intentionally cheap and based only
    /// on list metadata. This method enriches it from installed channel/tool
    /// schemas so surfaces can show the exact missing secrets and validation
    /// actions without duplicating file parsing.
    pub async fn integration_setup_status(
        &self,
        extension: &InstalledExtension,
        context: AuthRequestContext,
    ) -> IntegrationSetupStatus {
        let mut status = extension.setup_status();
        if !extension.installed {
            return status;
        }

        let Ok(schema) = self.get_setup_schema(&extension.name, context).await else {
            return status;
        };

        status.auth_mode = setup_auth_mode_from_schema(&schema.mode, status.auth_mode);
        status.required_secrets = schema
            .fields
            .iter()
            .map(|field| SetupSecretDescriptor {
                name: field.name.clone(),
                prompt: field.prompt.clone(),
                optional: field.optional,
                validation_hint: if field.provided {
                    Some("stored".to_string())
                } else if field.auto_generate {
                    Some("auto-generated when left empty".to_string())
                } else {
                    None
                },
            })
            .collect();
        status.setup_url = schema.setup_url;
        status.validation_url = schema.validation_url;

        if schema.auth_url.is_some()
            && matches!(
                status.auth_mode,
                SetupAuthMode::OAuth | SetupAuthMode::SharedOAuth
            )
            && !status.actions.contains(&SetupAction::StartOAuth)
        {
            status.actions.push(SetupAction::StartOAuth);
        }
        if !status.required_secrets.is_empty()
            && !status.actions.contains(&SetupAction::ConfigureSecrets)
        {
            status.actions.push(SetupAction::ConfigureSecrets);
        }
        if !status.actions.contains(&SetupAction::Validate) {
            status.actions.insert(0, SetupAction::Validate);
        }

        if status.state == SetupState::NeedsAuth && schema.auth_status == "authenticated" {
            status.state = if extension.active {
                SetupState::Ready
            } else {
                SetupState::InstalledUnconfigured
            };
        }
        if status.message.is_none() {
            status.message = schema.instructions.or(Some(schema.auth_status));
        }

        status
    }

    /// Validate an installed extension setup without mutating credentials.
    pub async fn validate_setup(
        &self,
        name: &str,
        _context: AuthRequestContext,
    ) -> Result<String, ExtensionError> {
        match self.determine_installed_kind(name).await? {
            ExtensionKind::WasmChannel => {
                let package = lock_wasm_package(&self.wasm_channels_dir, name).await?;
                let bytes = package.capabilities_bytes.as_deref().ok_or_else(|| {
                    ExtensionError::Other(format!("Capabilities file not found for '{}'", name))
                })?;
                let capabilities =
                    crate::channels::wasm::ChannelCapabilitiesFile::from_bytes(bytes)
                        .map_err(|error| ExtensionError::Other(error.to_string()))?;
                let mut missing = Vec::new();
                for required in &capabilities.setup.required_secrets {
                    if !required.optional
                        && !self
                            .secrets
                            .exists(&self.user_id, &required.name)
                            .await
                            .map_err(|error| ExtensionError::AuthFailed(error.to_string()))?
                    {
                        missing.push(required.name.clone());
                    }
                }
                if !missing.is_empty() {
                    return Err(ExtensionError::AuthFailed(format!(
                        "Missing required setup secrets for '{}': {}",
                        name,
                        missing.join(", ")
                    )));
                }
                let Some(endpoint) = capabilities.setup.validation_endpoint.as_ref() else {
                    return Ok(format!(
                        "Setup metadata for '{}' is complete; no live validation endpoint is declared.",
                        name
                    ));
                };
                let runtime = capabilities.to_capabilities();
                let grants = runtime
                    .tool_capabilities
                    .http
                    .as_ref()
                    .map(|http| {
                        http.allowlist
                            .iter()
                            .map(|grant| ValidationEndpointGrant {
                                host: grant.host.clone(),
                                path_prefix: grant.path_prefix.clone(),
                                methods: grant.methods.clone(),
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let request = endpoint.request();
                let method = request.map_or("GET", |request| request.method.as_str());
                let expected_status = request.map_or(200, |request| request.success_status);
                let secret = if let Some(secret_name) =
                    request.and_then(|request| request.secret_name.as_deref())
                {
                    if !capabilities
                        .setup
                        .required_secrets
                        .iter()
                        .any(|required| required.name == secret_name)
                    {
                        return Err(ExtensionError::AuthFailed(
                            "Validation credential is not declared as a setup secret".to_string(),
                        ));
                    }
                    Some(
                        self.secrets
                            .get_for_injection(
                                &self.user_id,
                                secret_name,
                                crate::secrets::SecretAccessContext::new(
                                    "extensions.manager",
                                    "extension_setup_validation",
                                ),
                            )
                            .await
                            .map_err(|error| ExtensionError::AuthFailed(error.to_string()))?,
                    )
                } else {
                    None
                };
                let credential = match (
                    request.and_then(|request| request.credential.as_ref()),
                    secret.as_ref(),
                ) {
                    (None, None) => ValidationCredential::None,
                    (
                        None | Some(crate::channels::wasm::CredentialLocationSchema::Bearer),
                        Some(secret),
                    ) => ValidationCredential::Bearer(secret.expose()),
                    (
                        Some(crate::channels::wasm::CredentialLocationSchema::Basic { username }),
                        Some(secret),
                    ) => ValidationCredential::Basic {
                        username,
                        password: secret.expose(),
                    },
                    (
                        Some(crate::channels::wasm::CredentialLocationSchema::Header {
                            name,
                            prefix,
                        }),
                        Some(secret),
                    ) => ValidationCredential::Header {
                        name,
                        prefix: prefix.as_deref(),
                        value: secret.expose(),
                    },
                    (
                        Some(
                            crate::channels::wasm::CredentialLocationSchema::QueryParam { .. }
                            | crate::channels::wasm::CredentialLocationSchema::UrlPath { .. }
                            | crate::channels::wasm::CredentialLocationSchema::UrlBase { .. }
                            | crate::channels::wasm::CredentialLocationSchema::Body { .. },
                        ),
                        Some(_),
                    ) => {
                        return Err(ExtensionError::AuthFailed(
                            "Validation credentials may not be placed in URLs".to_string(),
                        ));
                    }
                    (Some(_), None) => {
                        return Err(ExtensionError::AuthFailed(
                            "Validation credential location requires a secret_name".to_string(),
                        ));
                    }
                };
                validate_extension_credential(
                    endpoint.url(),
                    method,
                    expected_status,
                    &grants,
                    credential,
                )
                .await
                .map_err(|error| ExtensionError::AuthFailed(error.to_string()))?;
            }
            ExtensionKind::WasmTool => {
                let package = lock_wasm_package(&self.wasm_tools_dir, name).await?;
                let bytes = package.capabilities_bytes.as_deref().ok_or_else(|| {
                    ExtensionError::Other(format!("Capabilities file not found for '{}'", name))
                })?;
                let capabilities = crate::tools::wasm::CapabilitiesFile::from_bytes(bytes)
                    .map_err(|error| ExtensionError::Other(error.to_string()))?;
                let auth = capabilities.auth.as_ref().ok_or_else(|| {
                    ExtensionError::AuthFailed("Tool has no authentication schema".to_string())
                })?;
                let Some(endpoint) = auth.validation_endpoint.as_ref() else {
                    return Ok(format!(
                        "Setup metadata for '{}' is complete; no live validation endpoint is declared.",
                        name
                    ));
                };
                let secret = self
                    .secrets
                    .get_for_injection(
                        &self.user_id,
                        &auth.secret_name,
                        crate::secrets::SecretAccessContext::new(
                            "extensions.manager",
                            "extension_setup_validation",
                        ),
                    )
                    .await
                    .map_err(|error| ExtensionError::AuthFailed(error.to_string()))?;
                let runtime = capabilities.to_capabilities();
                let http = runtime.http.as_ref().ok_or_else(|| {
                    ExtensionError::AuthFailed(
                        "Validation endpoint has no declared HTTP capability".to_string(),
                    )
                })?;
                let grants = http
                    .allowlist
                    .iter()
                    .map(|grant| ValidationEndpointGrant {
                        host: grant.host.clone(),
                        path_prefix: grant.path_prefix.clone(),
                        methods: grant.methods.clone(),
                    })
                    .collect::<Vec<_>>();
                let credential = if let Some(mapping) = http.credentials.get(&auth.secret_name) {
                    if !mapping.host_patterns.is_empty() {
                        let credential_grants = mapping
                            .host_patterns
                            .iter()
                            .map(|host| ValidationEndpointGrant {
                                host: host.clone(),
                                path_prefix: None,
                                methods: vec![endpoint.method.clone()],
                            })
                            .collect::<Vec<_>>();
                        ensure_endpoint_is_granted(
                            &endpoint.url,
                            &endpoint.method,
                            &credential_grants,
                        )
                        .map_err(|error| ExtensionError::AuthFailed(error.to_string()))?;
                    }
                    match &mapping.location {
                        crate::secrets::CredentialLocation::AuthorizationBearer => {
                            ValidationCredential::Bearer(secret.expose())
                        }
                        crate::secrets::CredentialLocation::AuthorizationBasic { username } => {
                            ValidationCredential::Basic {
                                username,
                                password: secret.expose(),
                            }
                        }
                        crate::secrets::CredentialLocation::Header { name, prefix } => {
                            ValidationCredential::Header {
                                name,
                                prefix: prefix.as_deref(),
                                value: secret.expose(),
                            }
                        }
                        crate::secrets::CredentialLocation::QueryParam { .. }
                        | crate::secrets::CredentialLocation::UrlPath { .. }
                        | crate::secrets::CredentialLocation::UrlBase { .. }
                        | crate::secrets::CredentialLocation::Body { .. } => {
                            return Err(ExtensionError::AuthFailed(
                                "Validation credentials may not be placed in URLs".to_string(),
                            ));
                        }
                    }
                } else {
                    ValidationCredential::Bearer(secret.expose())
                };
                validate_extension_credential(
                    &endpoint.url,
                    &endpoint.method,
                    endpoint.success_status,
                    &grants,
                    credential,
                )
                .await
                .map_err(|error| ExtensionError::AuthFailed(error.to_string()))?;
            }
            _ => {
                return Ok(format!(
                    "Setup metadata for '{}' is complete; no live validation endpoint is declared.",
                    name
                ));
            }
        }

        Ok(format!("Setup validation passed for '{}'.", name))
    }

    /// Save setup secrets for an extension, validating names against the capabilities schema.
    ///
    /// After saving, attempts to hot-activate the channel. Returns a [`SetupResult`]
    /// indicating whether activation succeeded (so the frontend can show appropriate UI).
    pub async fn save_setup_secrets(
        &self,
        name: &str,
        secrets: &std::collections::HashMap<String, String>,
    ) -> Result<SetupResult, ExtensionError> {
        let kind = self.determine_installed_kind(name).await?;
        if kind == ExtensionKind::WasmTool {
            let auth = self.load_wasm_tool_auth(name).await?.ok_or_else(|| {
                ExtensionError::Other(format!("No auth schema found for '{}'", name))
            })?;
            let token = secrets
                .get(&auth.secret_name)
                .or_else(|| secrets.get("token"))
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    ExtensionError::Other(format!("Missing token value for extension '{}'", name))
                })?;

            let auth_result = self
                .auth_wasm_tool(name, Some(token), AuthRequestContext::default())
                .await?;
            if auth_result.auth_status != "authenticated"
                && auth_result.auth_status != "no_auth_required"
            {
                return Err(ExtensionError::AuthFailed(
                    auth_result.instructions.unwrap_or_else(|| {
                        format!("Authentication still incomplete for '{}'", name)
                    }),
                ));
            }

            return match self.activate_wasm_tool(name).await {
                Ok(result) => Ok(SetupResult {
                    message: format!(
                        "Configuration saved and tool '{}' activated. {}",
                        name, result.message
                    ),
                    activated: true,
                }),
                Err(error) => Ok(SetupResult {
                    message: format!(
                        "Credentials saved for '{}', but activation failed: {}",
                        name, error
                    ),
                    activated: false,
                }),
            };
        }

        if kind != ExtensionKind::WasmChannel {
            return Err(ExtensionError::Other(
                "Setup is only supported for WASM tools and channels".to_string(),
            ));
        }

        let package = lock_wasm_package(&self.wasm_channels_dir, name).await?;
        let cap_bytes = package.capabilities_bytes.as_deref().ok_or_else(|| {
            ExtensionError::Other(format!("Capabilities file not found for '{}'", name))
        })?;
        let cap_file = crate::channels::wasm::ChannelCapabilitiesFile::from_bytes(cap_bytes)
            .map_err(|e| ExtensionError::Other(e.to_string()))?;

        // Build allowed secret names from capabilities
        let allowed: std::collections::HashSet<String> = cap_file
            .setup
            .required_secrets
            .iter()
            .map(|s| s.name.clone())
            .collect();

        // Validate and store each submitted secret
        for (secret_name, secret_value) in secrets {
            if !allowed.contains(secret_name.as_str()) {
                return Err(ExtensionError::Other(format!(
                    "Unknown secret '{}' for extension '{}'",
                    secret_name, name
                )));
            }
            if secret_value.trim().is_empty() {
                continue;
            }
            let params =
                CreateSecretParams::new(secret_name, secret_value).with_provider(name.to_string());
            self.secrets
                .create(&self.user_id, params)
                .await
                .map_err(|e| ExtensionError::AuthFailed(e.to_string()))?;
        }

        // Auto-generate any missing secrets that have auto_generate set
        for secret_def in &cap_file.setup.required_secrets {
            if let Some(ref auto_gen) = secret_def.auto_generate {
                let already_provided = secrets
                    .get(&secret_def.name)
                    .is_some_and(|v| !v.trim().is_empty());
                let already_stored = self
                    .secrets
                    .exists(&self.user_id, &secret_def.name)
                    .await
                    .map_err(|error| ExtensionError::AuthFailed(error.to_string()))?;
                if !already_provided && !already_stored {
                    use rand::Rng;
                    let mut bytes = vec![0u8; auto_gen.length];
                    rand::rng().fill_bytes(&mut bytes);
                    let hex_value: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
                    let params = CreateSecretParams::new(&secret_def.name, &hex_value)
                        .with_provider(name.to_string());
                    self.secrets
                        .create(&self.user_id, params)
                        .await
                        .map_err(|e| ExtensionError::AuthFailed(e.to_string()))?;
                    tracing::info!(
                        "Auto-generated secret '{}' for channel '{}'",
                        secret_def.name,
                        name
                    );
                }
            }
        }

        drop(package);

        // Try to hot-activate the channel now that secrets are saved
        match self.activate_wasm_channel(name).await {
            Ok(result) => {
                self.activation_errors.write().await.remove(name);
                self.broadcast_extension_status(name, "active", None).await;
                Ok(SetupResult {
                    message: format!(
                        "Configuration saved and channel '{}' activated. {}",
                        name, result.message
                    ),
                    activated: true,
                })
            }
            Err(e) => {
                let error_msg = e.to_string();
                tracing::warn!(
                    channel = name,
                    error = %e,
                    "Saved configuration but hot-activation failed, restart may be needed"
                );
                self.activation_errors
                    .write()
                    .await
                    .insert(name.to_string(), error_msg.clone());
                self.broadcast_extension_status(name, "failed", Some(&error_msg))
                    .await;
                Ok(SetupResult {
                    message: format!(
                        "Configuration saved for '{}'. \
                         Automatic activation failed ({}), restart ThinClaw to activate.",
                        name, e
                    ),
                    activated: false,
                })
            }
        }
    }
}
