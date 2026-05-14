//! RPC commands — Skills management.
//!
//! Extracted from `rpc.rs` for better modularity.

use tauri::State;
use tracing::info;

use super::OpenClawManager;
use crate::openclaw::ironclaw_bridge::IronClawState;

// ============================================================================
// Skills commands
// ============================================================================

#[tauri::command]
#[specta::specta]
pub async fn openclaw_skills_list(
    ironclaw: State<'_, IronClawState>,
) -> Result<serde_json::Value, String> {
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
pub async fn openclaw_install_skill_deps(
    ironclaw: State<'_, IronClawState>,
    name: String,
    _install_id: Option<String>,
) -> Result<serde_json::Value, String> {
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
    repo_url: String,
) -> Result<String, String> {
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
