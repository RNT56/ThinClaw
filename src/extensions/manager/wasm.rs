//! WASM tool and channel auth-status checks, token/OAuth auth flows,
//! activation (tool load + channel hot-activation), and credential refresh of
//! an already-active channel.

use std::sync::Arc;

use crate::channels::wasm::{
    RegisteredWebhookAuth, SharedWasmChannel, WasmChannelLoader, apply_channel_host_config,
    inject_channel_credentials_from_secrets,
};
use crate::extensions::{ActivateResult, AuthResult, ExtensionError, ExtensionKind};
use crate::secrets::CreateSecretParams;
use crate::tools::wasm::{
    WasmToolAuthCheck, WasmToolAuthStatus, WasmToolAuthorizationRequest, WasmToolLoader,
    WasmToolOAuthFlow,
};

use super::AuthRequestContext;
use super::ExtensionManager;
use super::core::{PendingAuth, lock_wasm_package};

impl ExtensionManager {
    pub(super) async fn check_wasm_tool_auth_status(
        &self,
        name: &str,
    ) -> Result<WasmToolAuthCheck, ExtensionError> {
        let package = lock_wasm_package(&self.wasm_tools_dir, name).await?;
        let Some(cap_bytes) = package.capabilities_bytes.as_deref() else {
            return Ok(WasmToolAuthCheck::no_auth_required());
        };

        let cap_file = crate::tools::wasm::CapabilitiesFile::from_bytes(cap_bytes)
            .map_err(|e| ExtensionError::Other(e.to_string()))?;
        let Some(auth) = cap_file.auth else {
            return Ok(WasmToolAuthCheck::no_auth_required());
        };

        let flow =
            WasmToolOAuthFlow::new(self.secrets.as_ref(), &self.user_id, &self.wasm_tools_dir);
        flow.check_auth_status(&auth)
            .await
            .map_err(|e| ExtensionError::AuthFailed(e.to_string()))
    }

    pub(super) async fn load_wasm_tool_auth(
        &self,
        name: &str,
    ) -> Result<Option<crate::tools::wasm::AuthCapabilitySchema>, ExtensionError> {
        let package = lock_wasm_package(&self.wasm_tools_dir, name).await?;
        let Some(cap_bytes) = package.capabilities_bytes.as_deref() else {
            return Ok(None);
        };

        let cap_file = crate::tools::wasm::CapabilitiesFile::from_bytes(cap_bytes)
            .map_err(|e| ExtensionError::Other(e.to_string()))?;
        Ok(cap_file.auth)
    }

