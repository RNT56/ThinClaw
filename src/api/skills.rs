//! Skills API — list, search, install, remove skills.
//!
//! Extracted from `channels/web/handlers/skills.rs`.
//!
//! NOTE: `SkillRegistry` is behind `Arc<tokio::sync::RwLock<SkillRegistry>>` in agent
//! deps. Using `tokio::sync::RwLock` ensures locks are never held across `.await`
//! points on a Tokio worker thread (std::sync::RwLock would block the executor).

use std::sync::Arc;

use crate::channels::web::types::*;
use crate::skills::SkillRegistry;
use crate::skills::catalog::SkillCatalog;
use thinclaw_gateway::web::skills::{
    SkillCatalogSearchResultInput, SkillInfoInput, skill_action_error_response,
    skill_api_install_response, skill_api_remove_response, skill_catalog_search_result, skill_info,
    skill_list_response, skill_search_response,
};

use super::error::ApiResult;

/// List installed skills with their metadata.
pub async fn list_skills(
    skill_registry: &tokio::sync::RwLock<SkillRegistry>,
) -> ApiResult<SkillListResponse> {
    let registry = skill_registry.read().await;
    let skills = registry.skills();

    let skill_infos: Vec<SkillInfo> = skills
        .iter()
        .map(|s| {
            skill_info(SkillInfoInput {
                name: s.manifest.name.clone(),
                description: s.manifest.description.clone(),
                version: s.manifest.version.clone(),
                trust: s.trust.to_string(),
                source: format!("{:?}", s.source),
                keywords: s.manifest.activation.keywords.clone(),
            })
        })
        .collect();

    Ok(skill_list_response(skill_infos))
}

/// Search the skill catalog and installed skills.
pub async fn search_skills(
    skill_catalog: &Arc<SkillCatalog>,
    skill_registry: &tokio::sync::RwLock<SkillRegistry>,
    query: &str,
) -> ApiResult<SkillSearchResponse> {
    let outcome = skill_catalog.search(query).await;

    let registry = skill_registry.read().await;
    let installed: Vec<SkillInfo> = registry
        .skills()
        .iter()
        .map(|s| {
            skill_info(SkillInfoInput {
                name: s.manifest.name.clone(),
                description: s.manifest.description.clone(),
                version: s.manifest.version.clone(),
                trust: s.trust.to_string(),
                source: format!("{:?}", s.source),
                keywords: s.manifest.activation.keywords.clone(),
            })
        })
        .collect();

    Ok(skill_search_response(
        outcome
            .results
            .into_iter()
            .map(|entry| {
                skill_catalog_search_result(SkillCatalogSearchResultInput {
                    slug: entry.slug,
                    name: entry.name,
                    description: entry.description,
                    version: entry.version,
                    score: entry.score,
                    updated_at: entry.updated_at,
                    stars: entry.stars,
                    downloads: entry.downloads,
                    owner: entry.owner,
                })
            })
            .collect(),
        installed,
        skill_catalog.registry_url().to_string(),
        outcome.error,
    ))
}

/// Install a skill from content.
pub async fn install_skill(
    skill_registry: &tokio::sync::RwLock<SkillRegistry>,
    content: &str,
) -> ApiResult<ActionResponse> {
    let mut registry = skill_registry.write().await;
    match registry.install_skill(content).await {
        Ok(name) => Ok(skill_api_install_response(name)),
        Err(e) => Ok(skill_action_error_response(e.to_string())),
    }
}

/// Remove a skill by name.
pub async fn remove_skill(
    skill_registry: &tokio::sync::RwLock<SkillRegistry>,
    name: &str,
) -> ApiResult<ActionResponse> {
    let mut registry = skill_registry.write().await;
    match registry.remove_skill(name).await {
        Ok(()) => Ok(skill_api_remove_response(name)),
        Err(e) => Ok(skill_action_error_response(e.to_string())),
    }
}
