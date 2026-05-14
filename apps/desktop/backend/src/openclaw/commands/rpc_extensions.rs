//! RPC commands — Hooks, extensions, diagnostics, tools, pairing, compaction.
//!
//! Extracted from `rpc.rs` for better modularity.

use tauri::State;

use super::types::*;
use crate::openclaw::ironclaw_bridge::IronClawState;

// ============================================================================
// Hooks management
// ============================================================================

/// List all registered lifecycle hooks with their details.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_hooks_list(
    ironclaw: State<'_, IronClawState>,
) -> Result<HooksListResponse, String> {
    let agent = ironclaw.agent().await?;
    let hooks = agent.hooks();
    let details = hooks.list_with_details().await;

    let hooks_list: Vec<HookInfoItem> = details
        .into_iter()
        .map(|h| HookInfoItem {
            name: h.name,
            hook_points: h.hook_points,
            failure_mode: h.failure_mode,
            timeout_ms: h.timeout_ms as u32,
            priority: h.priority,
        })
        .collect();

    let total = hooks_list.len() as u32;
    Ok(HooksListResponse {
        hooks: hooks_list,
        total,
    })
}

/// Register hooks from a declarative JSON bundle (rules and/or outbound webhooks).
#[tauri::command]
#[specta::specta]
pub async fn openclaw_hooks_register(
    ironclaw: State<'_, IronClawState>,
    input: HookRegisterInput,
) -> Result<HookRegisterResponse, String> {
    let agent = ironclaw.agent().await?;
    let hooks = agent.hooks();

    // Parse the JSON bundle
    let value: serde_json::Value =
        serde_json::from_str(&input.bundle_json).map_err(|e| format!("Invalid JSON: {}", e))?;

    let bundle = ironclaw::hooks::bundled::HookBundleConfig::from_value(&value)
        .map_err(|e| format!("Invalid hook bundle: {}", e))?;

    let source = input.source.unwrap_or_else(|| "ui".to_string());
    let summary = ironclaw::hooks::bundled::register_bundle(hooks, &source, bundle).await;

    Ok(HookRegisterResponse {
        ok: summary.errors == 0,
        hooks_registered: summary.hooks as u32,
        webhooks_registered: summary.outbound_webhooks as u32,
        errors: summary.errors as u32,
        message: if summary.errors > 0 {
            Some(format!("{} hook(s) failed validation", summary.errors))
        } else {
            Some(format!(
                "Registered {} hook(s) and {} webhook(s)",
                summary.hooks, summary.outbound_webhooks
            ))
        },
    })
}

/// Unregister (remove) a hook by name.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_hooks_unregister(
    ironclaw: State<'_, IronClawState>,
    hook_name: String,
) -> Result<HookUnregisterResponse, String> {
    let agent = ironclaw.agent().await?;
    let hooks = agent.hooks();
    let removed = hooks.unregister(&hook_name).await;

    Ok(HookUnregisterResponse {
        ok: removed,
        removed,
        message: if removed {
            Some(format!("Hook '{}' removed", hook_name))
        } else {
            Some(format!("Hook '{}' not found", hook_name))
        },
    })
}

// ============================================================================
// Extensions (plugins) management
// ============================================================================

/// List all installed extensions/plugins.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_extensions_list(
    ironclaw: State<'_, IronClawState>,
) -> Result<ExtensionsListResponse, String> {
    let agent = ironclaw.agent().await?;
    let ext_mgr = agent
        .extension_manager()
        .ok_or("Extension manager not available")?;

    let extensions = ironclaw::api::extensions::list_extensions(ext_mgr)
        .await
        .map_err(|e| e.to_string())?;

    let items: Vec<ExtensionInfoItem> = extensions
        .into_iter()
        .map(|ext| ExtensionInfoItem {
            name: ext.name,
            kind: ext.kind,
            description: ext.description,
            active: ext.active,
            authenticated: ext.authenticated,
            tools: ext.tools,
            needs_setup: ext.needs_setup,
            activation_status: ext.activation_status,
            activation_error: ext.activation_error,
        })
        .collect();

    let total = items.len() as u32;
    Ok(ExtensionsListResponse {
        extensions: items,
        total,
    })
}

