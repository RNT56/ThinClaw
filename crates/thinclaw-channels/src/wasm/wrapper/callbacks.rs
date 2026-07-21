//! WASM callback execution and the status/typing/poll plumbing.
//!
//! Owns the [`WasmChannel`] methods that drive a fresh WASM instance per
//! callback (`on_start`, `on_http_request`, `on_poll`, `on_respond`,
//! `on_status`), the store/linker/instance setup helpers, the background
//! typing-indicator and polling tasks, the status-update dispatch, and the
//! emitted-message fan-out into the channel stream. These are the
//! channel-agnostic host mechanics shared by every WASM channel.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{RwLock, mpsc, oneshot};
use uuid::Uuid;
use wasmtime::Store;
use wasmtime::component::Linker;

use crate::wasm::capabilities::{ChannelCapabilities, WorkspaceCapability};
use crate::wasm::error::WasmChannelError;
use crate::wasm::host::LogLevel;
use crate::wasm::host::{
    ChannelEmitRateLimiter, ChannelHostState, ChannelWorkspaceStore, EmittedMessage,
};
use crate::wasm::router::RegisteredEndpoint;
use crate::wasm::runtime::{PreparedChannelModule, WasmChannelRuntime};
use crate::wasm::schema::ChannelConfig;
use thinclaw_channels_core::StatusUpdate;
use thinclaw_types::error::ChannelError;

use super::conversions::{
    HttpResponse, clone_wit_status_update, convert_channel_config, convert_http_response,
    emitted_message_to_incoming_message, status_to_wit,
};
use super::store::{ChannelStoreData, ToolEventEntry, html_escape};
use super::{SandboxedChannel, WasmChannel, wit_channel};

impl WasmChannel {
    fn effective_callback_timeout(
        runtime: &WasmChannelRuntime,
        prepared: &PreparedChannelModule,
        capabilities: &ChannelCapabilities,
    ) -> Duration {
        runtime
            .config()
            .callback_timeout
            .min(prepared.limits.timeout)
            .min(capabilities.callback_timeout)
    }

    /// Flush accumulated tool events as a single formatted summary message.
    ///
    /// Called at the start of `respond()` so the tool activity block appears
    /// before the final response.  Does nothing if the accumulator is empty.
    pub(super) async fn flush_tool_events(&self, metadata: &serde_json::Value) {
        let events: Vec<ToolEventEntry> = {
            let mut guard = self.pending_tool_events.write().await;
            if guard.is_empty() {
                return;
            }
            std::mem::take(&mut *guard)
        };

        // Build a grouped, single-message summary.
        //
        // Format (Telegram HTML):
        //   🔧 <b>Tool Activity</b>
        //   ✅ web_search
        //      "query text…"
        //   ✅ read_file
        //      main.rs (1,234 chars)
        //   ❌ list_dir — failed
        //   ─────────────
        //   3 calls · 2✅ 1❌

        let mut lines: Vec<String> = vec!["🔧 <b>Tool Activity</b>".to_string()];

        // Walk through events in order, emitting one visual block per tool
        let mut succeeded = 0u32;
        let mut failed = 0u32;
        let mut total_calls = 0u32;

        for event in &events {
            match event {
                ToolEventEntry::Started { name } => {
                    // We'll render the tool line when we see the Completed event.
                    // If we never get a Completed (edge case), the Started is
                    // just informational.  We track it for ordering context.
                    let _ = name; // used below via Completed
                }
                ToolEventEntry::Completed { name, success } => {
                    total_calls += 1;
                    let icon = if *success { "✅" } else { "❌" };
                    let suffix = if *success { "" } else { " — failed" };
                    lines.push(format!(
                        "{} <code>{}</code>{}",
                        icon,
                        html_escape(name),
                        suffix
                    ));
                    if *success {
                        succeeded += 1;
                    } else {
                        failed += 1;
                    }
                }
                ToolEventEntry::Result { preview } => {
                    if !preview.is_empty() {
                        // Truncate long results
                        let display: String = if preview.chars().count() > 120 {
                            let truncated: String = preview.chars().take(117).collect();
                            format!("{}…", truncated)
                        } else {
                            preview.clone()
                        };
                        lines.push(format!("   <i>{}</i>", html_escape(&display)));
                    }
                }
            }
        }

        // Footer
        if total_calls > 0 {
            lines.push("───────────────".to_string());
            let mut footer_parts = vec![format!(
                "{} call{}",
                total_calls,
                if total_calls == 1 { "" } else { "s" }
            )];
            if succeeded > 0 {
                footer_parts.push(format!("{}✅", succeeded));
            }
            if failed > 0 {
                footer_parts.push(format!("{}❌", failed));
            }
            lines.push(footer_parts.join(" · "));
        }

        let summary = lines.join("\n");

        // Send as a single message via on_respond (not on_status, which
        // would go through the WASM status handler and might be dropped)
        let metadata_json = serde_json::to_string(metadata).unwrap_or_default();
        if let Err(e) = self
            .call_on_respond(uuid::Uuid::new_v4(), &summary, None, &metadata_json)
            .await
        {
            tracing::debug!(
                channel = %self.name,
                error = %e,
                "Failed to send tool summary (best-effort)"
            );
        }
    }

    pub(super) fn registered_endpoints_from_config(
        &self,
        config: &ChannelConfig,
    ) -> Result<Vec<RegisteredEndpoint>, WasmChannelError> {
        if config.display_name.is_empty()
            || config.display_name.len() > 1024
            || config.display_name.chars().any(char::is_control)
            || config.http_endpoints.len() > 32
        {
            return Err(WasmChannelError::CallbackFailed {
                name: self.name.clone(),
                reason: "on_start returned an invalid display name or too many endpoints"
                    .to_string(),
            });
        }
        let mut endpoints = Vec::new();

        for endpoint in &config.http_endpoints {
            if !self.capabilities.is_path_allowed(&endpoint.path)
                || endpoint.path.len() > 256
                || endpoint.methods.len() > 8
                || endpoint.methods.iter().any(|method| {
                    !matches!(
                        method.to_ascii_uppercase().as_str(),
                        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD"
                    )
                })
                || !endpoint.require_secret
            {
                return Err(WasmChannelError::CallbackFailed {
                    name: self.name.clone(),
                    reason: "on_start returned an unauthorized or unauthenticated webhook endpoint"
                        .to_string(),
                });
            }

            endpoints.push(RegisteredEndpoint {
                channel_name: self.name.clone(),
                path: endpoint.path.clone(),
                methods: endpoint.methods.clone(),
                require_secret: endpoint.require_secret,
            });
        }

        if let Some(poll) = &config.poll
            && poll.enabled
        {
            self.capabilities
                .validate_poll_interval(poll.interval_ms)
                .map_err(|reason| WasmChannelError::CallbackFailed {
                    name: self.name.clone(),
                    reason,
                })?;
        }

        Ok(endpoints)
    }