    pub(super) async fn auth_wasm_tool(
        &self,
        name: &str,
        token: Option<&str>,
        context: AuthRequestContext,
    ) -> Result<AuthResult, ExtensionError> {
        let Some(auth) = self.load_wasm_tool_auth(name).await? else {
            return Ok(self.auth_result(name, ExtensionKind::WasmTool, "none", "no_auth_required"));
        };

        let flow =
            WasmToolOAuthFlow::new(self.secrets.as_ref(), &self.user_id, &self.wasm_tools_dir);

        if token.is_none()
            && let Some(ref env_var) = auth.env_var
            && let Ok(value) = std::env::var(env_var)
            && !value.trim().is_empty()
        {
            flow.store_manual_token(&auth, &value)
                .await
                .map_err(|e| ExtensionError::AuthFailed(e.to_string()))?;
        }

        if let Some(token_value) = token {
            flow.store_manual_token(&auth, token_value)
                .await
                .map_err(|e| ExtensionError::AuthFailed(e.to_string()))?;
        }

        let auth_check = flow
            .check_auth_status(&auth)
            .await
            .map_err(|e| ExtensionError::AuthFailed(e.to_string()))?;
        let mut result = self.auth_result(
            name,
            ExtensionKind::WasmTool,
            auth_check.auth_mode.as_str(),
            auth_check.auth_status.as_str(),
        );
        Self::apply_auth_check(&mut result, &auth_check);

        match auth_check.auth_status {
            WasmToolAuthStatus::Authenticated | WasmToolAuthStatus::NoAuthRequired => Ok(result),
            WasmToolAuthStatus::AwaitingToken => {
                let display = auth
                    .display_name
                    .clone()
                    .unwrap_or_else(|| name.to_string());
                result.instructions =
                    Some(auth.instructions.clone().unwrap_or_else(|| {
                        format!("Please provide your {} API token/key.", display)
                    }));
                result.setup_url = auth.setup_url.clone();
                result.awaiting_token = true;
                Ok(result)
            }
            WasmToolAuthStatus::AwaitingAuthorization
            | WasmToolAuthStatus::NeedsReauth
            | WasmToolAuthStatus::InsufficientScope => {
                let callback_base = self
                    .resolve_callback_base_url(context.callback_base_url.as_deref())
                    .await;
                if let Some(callback_base) = callback_base {
                    let redirect_uri = format!("{}/oauth/callback", callback_base);
                    let state_nonce = uuid::Uuid::new_v4().to_string();
                    let callback_type = context
                        .callback_type
                        .clone()
                        .unwrap_or_else(|| "web".to_string());
                    let auth_request: WasmToolAuthorizationRequest = flow
                        .prepare_authorization(
                            &auth,
                            &redirect_uri,
                            callback_type.clone(),
                            Some(&state_nonce),
                        )
                        .await
                        .map_err(|e| ExtensionError::AuthFailed(e.to_string()))?;

                    self.insert_pending_auth(
                        state_nonce.clone(),
                        PendingAuth {
                            name: name.to_string(),
                            kind: ExtensionKind::WasmTool,
                            code_verifier: auth_request.code_verifier.clone(),
                            mcp_authorization: None,
                            redirect_uri: Some(redirect_uri),
                            thread_id: context.thread_id.clone(),
                            created_at: std::time::Instant::now(),
                        },
                    )
                    .await;

                    result.auth_url = Some(auth_request.auth_url);
                    result.callback_type = Some(auth_request.callback_type);
                    result.instructions =
                        if auth_check.auth_status == WasmToolAuthStatus::NeedsReauth {
                            Some(format!(
                                "Reconnect {} to grant the additional scopes ThinClaw now needs.",
                                auth.display_name.as_deref().unwrap_or(name)
                            ))
                        } else {
                            Some(format!(
                                "Connect {} to continue.",
                                auth.display_name.as_deref().unwrap_or(name)
                            ))
                        };
                    result.awaiting_token = false;
                    return Ok(result);
                }

                result.instructions = Some(match auth_check.auth_status {
                    WasmToolAuthStatus::NeedsReauth | WasmToolAuthStatus::InsufficientScope => {
                        format!(
                            "This gateway cannot generate a browser callback URL right now. Run `thinclaw tool auth {}` locally to refresh the shared Google credential with the missing scopes.",
                            name
                        )
                    }
                    _ => format!(
                        "This gateway cannot generate a browser callback URL right now. Run `thinclaw tool auth {}` locally to connect the tool, or configure a public gateway/tunnel URL first.",
                        name
                    ),
                });
                Ok(result)
            }
        }
    }

    /// Check whether a WASM channel has all required secrets stored.
    /// Returns `(authenticated, needs_setup)`.
    pub(super) async fn check_channel_auth_status(
        &self,
        name: &str,
    ) -> Result<(bool, bool), ExtensionError> {
        let package = lock_wasm_package(&self.wasm_channels_dir, name).await?;
        let capabilities = package
            .capabilities_bytes
            .as_deref()
            .map(crate::channels::wasm::ChannelCapabilitiesFile::from_bytes)
            .transpose()
            .map_err(|error| ExtensionError::Other(error.to_string()))?;
        self.check_channel_manifest_auth_status(capabilities.as_ref())
            .await
    }

    pub(super) async fn check_channel_manifest_auth_status(
        &self,
        cap_file: Option<&crate::channels::wasm::ChannelCapabilitiesFile>,
    ) -> Result<(bool, bool), ExtensionError> {
        let Some(cap_file) = cap_file else {
            return Ok((true, false));
        };
        let required = &cap_file.setup.required_secrets;
        if required.is_empty() {
            return Ok((true, false));
        }
        let mut all_provided = true;
        for secret in required {
            if secret.optional {
                continue;
            }
            if !self
                .secrets
                .exists(&self.user_id, &secret.name)
                .await
                .map_err(|error| ExtensionError::AuthFailed(error.to_string()))?
            {
                all_provided = false;
                break;
            }
        }
        Ok((all_provided, true))
    }

