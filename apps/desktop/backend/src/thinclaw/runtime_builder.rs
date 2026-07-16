//! ThinClaw runtime builder — extracted from `runtime_bridge.rs`.
//!
//! Contains `build_inner()`, the ~950-line async function that:
//!   1. Configures bridge environment variables (IC-007)
//!   2. Resolves the LLM backend (local sidecar / MLX / cloud)
//!   3. Creates TauriChannel + ToolBridge
//!   4. Builds ThinClaw's AppComponents
//!   5. Wires sub-agent executor + SSE broadcast + background tasks
//!   6. Returns a fully assembled `ThinClawRuntimeInner`
//!
//! Separated for maintainability — the public API remains in `runtime_bridge.rs`.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, OnceLock};

use tokio::sync::Mutex as TokioMutex;
use tokio::sync::Mutex;

use thinclaw_core::agent::{Agent, AgentDeps, AgentRegistry, AgentRouter, SessionManager};
use thinclaw_core::app::{AppBuilder, AppBuilderFlags, PeriodicPersistencePlan};
use thinclaw_core::channels::web::log_layer::LogBroadcaster;
use thinclaw_core::channels::web::types::SseEvent;
use thinclaw_core::channels::web::GatewayChannel;
use thinclaw_core::channels::ChannelManager;
use thinclaw_core::extensions::clawhub::CatalogCache;
use thinclaw_core::extensions::manifest_validator::ManifestValidator;

use super::desktop_observer::DesktopObserver;
use super::runtime_bridge::ThinClawRuntimeInner;
use super::tauri_channel::TauriChannel;
use super::tool_bridge::TauriToolBridge;
use super::ui_types::UiEvent;

mod background_tasks;
mod environment;
mod event_forwarders;
mod sandbox;

#[cfg(feature = "docker-sandbox")]
use sandbox::build_desktop_container_job_manager;

const RUNTIME_DB_NAME: &str = "thinclaw-runtime.db";
const LEGACY_RUNTIME_DB_NAME: &str = "ironclaw.db";
const RUNTIME_TOML_NAME: &str = "thinclaw.toml";
const LEGACY_RUNTIME_TOML_NAME: &str = "ironclaw.toml";

#[derive(Debug, Clone, PartialEq, Eq)]
struct DesktopSendRoute {
    session_key: String,
}

fn is_desktop_send_platform(platform: &str) -> bool {
    matches!(
        platform.trim().to_ascii_lowercase().as_str(),
        "tauri" | "desktop" | "thinclaw_desktop" | "local" | "app" | "web"
    )
}

fn is_session_like_recipient(recipient: &str) -> bool {
    let trimmed = recipient.trim();
    trimmed.starts_with("agent:") || trimmed.starts_with("session:")
}

fn desktop_send_route(
    platform: &str,
    recipient: &str,
    thread_id: Option<&str>,
    attachment_count: usize,
) -> Result<DesktopSendRoute, String> {
    if !is_desktop_send_platform(platform) {
        return Err(format!(
            "Desktop local send_message supports only the Tauri/Desktop event surface; \
             platform '{}' must be routed by a configured channel.",
            platform
        ));
    }

    if attachment_count > 0 {
        return Err(
            "Desktop local send_message does not support attachments yet; use a configured channel."
                .to_string(),
        );
    }

    let session_key = thread_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| is_session_like_recipient(recipient).then(|| recipient.trim().to_string()))
        .unwrap_or_else(|| "system".to_string());

    Ok(DesktopSendRoute { session_key })
}

fn resolved_workspace_root() -> &'static std::sync::RwLock<Option<String>> {
    static ROOT: OnceLock<std::sync::RwLock<Option<String>>> = OnceLock::new();
    ROOT.get_or_init(|| std::sync::RwLock::new(None))
}

fn set_resolved_workspace_root(root: &std::path::Path) {
    let value = root.to_string_lossy().to_string();
    match resolved_workspace_root().write() {
        Ok(mut guard) => *guard = Some(value),
        Err(poisoned) => *poisoned.into_inner() = Some(value),
    }
}