/// Activate an extension by name.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_extension_activate(
    ironclaw: State<'_, IronClawState>,
    name: String,
) -> Result<ExtensionActionResponse, String> {
    let agent = ironclaw.agent().await?;
    let ext_mgr = agent
        .extension_manager()
        .ok_or("Extension manager not available")?;

    let resp = ironclaw::api::extensions::activate_extension(ext_mgr, &name)
        .await
        .map_err(|e| e.to_string())?;

    Ok(ExtensionActionResponse {
        ok: resp.success,
        message: Some(resp.message),
    })
}

/// Remove an extension by name.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_extension_remove(
    ironclaw: State<'_, IronClawState>,
    name: String,
) -> Result<ExtensionActionResponse, String> {
    let agent = ironclaw.agent().await?;
    let ext_mgr = agent
        .extension_manager()
        .ok_or("Extension manager not available")?;

    let resp = ironclaw::api::extensions::remove_extension(ext_mgr, &name)
        .await
        .map_err(|e| e.to_string())?;

    Ok(ExtensionActionResponse {
        ok: resp.success,
        message: Some(resp.message),
    })
}

// ============================================================================
// Diagnostics
// ============================================================================

#[tauri::command]
#[specta::specta]
pub async fn openclaw_diagnostics(
    ironclaw: State<'_, IronClawState>,
) -> Result<DiagnosticsResponse, String> {
    let mut checks = Vec::new();
    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut skipped = 0u32;

    // 1. IronClaw engine
    let engine_ok = ironclaw.agent().await.is_ok();
    if engine_ok {
        checks.push(DiagnosticCheck {
            name: "ThinClaw Engine".into(),
            status: "pass".into(),
            detail: "Agent is running and accessible".into(),
        });
        passed += 1;
    } else {
        checks.push(DiagnosticCheck {
            name: "ThinClaw Engine".into(),
            status: "fail".into(),
            detail: "Agent is not running".into(),
        });
        failed += 1;
    }

    if let Ok(agent) = ironclaw.agent().await {
        // 2. Database
        if let Some(store) = agent.store() {
            // Try listing settings to verify DB health
            match ironclaw::api::config::list_settings(store, "local_user").await {
                Ok(_) => {
                    checks.push(DiagnosticCheck {
                        name: "Database".into(),
                        status: "pass".into(),
                        detail: "Connected and responding to queries".into(),
                    });
                    passed += 1;
                }
                Err(e) => {
                    checks.push(DiagnosticCheck {
                        name: "Database".into(),
                        status: "fail".into(),
                        detail: format!("Query failed: {}", e),
                    });
                    failed += 1;
                }
            }
        } else {
            checks.push(DiagnosticCheck {
                name: "Database".into(),
                status: "skip".into(),
                detail: "No database configured (ephemeral mode)".into(),
            });
            skipped += 1;
        }

        // 3. Workspace
        if agent.workspace().is_some() {
            checks.push(DiagnosticCheck {
                name: "Workspace".into(),
                status: "pass".into(),
                detail: "Workspace directory available".into(),
            });
            passed += 1;
        } else {
            checks.push(DiagnosticCheck {
                name: "Workspace".into(),
                status: "warn".into(),
                detail: "No workspace configured (memory tools unavailable)".into(),
            });
            skipped += 1;
        }

        // 4. Tools
        let tool_count = agent.tools().count();
        if tool_count > 0 {
            checks.push(DiagnosticCheck {
                name: "Tool Registry".into(),
                status: "pass".into(),
                detail: format!("{} tools registered", tool_count),
            });
            passed += 1;
        } else {
            checks.push(DiagnosticCheck {
                name: "Tool Registry".into(),
                status: "warn".into(),
                detail: "No tools registered".into(),
            });
            skipped += 1;
        }

        // 5. Hooks
        let hook_count = agent.hooks().list_with_details().await.len();
        checks.push(DiagnosticCheck {
            name: "Hook Registry".into(),
            status: "pass".into(),
            detail: format!("{} hooks registered", hook_count),
        });
        passed += 1;

        // 6. Extensions
        if let Some(ext_mgr) = agent.extension_manager() {
            match ironclaw::api::extensions::list_extensions(ext_mgr).await {
                Ok(resp) => {
                    let active = resp.iter().filter(|e| e.active).count();
                    checks.push(DiagnosticCheck {
                        name: "Extensions".into(),
                        status: "pass".into(),
                        detail: format!("{} installed, {} active", resp.len(), active),
                    });
                    passed += 1;
                }
                Err(e) => {
                    checks.push(DiagnosticCheck {
                        name: "Extensions".into(),
                        status: "warn".into(),
                        detail: format!("Could not list: {}", e),
                    });
                    skipped += 1;
                }
            }
        } else {
            checks.push(DiagnosticCheck {
                name: "Extensions".into(),
                status: "skip".into(),
                detail: "Extension manager not available".into(),
            });
            skipped += 1;
        }

        // 7. Skills
        if let Some(registry) = agent.skill_registry() {
            match ironclaw::api::skills::list_skills(registry).await {
                Ok(resp) => {
                    checks.push(DiagnosticCheck {
                        name: "Skills".into(),
                        status: "pass".into(),
                        detail: format!("{} skills loaded", resp.skills.len()),
                    });
                    passed += 1;
                }
                Err(e) => {
                    checks.push(DiagnosticCheck {
                        name: "Skills".into(),
                        status: "warn".into(),
                        detail: format!("Could not list: {}", e),
                    });
                    skipped += 1;
                }
            }
        } else {
            checks.push(DiagnosticCheck {
                name: "Skills".into(),
                status: "skip".into(),
                detail: "Skill registry not available".into(),
            });
            skipped += 1;
        }
    }

    Ok(DiagnosticsResponse {
        checks,
        passed,
        failed,
        skipped,
    })
}

