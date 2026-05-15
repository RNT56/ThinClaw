//! IronClaw engine builder — extracted from `ironclaw_bridge.rs`.
//!
//! Contains `build_inner()`, the ~950-line async function that:
//!   1. Configures bridge environment variables (IC-007)
//!   2. Resolves the LLM backend (local sidecar / MLX / cloud)
//!   3. Creates TauriChannel + ToolBridge
//!   4. Builds IronClaw's AppComponents
//!   5. Wires sub-agent executor + SSE broadcast + background tasks
//!   6. Returns a fully assembled `IronClawInner`
//!
//! Separated for maintainability — the public API remains in `ironclaw_bridge.rs`.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, OnceLock};

use tokio::sync::Mutex as TokioMutex;
use tokio::sync::Mutex;

use ironclaw::agent::{Agent, AgentDeps, AgentRegistry, AgentRouter};
use ironclaw::app::{AppBuilder, AppBuilderFlags, PeriodicPersistencePlan};
use ironclaw::channels::web::log_layer::LogBroadcaster;
use ironclaw::channels::web::types::SseEvent;
use ironclaw::channels::ChannelManager;
use ironclaw::extensions::clawhub::CatalogCache;
use ironclaw::extensions::manifest_validator::ManifestValidator;

use super::ironclaw_bridge::IronClawInner;
use super::ironclaw_channel::TauriChannel;
use super::tool_bridge::TauriToolBridge;
use super::ui_types::UiEvent;

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

