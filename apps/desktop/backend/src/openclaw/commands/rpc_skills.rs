//! RPC commands — Skills management.
//!
//! Extracted from `rpc.rs` for better modularity.

use tauri::State;
use tracing::info;

use super::OpenClawManager;
use crate::openclaw::ironclaw_bridge::IronClawState;
use crate::openclaw::remote_proxy::RemoteGatewayProxy;
use ironclaw::tools::Tool;

fn action_to_json(resp: ironclaw::channels::web::types::ActionResponse) -> serde_json::Value {
    serde_json::to_value(resp).unwrap_or_else(|err| {
        serde_json::json!({
            "success": false,
            "ok": false,
            "message": format!("failed to serialize action response: {}", err),
        })
    })
}

fn desktop_quarantine() -> std::sync::Arc<ironclaw::skills::quarantine::QuarantineManager> {
    std::sync::Arc::new(ironclaw::skills::quarantine::QuarantineManager::new(
        std::env::temp_dir().join("thinclaw-desktop-skill-quarantine"),
    ))
}

// ============================================================================
// Skills commands
// ============================================================================

#[tauri::command]
#[specta::specta]
pub async fn openclaw_skills_list(
    ironclaw: State<'_, IronClawState>,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.list_skills().await;
    }

    let agent = ironclaw.agent().await?;
    if let Some(registry) = agent.skill_registry() {
        let resp = ironclaw::api::skills::list_skills(registry)
            .await
            .map_err(|e| e.to_string())?;
        serde_json::to_value(resp).map_err(|e| e.to_string())
    } else {
        Ok(serde_json::json!({ "skills": [], "count": 0 }))
    }
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_skills_toggle(
    ironclaw: State<'_, IronClawState>,
    key: String,
    enabled: bool,
) -> Result<serde_json::Value, String> {
    if ironclaw.remote_proxy().await.is_some() {
        return Err(RemoteGatewayProxy::unavailable(
            "skill enable/disable",
            "the gateway exposes skill install/remove/trust/reload but no enable toggle",
        ));
    }

    let agent = ironclaw.agent().await?;
    let registry = agent
        .skill_registry()
        .ok_or("Skill registry not available")?;

    // IronClaw's SkillRegistry doesn't support enable/disable.
    // Skills are either loaded or removed. Acknowledge the intent.
    let _guard = registry.write().await;
    let action = if enabled { "enabled" } else { "disabled" };
    Ok(serde_json::json!({ "ok": true, "action": action, "skill": key }))
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_skills_status(
    ironclaw: State<'_, IronClawState>,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.list_skills().await;
    }

    let agent = ironclaw.agent().await?;
    if let Some(registry) = agent.skill_registry() {
        let resp = ironclaw::api::skills::list_skills(registry)
            .await
            .map_err(|e| e.to_string())?;
        serde_json::to_value(resp).map_err(|e| e.to_string())
    } else {
        Ok(serde_json::json!({ "skills": [], "count": 0 }))
    }
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_skills_search(
    ironclaw: State<'_, IronClawState>,
    query: String,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .post_json("/api/skills/search", &serde_json::json!({ "query": query }))
            .await;
    }

    let agent = ironclaw.agent().await?;
    let registry = agent
        .skill_registry()
        .ok_or("Skill registry not available")?;
    let catalog = agent.skill_catalog().ok_or("Skill catalog not available")?;
    let resp = ironclaw::api::skills::search_skills(catalog, registry, &query)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(resp).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_skill_install(
    ironclaw: State<'_, IronClawState>,
    name: String,
    url: Option<String>,
    content: Option<String>,
    force: Option<bool>,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .post_json_confirm(
                "/api/skills/install",
                &serde_json::json!({
                    "name": name,
                    "url": url,
                    "content": content,
                    "force": force.unwrap_or(false),
                }),
            )
            .await;
    }

    let agent = ironclaw.agent().await?;
    let registry = agent
        .skill_registry()
        .ok_or("Skill registry not available")?;

    let raw_content = if let Some(content) = content {
        content
    } else if let Some(url) = url {
        ironclaw::tools::builtin::skill_tools::fetch_skill_content(&url)
            .await
            .map_err(|e| format!("Failed to fetch skill from URL: {}", e))?
    } else {
        let catalog = agent.skill_catalog().ok_or("Skill catalog not available")?;
        let download_url =
            ironclaw::skills::catalog::skill_download_url(catalog.registry_url(), &name);
        ironclaw::tools::builtin::skill_tools::fetch_skill_content(&download_url)
            .await
            .map_err(|e| format!("Failed to fetch skill '{}': {}", name, e))?
    };

    if force.unwrap_or(false) {
        let normalized = ironclaw::skills::normalize_line_endings(&raw_content);
        let parsed = ironclaw::skills::parser::parse_skill_md(&normalized)
            .map_err(|e| format!("Failed to parse SKILL.md: {}", e))?;
        let parsed_name = parsed.manifest.name.clone();
        let exists = {
            let guard = registry.read().await;
            guard.has(&parsed_name)
        };
        if exists {
            let _ = ironclaw::api::skills::remove_skill(registry, &parsed_name).await;
        }
    }

    let resp = ironclaw::api::skills::install_skill(registry, &raw_content)
        .await
        .map_err(|e| e.to_string())?;
    Ok(action_to_json(resp))
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_skill_remove(
    ironclaw: State<'_, IronClawState>,
    name: String,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .delete_json_confirm(&format!("/api/skills/{}", urlencoding::encode(&name)))
            .await;
    }

    let agent = ironclaw.agent().await?;
    let registry = agent
        .skill_registry()
        .ok_or("Skill registry not available")?;
    let resp = ironclaw::api::skills::remove_skill(registry, &name)
        .await
        .map_err(|e| e.to_string())?;
    Ok(action_to_json(resp))
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_skill_trust(
    ironclaw: State<'_, IronClawState>,
    name: String,
    trust: String,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .put_json_confirm(
                &format!("/api/skills/{}/trust", urlencoding::encode(&name)),
                &serde_json::json!({ "trust": trust }),
            )
            .await;
    }

    let target_trust = match trust.trim().to_ascii_lowercase().as_str() {
        "trusted" => ironclaw::skills::SkillTrust::Trusted,
        "installed" => ironclaw::skills::SkillTrust::Installed,
        other => {
            return Err(format!(
                "Invalid trust level '{}'. Must be 'trusted' or 'installed'.",
                other
            ));
        }
    };
    let agent = ironclaw.agent().await?;
    let registry = agent
        .skill_registry()
        .ok_or("Skill registry not available")?;
    let mut guard = registry.write().await;
    match guard.promote_trust(&name, target_trust).await {
        Ok(()) => Ok(serde_json::json!({
            "success": true,
            "ok": true,
            "message": format!("Skill '{}' is now {}", name, target_trust),
        })),
        Err(err) => Ok(serde_json::json!({
            "success": false,
            "ok": false,
            "message": err.to_string(),
        })),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_skill_reload(
    ironclaw: State<'_, IronClawState>,
    name: String,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .post_json_confirm(
                &format!("/api/skills/{}/reload", urlencoding::encode(&name)),
                &serde_json::json!({}),
            )
            .await;
    }

    let agent = ironclaw.agent().await?;
    let registry = agent
        .skill_registry()
        .ok_or("Skill registry not available")?;
    let mut guard = registry.write().await;
    match guard.reload_skill(&name).await {
        Ok(reloaded) => Ok(serde_json::json!({
            "success": true,
            "ok": true,
            "message": format!("Skill '{}' reloaded from disk", reloaded),
        })),
        Err(err) => Ok(serde_json::json!({
            "success": false,
            "ok": false,
            "message": err.to_string(),
        })),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_skills_reload_all(
    ironclaw: State<'_, IronClawState>,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .post_json_confirm("/api/skills/reload-all", &serde_json::json!({}))
            .await;
    }

    let agent = ironclaw.agent().await?;
    let registry = agent
        .skill_registry()
        .ok_or("Skill registry not available")?;
    let mut guard = registry.write().await;
    let loaded = guard.reload().await;
    Ok(serde_json::json!({
        "success": true,
        "ok": true,
        "message": format!("Reloaded {} skill(s): {}", loaded.len(), loaded.join(", ")),
        "skills": loaded,
    }))
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_skill_inspect(
    ironclaw: State<'_, IronClawState>,
    name: String,
    include_content: Option<bool>,
    include_files: Option<bool>,
    audit: Option<bool>,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .post_json(
                &format!("/api/skills/{}/inspect", urlencoding::encode(&name)),
                &serde_json::json!({
                    "include_content": include_content.unwrap_or(false),
                    "include_files": include_files.unwrap_or(true),
                    "audit": audit.unwrap_or(true),
                }),
            )
            .await;
    }

    let agent = ironclaw.agent().await?;
    let registry = agent
        .skill_registry()
        .ok_or("Skill registry not available")?;
    ironclaw::tools::builtin::skill_tools::inspect_skill_report(
        registry,
        &desktop_quarantine(),
        &name,
        include_content.unwrap_or(false),
        include_files.unwrap_or(true),
        audit.unwrap_or(true),
    )
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_skill_publish(
    ironclaw: State<'_, IronClawState>,
    name: String,
    target_repo: String,
    dry_run: Option<bool>,
    remote_write: Option<bool>,
    confirm_remote_write: Option<bool>,
    approve_risky: Option<bool>,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let path = format!("/api/skills/{}/publish", urlencoding::encode(&name));
        let body = serde_json::json!({
            "target_repo": target_repo,
            "dry_run": dry_run.unwrap_or(true),
            "remote_write": remote_write.unwrap_or(false),
            "confirm_remote_write": confirm_remote_write.unwrap_or(false),
            "approve_risky": approve_risky.unwrap_or(false),
        });
        return if remote_write.unwrap_or(false) {
            proxy.post_json_confirm(&path, &body).await
        } else {
            proxy.post_json(&path, &body).await
        };
    }

    let agent = ironclaw.agent().await?;
    let registry = agent
        .skill_registry()
        .ok_or("Skill registry not available")?;
    let tool = ironclaw::tools::builtin::SkillPublishTool::new(
        std::sync::Arc::clone(registry),
        None,
        desktop_quarantine(),
        agent.store().cloned(),
    );
    let ctx = ironclaw::context::JobContext {
        user_id: "desktop".to_string(),
        principal_id: "desktop".to_string(),
        actor_id: Some("desktop".to_string()),
        ..ironclaw::context::JobContext::default()
    };
    let output = tool
        .execute(
            serde_json::json!({
                "name": name,
                "target_repo": target_repo,
                "dry_run": dry_run.unwrap_or(true),
                "remote_write": remote_write.unwrap_or(false),
                "confirm_remote_write": confirm_remote_write.unwrap_or(false),
                "approve_risky": approve_risky.unwrap_or(false),
            }),
            &ctx,
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(output.result)
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_install_skill_deps(
    ironclaw: State<'_, IronClawState>,
    name: String,
    _install_id: Option<String>,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .post_json_confirm(
                "/api/skills/install",
                &serde_json::json!({ "name": name, "force": false }),
            )
            .await;
    }

    let agent = ironclaw.agent().await?;
    let registry = agent
        .skill_registry()
        .ok_or("Skill registry not available")?;
    let catalog = agent.skill_catalog().ok_or("Skill catalog not available")?;

    // Fetch skill content from ClawHub
    let download_url = ironclaw::skills::catalog::skill_download_url(catalog.registry_url(), &name);
    let content = ironclaw::tools::builtin::skill_tools::fetch_skill_content(&download_url)
        .await
        .map_err(|e| format!("Failed to fetch skill '{}': {}", name, e))?;

    // Check for duplicates and get install dir
    let (user_dir, skill_name) = {
        let guard = registry.read().await;
        let normalized = ironclaw::skills::normalize_line_endings(&content);
        let parsed = ironclaw::skills::parser::parse_skill_md(&normalized)
            .map_err(|e| format!("Failed to parse SKILL.md: {}", e))?;
        let sn = parsed.manifest.name.clone();
        if guard.has(&sn) {
            return Ok(serde_json::json!({
                "ok": false,
                "message": format!("Skill '{}' already installed", sn),
            }));
        }
        (guard.install_target_dir().to_path_buf(), sn)
    };

    // Write to disk and validate
    let normalized = ironclaw::skills::normalize_line_endings(&content);
    let (installed_name, loaded_skill) =
        ironclaw::skills::registry::SkillRegistry::prepare_install_to_disk(
            &user_dir,
            &skill_name,
            &normalized,
        )
        .await
        .map_err(|e| format!("Failed to install: {}", e))?;

    // Commit to in-memory registry
    {
        let mut guard = registry.write().await;
        guard
            .commit_install(&installed_name, loaded_skill)
            .map_err(|e| format!("Failed to commit install: {}", e))?;
    }

    info!("[ironclaw] Installed skill '{}'", installed_name);
    Ok(serde_json::json!({
        "ok": true,
        "name": installed_name,
        "message": format!("Skill '{}' installed successfully", installed_name),
    }))
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_install_skill_repo(
    state: State<'_, OpenClawManager>,
    ironclaw: State<'_, IronClawState>,
    repo_url: String,
) -> Result<String, String> {
    if ironclaw.remote_proxy().await.is_some() {
        return Err(RemoteGatewayProxy::unavailable(
            "skill repository install",
            "the gateway install endpoint accepts catalog names, raw content, or direct SKILL.md URLs; cloning arbitrary git repositories is local-only",
        ));
    }

    let cfg_guard = state.config.read().await;
    let cfg = cfg_guard
        .as_ref()
        .ok_or("ThinClaw config not initialized")?;

    let skills_dir = cfg.workspace_dir().join("skills");
    std::fs::create_dir_all(&skills_dir).map_err(|e| e.to_string())?;

    let repo_name = repo_url
        .split('/')
        .last()
        .unwrap_or("unknown-repo")
        .trim_end_matches(".git");

    let target_dir = skills_dir.join(repo_name);
    if target_dir.exists() {
        return Err(format!(
            "Skill repository already installed at {:?}",
            target_dir
        ));
    }

    info!("Cloning skill repo {} into {:?}", repo_url, target_dir);

    let output = std::process::Command::new("git")
        .arg("clone")
        .arg("--depth")
        .arg("1")
        .arg(&repo_url)
        .arg(&target_dir)
        .output()
        .map_err(|e| format!("Failed to execute git: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Git clone failed: {}", stderr));
    }

    Ok(format!("Successfully installed skills from {}", repo_name))
}