// ============================================================================
// Tool Listing
// ============================================================================

#[tauri::command]
#[specta::specta]
pub async fn openclaw_tools_list(
    ironclaw: State<'_, IronClawState>,
) -> Result<ToolsListResponse, String> {
    let agent = ironclaw.agent().await?;
    let registry = agent.tools();

    // Load the disabled-tools deny-list from settings (default: empty = all enabled).
    let disabled_tools: std::collections::HashSet<String> = if let Some(store) = agent.store() {
        if let Ok(Some(val)) = store.get_setting("local_user", "disabled_tools").await {
            let v: Vec<String> = serde_json::from_value(val).unwrap_or_default();
            v.into_iter().collect()
        } else {
            std::collections::HashSet::new()
        }
    } else {
        std::collections::HashSet::new()
    };

    let tool_defs = registry.tool_definitions().await;
    let tools: Vec<ToolInfoItem> = tool_defs
        .iter()
        .map(|td| {
            // Determine source from tool name heuristics
            let source = if ["echo", "time", "json", "device_info", "http", "browser"]
                .contains(&td.name.as_str())
            {
                "builtin"
            } else if [
                "shell",
                "read_file",
                "write_file",
                "list_dir",
                "apply_patch",
            ]
            .contains(&td.name.as_str())
            {
                "container"
            } else if [
                "memory_search",
                "memory_write",
                "memory_read",
                "memory_tree",
            ]
            .contains(&td.name.as_str())
            {
                "memory"
            } else if td.name.starts_with("tool_")
                || td.name.starts_with("skill_")
                || td.name.starts_with("routine_")
            {
                "management"
            } else {
                "extension"
            };

            ToolInfoItem {
                name: td.name.clone(),
                description: td.description.clone(),
                enabled: !disabled_tools.contains(&td.name),
                source: source.to_string(),
            }
        })
        .collect();

    let total = tools.len() as u32;
    Ok(ToolsListResponse { tools, total })
}

/// Get the set of globally disabled tools (deny-list stored in settings).
#[tauri::command]
#[specta::specta]
pub async fn openclaw_tool_policy_get(
    ironclaw: State<'_, IronClawState>,
) -> Result<Vec<String>, String> {
    let agent = ironclaw.agent().await?;
    let store = agent
        .store()
        .ok_or_else(|| "Settings store not available".to_string())?;

    let disabled: Vec<String> =
        if let Ok(Some(val)) = store.get_setting("local_user", "disabled_tools").await {
            serde_json::from_value(val).unwrap_or_default()
        } else {
            Vec::new()
        };

    Ok(disabled)
}