    async fn cache_channel_config(&self, config: &ChannelConfig) -> Result<(), WasmChannelError> {
        let endpoints = self.registered_endpoints_from_config(config)?;
        *self.channel_config.write().await = Some(config.clone());
        *self.endpoints.write().await = endpoints;
        Ok(())
    }

    pub(super) async fn ensure_on_start_config(
        &self,
        force_refresh: bool,
    ) -> Result<ChannelConfig, WasmChannelError> {
        if !force_refresh && let Some(existing) = self.channel_config.read().await.clone() {
            return Ok(existing);
        }

        let _on_start_guard = self.on_start_lock.lock().await;
        if !force_refresh && let Some(existing) = self.channel_config.read().await.clone() {
            return Ok(existing);
        }

        let config = self.call_on_start().await?;
        self.cache_channel_config(&config).await?;
        Ok(config)
    }

    /// Inject the workspace store as the reader into a capabilities clone.
    ///
    /// Ensures `workspace_read` capability is present with the store as its reader,
    /// so WASM callbacks can read previously written workspace state.
    fn inject_workspace_reader(
        capabilities: &ChannelCapabilities,
        store: &Arc<ChannelWorkspaceStore>,
    ) -> ChannelCapabilities {
        let mut caps = capabilities.clone();
        let ws_cap = caps
            .tool_capabilities
            .workspace_read
            .get_or_insert_with(|| WorkspaceCapability {
                allowed_prefixes: Vec::new(),
            });
        let _ = (store, ws_cap);
        caps
    }

    /// Add channel host functions to the linker using generated bindings.
    ///
    /// Uses the wasmtime::component::bindgen! generated `add_to_linker` function
    /// to properly register all host functions with correct component model signatures.
    fn add_host_functions(linker: &mut Linker<ChannelStoreData>) -> Result<(), WasmChannelError> {
        // Add WASI support (required by the component adapter)
        wasmtime_wasi::p2::add_to_linker_sync(linker).map_err(|e| {
            WasmChannelError::Config(format!("Failed to add WASI functions: {}", e))
        })?;

        // Use the generated add_to_linker function from bindgen for our custom interface
        super::near::agent::channel_host::add_to_linker::<
            ChannelStoreData,
            wasmtime::component::HasSelf<ChannelStoreData>,
        >(linker, |state| state)
        .map_err(|e| WasmChannelError::Config(format!("Failed to add host functions: {}", e)))?;

        Ok(())
    }

    /// Create a fresh store configured for WASM execution.
    fn create_store(
        runtime: &WasmChannelRuntime,
        prepared: &PreparedChannelModule,
        capabilities: &ChannelCapabilities,
        credentials: HashMap<String, String>,
        pairing_store: Arc<crate::pairing::PairingStore>,
        workspace_store: Arc<ChannelWorkspaceStore>,
    ) -> Result<Store<ChannelStoreData>, WasmChannelError> {
        let engine = runtime.engine();
        let limits = &prepared.limits;

        // Create fresh store with channel state (NEAR pattern: fresh instance per call)
        let store_data = ChannelStoreData::new(
            limits.memory_bytes,
            &prepared.name,
            capabilities.clone(),
            credentials,
            pairing_store,
            workspace_store,
        );
        let mut store = Store::new(engine, store_data);

        // Configure fuel if enabled
        if runtime.config().fuel_config.enabled {
            store
                .set_fuel(limits.fuel)
                .map_err(|e| WasmChannelError::Config(format!("Failed to set fuel: {}", e)))?;
        }

        // Configure the epoch deadline as a backup timeout. It is scaled to the
        // callback timeout (plus a margin) so it fires *after* the outer async
        // timeout — it exists only to terminate a guest still spinning on a
        // blocking thread, never to cut short legitimate in-budget work.
        store.epoch_deadline_trap();
        store.set_epoch_deadline(crate::wasm::runtime::epoch_deadline_ticks(
            Self::effective_callback_timeout(runtime, prepared, capabilities),
        ));

        // Set up resource limiter
        store.limiter(|data| &mut data.limiter);

        Ok(store)
    }

    /// Instantiate the WASM component using generated bindings.
    fn instantiate_component(
        runtime: &WasmChannelRuntime,
        prepared: &PreparedChannelModule,
        store: &mut Store<ChannelStoreData>,
    ) -> Result<SandboxedChannel, WasmChannelError> {
        let engine = runtime.engine();

        // Use the pre-compiled component (no recompilation needed)
        let component = prepared
            .component()
            .ok_or_else(|| {
                WasmChannelError::Compilation("No compiled component available".to_string())
            })?
            .clone();

        // Create linker and add host functions
        let mut linker = Linker::new(engine);
        Self::add_host_functions(&mut linker)?;

        // Instantiate using the generated bindings
        let instance = SandboxedChannel::instantiate(store, &component, &linker)
            .map_err(|e| WasmChannelError::Instantiation(e.to_string()))?;

        Ok(instance)
    }

    /// Map WASM execution errors to our error types.
    fn map_wasm_error(e: anyhow::Error, name: &str, fuel_limit: u64) -> WasmChannelError {
        let error_str = e.to_string();
        if error_str.contains("out of fuel") {
            WasmChannelError::FuelExhausted {
                name: name.to_string(),
                limit: fuel_limit,
            }
        } else if error_str.contains("unreachable") {
            WasmChannelError::Trapped {
                name: name.to_string(),
                reason: "unreachable code executed".to_string(),
            }
        } else {
            WasmChannelError::Trapped {
                name: name.to_string(),
                reason: error_str,
            }
        }
    }

