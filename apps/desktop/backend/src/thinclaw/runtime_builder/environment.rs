//! Desktop bridge environment and LLM-backend resolution.

use super::{runtime_db_path, set_resolved_workspace_root, RUNTIME_DB_NAME};

pub(super) async fn configure(
    app_handle: &tauri::AppHandle<tauri::Wry>,
    state_dir: &std::path::Path,
) -> Result<(), anyhow::Error> {
    // ── 1. Configure environment for ThinClaw ───────────────────────
    // IC-007: Use bridge overlay instead of unsafe set_var().
    // Build a HashMap of all config vars, then inject them atomically
    // into ThinClaw's BRIDGE_VARS overlay. optional_env() checks this
    // overlay first, so all config resolvers see these values.
    use thinclaw_core::config::{bridge_var_exists, inject_bridge_vars};
    let mut bridge_config = std::collections::HashMap::<String, String>::new();

    // OAuth credentials remain encrypted at rest and enter the core runtime
    // only through the in-memory bridge overlay. They are never serialized to
    // runtime.toml, settings rows, status payloads, or process-global env.
    {
        use tauri::Manager;
        let secrets = app_handle.state::<crate::secret_store::SecretStore>();
        if let Some(token) = secrets.get("gmail_oauth_token") {
            bridge_config.insert("GMAIL_OAUTH_TOKEN".into(), token);
        }
        if let Some(token) = secrets.get("gmail_refresh_token") {
            bridge_config.insert("GMAIL_REFRESH_TOKEN".into(), token);
        }
    }

    // Database — only set defaults if user hasn't explicitly configured
    if !bridge_var_exists("DATABASE_BACKEND") {
        bridge_config.insert("DATABASE_BACKEND".into(), "libsql".into());
    }
    let db_path = runtime_db_path(state_dir);
    if !bridge_var_exists("LIBSQL_PATH") {
        bridge_config.insert(
            "LIBSQL_PATH".into(),
            db_path.to_str().unwrap_or(RUNTIME_DB_NAME).into(),
        );
    }

    // ── 1a-2. Enable heartbeat for ThinClaw Desktop mode ─────────────
    // The heartbeat checks HEARTBEAT.md every 30 minutes and proactively
    // notifies the user if any tasks need attention. This is the ThinClaw
    // equivalent of ThinClaw's periodic heartbeat system.
    // Route heartbeat alerts to the Tauri "local_user" channel.
    // Allow env override (e.g. HEARTBEAT_ENABLED=false for testing).
    if !bridge_var_exists("HEARTBEAT_ENABLED") {
        bridge_config.insert("HEARTBEAT_ENABLED".into(), "true".into());
        bridge_config.insert("HEARTBEAT_NOTIFY_CHANNEL".into(), "tauri".into());
        bridge_config.insert("HEARTBEAT_NOTIFY_USER".into(), "local_user".into());
        // 30 minutes — matches ThinClaw default
        bridge_config.insert("HEARTBEAT_INTERVAL_SECS".into(), "1800".into());
        tracing::info!("[thinclaw-runtime] Heartbeat enabled (30-min interval, tauri channel)");
    }

    // ── 1b. Set WHISPER_HTTP_ENDPOINT for ThinClaw voice/talk mode ───
    // ThinClaw Desktop's STT sidecar runs on port 53757 (fixed). ThinClaw uses
    // this env var to call the local whisper server instead of bundling
    // its own whisper-rs. The endpoint is OpenAI-compatible.
    if !bridge_var_exists("WHISPER_HTTP_ENDPOINT") {
        bridge_config.insert(
            "WHISPER_HTTP_ENDPOINT".into(),
            "http://127.0.0.1:53757/v1/audio/transcriptions".into(),
        );
        tracing::debug!(
            "[thinclaw-runtime] Set WHISPER_HTTP_ENDPOINT=http://127.0.0.1:53757/v1/audio/transcriptions"
        );
    }

    // ── 1b-2. Set Extended Thinking env vars for ThinClaw ───────────
    // ThinClaw v0.12.0 supports chain-of-thought reasoning via
    // AGENT_THINKING_ENABLED + AGENT_THINKING_BUDGET_TOKENS env vars.
    // Only set if not already overridden by the user.
    if !bridge_var_exists("AGENT_THINKING_ENABLED") {
        // Thinking is opt-in — providers that support it (Claude, etc.)
        // will emit StatusUpdate::Thinking() events before the response.
        // Set to "true" to enable; defaults to off.
        bridge_config.insert("AGENT_THINKING_ENABLED".into(), "false".into());
        tracing::debug!("[thinclaw-runtime] Set AGENT_THINKING_ENABLED=false (default)");
    }
    if !bridge_var_exists("AGENT_THINKING_BUDGET_TOKENS") {
        bridge_config.insert("AGENT_THINKING_BUDGET_TOKENS".into(), "10000".into());
    }

    // ── 1b-3. Enable local dev tools (file write, shell, etc.) ──────
    // ThinClaw defaults ALLOW_LOCAL_TOOLS to false (designed for SaaS where
    // tools run in sandboxed containers). In ThinClaw Desktop's context the
    // agent should be able to create files, run commands, and edit code.
    // The setting is controlled by the user via Gateway Settings toggle.
    {
        use tauri::Manager;
        let thinclaw_mgr = app_handle.state::<crate::thinclaw::ThinClawManager>();
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
                "[thinclaw-runtime] Could not create workspace root {:?}: {}",
                resolved_root,
                e
            );
        } else {
            tracing::info!("[thinclaw-runtime] Workspace root: {:?}", resolved_root);
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
        tracing::info!(
            "[thinclaw-runtime] Set AGENT_AUTO_APPROVE_TOOLS={}",
            auto_approve
        );

        // ── OS Governance: wire macOS permissions to ThinClaw tool gates ──
        // ThinClaw's ScreenCaptureTool checks SCREEN_CAPTURE_ENABLED (app.rs:820).
        // Only enable when BOTH screen recording is granted AND dev tools are on.
        let perms = crate::permissions::get_permission_status();
        if perms.screen_recording && allow_local {
            bridge_config.insert("SCREEN_CAPTURE_ENABLED".into(), "true".into());
            tracing::info!(
                "[thinclaw-runtime] Screen capture enabled (macOS permission granted + dev tools on)"
            );
        }

        tracing::info!(
            "[thinclaw-runtime] Set ALLOW_LOCAL_TOOLS={}, WORKSPACE_MODE={}, WORKSPACE_ROOT={:?}, SAFE_BINS_ONLY={}",
            allow_local,
            workspace_mode,
            resolved_root,
            workspace_mode == "sandboxed",
        );
    }

    // ── 1c. Set LLM_BACKEND / LLM_BASE_URL from ThinClaw Desktop config ───
    // ThinClaw's LlmConfig::resolve() defaults to openai_compatible which
    // requires LLM_BASE_URL. We must tell it which backend to use based on
    // the user's gateway settings (local core vs cloud brain).
    //
    // IMPORTANT: always overwrite — do NOT check is_err() here. A previous
    // failed start (e.g. MLX not ready yet) may have written "ollama" as a
    // placeholder. When the user restarts the gateway after MLX is up, we
    // must overwrite with the real URL, not keep the stale placeholder.
    {
        use tauri::Manager;
        let thinclaw_mgr = app_handle.state::<crate::thinclaw::ThinClawManager>();
        let oc_config = thinclaw_mgr.get_config().await;

        if let Some(ref cfg) = oc_config {
            if cfg.local_inference_enabled {
                let sidecar = app_handle.state::<crate::sidecar::SidecarManager>();
                let engine_mgr = app_handle.state::<crate::engine::EngineManager>();
                let snapshot = crate::engine::local_runtime_snapshot(&sidecar, &engine_mgr).await;

                if let Some(endpoint) = snapshot.endpoint {
                    tracing::info!(
                        "[thinclaw-runtime] Local inference: LLM_BACKEND=openai_compatible, LLM_BASE_URL={}",
                        endpoint.base_url
                    );
                    bridge_config.insert("LLM_BACKEND".into(), "openai_compatible".into());
                    bridge_config.insert("LLM_BASE_URL".into(), endpoint.base_url);
                    if let Some(token) = endpoint.api_key.filter(|token| !token.is_empty()) {
                        bridge_config.insert("LLM_API_KEY".into(), token);
                    }
                } else {
                    // If local is preferred but unavailable and the user has a
                    // cloud brain selected, use that explicit provider. Do not
                    // invent an Ollama fallback: that hides runtime failures.
                    if let Some(ref brain) = cfg.selected_cloud_brain {
                        tracing::info!(
                            "[thinclaw-runtime] Local inference not ready, falling back to cloud brain '{}'",
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
                                    bridge_config.insert("OPENAI_MODEL".into(), model.to_string());
                                }
                            }
                            other => {
                                if let Some(ep) =
                                    thinclaw_config::provider_catalog::endpoint_for(other)
                                {
                                    bridge_config
                                        .insert("LLM_BACKEND".into(), "openai_compatible".into());
                                    bridge_config
                                        .insert("LLM_BASE_URL".into(), ep.base_url.to_string());
                                    if let Some(model) = selected_model {
                                        bridge_config.insert("LLM_MODEL".into(), model.to_string());
                                    }
                                } else {
                                    return Err(anyhow::anyhow!(
                                        "Unknown selected cloud brain '{other}' and local inference is unavailable"
                                    ));
                                }
                            }
                        }
                    } else {
                        use tauri::Emitter;
                        let message = snapshot.unavailable_reason.unwrap_or_else(|| {
                            "No local inference runtime is available".to_string()
                        });
                        let warning = crate::thinclaw::ui_types::UiEvent::Error {
                            message: format!(
                                "{message}. Configure a cloud brain or start local inference."
                            ),
                            code: "LLM_RUNTIME_UNAVAILABLE".to_string(),
                            details: serde_json::Value::Null,
                        };
                        let _ = app_handle.emit("thinclaw-event", &warning);
                        return Err(anyhow::anyhow!(
                            "{message}. Configure a cloud brain or start local inference."
                        ));
                    }
                }
            } else if let Some(ref brain) = cfg.selected_cloud_brain {
                // Cloud brain selected: set the matching backend + model
                // ThinClaw's LlmConfig::resolve() reads provider-specific env
                // vars (OPENAI_MODEL, ANTHROPIC_MODEL, LLM_MODEL) to determine
                // which model to use. Without setting these, it falls through
                // to the hardcoded default (e.g. gpt-4o for OpenAI).
                let selected_model = cfg.selected_cloud_model.as_deref();
                match brain.as_str() {
                    "anthropic" => {
                        tracing::info!("[thinclaw-runtime] Cloud brain: LLM_BACKEND=anthropic");
                        bridge_config.insert("LLM_BACKEND".into(), "anthropic".into());
                        if let Some(model) = selected_model {
                            bridge_config.insert("ANTHROPIC_MODEL".into(), model.to_string());
                            tracing::info!(
                                "[thinclaw-runtime] Cloud model: ANTHROPIC_MODEL={}",
                                model
                            );
                        }
                    }
                    "openai" => {
                        tracing::info!("[thinclaw-runtime] Cloud brain: LLM_BACKEND=openai");
                        bridge_config.insert("LLM_BACKEND".into(), "openai".into());
                        if let Some(model) = selected_model {
                            bridge_config.insert("OPENAI_MODEL".into(), model.to_string());
                            tracing::info!(
                                "[thinclaw-runtime] Cloud model: OPENAI_MODEL={}",
                                model
                            );
                        }
                    }
                    // All other providers use OpenAI-compatible endpoints
                    other => {
                        if let Some(ep) = thinclaw_config::provider_catalog::endpoint_for(other) {
                            tracing::info!(
                                "[thinclaw-runtime] Cloud brain '{}': LLM_BACKEND=openai_compatible, LLM_BASE_URL={}",
                                other,
                                ep.base_url
                            );
                            bridge_config.insert("LLM_BACKEND".into(), "openai_compatible".into());
                            bridge_config.insert("LLM_BASE_URL".into(), ep.base_url.to_string());
                            if let Some(model) = selected_model {
                                bridge_config.insert("LLM_MODEL".into(), model.to_string());
                                tracing::info!(
                                    "[thinclaw-runtime] Cloud model: LLM_MODEL={}",
                                    model
                                );
                            }
                        } else {
                            tracing::warn!(
                                "[thinclaw-runtime] Unknown cloud brain '{}', defaulting to ollama",
                                other
                            );
                            bridge_config.insert("LLM_BACKEND".into(), "ollama".into());
                        }
                    }
                }
            }
        }

        if !bridge_config.contains_key("LLM_BACKEND") && !bridge_var_exists("LLM_BACKEND") {
            return Err(anyhow::anyhow!(
                "No LLM backend configured. Configure a cloud brain or start local inference."
            ));
        }
    }

    // ── IC-007: Inject all bridge config vars atomically ─────────────
    // This single call replaces ~47 scattered unsafe set_var() calls.
    // All values are now visible to ThinClaw's config resolvers via
    // optional_env() which checks BRIDGE_VARS before real env vars.
    let bridge_var_count = bridge_config.len();
    inject_bridge_vars(bridge_config);
    tracing::info!(
        "[thinclaw-runtime] IC-007: Injected {} bridge config vars into overlay (no unsafe set_var)",
        bridge_var_count
    );

    Ok(())
}