    async fn load_channel_manifest_secret(
        &self,
        cap_file: &crate::channels::wasm::ChannelCapabilitiesFile,
        secret_name: &str,
        purpose: &'static str,
    ) -> Result<Option<String>, ExtensionError> {
        let definition = cap_file
            .setup
            .required_secrets
            .iter()
            .find(|secret| secret.name == secret_name)
            .ok_or_else(|| {
                ExtensionError::ActivationFailed(format!(
                    "Channel manifest references undeclared secret '{}'",
                    secret_name
                ))
            })?;
        if definition.optional
            && !self
                .secrets
                .exists(&self.user_id, secret_name)
                .await
                .map_err(|error| ExtensionError::ActivationFailed(error.to_string()))?
        {
            return Ok(None);
        }
        let secret = self
            .secrets
            .get_for_injection(
                &self.user_id,
                secret_name,
                crate::secrets::SecretAccessContext::new("extensions.manager", purpose),
            )
            .await
            .map_err(|error| {
                ExtensionError::ActivationFailed(format!(
                    "failed to load channel secret '{}': {error}",
                    secret_name
                ))
            })?;
        Ok(Some(secret.expose().to_string()))
    }

    async fn load_channel_webhook_auth(
        &self,
        cap_file: Option<&crate::channels::wasm::ChannelCapabilitiesFile>,
    ) -> Result<(RegisteredWebhookAuth, Option<String>), ExtensionError> {
        let Some(cap_file) = cap_file else {
            return Ok((RegisteredWebhookAuth::default(), None));
        };
        let Some(webhook) = cap_file
            .capabilities
            .channel
            .as_ref()
            .and_then(|channel| channel.webhook.as_ref())
        else {
            return Ok((RegisteredWebhookAuth::default(), None));
        };

        let signature_secret_name = cap_file.webhook_secret_name();
        let signature_secret = self
            .load_channel_manifest_secret(
                cap_file,
                &signature_secret_name,
                "webhook_signature_validation",
            )
            .await?;
        let verify_token_secret = if webhook.verify_token_param.is_some() {
            let verify_name = cap_file.webhook_verify_token_secret_name().ok_or_else(|| {
                ExtensionError::ActivationFailed(
                    "Channel verify-token configuration is incomplete".to_string(),
                )
            })?;
            if verify_name == signature_secret_name {
                signature_secret.clone()
            } else {
                self.load_channel_manifest_secret(cap_file, &verify_name, "webhook_verify_token")
                    .await?
            }
        } else {
            None
        };

        Ok((
            RegisteredWebhookAuth {
                secret_header: webhook.secret_header.clone(),
                secret_validation: webhook.secret_validation,
                signature_secret: signature_secret.clone(),
                verify_token_param: webhook.verify_token_param.clone(),
                verify_token_secret,
            },
            signature_secret,
        ))
    }

    pub(super) async fn auth_wasm_channel(
        &self,
        name: &str,
        token: Option<&str>,
    ) -> Result<AuthResult, ExtensionError> {
        let package = lock_wasm_package(&self.wasm_channels_dir, name).await?;
        let Some(cap_bytes) = package.capabilities_bytes.as_deref() else {
            return Ok(self.auth_result(
                name,
                ExtensionKind::WasmChannel,
                "none",
                "no_auth_required",
            ));
        };

        let cap_file = crate::channels::wasm::ChannelCapabilitiesFile::from_bytes(cap_bytes)
            .map_err(|e| ExtensionError::Other(e.to_string()))?;

        // Get required secrets from the setup section
        let required_secrets = &cap_file.setup.required_secrets;
        if required_secrets.is_empty() {
            return Ok(self.auth_result(
                name,
                ExtensionKind::WasmChannel,
                "none",
                "no_auth_required",
            ));
        }

        // Find the first non-optional secret that isn't yet stored
        let mut missing = Vec::new();
        for secret in required_secrets {
            if secret.optional {
                continue;
            }
            if !self
                .secrets
                .exists(&self.user_id, &secret.name)
                .await
                .map_err(|error| ExtensionError::AuthFailed(error.to_string()))?
            {
                missing.push(secret);
            }
        }

        if missing.is_empty() {
            return Ok(self.auth_result(
                name,
                ExtensionKind::WasmChannel,
                "secrets",
                "authenticated",
            ));
        }

        // If a token was provided, store it for the first missing secret
        if let Some(token_value) = token {
            let secret = &missing[0];
            let params =
                CreateSecretParams::new(&secret.name, token_value).with_provider(name.to_string());
            self.secrets
                .create(&self.user_id, params)
                .await
                .map_err(|e| ExtensionError::AuthFailed(e.to_string()))?;

            // Check if there are more missing secrets
            if missing.len() <= 1 {
                return Ok(self.auth_result(
                    name,
                    ExtensionKind::WasmChannel,
                    "secrets",
                    "authenticated",
                ));
            }

            // More secrets needed; prompt for the next one
            let next = &missing[1];
            let mut result = self.auth_result(
                name,
                ExtensionKind::WasmChannel,
                "secrets",
                "awaiting_token",
            );
            result.instructions = Some(next.prompt.clone());
            result.setup_url = cap_file
                .setup
                .validation_endpoint
                .as_ref()
                .map(|endpoint| endpoint.url().to_string());
            result.awaiting_token = true;
            return Ok(result);
        }

        // Prompt for the first missing secret
        let secret = &missing[0];
        let mut result = self.auth_result(
            name,
            ExtensionKind::WasmChannel,
            "secrets",
            "awaiting_token",
        );
        result.instructions = Some(secret.prompt.clone());
        result.setup_url = cap_file
            .setup
            .validation_endpoint
            .as_ref()
            .map(|endpoint| endpoint.url().to_string());
        result.awaiting_token = true;
        Ok(result)
    }