    /// Extract host state after callback execution.
    fn extract_host_state(
        store: &mut Store<ChannelStoreData>,
        channel_name: &str,
        capabilities: &ChannelCapabilities,
    ) -> ChannelHostState {
        std::mem::replace(
            &mut store.data_mut().host_state,
            ChannelHostState::new(channel_name, capabilities.clone()),
        )
    }

    /// Execute the on_start callback.
    ///
    /// Returns the channel configuration for HTTP endpoint registration.
    /// Call the WASM module's `on_start` callback.
    ///
    /// Typically called once during `start()`, but can be called again after
    /// credentials are refreshed to re-trigger webhook registration and
    /// other one-time setup that depends on credentials.
    pub async fn call_on_start(&self) -> Result<ChannelConfig, WasmChannelError> {
        // If no WASM bytes, return default config (for testing)
        if self.prepared.component().is_none() {
            tracing::info!(
                channel = %self.name,
                "WASM channel on_start called (no WASM module, returning defaults)"
            );
            return Ok(ChannelConfig {
                display_name: self.prepared.description.clone(),
                http_endpoints: Vec::new(),
                poll: None,
            });
        }

        let runtime = Arc::clone(&self.runtime);
        let prepared = Arc::clone(&self.prepared);
        let capabilities = Self::inject_workspace_reader(&self.capabilities, &self.workspace_store);
        let config_json = self.apply_telegram_runtime_state(
            self.config_json.read().await.clone(),
            &self.load_runtime_state(),
        );
        let timeout =
            Self::effective_callback_timeout(&self.runtime, &self.prepared, &self.capabilities);
        let channel_name = self.name.clone();
        let credentials = self.get_credentials().await;
        let pairing_store = self.pairing_store.clone();
        let workspace_store = self.workspace_store.clone();

        // Execute in blocking task with timeout
        let result = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                let mut store = Self::create_store(
                    &runtime,
                    &prepared,
                    &capabilities,
                    credentials,
                    pairing_store,
                    workspace_store.clone(),
                )?;
                let instance = Self::instantiate_component(&runtime, &prepared, &mut store)?;

                // Call on_start using the generated typed interface
                let channel_iface = instance.near_agent_channel();
                let wasm_result = channel_iface
                    .call_on_start(&mut store, &config_json)
                    .map_err(|e| Self::map_wasm_error(e, &prepared.name, prepared.limits.fuel))?;

                // Convert the result
                let config = match wasm_result {
                    Ok(wit_config) => convert_channel_config(wit_config),
                    Err(err_msg) => {
                        return Err(WasmChannelError::CallbackFailed {
                            name: prepared.name.clone(),
                            reason: err_msg,
                        });
                    }
                };

                let mut host_state =
                    Self::extract_host_state(&mut store, &prepared.name, &capabilities);

                // Commit pending workspace writes to the persistent store
                let pending_writes = host_state.take_pending_writes();
                workspace_store
                    .commit_writes(&pending_writes)
                    .map_err(|reason| WasmChannelError::CallbackFailed {
                        name: prepared.name.clone(),
                        reason: format!("failed to persist channel workspace: {reason}"),
                    })?;

                Ok((config, host_state))
            })
            .await
            .map_err(|e| WasmChannelError::ExecutionPanicked {
                name: channel_name.clone(),
                reason: e.to_string(),
            })?
        })
        .await;

        match result {
            Ok(Ok((config, mut host_state))) => {
                // Surface WASM guest logs (errors/warnings from webhook setup, etc.)
                for entry in host_state.take_logs() {
                    match entry.level {
                        LogLevel::Error => {
                            tracing::error!(channel = %self.name, message_bytes = entry.message.len(), "WASM guest logged an error");
                        }
                        LogLevel::Warn => {
                            tracing::warn!(channel = %self.name, message_bytes = entry.message.len(), "WASM guest logged a warning");
                        }
                        _ => {
                            tracing::debug!(channel = %self.name, level = ?entry.level, message_bytes = entry.message.len(), "WASM guest log entry");
                        }
                    }
                }
                tracing::info!(
                    channel = %self.name,
                    display_name = %config.display_name,
                    endpoints = config.http_endpoints.len(),
                    "WASM channel on_start completed"
                );
                Ok(config)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(WasmChannelError::Timeout {
                name: self.name.clone(),
                callback: "on_start".to_string(),
            }),
        }
    }

    /// Execute the on_http_request callback.
    ///
    /// Called when an HTTP request arrives at a registered endpoint.
    pub async fn call_on_http_request(
        &self,
        method: &str,
        path: &str,
        headers: &HashMap<String, String>,
        query: &HashMap<String, String>,
        body: &[u8],
        secret_validated: bool,
    ) -> Result<HttpResponse, WasmChannelError> {
        tracing::info!(
            channel = %self.name,
            method = method,
            path = path,
            body_len = body.len(),
            secret_validated = secret_validated,
            "call_on_http_request invoked (webhook received)"
        );

        // Log credentials state (without values)
        let creds = self.get_credentials().await;
        tracing::info!(
            credential_count = creds.len(),
            credential_names = ?creds.keys().collect::<Vec<_>>(),
            "Credentials available for on_http_request"
        );

        // If no WASM bytes, return 200 OK (for testing)
        if self.prepared.component().is_none() {
            tracing::debug!(
                channel = %self.name,
                method = method,
                path = path,
                "WASM channel on_http_request called (no WASM module)"
            );
            return Ok(HttpResponse::ok());
        }

        let runtime = Arc::clone(&self.runtime);
        let prepared = Arc::clone(&self.prepared);
        let capabilities = Self::inject_workspace_reader(&self.capabilities, &self.workspace_store);
        let timeout =
            Self::effective_callback_timeout(&self.runtime, &self.prepared, &self.capabilities);
        let credentials = self.get_credentials().await;
        let pairing_store = self.pairing_store.clone();
        let workspace_store = self.workspace_store.clone();

        // Prepare request data
        let method = method.to_string();
        let path = path.to_string();
        let headers_json = serde_json::to_string(&headers).unwrap_or_default();
        let query_json = serde_json::to_string(&query).unwrap_or_default();
        let body = body.to_vec();

        let channel_name = self.name.clone();

        // Execute in blocking task with timeout
        let result = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                let mut store = Self::create_store(
                    &runtime,
                    &prepared,
                    &capabilities,
                    credentials,
                    pairing_store,
                    workspace_store.clone(),
                )?;
                let instance = Self::instantiate_component(&runtime, &prepared, &mut store)?;

                // Build the WIT request type
                let wit_request = wit_channel::IncomingHttpRequest {
                    method,
                    path,
                    headers_json,
                    query_json,
                    body,
                    secret_validated,
                };

                // Call on_http_request using the generated typed interface
                let channel_iface = instance.near_agent_channel();
                let wit_response = channel_iface
                    .call_on_http_request(&mut store, &wit_request)
                    .map_err(|e| Self::map_wasm_error(e, &prepared.name, prepared.limits.fuel))?;

                let response = convert_http_response(wit_response);
                let mut host_state =
                    Self::extract_host_state(&mut store, &prepared.name, &capabilities);

                // Commit pending workspace writes to the persistent store
                let pending_writes = host_state.take_pending_writes();
                workspace_store
                    .commit_writes(&pending_writes)
                    .map_err(|reason| WasmChannelError::CallbackFailed {
                        name: prepared.name.clone(),
                        reason: format!("failed to persist channel workspace: {reason}"),
                    })?;

                Ok((response, host_state))
            })
            .await
            .map_err(|e| WasmChannelError::ExecutionPanicked {
                name: channel_name.clone(),
                reason: e.to_string(),
            })?
        })
        .await;

        let channel_name = self.name.clone();
        match result {
            Ok(Ok((response, mut host_state))) => {
                // Process emitted messages
                let emitted = host_state.take_emitted_messages();
                self.process_emitted_messages(emitted).await?;

                tracing::debug!(
                    channel = %channel_name,
                    status = response.status,
                    "WASM channel on_http_request completed"
                );
                Ok(response)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(WasmChannelError::Timeout {
                name: channel_name,
                callback: "on_http_request".to_string(),
            }),
        }
    }

    /// Execute the on_poll callback.
    ///
    /// Called periodically if polling is configured.
    pub async fn call_on_poll(&self) -> Result<(), WasmChannelError> {
        // If no WASM bytes, do nothing (for testing)
        if self.prepared.component().is_none() {
            tracing::debug!(
                channel = %self.name,
                "WASM channel on_poll called (no WASM module)"
            );
            return Ok(());
        }

        let runtime = Arc::clone(&self.runtime);
        let prepared = Arc::clone(&self.prepared);
        let capabilities = Self::inject_workspace_reader(&self.capabilities, &self.workspace_store);
        let timeout =
            Self::effective_callback_timeout(&self.runtime, &self.prepared, &self.capabilities);
        let channel_name = self.name.clone();
        let credentials = self.get_credentials().await;
        let pairing_store = self.pairing_store.clone();
        let workspace_store = self.workspace_store.clone();

        // Execute in blocking task with timeout
        let result = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                let mut store = Self::create_store(
                    &runtime,
                    &prepared,
                    &capabilities,
                    credentials,
                    pairing_store,
                    workspace_store.clone(),
                )?;
                let instance = Self::instantiate_component(&runtime, &prepared, &mut store)?;

                // Call on_poll using the generated typed interface
                let channel_iface = instance.near_agent_channel();
                channel_iface
                    .call_on_poll(&mut store)
                    .map_err(|e| Self::map_wasm_error(e, &prepared.name, prepared.limits.fuel))?;

                let mut host_state =
                    Self::extract_host_state(&mut store, &prepared.name, &capabilities);

                // Commit pending workspace writes to the persistent store
                let pending_writes = host_state.take_pending_writes();
                workspace_store
                    .commit_writes(&pending_writes)
                    .map_err(|reason| WasmChannelError::CallbackFailed {
                        name: prepared.name.clone(),
                        reason: format!("failed to persist channel workspace: {reason}"),
                    })?;

                Ok(((), host_state))
            })
            .await
            .map_err(|e| WasmChannelError::ExecutionPanicked {
                name: channel_name.clone(),
                reason: e.to_string(),
            })?
        })
        .await;

        let channel_name = self.name.clone();
        match result {
            Ok(Ok(((), mut host_state))) => {
                // Process emitted messages
                let emitted = host_state.take_emitted_messages();
                self.process_emitted_messages(emitted).await?;

                tracing::debug!(
                    channel = %channel_name,
                    "WASM channel on_poll completed"
                );
                Ok(())
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(WasmChannelError::Timeout {
                name: channel_name,
                callback: "on_poll".to_string(),
            }),
        }
    }

    /// Execute the on_respond callback.
    ///
    /// Called when the agent has a response to send back.
    pub async fn call_on_respond(
        &self,
        message_id: Uuid,
        content: &str,
        thread_id: Option<&str>,
        metadata_json: &str,
    ) -> Result<(), WasmChannelError> {
        tracing::info!(
            channel = %self.name,
            message_id = %message_id,
            content_len = content.len(),
            thread_id = ?thread_id,
            "call_on_respond invoked"
        );

        // Log credentials state (without values)
        let creds = self.get_credentials().await;
        tracing::info!(
            credential_count = creds.len(),
            credential_names = ?creds.keys().collect::<Vec<_>>(),
            "Credentials available for on_respond"
        );

        // If no WASM bytes, do nothing (for testing)
        if self.prepared.component().is_none() {
            tracing::debug!(
                channel = %self.name,
                message_id = %message_id,
                "WASM channel on_respond called (no WASM module)"
            );
            return Ok(());
        }

        let runtime = Arc::clone(&self.runtime);
        let prepared = Arc::clone(&self.prepared);
        let capabilities = Self::inject_workspace_reader(&self.capabilities, &self.workspace_store);
        let timeout =
            Self::effective_callback_timeout(&self.runtime, &self.prepared, &self.capabilities);
        let channel_name = self.name.clone();
        let credentials = self.get_credentials().await;
        let pairing_store = self.pairing_store.clone();
        let workspace_store = self.workspace_store.clone();

        // Prepare response data
        let message_id_str = message_id.to_string();
        let content = content.to_string();
        let thread_id = thread_id.map(|s| s.to_string());
        let metadata_json = metadata_json.to_string();

        // Execute in blocking task with timeout
        tracing::info!(channel = %channel_name, "Starting on_respond WASM execution");

        let result = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                tracing::info!("Creating WASM store for on_respond");
                let mut store = Self::create_store(
                    &runtime,
                    &prepared,
                    &capabilities,
                    credentials,
                    pairing_store,
                    workspace_store.clone(),
                )?;

                tracing::info!("Instantiating WASM component for on_respond");
                let instance = Self::instantiate_component(&runtime, &prepared, &mut store)?;

                // Build the WIT response type
                let wit_response = wit_channel::AgentResponse {
                    message_id: message_id_str,
                    content: content.clone(),
                    thread_id,
                    metadata_json,
                };

                tracing::info!(content_bytes = content.len(), "Calling WASM on_respond");

                // Call on_respond using the generated typed interface
                let channel_iface = instance.near_agent_channel();
                let wasm_result = channel_iface
                    .call_on_respond(&mut store, &wit_response)
                    .map_err(|e| {
                        tracing::error!("WASM on_respond call failed");
                        Self::map_wasm_error(e, &prepared.name, prepared.limits.fuel)
                    })?;

                tracing::info!(success = wasm_result.is_ok(), "WASM on_respond returned");

                // Check for WASM-level errors
                if let Err(ref err_msg) = wasm_result {
                    tracing::error!(
                        error_bytes = err_msg.len(),
                        "WASM on_respond returned error"
                    );
                    return Err(WasmChannelError::CallbackFailed {
                        name: prepared.name.clone(),
                        reason: err_msg.clone(),
                    });
                }

                let mut host_state =
                    Self::extract_host_state(&mut store, &prepared.name, &capabilities);
                // Commit pending workspace writes to the persistent store
                // so state mutations from on_respond survive restarts.
                let pending_writes = host_state.take_pending_writes();
                workspace_store
                    .commit_writes(&pending_writes)
                    .map_err(|reason| WasmChannelError::CallbackFailed {
                        name: prepared.name.clone(),
                        reason: format!("failed to persist channel workspace: {reason}"),
                    })?;
                tracing::info!("on_respond WASM execution completed successfully");
                Ok(((), host_state))
            })
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "spawn_blocking panicked");
                WasmChannelError::ExecutionPanicked {
                    name: channel_name.clone(),
                    reason: e.to_string(),
                }
            })?
        })
        .await;

        let channel_name = self.name.clone();
        match result {
            Ok(Ok(((), _host_state))) => {
                tracing::info!(
                    channel = %channel_name,
                    message_id = %message_id,
                    "WASM channel on_respond completed successfully"
                );
                Ok(())
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(WasmChannelError::Timeout {
                name: channel_name,
                callback: "on_respond".to_string(),
            }),
        }
    }

    /// Execute the on_status callback.
    ///
    /// Called to notify the WASM channel of agent status changes (e.g., typing).
    pub async fn call_on_status(
        &self,
        status: &StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), WasmChannelError> {
        // If no WASM bytes, do nothing (for testing)
        if self.prepared.component().is_none() {
            return Ok(());
        }

        let runtime = Arc::clone(&self.runtime);
        let prepared = Arc::clone(&self.prepared);
        let capabilities = Self::inject_workspace_reader(&self.capabilities, &self.workspace_store);
        let timeout =
            Self::effective_callback_timeout(&self.runtime, &self.prepared, &self.capabilities);
        let channel_name = self.name.clone();
        let credentials = self.get_credentials().await;
        let pairing_store = self.pairing_store.clone();
        let workspace_store = self.workspace_store.clone();

        let wit_update = status_to_wit(status, metadata);

        let result = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                let mut store = Self::create_store(
                    &runtime,
                    &prepared,
                    &capabilities,
                    credentials,
                    pairing_store,
                    workspace_store.clone(),
                )?;
                let instance = Self::instantiate_component(&runtime, &prepared, &mut store)?;

                let channel_iface = instance.near_agent_channel();
                channel_iface
                    .call_on_status(&mut store, &wit_update)
                    .map_err(|e| Self::map_wasm_error(e, &prepared.name, prepared.limits.fuel))?;

                let mut host_state =
                    Self::extract_host_state(&mut store, &prepared.name, &capabilities);
                // Commit pending workspace writes to the persistent store
                // so state mutations from on_status survive restarts.
                let pending_writes = host_state.take_pending_writes();
                workspace_store
                    .commit_writes(&pending_writes)
                    .map_err(|reason| WasmChannelError::CallbackFailed {
                        name: prepared.name.clone(),
                        reason: format!("failed to persist channel workspace: {reason}"),
                    })?;

                Ok(())
            })
            .await
            .map_err(|e| WasmChannelError::ExecutionPanicked {
                name: channel_name.clone(),
                reason: e.to_string(),
            })?
        })
        .await;

        match result {
            Ok(Ok(())) => {
                tracing::debug!(
                    channel = %self.name,
                    "WASM channel on_status completed"
                );
                Ok(())
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(WasmChannelError::Timeout {
                name: self.name.clone(),
                callback: "on_status".to_string(),
            }),
        }
    }

    /// Execute a single on_status callback with a fresh WASM instance.
    ///
    /// Static method for use by the background typing repeat task (which
    /// doesn't have access to `&self`).
    #[allow(clippy::too_many_arguments)]
    async fn execute_status(
        channel_name: &str,
        runtime: &Arc<WasmChannelRuntime>,
        prepared: &Arc<PreparedChannelModule>,
        capabilities: &ChannelCapabilities,
        credentials: &RwLock<HashMap<String, String>>,
        workspace_store: &Arc<ChannelWorkspaceStore>,
        pairing_store: Arc<crate::pairing::PairingStore>,
        timeout: Duration,
        wit_update: wit_channel::StatusUpdate,
    ) -> Result<(), WasmChannelError> {
        if prepared.component().is_none() {
            return Ok(());
        }

        let runtime = Arc::clone(runtime);
        let prepared = Arc::clone(prepared);
        let capabilities = Self::inject_workspace_reader(capabilities, workspace_store);
        let credentials_snapshot = credentials.read().await.clone();
        let channel_name_owned = channel_name.to_string();
        let workspace_store = Arc::clone(workspace_store);

        let result = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                let mut store = Self::create_store(
                    &runtime,
                    &prepared,
                    &capabilities,
                    credentials_snapshot,
                    pairing_store,
                    workspace_store.clone(),
                )?;
                let instance = Self::instantiate_component(&runtime, &prepared, &mut store)?;

                let channel_iface = instance.near_agent_channel();
                channel_iface
                    .call_on_status(&mut store, &wit_update)
                    .map_err(|e| Self::map_wasm_error(e, &prepared.name, prepared.limits.fuel))?;

                let mut host_state =
                    Self::extract_host_state(&mut store, &prepared.name, &capabilities);
                // Commit pending workspace writes to the persistent store for
                // background typing/status callbacks.
                let pending_writes = host_state.take_pending_writes();
                workspace_store
                    .commit_writes(&pending_writes)
                    .map_err(|reason| WasmChannelError::CallbackFailed {
                        name: prepared.name.clone(),
                        reason: format!("failed to persist channel workspace: {reason}"),
                    })?;

                Ok(())
            })
            .await
            .map_err(|e| WasmChannelError::ExecutionPanicked {
                name: channel_name_owned.clone(),
                reason: e.to_string(),
            })?
        })
        .await;

        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(WasmChannelError::Timeout {
                name: channel_name.to_string(),
                callback: "on_status".to_string(),
            }),
        }
    }

    /// Cancel the background typing indicator task if running.
    pub(super) async fn cancel_typing_task(&self) {
        if let Some(handle) = self.typing_task.write().await.take() {
            handle.abort();
            let _ = handle.await;
        }
    }

    /// Handle a status update, managing the typing repeat timer.
    ///
    /// On Thinking: fires on_status once, then spawns a background task
    /// that repeats the call every 4 seconds (Telegram's typing indicator
    /// expires after ~5s).
    ///
    /// On terminal or user-action-required states: cancels the repeat task,
    /// then fires on_status once.
    ///
    /// On intermediate progress states (tool/auth/job/status updates), keeps
    /// the typing repeater running and fires on_status once.
    /// On StreamChunk: no-op (too noisy).
    pub(super) async fn handle_status_update(
        &self,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        fn is_terminal_text_status(msg: &str) -> bool {
            let trimmed = msg.trim();
            trimmed.eq_ignore_ascii_case("done")
                || trimmed.eq_ignore_ascii_case("interrupted")
                || trimmed.eq_ignore_ascii_case("awaiting approval")
                || trimmed.eq_ignore_ascii_case("rejected")
        }

        match &status {
            StatusUpdate::Thinking(_) => {
                // Cancel any existing typing task
                self.cancel_typing_task().await;

                tracing::debug!(
                    channel = %self.name,
                    metadata_bytes = metadata.to_string().len(),
                    has_thread_id = metadata.get("message_thread_id").is_some(),
                    "Handling thinking status metadata"
                );

                // Fire once immediately
                if let Err(e) = self.call_on_status(&status, metadata).await {
                    tracing::debug!(
                        channel = %self.name,
                        error = %e,
                        "on_status(Thinking) failed (best-effort)"
                    );
                }

                // Spawn background repeater
                let channel_name = self.name.clone();
                let runtime = Arc::clone(&self.runtime);
                let prepared = Arc::clone(&self.prepared);
                let capabilities = self.capabilities.clone();
                let credentials = self.credentials.clone();
                let workspace_store = self.workspace_store.clone();
                let pairing_store = self.pairing_store.clone();
                let callback_timeout = Self::effective_callback_timeout(
                    &self.runtime,
                    &self.prepared,
                    &self.capabilities,
                );
                let wit_update = status_to_wit(&status, metadata);

                let handle = tokio::spawn(async move {
                    let mut interval = tokio::time::interval(Duration::from_secs(4));
                    // Skip the first tick (we already fired above)
                    interval.tick().await;

                    loop {
                        interval.tick().await;

                        let wit_update_clone = clone_wit_status_update(&wit_update);

                        if let Err(e) = Self::execute_status(
                            &channel_name,
                            &runtime,
                            &prepared,
                            &capabilities,
                            &credentials,
                            &workspace_store,
                            pairing_store.clone(),
                            callback_timeout,
                            wit_update_clone,
                        )
                        .await
                        {
                            tracing::debug!(
                                channel = %channel_name,
                                error = %e,
                                "Typing repeat on_status failed (best-effort)"
                            );
                        }
                    }
                });

                *self.typing_task.write().await = Some(handle);
            }
            StatusUpdate::StreamChunk(_) => {
                // No-op, too noisy
            }
            StatusUpdate::ApprovalNeeded {
                tool_name,
                description,
                parameters,
                ..
            } => {
                // WASM channels (Telegram, Slack, etc.) cannot render
                // interactive approval overlays.  Send the approval prompt
                // as an actual message so the user can reply yes/no.
                self.cancel_typing_task().await;

                let params_preview = parameters
                    .as_object()
                    .map(|obj| {
                        obj.iter()
                            .map(|(k, v)| {
                                let val = match v {
                                    serde_json::Value::String(s) => {
                                        if s.chars().count() > 80 {
                                            let truncated: String = s.chars().take(77).collect();
                                            format!("\"{}...\"", truncated)
                                        } else {
                                            format!("\"{}\"", s)
                                        }
                                    }
                                    other => {
                                        let s = other.to_string();
                                        if s.chars().count() > 80 {
                                            let truncated: String = s.chars().take(77).collect();
                                            format!("{}...", truncated)
                                        } else {
                                            s
                                        }
                                    }
                                };
                                format!("  {}: {}", k, val)
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();

                let prompt = format!(
                    "Approval needed: {tool_name}\n\
                     {description}\n\
                     \n\
                     Parameters:\n\
                     {params_preview}\n\
                     \n\
                     Reply \"yes\" to approve, \"no\" to deny, or \"always\" to auto-approve."
                );

                let metadata_json = serde_json::to_string(metadata).unwrap_or_default();
                if let Err(e) = self
                    .call_on_respond(uuid::Uuid::new_v4(), &prompt, None, &metadata_json)
                    .await
                {
                    tracing::warn!(
                        channel = %self.name,
                        error = %e,
                        "Failed to send approval prompt via on_respond, falling back to on_status"
                    );
                    // Fall back to status update (typing indicator)
                    let _ = self.call_on_status(&status, metadata).await;
                }
            }
            StatusUpdate::AuthRequired { .. } => {
                // Waiting on user action: stop typing and fire once.
                self.cancel_typing_task().await;

                if let Err(e) = self.call_on_status(&status, metadata).await {
                    tracing::debug!(
                        channel = %self.name,
                        error = %e,
                        "on_status failed (best-effort)"
                    );
                }
            }
            StatusUpdate::Status(msg) if is_terminal_text_status(msg) => {
                // Waiting on user or terminal states: stop typing and fire once.
                self.cancel_typing_task().await;

                if let Err(e) = self.call_on_status(&status, metadata).await {
                    tracing::debug!(
                        channel = %self.name,
                        error = %e,
                        "on_status failed (best-effort)"
                    );
                }
            }
            StatusUpdate::ToolStarted { name, .. } => {
                // Accumulate in debug mode; suppress entirely in standard mode.
                let is_debug = self.debug_mode.read().map(|g| *g).unwrap_or(false);
                if is_debug {
                    self.pending_tool_events
                        .write()
                        .await
                        .push(ToolEventEntry::Started { name: name.clone() });
                }
            }
            StatusUpdate::ToolCompleted { name, success, .. } => {
                let is_debug = self.debug_mode.read().map(|g| *g).unwrap_or(false);
                if is_debug {
                    self.pending_tool_events
                        .write()
                        .await
                        .push(ToolEventEntry::Completed {
                            name: name.clone(),
                            success: *success,
                        });
                }
            }
            StatusUpdate::ToolResult { preview, .. } => {
                let is_debug = self.debug_mode.read().map(|g| *g).unwrap_or(false);
                if is_debug {
                    self.pending_tool_events
                        .write()
                        .await
                        .push(ToolEventEntry::Result {
                            preview: preview.clone(),
                        });
                }
            }
            // Sub-agent lifecycle: debug-only (noisy orchestration detail).
            StatusUpdate::SubagentSpawned { .. }
            | StatusUpdate::SubagentProgress { .. }
            | StatusUpdate::SubagentCompleted { .. } => {
                let is_debug = self.debug_mode.read().map(|g| *g).unwrap_or(false);
                if is_debug {
                    let _ = self.call_on_status(&status, metadata).await;
                } else {
                    tracing::trace!(
                        channel = %self.name,
                        "Suppressed subagent status (enable /debug to show)"
                    );
                }
            }
            // Canvas actions: debug-only (UI panels have no chat equivalent).
            StatusUpdate::CanvasAction(_) => {
                let is_debug = self.debug_mode.read().map(|g| *g).unwrap_or(false);
                if is_debug {
                    let _ = self.call_on_status(&status, metadata).await;
                }
            }
            // Lifecycle markers are internal bookkeeping, never user-facing.
            StatusUpdate::LifecycleStart { .. } | StatusUpdate::LifecycleEnd { .. } => {}
            _ => {
                // Other intermediate progress status: keep any existing typing task alive.
                if let Err(e) = self.call_on_status(&status, metadata).await {
                    tracing::debug!(
                        channel = %self.name,
                        error = %e,
                        "on_status failed (best-effort)"
                    );
                }
            }
        }

        Ok(())
    }

    /// Process emitted messages from a callback.
    pub(super) async fn process_emitted_messages(
        &self,
        messages: Vec<EmittedMessage>,
    ) -> Result<(), WasmChannelError> {
        tracing::info!(
            channel = %self.name,
            message_count = messages.len(),
            "Processing emitted messages from WASM callback"
        );

        if messages.is_empty() {
            tracing::debug!(channel = %self.name, "No messages emitted");
            return Ok(());
        }

        let tx_guard = self.message_tx.read().await;
        let Some(tx) = tx_guard.as_ref() else {
            tracing::error!(
                channel = %self.name,
                count = messages.len(),
                "Messages emitted but no sender available - channel may not be started!"
            );
            return Ok(());
        };

        let mut rate_limiter = self.rate_limiter.write().await;

        for emitted in messages {
            // Check rate limit
            if !rate_limiter.check_and_record() {
                tracing::warn!(
                    channel = %self.name,
                    "Message emission rate limited"
                );
                return Err(WasmChannelError::EmitRateLimited {
                    name: self.name.clone(),
                });
            }

            let msg = emitted_message_to_incoming_message(&self.name, emitted);

            // Send to stream
            tracing::info!(
                channel = %self.name,
                user_id = %msg.user_id,
                content_len = msg.content.len(),
                "Sending emitted message to agent"
            );

            if tx.send(msg).await.is_err() {
                tracing::error!(
                    channel = %self.name,
                    "Failed to send emitted message, channel closed"
                );
                break;
            }

            tracing::info!(
                channel = %self.name,
                "Message successfully sent to agent queue"
            );
        }

        Ok(())
    }

    /// Start the polling loop if configured.
    ///
    /// Since we can't hold `Arc<Self>` from `&self`, we pass all the components
    /// needed for polling to a spawned task. Each poll tick creates a fresh WASM
    /// instance (matching our "fresh instance per callback" pattern).
    pub(super) fn start_polling(
        &self,
        interval: Duration,
        shutdown_rx: oneshot::Receiver<()>,
    ) -> tokio::task::JoinHandle<()> {
        let channel_name = self.name.clone();
        let runtime = Arc::clone(&self.runtime);
        let prepared = Arc::clone(&self.prepared);
        let capabilities = self.capabilities.clone();
        let message_tx = self.message_tx.clone();
        let rate_limiter = self.rate_limiter.clone();
        let credentials = self.credentials.clone();
        let pairing_store = self.pairing_store.clone();
        let callback_timeout =
            Self::effective_callback_timeout(&self.runtime, &self.prepared, &self.capabilities);
        let workspace_store = self.workspace_store.clone();

        tokio::spawn(async move {
            let mut interval_timer = tokio::time::interval(interval);
            let mut shutdown = std::pin::pin!(shutdown_rx);

            loop {
                tokio::select! {
                    _ = interval_timer.tick() => {
                        tracing::debug!(
                            channel = %channel_name,
                            "Polling tick - calling on_poll"
                        );

                        // Execute on_poll with fresh WASM instance
                        let result = Self::execute_poll(
                            &channel_name,
                            &runtime,
                            &prepared,
                            &capabilities,
                            &credentials,
                            pairing_store.clone(),
                            callback_timeout,
                            &workspace_store,
                        ).await;

                        match result {
                            Ok(emitted_messages) => {
                                // Process any emitted messages
                                if !emitted_messages.is_empty()
                                    && let Err(e) = Self::dispatch_emitted_messages(
                                        &channel_name,
                                        emitted_messages,
                                        &message_tx,
                                        &rate_limiter,
                                    ).await {
                                        tracing::warn!(
                                            channel = %channel_name,
                                            error = %e,
                                            "Failed to dispatch emitted messages from poll"
                                        );
                                    }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    channel = %channel_name,
                                    error = %e,
                                    "Polling callback failed"
                                );
                            }
                        }
                    }
                    _ = &mut shutdown => {
                        tracing::info!(
                            channel = %channel_name,
                            "Polling stopped"
                        );
                        break;
                    }
                }
            }
        })
    }

    /// Execute a single poll callback with a fresh WASM instance.
    ///
    /// Returns any emitted messages from the callback. Pending workspace writes
    /// are committed to the shared `ChannelWorkspaceStore` so state persists
    /// across poll ticks (e.g., Telegram polling offset).
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn execute_poll(
        channel_name: &str,
        runtime: &Arc<WasmChannelRuntime>,
        prepared: &Arc<PreparedChannelModule>,
        capabilities: &ChannelCapabilities,
        credentials: &RwLock<HashMap<String, String>>,
        pairing_store: Arc<crate::pairing::PairingStore>,
        timeout: Duration,
        workspace_store: &Arc<ChannelWorkspaceStore>,
    ) -> Result<Vec<EmittedMessage>, WasmChannelError> {
        // Skip if no WASM bytes (testing mode)
        if prepared.component().is_none() {
            tracing::debug!(
                channel = %channel_name,
                "WASM channel on_poll called (no WASM module)"
            );
            return Ok(Vec::new());
        }

        let runtime = Arc::clone(runtime);
        let prepared = Arc::clone(prepared);
        let capabilities = Self::inject_workspace_reader(capabilities, workspace_store);
        let credentials_snapshot = credentials.read().await.clone();
        let channel_name_owned = channel_name.to_string();
        let workspace_store = Arc::clone(workspace_store);

        // Execute in blocking task with timeout
        let result = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                let mut store = Self::create_store(
                    &runtime,
                    &prepared,
                    &capabilities,
                    credentials_snapshot,
                    pairing_store,
                    workspace_store.clone(),
                )?;
                let instance = Self::instantiate_component(&runtime, &prepared, &mut store)?;

                // Call on_poll using the generated typed interface
                let channel_iface = instance.near_agent_channel();
                channel_iface
                    .call_on_poll(&mut store)
                    .map_err(|e| Self::map_wasm_error(e, &prepared.name, prepared.limits.fuel))?;

                let mut host_state =
                    Self::extract_host_state(&mut store, &prepared.name, &capabilities);

                // Commit pending workspace writes to the persistent store
                let pending_writes = host_state.take_pending_writes();
                workspace_store
                    .commit_writes(&pending_writes)
                    .map_err(|reason| WasmChannelError::CallbackFailed {
                        name: prepared.name.clone(),
                        reason: format!("failed to persist channel workspace: {reason}"),
                    })?;

                Ok(host_state)
            })
            .await
            .map_err(|e| WasmChannelError::ExecutionPanicked {
                name: channel_name_owned.clone(),
                reason: e.to_string(),
            })?
        })
        .await;

        match result {
            Ok(Ok(mut host_state)) => {
                let emitted = host_state.take_emitted_messages();
                tracing::debug!(
                    channel = %channel_name,
                    emitted_count = emitted.len(),
                    "WASM channel on_poll completed"
                );
                Ok(emitted)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(WasmChannelError::Timeout {
                name: channel_name.to_string(),
                callback: "on_poll".to_string(),
            }),
        }
    }

    /// Dispatch emitted messages to the message channel.
    ///
    /// This is a static helper used by the polling loop since it doesn't have
    /// access to `&self`.
    pub(super) async fn dispatch_emitted_messages(
        channel_name: &str,
        messages: Vec<EmittedMessage>,
        message_tx: &RwLock<Option<mpsc::Sender<thinclaw_channels_core::IncomingMessage>>>,
        rate_limiter: &RwLock<ChannelEmitRateLimiter>,
    ) -> Result<(), WasmChannelError> {
        tracing::info!(
            channel = %channel_name,
            message_count = messages.len(),
            "Processing emitted messages from polling callback"
        );

        let tx_guard = message_tx.read().await;
        let Some(tx) = tx_guard.as_ref() else {
            tracing::error!(
                channel = %channel_name,
                count = messages.len(),
                "Messages emitted but no sender available - channel may not be started!"
            );
            return Ok(());
        };

        let mut limiter = rate_limiter.write().await;

        for emitted in messages {
            // Check rate limit
            if !limiter.check_and_record() {
                tracing::warn!(
                    channel = %channel_name,
                    "Message emission rate limited"
                );
                return Err(WasmChannelError::EmitRateLimited {
                    name: channel_name.to_string(),
                });
            }

            let msg = emitted_message_to_incoming_message(channel_name, emitted);

            // Send to stream
            tracing::info!(
                channel = %channel_name,
                user_id = %msg.user_id,
                content_len = msg.content.len(),
                "Sending polled message to agent"
            );

            if tx.send(msg).await.is_err() {
                tracing::error!(
                    channel = %channel_name,
                    "Failed to send polled message, channel closed"
                );
                break;
            }

            tracing::info!(
                channel = %channel_name,
                "Message successfully sent to agent queue"
            );
        }

        Ok(())
    }
}
