//! Skills API — list, search, install, remove skills.
//!
//! Extracted from `channels/web/handlers/skills.rs`.
//!
//! NOTE: `SkillRegistry` is behind `Arc<RwLock<SkillRegistry>>` in the agent
//! deps, so the API takes that wrapped type. The `SkillCatalog` has its own
//! query/install interface.

use std::sync::Arc;

use crate::channels::web::types::*;
use crate::skills::SkillRegistry;
use crate::skills::catalog::SkillCatalog;

use super::error::ApiResult;

/// List installed skills with their metadata.
pub fn list_skills(
    skill_registry: &std::sync::RwLock<SkillRegistry>,
) -> ApiResult<SkillListResponse> {
    let registry = skill_registry.read().unwrap();
    let skills = registry.skills();

    let skill_infos: Vec<SkillInfo> = skills
        .iter()
        .map(|s| SkillInfo {
            name: s.manifest.name.clone(),
            description: s.manifest.description.clone(),
            version: s.manifest.version.clone(),
            trust: format!("{:?}", s.trust),
            source: format!("{:?}", s.source),
            keywords: s.manifest.activation.keywords.clone(),
        })
        .collect();

    let count = skill_infos.len();
    Ok(SkillListResponse {
        skills: skill_infos,
        count,
    })
}

/// Search the skill catalog and installed skills.
pub async fn search_skills(
    skill_catalog: &Arc<SkillCatalog>,
    skill_registry: &std::sync::RwLock<SkillRegistry>,
    query: &str,
) -> ApiResult<SkillSearchResponse> {
    let outcome = skill_catalog.search(query).await;

    let registry = skill_registry.read().unwrap();
    let installed: Vec<SkillInfo> = registry
        .skills()
        .iter()
        .map(|s| SkillInfo {
            name: s.manifest.name.clone(),
            description: s.manifest.description.clone(),
            version: s.manifest.version.clone(),
            trust: format!("{:?}", s.trust),
            source: format!("{:?}", s.source),
            keywords: s.manifest.activation.keywords.clone(),
        })
        .collect();

    Ok(SkillSearchResponse {
        catalog: outcome
            .results
            .into_iter()
            .filter_map(|e| serde_json::to_value(e).ok())
            .collect(),
        installed,
        registry_url: skill_catalog.registry_url().to_string(),
        catalog_error: outcome.error,
    })
}

/// Install a skill from content.
///
/// The write lock is held across `.await` because `SkillRegistry::install_skill`
/// requires `&mut self`. This is acceptable for an infrequent admin operation.
#[allow(clippy::await_holding_lock, clippy::readonly_write_lock)]
pub async fn install_skill(
    skill_registry: &std::sync::RwLock<SkillRegistry>,
    content: &str,
) -> ApiResult<ActionResponse> {
    let mut registry = skill_registry.write().unwrap();
    match registry.install_skill(content).await {
        Ok(name) => Ok(ActionResponse::ok(format!("Installed skill '{}'", name))),
        Err(e) => Ok(ActionResponse::fail(e.to_string())),
    }
}

/// Remove a skill by name.
#[allow(clippy::await_holding_lock, clippy::readonly_write_lock)]
pub async fn remove_skill(
    skill_registry: &std::sync::RwLock<SkillRegistry>,
    name: &str,
) -> ApiResult<ActionResponse> {
    let mut registry = skill_registry.write().unwrap();
    match registry.remove_skill(name).await {
        Ok(()) => Ok(ActionResponse::ok(format!("Removed skill '{}'", name))),
        Err(e) => Ok(ActionResponse::fail(e.to_string())),
    }
}