    pub(super) async fn activate_wasm_tool(
        &self,
        name: &str,
    ) -> Result<ActivateResult, ExtensionError> {
        let _operation = self.wasm_operation_lock.lock().await;
        // Check if already active
        if self.tool_registry.has(name).await {
            return Ok(ActivateResult {
                name: name.to_string(),
                kind: ExtensionKind::WasmTool,
                tools_loaded: vec![name.to_string()],
                message: format!("WASM tool '{}' already active", name),
            });
        }

        let runtime = self.wasm_tool_runtime.as_ref().ok_or_else(|| {
            ExtensionError::ActivationFailed("WASM runtime not available".to_string())
        })?;

        let wasm_path = self.wasm_tools_dir.join(format!("{}.wasm", name));
        let package = lock_wasm_package(&self.wasm_tools_dir, name).await?;
        let capabilities_file = package
            .capabilities_bytes
            .as_deref()
            .map(crate::tools::wasm::CapabilitiesFile::from_bytes)
            .transpose()
            .map_err(|error| ExtensionError::ActivationFailed(error.to_string()))?;
        if let Some(auth) = capabilities_file
            .as_ref()
            .and_then(|capabilities| capabilities.auth.as_ref())
        {
            let flow =
                WasmToolOAuthFlow::new(self.secrets.as_ref(), &self.user_id, &self.wasm_tools_dir);
            let auth_check = flow
                .check_auth_status(auth)
                .await
                .map_err(|error| ExtensionError::ActivationFailed(error.to_string()))?;
            if !matches!(
                auth_check.auth_status,
                WasmToolAuthStatus::Authenticated | WasmToolAuthStatus::NoAuthRequired
            ) {
                return Err(ExtensionError::ActivationFailed(format!(
                    "WASM tool '{}' is not authenticated (status: {})",
                    name,
                    auth_check.auth_status.as_str()
                )));
            }
        }

        let mut loader = WasmToolLoader::new(Arc::clone(runtime), Arc::clone(&self.tool_registry))
            .with_secrets_store(Arc::clone(&self.secrets));
        if let Some(invoker) = &self.wasm_tool_invoker {
            loader = loader.with_tool_invoker(Arc::clone(invoker));
        }
        let loaded = loader
            .load_from_files_with_metadata(
                name,
                &wasm_path,
                Some(package.capabilities_path.as_path()),
            )
            .await
            .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?;
        drop(package);

        if let Some(ref hooks) = self.hooks {
            let source = format!("plugin.tool:{}", name);
            let registration = crate::hooks::bootstrap::register_plugin_bundle_from_hooks_value(
                hooks,
                &source,
                loaded
                    .capabilities_file
                    .and_then(|capabilities| capabilities.hooks),
            )
            .await;

            if registration.total_registered() > 0 {
                tracing::info!(
                    extension = name,
                    hooks = registration.hooks,
                    outbound_webhooks = registration.outbound_webhooks,
                    "Registered plugin hooks for activated WASM tool"
                );
            }

            if registration.errors > 0 {
                tracing::warn!(
                    extension = name,
                    errors = registration.errors,
                    "Some plugin hooks failed to register"
                );
            }
        }

        tracing::info!("Activated WASM tool '{}'", name);

        Ok(ActivateResult {
            name: name.to_string(),
            kind: ExtensionKind::WasmTool,
            tools_loaded: vec![name.to_string()],
            message: format!("WASM tool '{}' loaded and ready", name),
        })
    }