pub(crate) fn get_resolved_workspace_root() -> Option<String> {
    match resolved_workspace_root().read() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

pub(crate) fn runtime_db_path(state_dir: &std::path::Path) -> std::path::PathBuf {
    state_dir.join(RUNTIME_DB_NAME)
}

pub(crate) fn runtime_toml_path(state_dir: &std::path::Path) -> std::path::PathBuf {
    state_dir.join(RUNTIME_TOML_NAME)
}

fn migrate_legacy_runtime_file(state_dir: &std::path::Path, legacy_name: &str, current_name: &str) {
    let legacy_path = state_dir.join(legacy_name);
    let current_path = state_dir.join(current_name);
    if current_path.exists() || !legacy_path.exists() {
        return;
    }

    match std::fs::rename(&legacy_path, &current_path) {
        Ok(()) => tracing::info!(
            "[thinclaw-runtime] Migrated legacy runtime file {} to {}",
            legacy_name,
            current_name
        ),
        Err(error) => tracing::warn!(
            "[thinclaw-runtime] Failed to migrate legacy runtime file {} to {}: {}",
            legacy_name,
            current_name,
            error
        ),
    }
}

fn migrate_legacy_runtime_files(state_dir: &std::path::Path) {
    migrate_legacy_runtime_file(state_dir, LEGACY_RUNTIME_DB_NAME, RUNTIME_DB_NAME);
    migrate_legacy_runtime_file(
        state_dir,
        &format!("{LEGACY_RUNTIME_DB_NAME}-wal"),
        &format!("{RUNTIME_DB_NAME}-wal"),
    );
    migrate_legacy_runtime_file(
        state_dir,
        &format!("{LEGACY_RUNTIME_DB_NAME}-shm"),
        &format!("{RUNTIME_DB_NAME}-shm"),
    );
    migrate_legacy_runtime_file(state_dir, LEGACY_RUNTIME_TOML_NAME, RUNTIME_TOML_NAME);
}

/// Build a fully-configured `ThinClawRuntimeInner` from scratch.
///
/// This is the heavyweight initialization path — called by `ThinClawRuntimeState::start()`.
/// It configures bridge vars, resolves the LLM backend, creates channels,
/// builds engine components, and wires up all background tasks.
pub(crate) async fn build_inner(
    app_handle: tauri::AppHandle<tauri::Wry>,
    state_dir: std::path::PathBuf,
    secrets_store: Option<Arc<dyn thinclaw_core::secrets::SecretsStore + Send + Sync>>,
) -> Result<ThinClawRuntimeInner, anyhow::Error> {
    migrate_legacy_runtime_files(&state_dir);

    environment::configure(&app_handle, &state_dir).await?;

    // ── 2. Load config ──────────────────────────────────────────────
    let toml_path = runtime_toml_path(&state_dir);
    let toml_path_ref = if toml_path.exists() {
        Some(toml_path.as_path())
    } else {
        None
    };

    let config = match thinclaw_core::Config::from_env_with_toml(toml_path_ref).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Failed to load ThinClaw config, using env-only: {}", e);
            thinclaw_core::Config::from_env().await?
        }
    };

    // ── 3. Create TauriChannel + ToolBridge ────────────────────────────
    let (tauri_channel, inject_tx, active_sessions) = TauriChannel::new(app_handle.clone());
    let tool_bridge = TauriToolBridge::new(app_handle.clone());

    // ── 4. Build engine components ──────────────────────────────────
    // Reuse the global LogBroadcaster that was wired to the tracing
    // subscriber in lib.rs::run(). This ensures all tracing::info!()/debug!()
    // calls flow into the same broadcaster that the UI Logs tab reads.
    let log_broadcaster = crate::GLOBAL_LOG_BROADCASTER
        .get()
        .cloned()
        .unwrap_or_else(|| {
            tracing::warn!(
                "[thinclaw-runtime] GLOBAL_LOG_BROADCASTER not set — creating standalone broadcaster. \
                 Tracing events will NOT reach the UI Logs tab."
            );
            Arc::new(LogBroadcaster::new())
        });

    let runtime_toml_path = runtime_toml_path(&state_dir);
    let toml_path_opt = if runtime_toml_path.exists() {
        Some(runtime_toml_path)
    } else {
        None
    };

    let mut builder = AppBuilder::new(
        config,
        AppBuilderFlags::default(),
        toml_path_opt,
        log_broadcaster.clone(),
    );

    // The embedded libSQL runtime receives the exact store already opened by
    // Desktop, so the Agent Cockpit and Direct Workbench cannot diverge.
    #[cfg(feature = "runtime-libsql")]
    {
        use tauri::Manager;
        let history = app_handle.state::<crate::history::SharedHistoryStore>();
        builder = builder.with_database(history.runtime_store());
    }

    if let Some(store) = secrets_store {
        builder = builder.with_secrets_store(store);
    }

    // Wire TauriToolBridge into the ThinClaw runtime — enables hardware
    // sensor tools (camera, mic, screen) with 3-tier user approval.
    builder = builder.with_tool_bridge(tool_bridge.clone());

    // ── 4b. Translate ThinClaw Desktop's cloud intelligence config into ThinClaw
    //        ProvidersSettings for multi-provider failover + smart routing.
    {
        use tauri::Manager;
        let thinclaw_mgr = app_handle.state::<super::ThinClawManager>();
        let oc_config = thinclaw_mgr.get_config().await;

        if let Some(ref cfg) = oc_config {
            // Fallback is generated from enabled providers by FailoverProvider.
            let providers = thinclaw_core::settings::ProvidersSettings {
                enabled: cfg.enabled_cloud_providers.clone(),
                primary: cfg.selected_cloud_brain.clone(),
                primary_model: cfg.selected_cloud_model.clone(),
                allowed_models: cfg.enabled_cloud_models.clone(),
                fallback_chain: Vec::new(),
                ..Default::default()
            };

            if !providers.enabled.is_empty() {
                tracing::info!(
                    "[thinclaw-runtime] Cloud intelligence config translated: {} provider(s) enabled, \
                     primary={:?}, model={:?}",
                    providers.enabled.len(),
                    providers.primary,
                    providers.primary_model,
                );
                builder = builder.with_providers_settings(providers);
            }
        }
    }

    let mut components = builder.build_all().await?;

    // Preserve the operator-selected core backend (log/Prometheus/noop) while
    // adding Desktop's typed event sink and bounded local crash reports.
    let crash_reporter = {
        use tauri::Manager as _;
        app_handle
            .state::<super::desktop_observer::DesktopCrashReporter>()
            .inner()
            .clone()
    };
    let desktop_observer = Arc::new(DesktopObserver::new(
        Arc::clone(&components.observer),
        app_handle.clone(),
        crash_reporter,
    ));
    // AppBuilder emitted the core startup record before the Desktop adapter
    // existed. Mirror it to the Desktop sink without double-counting it in the
    // selected core backend.
    desktop_observer.emit_desktop_event(&thinclaw_core::observability::ObserverEvent::AgentStart {
        provider: components.config.llm.backend.to_string(),
        model: components.llm.model_name().to_string(),
    });
    components.observer = desktop_observer;

    // ── 5. Create channel manager and register Tauri + gateway channels ─
    let channel_manager = Arc::new(ChannelManager::new());
    channel_manager.add(Box::new(tauri_channel)).await;

    // Share one session manager between the embedded agent and HTTP gateway.
    // Without this, remote clients would see a different in-memory thread view
    // from the Desktop surface even though both use the same database.
    let session_manager = Arc::new(SessionManager::new().with_hooks(components.hooks.clone()));
    let mut gateway_state = None;
    let mut repo_project_supervisor_slot = None;

    if let Some(gateway_config) = components.config.channels.gateway.clone() {
        let mut gateway = GatewayChannel::new(gateway_config)
            .with_llm_provider(Arc::clone(&components.llm))
            .with_llm_runtime(Arc::clone(&components.llm_runtime))
            .with_session_manager(Arc::clone(&session_manager))
            .with_log_broadcaster(Arc::clone(&log_broadcaster))
            .with_tool_registry(Arc::clone(&components.tools))
            .with_context_manager(Arc::clone(&components.context_manager))
            .with_registry_entries(components.catalog_entries.clone())
            .with_cost_guard(Arc::clone(&components.cost_guard))
            .with_cost_tracker(Arc::clone(&components.cost_tracker))
            .with_response_cache(Arc::clone(&components.response_cache))
            .with_hooks(Arc::clone(&components.hooks))
            .with_channel_manager(Arc::clone(&channel_manager));
        if let Some(workspace) = components.workspace.as_ref() {
            gateway = gateway.with_workspace(Arc::clone(workspace));
        }
        if let Some(extension_manager) = components.extension_manager.as_ref() {
            gateway = gateway.with_extension_manager(Arc::clone(extension_manager));
        }
        if let Some(store) = components.db.as_ref() {
            gateway = gateway.with_store(Arc::clone(store));
        }
        if let Some(skill_registry) = components.skill_registry.as_ref() {
            gateway = gateway.with_skill_registry(Arc::clone(skill_registry));
        }
        if let Some(skill_catalog) = components.skill_catalog.as_ref() {
            gateway = gateway.with_skill_catalog(Arc::clone(skill_catalog));
        }
        if let Some(skill_remote_hub) = components.skill_remote_hub.as_ref() {
            gateway = gateway.with_skill_remote_hub(skill_remote_hub.clone());
        }
        if let Some(skill_quarantine) = components.skill_quarantine.as_ref() {
            gateway = gateway.with_skill_quarantine(Arc::clone(skill_quarantine));
        }
        if let Some(metrics_registry) = components.metrics_registry.as_ref() {
            gateway = gateway.with_metrics_registry(Arc::clone(metrics_registry));
        }
        if let Some(secrets_store) = components.secrets_store.as_ref() {
            gateway = gateway.with_secrets_store(Arc::clone(secrets_store));
        }
        repo_project_supervisor_slot = Some(gateway.repo_project_supervisor_cell());
        gateway_state = Some(Arc::clone(gateway.state()));
        channel_manager.add(Box::new(gateway)).await;
    }

    {
        let send_handle = app_handle.clone();
        components.tools.register_send_message_tool(Some(Arc::new(
            move |platform: String,
                  recipient: String,
                  text: String,
                  thread_id: Option<String>,
                  attachments: Vec<thinclaw_core::media::MediaContent>| {
                let send_handle = send_handle.clone();
                Box::pin(async move {
                    let route = desktop_send_route(
                        &platform,
                        &recipient,
                        thread_id.as_deref(),
                        attachments.len(),
                    )?;
                    let message_id = uuid::Uuid::new_v4().to_string();
                    let event = UiEvent::AssistantFinal {
                        session_key: route.session_key,
                        run_id: None,
                        message_id: message_id.clone(),
                        text,
                        usage: None,
                    };

                    use tauri::Emitter as _;
                    send_handle
                        .emit("thinclaw-event", &event)
                        .map_err(|error| format!("Failed to emit desktop message: {}", error))?;
                    Ok(message_id)
                })
            },
        )));
        tracing::info!(
            "[thinclaw-runtime] send_message tool registered for local Tauri/Desktop delivery"
        );
    }

    // ── 6. Create SSE broadcast channel + agent ─────────────────────
    // Channel must be created BEFORE AgentDeps so we can wire sse_sender in.
    // The forwarder below subscribes and forwards RoutineLifecycle events
    // as 'thinclaw-event' Tauri emissions to the frontend.
    let (sse_tx, _sse_rx_seed) = tokio::sync::broadcast::channel::<SseEvent>(64);
    {
        let status_tx = sse_tx.clone();
        channel_manager
            .set_status_change_sink(move |event| {
                let _ = status_tx.send(SseEvent::ChannelStatusChange {
                    channel: event.channel,
                    status: event.status,
                    message: event.message,
                });
            })
            .await;
    }

    // ── 5b. Create sub-agent executor ───────────────────────────────
    // Shares the same LLM, safety layer, tool registry, and channel
    // manager as the main agent. This lets the agent use spawn_subagent
    // to delegate parallel work to isolated in-process agentic loops.
    //
    // The dispatcher in dispatcher.rs intercepts spawn_subagent tool
    // results (JSON action descriptors) and calls executor.spawn() here.
    // Without this wiring the tool silently returns "not initialized".
    let (subagent_executor, subagent_result_rx) =
        thinclaw_core::agent::subagent_executor::SubagentExecutor::new(
            components.llm.clone(),
            components.safety.clone(),
            components.tools.clone(),
            channel_manager.clone(),
            thinclaw_core::agent::subagent_executor::SubagentConfig {
                max_concurrent: 5,
                max_per_principal: components.config.agent.subagent_max_per_principal,
                default_timeout_secs: 300, // 5 minutes
                allow_nested: false,       // sub-agents cannot spawn sub-agents
                max_tool_iterations: 30,
                default_tool_profile: components.config.agent.subagent_tool_profile,
            },
        );
    let mut subagent_executor = subagent_executor;
    if let Some(ref db) = components.db {
        subagent_executor = subagent_executor.with_store(Arc::clone(db));
    }
    subagent_executor = subagent_executor.with_sse_tx(sse_tx.clone());
    subagent_executor = subagent_executor.with_cost_tracker(Arc::clone(&components.cost_tracker));
    // Same guard instance as the main loop: sub-agent spend is gated by the
    // operator's daily-budget/hourly-rate limits instead of bypassing them.
    subagent_executor = subagent_executor.with_cost_guard(Arc::clone(&components.cost_guard));
    if let Some(ref workspace) = components.workspace {
        subagent_executor = subagent_executor.with_workspace(Arc::clone(workspace));
    }
    if let Some(ref skill_registry) = components.skill_registry {
        subagent_executor = subagent_executor
            .with_skill_registry(Arc::clone(skill_registry), components.config.skills.clone());
    }
    let subagent_executor = Arc::new(subagent_executor);

    let model_override = thinclaw_core::tools::builtin::new_shared_model_override();
    components.tools.register_llm_tools(
        model_override.clone(),
        Arc::clone(&components.llm),
        components.cheap_llm.as_ref().map(Arc::clone),
    );
    components.llm_runtime.set_advisor_ready_callback({
        let tools = Arc::clone(&components.tools);
        move |advisor_ready| {
            if advisor_ready {
                tools.register_advisor_tool(true);
            } else if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let tools = Arc::clone(&tools);
                handle.spawn(async move {
                    tools.reconcile_advisor_tool_readiness(false).await;
                });
            }
        }
    });

    // Register sub-agent tools so the LLM can see and call them.
    // Without this, the dispatcher can handle results but the LLM
    // never has spawn_subagent/list_subagents/cancel_subagent in
    // its tool definitions — it literally cannot invoke them.
    components.tools.register_sync(Arc::new(
        thinclaw_core::tools::builtin::SpawnSubagentTool::new(),
    ));
    components.tools.register_sync(Arc::new(
        thinclaw_core::tools::builtin::ListSubagentsTool::new(subagent_executor.clone()),
    ));
    components.tools.register_sync(Arc::new(
        thinclaw_core::tools::builtin::CancelSubagentTool::new(subagent_executor.clone()),
    ));
    tracing::info!("[thinclaw-runtime] Sub-agent tools registered (spawn, list, cancel)");

    // Re-register MemoryDeleteTool with the SSE sender now that we have the channel.
    // build_all() registered it with None; we replace it here with the live sender.
    // register_sync() replaces existing entries by name, so no duplicates occur.
    if let Some(ref ws) = components.workspace {
        use thinclaw_core::tools::builtin::MemoryDeleteTool;
        let delete_tool = MemoryDeleteTool::new(ws.clone()).with_sse_sender(sse_tx.clone());
        components
            .tools
            .register_sync(std::sync::Arc::new(delete_tool));
        tracing::info!(
            "[thinclaw-runtime] MemoryDeleteTool re-registered with SSE sender (BOOTSTRAP.md delete detection enabled)"
        );
    }

    // Persistent multi-agent management parity with the root runtime.
    let shared_agent_router = Arc::new(AgentRouter::new());
    let agent_registry = {
        let registry = AgentRegistry::new(Arc::clone(&shared_agent_router), components.db.clone());
        if components.db.is_some() {
            match registry.load_from_db().await {
                Ok(count) if count > 0 => {
                    tracing::info!(
                        "[thinclaw-runtime] Loaded {} persisted agent workspace(s) into desktop router",
                        count
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        "[thinclaw-runtime] Failed to load persisted agent workspaces: {}",
                        error
                    );
                }
                _ => {}
            }
        }
        let registry = Arc::new(registry);
        components
            .tools
            .register_agent_management_tools(Arc::clone(&registry));
        registry
    };

    let mut auxiliary_tasks: Vec<tokio::task::JoinHandle<()>> = Vec::new();

    if let Some(ref db) = components.db {
        let persistence_plan = PeriodicPersistencePlan::cost_entries();
        let persist_db = Arc::clone(db);
        let persist_tracker = Arc::clone(&components.cost_tracker);
        auxiliary_tasks.push(tokio::spawn(async move {
            let mut interval = tokio::time::interval(persistence_plan.interval);
            interval.tick().await;
            let mut last_count: usize = 0;
            loop {
                interval.tick().await;
                let (snapshot, count) = {
                    let guard = persist_tracker.lock().await;
                    (guard.to_json(), guard.entry_count())
                };
                if count != last_count {
                    match persist_db
                        .set_setting("default", persistence_plan.setting_key, &snapshot)
                        .await
                    {
                        Ok(()) => {
                            tracing::debug!("[cost] Persisted {} cost entries to DB", count);
                            last_count = count;
                        }
                        Err(error) => {
                            tracing::warn!("[cost] Failed to persist cost entries: {}", error);
                        }
                    }
                }
            }
        }));
        tracing::info!("[thinclaw-runtime] Cost persistence background task started");
    }

    auxiliary_tasks.push(thinclaw_core::llm::pricing_sync::spawn_pricing_sync(
        components.db.as_ref().map(Arc::clone),
    ));
    tracing::info!("[thinclaw-runtime] Pricing sync background task started");

    // Local Docker sandbox: lets the repo project supervisor (and other sandbox
    // tools) spawn coding/worker containers on the desktop when the sandbox is
    // enabled in config. Actual container spawning is further gated at runtime.
    #[cfg(feature = "docker-sandbox")]
    let job_manager = build_desktop_container_job_manager(
        &components.config,
        components.llm.clone(),
        components.db.clone(),
        components.secrets_store.clone(),
    )
    .await;
    #[cfg(not(feature = "docker-sandbox"))]
    let job_manager: Option<Arc<thinclaw_core::sandbox_types::ContainerJobManager>> = None;

    let agent_deps = AgentDeps {
        observer: components.observer.clone(),
        store: components.db.clone(),
        llm: components.llm.clone(),
        cheap_llm: components.cheap_llm.clone(),
        safety: components.safety.clone(),
        tools: components.tools.clone(),
        desktop_autonomy_manager: components.desktop_autonomy_manager.clone(),
        workspace: components.workspace.clone(),
        extension_manager: components.extension_manager.clone(),
        skill_registry: components.skill_registry.clone(),
        skill_catalog: components.skill_catalog.clone(),
        skills_config: components.config.skills.clone(),
        hooks: components.hooks.clone(),
        cost_guard: components.cost_guard.clone(),
        cost_tracker: Some(components.cost_tracker.clone()),
        response_cache: Some(components.response_cache.clone()),
        llm_runtime: Some(components.llm_runtime.clone()),
        routing_policy: Some(components.routing_policy.clone()),
        sse_sender: Some(sse_tx.clone()), // ← wired into RoutineEngine + Dispatcher
        job_manager,
        secrets_store: components.secrets_store.clone(),
        repo_project_supervisor_slot,
        agent_router: Some(shared_agent_router),
        agent_registry: Some(agent_registry),
        canvas_store: Some(thinclaw_core::channels::canvas_gateway::CanvasStore::new(
            std::time::Duration::from_secs(30 * 60), // 30 minute TTL
        )),
        subagent_executor: Some(subagent_executor.clone()),
        model_override: Some(model_override),
        restart_requested: Arc::new(AtomicBool::new(false)),
        sandbox_children: None,
        runtime_ports: None,
    };

    let agent = Arc::new(Agent::new(
        components.config.agent.clone(),
        agent_deps,
        channel_manager,
        Some(components.config.heartbeat.clone()),
        Some(components.config.hygiene.clone()),
        Some(components.config.routines.clone()),
        Some(components.context_manager.clone()),
        Some(session_manager),
    ));
    if let Some(gateway_state) = gateway_state.as_ref() {
        *gateway_state.scheduler.write().await = Some(Arc::clone(agent.scheduler()));
    }

    // Tauri commands invoke the agent directly, so Desktop historically never
    // started its registered Channel streams. The HTTP gateway is an actual
    // inbound channel and must be started and consumed. Keep this small bridge
    // loop runtime-owned so shutdown aborts it with the rest of the embedded
    // auxiliary tasks.
    match agent.channels().start_all().await {
        Ok(mut messages) => {
            use futures::StreamExt as _;
            let channel_agent = Arc::clone(&agent);
            auxiliary_tasks.push(tokio::spawn(async move {
                while let Some(message) = messages.next().await {
                    channel_agent
                        .channels()
                        .record_received(&message.channel)
                        .await;
                    match channel_agent.handle_message_external(&message).await {
                        Ok(Some(response)) if !response.is_empty() => {
                            let outbound = thinclaw_core::hooks::HookEvent::Outbound {
                                user_id: message.user_id.clone(),
                                channel: message.channel.clone(),
                                content: response.clone(),
                                thread_id: message.thread_id.clone(),
                            };
                            let response = match channel_agent.hooks().run(&outbound).await {
                                Err(error) => {
                                    tracing::warn!(
                                        channel = %message.channel,
                                        %error,
                                        "Embedded gateway response blocked by outbound hook"
                                    );
                                    None
                                }
                                Ok(thinclaw_core::hooks::HookOutcome::Continue {
                                    modified: Some(content),
                                }) => Some(content),
                                _ => Some(response),
                            };
                            if let Some(response) = response {
                                if let Err(error) = channel_agent
                                    .channels()
                                    .respond(
                                        &message,
                                        thinclaw_core::channels::OutgoingResponse::text(response),
                                    )
                                    .await
                                {
                                    tracing::warn!(
                                        channel = %message.channel,
                                        %error,
                                        "Failed to deliver embedded gateway response"
                                    );
                                }
                            }
                        }
                        Ok(_) => {}
                        Err(error) => tracing::warn!(
                            channel = %message.channel,
                            %error,
                            "Embedded gateway message failed"
                        ),
                    }
                }
            }));
        }
        Err(error) => tracing::warn!(
            %error,
            "No embedded Desktop channels could be started"
        ),
    }

    agent.tools().register_job_tools(
        components.context_manager.clone(),
        None,
        components.db.clone(),
        Some(Arc::clone(agent.scheduler())),
        None,
        Some(inject_tx.clone()),
        None,
        None,
        components.secrets_store.clone(),
    );
    tracing::info!(
        "[thinclaw-runtime] Job tools registered with desktop scheduler-backed execution"
    );

    background_tasks::spawn_subagent_result_injector(&agent, subagent_result_rx);

    let (bg_handle, routine_engine) = background_tasks::start(&agent).await;
    if let Some(gateway_state) = gateway_state.as_ref() {
        gateway_state.set_routine_engine(routine_engine.clone());
    }

    // ── 8. Emit Connected event ─────────────────────────────────────
    use tauri::Emitter;
    let connected = UiEvent::Connected { protocol: 2 };
    if let Err(e) = app_handle.emit("thinclaw-event", &connected) {
        tracing::warn!("Failed to emit Connected event: {}", e);
    }

    tracing::info!("ThinClaw runtime initialized successfully");

    event_forwarders::spawn_sse(&app_handle, &sse_tx);

    event_forwarders::spawn_logs(&app_handle, &log_broadcaster);

    // Use the SAME cost_tracker that AgentDeps uses — so every LLM call
    // in the dispatcher records costs to this tracker.
    let cost_tracker = components.cost_tracker.clone();

    // Use ExtensionManager's catalog cache — already prefetched at startup.
    let catalog_cache = if let Some(ref ext_mgr) = components.extension_manager {
        ext_mgr.catalog_cache()
    } else {
        Arc::new(TokioMutex::new(CatalogCache::new(3600)))
    };

    let response_cache = components.response_cache.clone();
    let sandbox_config = components.config.sandbox.clone();
    // Use AppComponents' audit hook — this is the one ThinClaw's extension
    // lifecycle system actually writes events to.
    let audit_log_hook = components.audit_hook.clone();
    let manifest_validator = Arc::new(ManifestValidator::new());

    Ok(ThinClawRuntimeInner {
        agent,
        bg_handle: Mutex::new(Some(bg_handle)),
        inject_tx,
        log_broadcaster,
        active_sessions,
        tool_bridge,
        routine_engine,
        cost_tracker,
        catalog_cache,
        response_cache,
        audit_log_hook,
        manifest_validator,
        oauth_credential_sync: components.oauth_credential_sync,
        llm_runtime: components.llm_runtime.clone(),
        sandbox_config,
        auxiliary_tasks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_send_route_accepts_local_platform_aliases() {
        for platform in [
            "tauri",
            "desktop",
            "thinclaw_desktop",
            "local",
            "app",
            "web",
        ] {
            let route = desktop_send_route(platform, "agent:main", None, 0)
                .expect("desktop platform should be accepted");
            assert_eq!(route.session_key, "agent:main");
        }
    }

    #[test]
    fn desktop_send_route_rejects_external_channel_platforms() {
        for platform in [
            "slack",
            "telegram",
            "gmail",
            "email",
            "apple_mail",
            "discord",
        ] {
            let error = desktop_send_route(platform, "agent:main", None, 0)
                .expect_err("external channel should not use local desktop route");
            assert!(error.contains("supports only the Tauri/Desktop event surface"));
        }
    }

    #[test]
    fn desktop_send_route_thread_id_wins_over_recipient() {
        let route = desktop_send_route("desktop", "agent:wrong", Some("agent:right"), 0)
            .expect("desktop route should resolve");
        assert_eq!(route.session_key, "agent:right");
    }

    #[test]
    fn desktop_send_route_non_session_recipient_uses_system_session() {
        let route = desktop_send_route("desktop", "local_user", None, 0)
            .expect("desktop route should resolve");
        assert_eq!(route.session_key, "system");
    }

    #[test]
    fn desktop_send_route_rejects_attachments() {
        let error = desktop_send_route("desktop", "agent:main", None, 1)
            .expect_err("attachments should be explicit unsupported");
        assert!(error.contains("does not support attachments"));
    }

    #[test]
    fn agent_deps_keeps_desktop_runtime_parity_handles_wired() {
        let source = include_str!("runtime_builder.rs");
        let deps_block = source
            .split("let agent_deps = AgentDeps")
            .nth(1)
            .expect("desktop AgentDeps construction should stay explicit");

        for required in [
            "store: components.db.clone()",
            "tools: components.tools.clone()",
            "desktop_autonomy_manager: components.desktop_autonomy_manager.clone()",
            "extension_manager: components.extension_manager.clone()",
            "skill_registry: components.skill_registry.clone()",
            "cost_tracker: Some(components.cost_tracker.clone())",
            "response_cache: Some(components.response_cache.clone())",
            "llm_runtime: Some(components.llm_runtime.clone())",
            "routing_policy: Some(components.routing_policy.clone())",
            "sse_sender: Some(sse_tx.clone())",
            "agent_router: Some(shared_agent_router)",
            "agent_registry: Some(agent_registry)",
            "canvas_store: Some(",
            "subagent_executor: Some(subagent_executor.clone())",
            "model_override: Some(model_override)",
        ] {
            assert!(
                deps_block.contains(required),
                "desktop AgentDeps should wire {required}"
            );
        }
    }
}