/// Set (overwrite) the list of globally disabled tools.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_tool_policy_set(
    ironclaw: State<'_, IronClawState>,
    disabled_tools: Vec<String>,
) -> Result<(), String> {
    let agent = ironclaw.agent().await?;
    let store = agent
        .store()
        .ok_or_else(|| "Settings store not available".to_string())?;

    let val = serde_json::to_value(&disabled_tools).map_err(|e| e.to_string())?;
    store
        .set_setting("local_user", "disabled_tools", &val)
        .await
        .map_err(|e| e.to_string())
}

// ============================================================================
// DM Pairing Management
// ============================================================================

#[tauri::command]
#[specta::specta]
pub async fn openclaw_pairing_list(channel: String) -> Result<PairingListResponse, String> {
    let store = ironclaw::pairing::PairingStore::new();

    // Collect pending pairing requests
    let pending = store
        .list_pending(&channel)
        .map_err(|e| format!("Failed to list pairings: {}", e))?;

    let mut pairings: Vec<PairingItem> = pending
        .iter()
        .map(|req| PairingItem {
            channel: channel.clone(),
            user_id: req.id.clone(),
            paired_at: req.created_at.clone(),
            status: "pending".to_string(),
        })
        .collect();

    // Also include approved senders from allowFrom list
    if let Ok(allowed) = store.read_allow_from(&channel) {
        for user_id in allowed {
            pairings.push(PairingItem {
                channel: channel.clone(),
                user_id,
                paired_at: String::new(),
                status: "active".to_string(),
            });
        }
    }

    let total = pairings.len() as u32;
    Ok(PairingListResponse { pairings, total })
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_pairing_approve(
    channel: String,
    code: String,
) -> Result<serde_json::Value, String> {
    let store = ironclaw::pairing::PairingStore::new();
    store
        .approve(&channel, &code)
        .map_err(|e| format!("Failed to approve pairing: {}", e))?;
    Ok(serde_json::json!({ "ok": true }))
}

// ============================================================================
// Context Compaction
// ============================================================================

#[tauri::command]
#[specta::specta]
pub async fn openclaw_compact_session(
    ironclaw: State<'_, IronClawState>,
    _session_key: String,
) -> Result<CompactSessionResponse, String> {
    let agent = ironclaw.agent().await?;

    // Get the session and thread to check turn count
    let session_mgr = agent.session_manager();
    let session = session_mgr.get_or_create_session("local_user").await;
    let sess = session.lock().await;

    // Count total turns across threads
    let total_turns: usize = sess.threads.values().map(|t| t.turns.len()).sum();

    if total_turns <= 2 {
        return Ok(CompactSessionResponse {
            tokens_before: 0,
            tokens_after: 0,
            turns_removed: 0,
            summary: Some("Session too short to compact".into()),
        });
    }

    // Estimate "tokens" from turn text length (rough: 1 token ≈ 4 chars)
    let est_tokens_before: u32 = sess
        .threads
        .values()
        .flat_map(|t| t.turns.iter())
        .map(|turn| {
            let input_len = turn.user_input.len();
            let response_len = turn.response.as_ref().map(|r| r.len()).unwrap_or(0);
            ((input_len + response_len) / 4) as u32
        })
        .sum();

    // For now return the estimate — actual compaction happens automatically
    // when context hits 80% capacity in the agent loop
    let keep_recent = 3;
    let turns_to_remove = total_turns.saturating_sub(keep_recent);

    Ok(CompactSessionResponse {
        tokens_before: est_tokens_before,
        tokens_after: est_tokens_before
            .saturating_sub(est_tokens_before * turns_to_remove as u32 / total_turns as u32),
        turns_removed: turns_to_remove as u32,
        summary: Some(format!(
            "Estimated compaction: {} turns would be removed, keeping {} recent turns",
            turns_to_remove, keep_recent
        )),
    })
}