    /// Activate a WASM channel at runtime without restarting.
    ///
    /// Loads the channel from its WASM file, injects credentials and config,
    /// registers it with the webhook router, and hot-adds it to the channel manager
    /// so its stream feeds into the agent loop.
    pub(super) async fn activate_wasm_channel(
        &self,
        name: &str,
    ) -> Result<ActivateResult, ExtensionError> {
        let _operation = self.wasm_operation_lock.lock().await;
        // Verify runtime infrastructure is available and clone Arcs so we don't
        // hold the RwLock guard across awaits.
        let (channel_runtime, channel_manager, pairing_store, wasm_channel_router, host_config) = {
            let rt_guard = self.channel_runtime.read().await;
            let rt = rt_guard.as_ref().ok_or_else(|| {
                ExtensionError::ActivationFailed(
                    "WASM channel runtime not configured. Restart ThinClaw to activate."
                        .to_string(),
                )
            })?;
            (
                Arc::clone(&rt.wasm_channel_runtime),
                Arc::clone(&rt.channel_manager),
                Arc::clone(&rt.pairing_store),
                Arc::clone(&rt.wasm_channel_router),
                rt.host_config.clone(),
            )
        };

        // Hold the package snapshot lock across both the auth decision and the
        // loader read so neither can observe a different published generation.
        let package = lock_wasm_package(&self.wasm_channels_dir, name).await?;
        let wasm_path = self.wasm_channels_dir.join(format!("{}.wasm", name));

        let loader =
            WasmChannelLoader::new(Arc::clone(&channel_runtime), Arc::clone(&pairing_store));
        let loaded = loader
            .load_from_files(name, &wasm_path, Some(package.capabilities_path.as_path()))
            .await
            .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?;

        let (authenticated, _needs_setup) = self
            .check_channel_manifest_auth_status(loaded.capabilities_file())
            .await?;
        if !authenticated {
            return Err(ExtensionError::ActivationFailed(format!(
                "Channel '{}' requires configuration. Use the setup form to provide credentials.",
                name
            )));
        }
        drop(package);

        let channel_name = loaded.name().to_string();
        let (webhook_auth, signature_secret) = self
            .load_channel_webhook_auth(loaded.capabilities_file())
            .await?;

        let channel_arc = Arc::new(loaded.channel);

        let runtime_update_count = apply_channel_host_config(
            &channel_arc,
            &channel_name,
            &host_config,
            signature_secret.as_deref(),
        )
        .await;
        if runtime_update_count > 0 {
            tracing::info!(
                channel = %channel_name,
                runtime_updates = runtime_update_count,
                "Injected runtime config into hot-activated channel"
            );
        }

        // Inject credentials
        match inject_channel_credentials_from_secrets(
            &channel_arc,
            self.secrets.as_ref(),
            &channel_name,
            &self.user_id,
        )
        .await
        {
            Ok(count) => {
                if count > 0 {
                    tracing::info!(
                        channel = %channel_name,
                        credentials_injected = count,
                        "Credentials injected into hot-activated channel"
                    );
                }
            }
            Err(e) => {
                return Err(ExtensionError::ActivationFailed(format!(
                    "credential injection failed: {e}"
                )));
            }
        }

        channel_arc
            .prime_on_start_config()
            .await
            .map_err(|error| ExtensionError::ActivationFailed(error.to_string()))?;

        let endpoints = channel_arc.endpoints().await;
        wasm_channel_router
            .validate_registration(&channel_name, &endpoints, &webhook_auth)
            .await
            .map_err(ExtensionError::ActivationFailed)?;

        // Start the replacement before publishing its webhook routes. hot_add
        // starts successfully before it swaps and drains any previous channel.
        channel_manager
            .hot_add(Box::new(SharedWasmChannel::new(Arc::clone(&channel_arc))))
            .await
            .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?;

        wasm_channel_router
            .register(Arc::clone(&channel_arc), endpoints, webhook_auth)
            .await
            .map_err(ExtensionError::ActivationFailed)?;
        tracing::info!(channel = %channel_name, "Registered hot-activated channel with webhook router");

        // Mark as active and persist
        self.active_channel_names
            .write()
            .await
            .insert(channel_name.clone());
        self.persist_active_channels().await;

        tracing::info!(channel = %channel_name, "Hot-activated WASM channel");

        Ok(ActivateResult {
            name: channel_name,
            kind: ExtensionKind::WasmChannel,
            tools_loaded: Vec::new(),
            message: format!("Channel '{}' activated and running", name),
        })
    }
}