/// Build a fully-configured `IronClawInner` from scratch.
///
/// This is the heavyweight initialization path — called by `IronClawState::start()`.
/// It configures bridge vars, resolves the LLM backend, creates channels,
/// builds engine components, and wires up all background tasks.
pub(crate) async fn build_inner(
    app_handle: tauri::AppHandle<tauri::Wry>,
    state_dir: std::path::PathBuf,
    secrets_store: Option<Arc<dyn ironclaw::secrets::SecretsStore + Send + Sync>>,
) -> Result<IronClawInner, anyhow::Error> {
    // ── 1. Configure environment for IronClaw ───────────────────────
    // IC-007: Use bridge overlay instead of unsafe set_var().
    // Build a HashMap of all config vars, then inject them atomically
    // into IronClaw's BRIDGE_VARS overlay. optional_env() checks this
    // overlay first, so all config resolvers see these values.
    use ironclaw::config::{bridge_var_exists, inject_bridge_vars};
    let mut bridge_config = std::collections::HashMap::<String, String>::new();

    // Database — only set defaults if user hasn't explicitly configured
    if !bridge_var_exists("DATABASE_BACKEND") {
        bridge_config.insert("DATABASE_BACKEND".into(), "libsql".into());
    }
    let db_path = state_dir.join("ironclaw.db");
    if !bridge_var_exists("LIBSQL_PATH") {
        bridge_config.insert(
            "LIBSQL_PATH".into(),
            db_path.to_str().unwrap_or("ironclaw.db").into(),
        );
    }

    // ── 1a-2. Enable heartbeat for ThinClaw Desktop mode ─────────────
    // The heartbeat checks HEARTBEAT.md every 30 minutes and proactively
    // notifies the user if any tasks need attention. This is the IronClaw
    // equivalent of ThinClaw's periodic heartbeat system.
    // Route heartbeat alerts to the Tauri "local_user" channel.
    // Allow env override (e.g. HEARTBEAT_ENABLED=false for testing).
    if !bridge_var_exists("HEARTBEAT_ENABLED") {
        bridge_config.insert("HEARTBEAT_ENABLED".into(), "true".into());
        bridge_config.insert("HEARTBEAT_NOTIFY_CHANNEL".into(), "tauri".into());
        bridge_config.insert("HEARTBEAT_NOTIFY_USER".into(), "local_user".into());
        // 30 minutes — matches ThinClaw default
        bridge_config.insert("HEARTBEAT_INTERVAL_SECS".into(), "1800".into());
        tracing::info!("[ironclaw] Heartbeat enabled (30-min interval, tauri channel)");
    }

    // ── 1b. Set WHISPER_HTTP_ENDPOINT for IronClaw voice/talk mode ───
    // ThinClaw Desktop's STT sidecar runs on port 53757 (fixed). IronClaw uses
    // this env var to call the local whisper server instead of bundling
    // its own whisper-rs. The endpoint is OpenAI-compatible.
    if !bridge_var_exists("WHISPER_HTTP_ENDPOINT") {
        bridge_config.insert(
            "WHISPER_HTTP_ENDPOINT".into(),
            "http://127.0.0.1:53757/v1/audio/transcriptions".into(),
        );
        tracing::debug!(
            "[ironclaw] Set WHISPER_HTTP_ENDPOINT=http://127.0.0.1:53757/v1/audio/transcriptions"
        );
    }

    // ── 1b-2. Set Extended Thinking env vars for IronClaw ───────────
    // IronClaw v0.12.0 supports chain-of-thought reasoning via
    // AGENT_THINKING_ENABLED + AGENT_THINKING_BUDGET_TOKENS env vars.
    // Only set if not already overridden by the user.
    if !bridge_var_exists("AGENT_THINKING_ENABLED") {
        // Thinking is opt-in — providers that support it (Claude, etc.)
        // will emit StatusUpdate::Thinking() events before the response.
        // Set to "true" to enable; defaults to off.
        bridge_config.insert("AGENT_THINKING_ENABLED".into(), "false".into());
        tracing::debug!("[ironclaw] Set AGENT_THINKING_ENABLED=false (default)");
    }
    if !bridge_var_exists("AGENT_THINKING_BUDGET_TOKENS") {
        bridge_config.insert("AGENT_THINKING_BUDGET_TOKENS".into(), "10000".into());
    }

    // ── 1b-3. Enable local dev tools (file write, shell, etc.) ──────
    // IronClaw defaults ALLOW_LOCAL_TOOLS to false (designed for SaaS where
    // tools run in sandboxed containers). In ThinClaw Desktop's context the
    // agent should be able to create files, run commands, and edit code.
    // The setting is controlled by the user via Gateway Settings toggle.
    {
        use tauri::Manager;
        let thinclaw_mgr = app_handle.state::<super::ThinClawManager>();
        let oc_config = thinclaw_mgr.get_config().await;
        let allow_local = oc_config
            .as_ref()
            .map(|c| c.allow_local_tools)
            .unwrap_or(true); // default true for desktop

        let workspace_mode = oc_config
            .as_ref()
            .map(|c| c.workspace_mode.clone())
            .unwrap_or_else(|| "sandboxed".to_string()); // default: sandboxed on desktop

        let workspace_root = oc_config.as_ref().and_then(|c| c.workspace_root.clone());

        // Resolve the base_dir for auto-generating a workspace fallback path
        let base_dir = oc_config.as_ref().map(|c| c.base_dir.clone());

        bridge_config.insert("ALLOW_LOCAL_TOOLS".into(), allow_local.to_string());
        bridge_config.insert("WORKSPACE_MODE".into(), workspace_mode.clone());

        // ── Workspace root resolution ─────────────────────────────────
        // Priority: user config → agent_workspace in app data dir.
        // WORKSPACE_ROOT is a ThinClaw bridge overlay value, not a dependable
        // process env var for desktop-side file event handling.
        // The default uses agent_workspace (already created at first launch)
        // so files are visible in the ThinClaw folder the user can see in Finder.
        let resolved_root = if let Some(ref root) = workspace_root {
            // User explicitly configured a root in Gateway settings
            std::path::PathBuf::from(root)
        } else if let Some(ref bd) = base_dir {
            // Default: <app_data>/ThinClaw/agent_workspace
            // (visible folder the user can already see in Finder)
            bd.join("agent_workspace")
        } else {
            // Absolute last resort fallback
            std::env::var("HOME")
                .map(|h| {
                    std::path::PathBuf::from(h)
                        .join("ThinClaw")
                        .join("agent_workspace")
                })
                .unwrap_or_else(|_| std::path::PathBuf::from("agent_workspace"))
        };

        // Create the directory if it doesn't exist yet
        if let Err(e) = std::fs::create_dir_all(&resolved_root) {
            tracing::warn!(
                "[ironclaw] Could not create workspace root {:?}: {}",
                resolved_root,
                e
            );
        } else {
            tracing::info!("[ironclaw] Workspace root: {:?}", resolved_root);
        }

        set_resolved_workspace_root(&resolved_root);
        bridge_config.insert(
            "WORKSPACE_ROOT".into(),
            resolved_root.to_str().unwrap_or("ThinClaw").into(),
        );

        // Enable safe bins allowlist for sandboxed mode (belt-and-suspenders
        // with ShellTool's own base_dir enforcement)
        if workspace_mode == "sandboxed" {
            bridge_config.insert("IRONCLAW_SAFE_BINS_ONLY".into(), "true".into());
        }
        // Note: for non-sandboxed mode, we simply don't insert the key.
        // The overlay check returns None, and optional_env falls through
        // to std::env::var which also returns NotPresent → disabled.

        // IC-001: Always set from config — stop() clears the overlay key,
        // so start() must re-read the persisted value unconditionally.
        let auto_approve = oc_config
            .as_ref()
            .map(|c| c.auto_approve_tools)
            .unwrap_or(false);
        bridge_config.insert("AGENT_AUTO_APPROVE_TOOLS".into(), auto_approve.to_string());
        tracing::info!("[ironclaw] Set AGENT_AUTO_APPROVE_TOOLS={}", auto_approve);

        // ── OS Governance: wire macOS permissions to IronClaw tool gates ──
        // IronClaw's ScreenCaptureTool checks SCREEN_CAPTURE_ENABLED (app.rs:820).
        // Only enable when BOTH screen recording is granted AND dev tools are on.
        let perms = crate::permissions::get_permission_status();
        if perms.screen_recording && allow_local {
            bridge_config.insert("SCREEN_CAPTURE_ENABLED".into(), "true".into());
            tracing::info!(
                "[ironclaw] Screen capture enabled (macOS permission granted + dev tools on)"
            );
        }

        tracing::info!(
            "[ironclaw] Set ALLOW_LOCAL_TOOLS={}, WORKSPACE_MODE={}, WORKSPACE_ROOT={:?}, SAFE_BINS_ONLY={}",
            allow_local,
            workspace_mode,
            resolved_root,
            workspace_mode == "sandboxed",
        );
    }

    // ── 1c. Set LLM_BACKEND / LLM_BASE_URL from ThinClaw Desktop config ───
    // IronClaw's LlmConfig::resolve() defaults to openai_compatible which
    // requires LLM_BASE_URL. We must tell it which backend to use based on
    // the user's gateway settings (local core vs cloud brain).
    //
    // IMPORTANT: always overwrite — do NOT check is_err() here. A previous
    // failed start (e.g. MLX not ready yet) may have written "ollama" as a
    // placeholder. When the user restarts the gateway after MLX is up, we
    // must overwrite with the real URL, not keep the stale placeholder.
    {
        use tauri::Manager;
        let thinclaw_mgr = app_handle.state::<super::ThinClawManager>();
        let oc_config = thinclaw_mgr.get_config().await;

        if let Some(ref cfg) = oc_config {
            if cfg.local_inference_enabled {
                // Local inference: point to llama.cpp / MLX sidecar
                let sidecar = app_handle.state::<crate::sidecar::SidecarManager>();
                if let Some((port, token, _ctx, _family)) = sidecar.get_chat_config() {
                    let base_url = format!("http://127.0.0.1:{}/v1", port);
                    tracing::info!(
                        "[ironclaw] Local inference (sidecar): LLM_BACKEND=openai_compatible, LLM_BASE_URL={}",
                        base_url
                    );
                    bridge_config.insert("LLM_BACKEND".into(), "openai_compatible".into());
                    bridge_config.insert("LLM_BASE_URL".into(), base_url);
                    if !token.is_empty() {
                        bridge_config.insert("LLM_API_KEY".into(), token);
                    }
                } else {
                    // Sidecar not running yet — try engine manager (MLX/vLLM)
                    let engine_mgr = app_handle.state::<crate::engine::EngineManager>();
                    let guard = engine_mgr.engine.lock().await;
                    let engine_url = guard.as_ref().and_then(|e| e.base_url());

                    if let Some(url) = engine_url {
                        tracing::info!(
                            "[ironclaw] Local inference (engine): LLM_BACKEND=openai_compatible, LLM_BASE_URL={}",
                            url
                        );
                        bridge_config.insert("LLM_BACKEND".into(), "openai_compatible".into());
                        bridge_config.insert("LLM_BASE_URL".into(), url);
                    } else {
                        // Neither sidecar nor engine running yet.
                        // If the user has a cloud brain selected, fall back to that
                        // instead of ollama — prevents "Provider llama3 request failed"
                        // errors when cloud intelligence is actually configured.
                        if let Some(ref brain) = cfg.selected_cloud_brain {
                            tracing::info!(
                                "[ironclaw] Local inference not ready, falling back to cloud brain '{}'",
                                brain
                            );
                            let selected_model = cfg.selected_cloud_model.as_deref();
                            match brain.as_str() {
                                "anthropic" => {
                                    bridge_config.insert("LLM_BACKEND".into(), "anthropic".into());
                                    if let Some(model) = selected_model {
                                        bridge_config
                                            .insert("ANTHROPIC_MODEL".into(), model.to_string());
                                    }
                                }
                                "openai" => {
                                    bridge_config.insert("LLM_BACKEND".into(), "openai".into());
                                    if let Some(model) = selected_model {
                                        bridge_config
                                            .insert("OPENAI_MODEL".into(), model.to_string());
                                    }
                                }
                                other => {
                                    if let Some(ep) =
                                        crate::inference::provider_endpoints::endpoint_for(other)
                                    {
                                        bridge_config.insert(
                                            "LLM_BACKEND".into(),
                                            "openai_compatible".into(),
                                        );
                                        bridge_config
                                            .insert("LLM_BASE_URL".into(), ep.base_url.to_string());
                                        if let Some(model) = selected_model {
                                            bridge_config
                                                .insert("LLM_MODEL".into(), model.to_string());
                                        }
                                    } else {
                                        bridge_config.insert("LLM_BACKEND".into(), "ollama".into());
                                    }
                                }
                            }
                        } else {
                            // No cloud brain configured either — use ollama as last resort
                            tracing::warn!(
                                "[ironclaw] Local inference not ready, no cloud brain configured, \
                                 using LLM_BACKEND=ollama as placeholder"
                            );
                            // IC-008: Emit UiEvent::Error so the user sees a toast
                            use tauri::Emitter;
                            let warning = crate::thinclaw::ui_types::UiEvent::Error {
                                message: "No LLM backend available — falling back to ollama. \
                                         Please configure a cloud brain or start local inference."
                                    .to_string(),
                                code: "LLM_FALLBACK".to_string(),
                                details: serde_json::Value::Null,
                            };
                            let _ = app_handle.emit("thinclaw-event", &warning);
                            bridge_config.insert("LLM_BACKEND".into(), "ollama".into());
                        }
                    }
                }
            } else if let Some(ref brain) = cfg.selected_cloud_brain {
                // Cloud brain selected: set the matching backend + model
                // IronClaw's LlmConfig::resolve() reads provider-specific env
                // vars (OPENAI_MODEL, ANTHROPIC_MODEL, LLM_MODEL) to determine
                // which model to use. Without setting these, it falls through
                // to the hardcoded default (e.g. gpt-4o for OpenAI).
                let selected_model = cfg.selected_cloud_model.as_deref();
                match brain.as_str() {
                    "anthropic" => {
                        tracing::info!("[ironclaw] Cloud brain: LLM_BACKEND=anthropic");
                        bridge_config.insert("LLM_BACKEND".into(), "anthropic".into());
                        if let Some(model) = selected_model {
                            bridge_config.insert("ANTHROPIC_MODEL".into(), model.to_string());
                            tracing::info!("[ironclaw] Cloud model: ANTHROPIC_MODEL={}", model);
                        }
                    }
                    "openai" => {
                        tracing::info!("[ironclaw] Cloud brain: LLM_BACKEND=openai");
                        bridge_config.insert("LLM_BACKEND".into(), "openai".into());
                        if let Some(model) = selected_model {
                            bridge_config.insert("OPENAI_MODEL".into(), model.to_string());
                            tracing::info!("[ironclaw] Cloud model: OPENAI_MODEL={}", model);
                        }
                    }
                    // All other providers use OpenAI-compatible endpoints
                    other => {
                        if let Some(ep) = crate::inference::provider_endpoints::endpoint_for(other)
                        {
                            tracing::info!(
                                "[ironclaw] Cloud brain '{}': LLM_BACKEND=openai_compatible, LLM_BASE_URL={}",
                                other,
                                ep.base_url
                            );
                            bridge_config.insert("LLM_BACKEND".into(), "openai_compatible".into());
                            bridge_config.insert("LLM_BASE_URL".into(), ep.base_url.to_string());
                            if let Some(model) = selected_model {
                                bridge_config.insert("LLM_MODEL".into(), model.to_string());
                                tracing::info!("[ironclaw] Cloud model: LLM_MODEL={}", model);
                            }
                        } else {
                            tracing::warn!(
                                "[ironclaw] Unknown cloud brain '{}', defaulting to ollama",
                                other
                            );
                            bridge_config.insert("LLM_BACKEND".into(), "ollama".into());
                        }
                    }
                }
            }
        }

        // Final safety net: if still no LLM_BACKEND is set (no ThinClaw config
        // loaded), use ollama — it needs no API key or base URL, so config
        // resolution always succeeds.
        if !bridge_config.contains_key("LLM_BACKEND") && !bridge_var_exists("LLM_BACKEND") {
            tracing::info!("[ironclaw] No provider config found, defaulting LLM_BACKEND=ollama");
            bridge_config.insert("LLM_BACKEND".into(), "ollama".into());
        }
    }

    // ── IC-007: Inject all bridge config vars atomically ─────────────
    // This single call replaces ~47 scattered unsafe set_var() calls.
    // All values are now visible to IronClaw's config resolvers via
    // optional_env() which checks BRIDGE_VARS before real env vars.
    let bridge_var_count = bridge_config.len();
    inject_bridge_vars(bridge_config);
    tracing::info!(
        "[ironclaw] IC-007: Injected {} bridge config vars into overlay (no unsafe set_var)",
        bridge_var_count
    );

    // ── 2. Load config ──────────────────────────────────────────────
    let toml_path = state_dir.join("ironclaw.toml");
    let toml_path_ref = if toml_path.exists() {
        Some(toml_path.as_path())
    } else {
        None
    };

    let config = match ironclaw::Config::from_env_with_toml(toml_path_ref).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Failed to load IronClaw config, using env-only: {}", e);
            ironclaw::Config::from_env().await?
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
                "[ironclaw] GLOBAL_LOG_BROADCASTER not set — creating standalone broadcaster. \
                 Tracing events will NOT reach the UI Logs tab."
            );
            Arc::new(LogBroadcaster::new())
        });

    let toml_path_opt = if state_dir.join("ironclaw.toml").exists() {
        Some(state_dir.join("ironclaw.toml"))
    } else {
        None
    };

    let mut builder = AppBuilder::new(
        config,
        AppBuilderFlags::default(),
        toml_path_opt,
        log_broadcaster.clone(),
    );

    if let Some(store) = secrets_store {
        builder = builder.with_secrets_store(store);
    }

    // Wire TauriToolBridge into the IronClaw engine — enables hardware
    // sensor tools (camera, mic, screen) with 3-tier user approval.
    builder = builder.with_tool_bridge(tool_bridge.clone());

    // ── 4b. Translate ThinClaw Desktop's cloud intelligence config into IronClaw
    //        ProvidersSettings for multi-provider failover + smart routing.
    {
        use tauri::Manager;
        let thinclaw_mgr = app_handle.state::<super::ThinClawManager>();
        let oc_config = thinclaw_mgr.get_config().await;

        if let Some(ref cfg) = oc_config {
            let mut providers = ironclaw::settings::ProvidersSettings::default();

            // Map enabled cloud providers
            providers.enabled = cfg.enabled_cloud_providers.clone();

            // Map primary provider + model
            providers.primary = cfg.selected_cloud_brain.clone();
            providers.primary_model = cfg.selected_cloud_model.clone();

            // Map per-provider model allowlists
            providers.allowed_models = cfg.enabled_cloud_models.clone();

            // Fallback chain is auto-generated from enabled providers
            // (FailoverProvider will use all enabled providers in order)
            providers.fallback_chain = Vec::new();

            if !providers.enabled.is_empty() {
                tracing::info!(
                    "[ironclaw] Cloud intelligence config translated: {} provider(s) enabled, \
                     primary={:?}, model={:?}",
                    providers.enabled.len(),
                    providers.primary,
                    providers.primary_model,
                );
                builder = builder.with_providers_settings(providers);
            }
        }
    }

    let components = builder.build_all().await?;

    // ── 5. Create channel manager and register TauriChannel ─────────
    let channel_manager = Arc::new(ChannelManager::new());
    channel_manager.add(Box::new(tauri_channel)).await;

    {
        let send_handle = app_handle.clone();
        components.tools.register_send_message_tool(Some(Arc::new(
            move |platform: String,
                  recipient: String,
                  text: String,
                  thread_id: Option<String>,
                  attachments: Vec<ironclaw::media::MediaContent>| {
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
        tracing::info!("[ironclaw] send_message tool registered for local Tauri/Desktop delivery");
    }

    // ── 6. Create SSE broadcast channel + agent ─────────────────────
    // Channel must be created BEFORE AgentDeps so we can wire sse_sender in.
    // The forwarder below subscribes and forwards RoutineLifecycle events
    // as 'thinclaw-event' Tauri emissions to the frontend.
    let (sse_tx, _sse_rx_seed) = tokio::sync::broadcast::channel::<SseEvent>(64);

    // ── 5b. Create sub-agent executor ───────────────────────────────
    // Shares the same LLM, safety layer, tool registry, and channel
    // manager as the main agent. This lets the agent use spawn_subagent
    // to delegate parallel work to isolated in-process agentic loops.
    //
    // The dispatcher in dispatcher.rs intercepts spawn_subagent tool
    // results (JSON action descriptors) and calls executor.spawn() here.
    // Without this wiring the tool silently returns "not initialized".
    let (subagent_executor, subagent_result_rx) =
        ironclaw::agent::subagent_executor::SubagentExecutor::new(
            components.llm.clone(),
            components.safety.clone(),
            components.tools.clone(),
            channel_manager.clone(),
            ironclaw::agent::subagent_executor::SubagentConfig {
                max_concurrent: 5,
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
    if let Some(ref workspace) = components.workspace {
        subagent_executor = subagent_executor.with_workspace(Arc::clone(workspace));
    }
    if let Some(ref skill_registry) = components.skill_registry {
        subagent_executor = subagent_executor
            .with_skill_registry(Arc::clone(skill_registry), components.config.skills.clone());
    }
    let subagent_executor = Arc::new(subagent_executor);

    let model_override = ironclaw::tools::builtin::new_shared_model_override();
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
    components
        .tools
        .register_sync(Arc::new(ironclaw::tools::builtin::SpawnSubagentTool::new(
            subagent_executor.clone(),
        )));
    components
        .tools
        .register_sync(Arc::new(ironclaw::tools::builtin::ListSubagentsTool::new(
            subagent_executor.clone(),
        )));
    components
        .tools
        .register_sync(Arc::new(ironclaw::tools::builtin::CancelSubagentTool::new(
            subagent_executor.clone(),
        )));
    tracing::info!("[ironclaw] Sub-agent tools registered (spawn, list, cancel)");

    // Re-register MemoryDeleteTool with the SSE sender now that we have the channel.
    // build_all() registered it with None; we replace it here with the live sender.
    // register_sync() replaces existing entries by name, so no duplicates occur.
    if let Some(ref ws) = components.workspace {
        use ironclaw::tools::builtin::MemoryDeleteTool;
        let delete_tool = MemoryDeleteTool::new(ws.clone()).with_sse_sender(sse_tx.clone());
        components
            .tools
            .register_sync(std::sync::Arc::new(delete_tool));
        tracing::info!(
            "[ironclaw] MemoryDeleteTool re-registered with SSE sender (BOOTSTRAP.md delete detection enabled)"
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
                        "[ironclaw] Loaded {} persisted agent workspace(s) into desktop router",
                        count
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        "[ironclaw] Failed to load persisted agent workspaces: {}",
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
        tracing::info!("[ironclaw] Cost persistence background task started");
    }

    auxiliary_tasks.push(ironclaw::llm::pricing_sync::spawn_pricing_sync(
        components.db.as_ref().map(Arc::clone),
    ));
    tracing::info!("[ironclaw] Pricing sync background task started");

    let agent_deps = AgentDeps {
        store: components.db.clone(),
        llm: components.llm.clone(),
        cheap_llm: components.cheap_llm.clone(),
        safety: components.safety.clone(),
        tools: components.tools.clone(),
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
        agent_router: Some(shared_agent_router),
        agent_registry: Some(agent_registry),
        canvas_store: Some(ironclaw::channels::canvas_gateway::CanvasStore::new(
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
        None,
    ));

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
    tracing::info!("[ironclaw] Job tools registered with desktop scheduler-backed execution");

    // ── 6b. Sub-agent result injector ───────────────────────────────
    // Polls the SubagentExecutor's result channel and re-injects
    // completed sub-agent results back into the main agent as new
    // user-invisible turns. This is the "fire-and-forget → re-inject"
    // pattern that enables true parallelism.
    {
        let agent_for_subagent = Arc::clone(&agent);
        let mut rx = subagent_result_rx;
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                let result = &msg.result;
                let synthetic_content = if result.success {
                    format!(
                        "[Sub-agent '{}' completed ({} iterations, {:.1}s)]\n\n{}",
                        result.name,
                        result.iterations,
                        result.duration_ms as f64 / 1000.0,
                        result.response
                    )
                } else {
                    format!(
                        "[Sub-agent '{}' failed ({:.1}s)]\n\nError: {}",
                        result.name,
                        result.duration_ms as f64 / 1000.0,
                        result.error.as_deref().unwrap_or("unknown"),
                    )
                };

                // Mark as completed in the executor
                if let Some(exec) = agent_for_subagent.subagent_executor() {
                    exec.mark_completed(result.agent_id, result.success, result.error.clone())
                        .await;
                }

                tracing::info!(
                    agent_id = %result.agent_id,
                    name = %result.name,
                    success = result.success,
                    iterations = result.iterations,
                    duration_ms = result.duration_ms,
                    "Sub-agent result received, injecting into main agent"
                );

                // Build an IncomingMessage that goes through the normal pipeline
                let incoming = ironclaw::channels::IncomingMessage::new(
                    "subagent",
                    "system",
                    &synthetic_content,
                )
                .with_thread(&msg.parent_thread_id)
                .with_metadata(msg.channel_metadata.clone());

                match agent_for_subagent.handle_message_external(&incoming).await {
                    Ok(Some(response)) if !response.is_empty() => {
                        tracing::debug!(
                            "Main agent response to sub-agent result: {} chars",
                            response.len()
                        );
                        // The response goes through TauriChannel automatically
                        // via the normal respond() path in handle_message
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!("Failed to inject sub-agent result: {}", e);
                    }
                }
            }
            tracing::debug!("[subagent] Result injector task ended");
        });
    }

    // ── 7. Start background tasks ───────────────────────────────────
    let bg_handle = agent.start_background_tasks().await;

    // Extract routine engine Arc for easy access (parity with run() loop's
    // routine_engine_for_loop). The same Arc stays in bg_handle too.
    let routine_engine = bg_handle.routine_engine().map(Arc::clone);

    // ── 7a. System event consumer (heartbeat → livechat) ─────────────
    // In standalone mode, agent.run() reads from system_event_rx in its
    // main select! loop. In Tauri mode, there IS no message loop — each
    // user message is processed on-demand via handle_message_external().
    // Without this consumer, heartbeat messages pile up in the channel
    // buffer (capacity 16) and are silently dropped.
    {
        let mut bg_lock = bg_handle.lock_system_events().await;
        if let Some(mut system_rx) = bg_lock.take() {
            let agent_for_sys = Arc::clone(&agent);
            tokio::spawn(async move {
                tracing::info!("[ironclaw] System event consumer started (heartbeat → livechat)");
                while let Some(msg) = system_rx.recv().await {
                    tracing::info!(
                        channel = %msg.channel,
                        "[ironclaw] Processing system event in Tauri mode"
                    );

                    match agent_for_sys.handle_message_external(&msg).await {
                        Ok(Some(response)) if !response.is_empty() => {
                            // Suppress HEARTBEAT_OK — parity with run() loop
                            if msg.channel == "heartbeat" && response.contains("HEARTBEAT_OK") {
                                tracing::debug!(
                                    "[ironclaw] Heartbeat returned HEARTBEAT_OK — suppressed"
                                );
                                continue;
                            }

                            // Deliver via broadcast_all (→ TauriChannel → thinclaw-event)
                            // We use broadcast_all instead of respond() because the
                            // message's channel is "heartbeat" which isn't a registered
                            // channel — TauriChannel registers as "tauri".
                            let results = agent_for_sys
                                .channels()
                                .broadcast_all(
                                    &msg.user_id,
                                    ironclaw::channels::OutgoingResponse::text(response),
                                )
                                .await;
                            for (ch, result) in results {
                                if let Err(e) = result {
                                    tracing::error!(
                                        "[ironclaw] System event broadcast to {} failed: {}",
                                        ch,
                                        e
                                    );
                                }
                            }
                        }
                        Ok(_) => {
                            tracing::debug!(
                                "[ironclaw] System event processed (no visible response)"
                            );
                        }
                        Err(e) => {
                            tracing::error!("[ironclaw] System event processing failed: {}", e);
                        }
                    }
                }
                tracing::info!("[ironclaw] System event consumer ended");
            });
        }
    }

    // ── 7b. Job TTL reaper — force-cancel zombie jobs ────────────────
    // Prevents the "Maximum parallel jobs (5) exceeded" cascade.
    // If a job is active for longer than JOB_MAX_TTL, we force-cancel it
    // to free the slot. The existing cleanup tasks in scheduler.rs only
    // remove finished handles from the jobs HashMap — they don't touch
    // the ContextManager, which is where the slot-counting happens.
    {
        const JOB_MAX_TTL_SECS: i64 = 600; // 10 minutes
        const REAPER_INTERVAL_SECS: u64 = 60; // check every minute

        let agent_for_reaper = Arc::clone(&agent);
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(REAPER_INTERVAL_SECS));
            // Skip immediate first tick
            interval.tick().await;

            loop {
                interval.tick().await;

                let cm = agent_for_reaper.context_manager();
                let active = cm.active_jobs().await;

                if active.is_empty() {
                    continue;
                }

                let now = chrono::Utc::now();
                let mut reaped = 0usize;

                for job_id in active {
                    if let Ok(ctx) = cm.get_context(job_id).await {
                        // Only reap InProgress or Pending jobs (not Stuck — self-repair handles those)
                        if !matches!(
                            ctx.state,
                            ironclaw::context::JobState::InProgress
                                | ironclaw::context::JobState::Pending
                        ) {
                            continue;
                        }

                        let age = now.signed_duration_since(ctx.created_at);
                        if age.num_seconds() > JOB_MAX_TTL_SECS {
                            tracing::warn!(
                                job_id = %job_id,
                                age_secs = age.num_seconds(),
                                title = %ctx.title,
                                "[reaper] Force-cancelling zombie job (exceeded {}s TTL)",
                                JOB_MAX_TTL_SECS
                            );

                            // Try to cancel via scheduler first (sends Stop + abort)
                            agent_for_reaper.scheduler().stop(job_id).await.ok();

                            // Also force the ContextManager state to terminal
                            // in case the scheduler didn't clean it up
                            let _ = cm
                                .update_context(job_id, |c| {
                                    let _ = c.transition_to(
                                        ironclaw::context::JobState::Failed,
                                        Some(format!(
                                            "Force-cancelled by TTL reaper (alive {}s, limit {}s)",
                                            age.num_seconds(),
                                            JOB_MAX_TTL_SECS
                                        )),
                                    );
                                })
                                .await;

                            reaped += 1;
                        }
                    }
                }

                if reaped > 0 {
                    tracing::info!(
                        "[reaper] Force-cancelled {} zombie job(s), freeing slots",
                        reaped
                    );
                }
            }
        });
    }

    // ── 7c. BeforeAgentStart hook ────────────────────────────────────
    // Parity with run() loop — allows hooks to inspect startup config.
    {
        let event = ironclaw::hooks::HookEvent::AgentStart {
            model: "tauri-direct".to_string(),
            provider: "ironclaw".to_string(),
        };
        match agent.hooks().run(&event).await {
            Err(ironclaw::hooks::HookError::Rejected { reason }) => {
                tracing::error!("BeforeAgentStart hook rejected startup: {}", reason);
                // Don't fail the engine start — just log. The hook can still
                // do pre-flight checks, but we don't want to prevent the UI.
            }
            Err(err) => {
                tracing::warn!("BeforeAgentStart hook error (fail-open): {}", err);
            }
            Ok(_) => {}
        }
    }

    // ── 8. Emit Connected event ─────────────────────────────────────
    use tauri::Emitter;
    let connected = UiEvent::Connected { protocol: 2 };
    if let Err(e) = app_handle.emit("thinclaw-event", &connected) {
        tracing::warn!("Failed to emit Connected event: {}", e);
    }

    tracing::info!("IronClaw engine initialized successfully");

    // ── 8b. Spawn SSE → Tauri forwarder ─────────────────────────────────────────────────
    // Forward RoutineLifecycle events from the SSE channel to the frontend.
    {
        let mut sse_rx = sse_tx.subscribe();
        let fwd_handle = app_handle.clone();
        tokio::spawn(async move {
            use tauri::Emitter as _;
            loop {
                match sse_rx.recv().await {
                    Ok(event) => {
                        let ui_event: Option<UiEvent> = match &event {
                            SseEvent::RoutineLifecycle {
                                routine_name,
                                event,
                                run_id,
                                result_summary,
                            } => Some(UiEvent::RoutineLifecycle {
                                routine_name: routine_name.clone(),
                                event: event.clone(),
                                run_id: run_id.clone(),
                                result_summary: result_summary.clone(),
                            }),
                            SseEvent::BootstrapCompleted => Some(UiEvent::BootstrapCompleted),
                            SseEvent::ToolResult { name, preview, .. } if name == "write_file" => {
                                // Parse the write_file result JSON to extract path & bytes
                                let val: serde_json::Value = serde_json::from_str(preview)
                                    .unwrap_or_else(|_| serde_json::Value::Null);
                                if let (Some(path), Some(bytes)) = (
                                    val.get("path").and_then(|v| v.as_str()),
                                    val.get("bytes_written").and_then(|v| v.as_u64()),
                                ) {
                                    // Compute workspace-relative display path
                                    let workspace_root = get_resolved_workspace_root();
                                    let relative = if let Some(workspace_root) = workspace_root {
                                        path.strip_prefix(&workspace_root)
                                            .unwrap_or(path)
                                            .trim_start_matches('/')
                                            .to_string()
                                    } else {
                                        // Fall back to just the filename
                                        std::path::Path::new(path)
                                            .file_name()
                                            .and_then(|n| n.to_str())
                                            .unwrap_or(path)
                                            .to_string()
                                    };
                                    tracing::info!(
                                        "[ironclaw] FileCreated: {} ({} bytes)",
                                        relative,
                                        bytes
                                    );
                                    Some(UiEvent::FileCreated {
                                        path: path.to_string(),
                                        relative_path: relative,
                                        bytes,
                                    })
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        };
                        if let Some(ev) = ui_event {
                            if let Err(e) = fwd_handle.emit("thinclaw-event", &ev) {
                                tracing::warn!("[sse-fwd] emit failed: {}", e);
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("[sse-fwd] dropped {} events", n);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    // ── 8c. Live log push → Tauri frontend ──────────────────────────────────────────────
    // Subscribe to LogBroadcaster and forward each new entry as a
    // UiEvent::LogEntry so the UI Logs tab updates in real-time
    // instead of relying on the 2s polling interval.
    {
        let mut log_rx = log_broadcaster.subscribe();
        let log_fwd_handle = app_handle.clone();
        tokio::spawn(async move {
            use tauri::Emitter as _;
            loop {
                match log_rx.recv().await {
                    Ok(entry) => {
                        let ev = UiEvent::LogEntry {
                            timestamp: entry.timestamp,
                            level: entry.level,
                            target: entry.target,
                            message: entry.message,
                        };
                        // Fire-and-forget: if no UI is listening, drop the event.
                        let _ = log_fwd_handle.emit("thinclaw-event", &ev);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("[log-fwd] dropped {} log events (UI too slow)", n);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

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
    // Use AppComponents' audit hook — this is the one IronClaw's extension
    // lifecycle system actually writes events to.
    let audit_log_hook = components.audit_hook.clone();
    let manifest_validator = Arc::new(ManifestValidator::new());

    Ok(IronClawInner {
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
        let source = include_str!("ironclaw_builder.rs");
        let deps_block = source
            .split("let agent_deps = AgentDeps")
            .nth(1)
            .expect("desktop AgentDeps construction should stay explicit");

        for required in [
            "store: components.db.clone()",
            "tools: components.tools.clone()",
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
