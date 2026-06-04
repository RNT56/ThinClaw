//! Skills management API handlers.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

use crate::channels::web::identity_helpers::GatewayRequestIdentity;
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;
use crate::context::JobContext;
use crate::tools::Tool;
use crate::tools::builtin::{
    root_skill_publish_tool_host, root_skill_tap_tool_host, root_skill_tool_host,
};
use thinclaw_gateway::web::skills::{
    SkillCatalogSearchResultInput, SkillInfoInput, SkillSearchMatchInput,
    has_confirm_action_header, invalid_skill_trust_level_error, skill_action_error_response,
    skill_catalog_search_result, skill_catalog_unavailable_error, skill_duplicate_response,
    skill_info, skill_install_commit_response, skill_install_confirmation_error,
    skill_install_missing_source_response, skill_list_response, skill_matches_query,
    skill_publish_remote_write_confirmation_error, skill_quarantine_unavailable_error,
    skill_reload_all_response, skill_reload_confirmation_error, skill_reload_response,
    skill_removal_confirmation_error, skill_remove_response, skill_search_response,
    skill_tap_add_confirmation_error, skill_tap_refresh_confirmation_error,
    skill_tap_remove_confirmation_error, skill_trust_confirmation_error, skill_trust_response,
    skills_system_unavailable_error,
};
use thinclaw_tools::builtin::{
    SkillInspectHostTool, SkillPublishHostTool, SkillTapAddHostTool, SkillTapListHostTool,
    SkillTapRefreshHostTool, SkillTapRemoveHostTool,
};

fn api_job_context(identity: &GatewayRequestIdentity) -> JobContext {
    JobContext {
        user_id: identity.principal_id.clone(),
        principal_id: identity.principal_id.clone(),
        actor_id: Some(identity.actor_id.clone()),
        ..JobContext::default()
    }
}

pub async fn skills_list_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<SkillListResponse>, (StatusCode, String)> {
    let registry = state
        .skill_registry
        .as_ref()
        .ok_or_else(skills_system_unavailable_error)?;

    let guard = registry.read().await;

    let skills: Vec<SkillInfo> = guard
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

    Ok(Json(skill_list_response(skills)))
}

pub async fn skills_search_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<SkillSearchRequest>,
) -> Result<Json<SkillSearchResponse>, (StatusCode, String)> {
    let registry = state
        .skill_registry
        .as_ref()
        .ok_or_else(skills_system_unavailable_error)?;

    let catalog = state
        .skill_catalog
        .as_ref()
        .ok_or_else(skill_catalog_unavailable_error)?;

    // Search ClawHub catalog
    let catalog_outcome = catalog.search(&req.query).await;
    let catalog_error = catalog_outcome.error.clone();

    // Enrich top results with detail data (stars, downloads, owner)
    let mut entries = catalog_outcome.results;
    catalog.enrich_search_results(&mut entries, 5).await;

    let catalog_results: Vec<SkillCatalogSearchResult> = entries
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
        .collect();

    // Search local skills
    let installed: Vec<SkillInfo> = {
        let guard = registry.read().await;
        guard
            .skills()
            .iter()
            .filter(|s| {
                skill_matches_query(
                    SkillSearchMatchInput {
                        name: &s.manifest.name,
                        description: &s.manifest.description,
                    },
                    &req.query,
                )
            })
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
            .collect()
    };

    Ok(Json(skill_search_response(
        catalog_results,
        installed,
        catalog.registry_url().to_string(),
        catalog_error,
    )))
}

pub async fn skills_inspect_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Path(name): Path<String>,
    Json(req): Json<SkillInspectRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let registry = state
        .skill_registry
        .as_ref()
        .ok_or_else(skills_system_unavailable_error)?;
    let quarantine = state
        .skill_quarantine
        .as_ref()
        .ok_or_else(skill_quarantine_unavailable_error)?;

    let tool = SkillInspectHostTool::new(root_skill_tool_host(
        Arc::clone(registry),
        Arc::clone(quarantine),
    ));
    let ctx = api_job_context(&request_identity);
    tool.execute(
        serde_json::json!({
            "name": name,
            "include_content": req.include_content.unwrap_or(false),
            "include_files": req.include_files.unwrap_or(true),
            "audit": req.audit.unwrap_or(true),
        }),
        &ctx,
    )
    .await
    .map(|output| Json(output.result))
    .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))
}

pub async fn skills_publish_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    headers: axum::http::HeaderMap,
    Path(name): Path<String>,
    Json(req): Json<SkillPublishRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let remote_write = req.remote_write.unwrap_or(false);
    let confirm_remote_write = req.confirm_remote_write.unwrap_or(false);
    if remote_write && !has_confirm_action_header(&headers) {
        return Err(skill_publish_remote_write_confirmation_error());
    }

    let registry = state
        .skill_registry
        .as_ref()
        .ok_or_else(skills_system_unavailable_error)?;
    let quarantine = state
        .skill_quarantine
        .as_ref()
        .ok_or_else(skill_quarantine_unavailable_error)?;

    let tool = SkillPublishHostTool::new(root_skill_publish_tool_host(
        Arc::clone(registry),
        state.skill_remote_hub.clone(),
        Arc::clone(quarantine),
        state.store.clone(),
    ));
    let params = serde_json::json!({
        "name": name,
        "target_repo": req.target_repo,
        "dry_run": req.dry_run.unwrap_or(true),
        "remote_write": remote_write,
        "confirm_remote_write": confirm_remote_write,
        "approve_risky": req.approve_risky.unwrap_or(false),
    });
    let ctx = api_job_context(&request_identity);
    tool.execute(params, &ctx)
        .await
        .map(|output| Json(output.result))
        .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))
}

pub async fn skill_taps_list_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let tool = SkillTapListHostTool::new(root_skill_tap_tool_host(
        state.store.clone(),
        state.skill_remote_hub.clone(),
    ));
    let ctx = api_job_context(&request_identity);
    tool.execute(serde_json::json!({"include_health": true}), &ctx)
        .await
        .map(|output| Json(output.result))
        .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))
}

pub async fn skill_taps_add_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    headers: axum::http::HeaderMap,
    Json(req): Json<SkillTapAddRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if !has_confirm_action_header(&headers) {
        return Err(skill_tap_add_confirmation_error());
    }
    let tool = SkillTapAddHostTool::new(root_skill_tap_tool_host(
        state.store.clone(),
        state.skill_remote_hub.clone(),
    ));
    let ctx = api_job_context(&request_identity);
    tool.execute(
        serde_json::json!({
            "repo": req.repo,
            "path": req.path.unwrap_or_default(),
            "branch": req.branch,
            "trust_level": req.trust_level.unwrap_or_else(|| "community".to_string()),
            "replace": req.replace.unwrap_or(false),
        }),
        &ctx,
    )
    .await
    .map(|output| Json(output.result))
    .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))
}

pub async fn skill_taps_remove_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    headers: axum::http::HeaderMap,
    Json(req): Json<SkillTapRemoveRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if !has_confirm_action_header(&headers) {
        return Err(skill_tap_remove_confirmation_error());
    }
    let tool = SkillTapRemoveHostTool::new(root_skill_tap_tool_host(
        state.store.clone(),
        state.skill_remote_hub.clone(),
    ));
    let ctx = api_job_context(&request_identity);
    tool.execute(
        serde_json::json!({
            "repo": req.repo,
            "path": req.path.unwrap_or_default(),
            "branch": req.branch,
        }),
        &ctx,
    )
    .await
    .map(|output| Json(output.result))
    .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))
}

pub async fn skill_taps_refresh_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    headers: axum::http::HeaderMap,
    Json(req): Json<SkillTapRefreshRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if !has_confirm_action_header(&headers) {
        return Err(skill_tap_refresh_confirmation_error());
    }
    let tool = SkillTapRefreshHostTool::new(root_skill_tap_tool_host(
        state.store.clone(),
        state.skill_remote_hub.clone(),
    ));
    let ctx = api_job_context(&request_identity);
    tool.execute(
        serde_json::json!({
            "repo": req.repo,
            "path": req.path,
        }),
        &ctx,
    )
    .await
    .map(|output| Json(output.result))
    .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))
}

pub async fn skills_install_handler(
    State(state): State<Arc<GatewayState>>,
    headers: axum::http::HeaderMap,
    Json(req): Json<SkillInstallRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    // Require explicit confirmation header to prevent accidental installs.
    // Chat tools have requires_approval(); this is the equivalent for the web API.
    if !has_confirm_action_header(&headers) {
        return Err(skill_install_confirmation_error());
    }

    let registry = state
        .skill_registry
        .as_ref()
        .ok_or_else(skills_system_unavailable_error)?;

    // Check whether the caller wants to force-update an existing skill.
    let force = req.force.unwrap_or(false);

    let content = if let Some(ref raw) = req.content {
        raw.clone()
    } else if let Some(ref url) = req.url {
        // Fetch from explicit URL (with SSRF protection)
        crate::tools::builtin::skill_tools::fetch_skill_content(url)
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
    } else if let Some(ref catalog) = state.skill_catalog {
        let url = crate::skills::catalog::skill_download_url(catalog.registry_url(), &req.name);
        crate::tools::builtin::skill_tools::fetch_skill_content(&url)
            .await
            .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?
    } else {
        return Ok(Json(skill_install_missing_source_response()));
    };

    // Parse to extract the skill name (cheap, in-memory).
    let normalized = crate::skills::normalize_line_endings(&content);
    let parsed = crate::skills::parser::parse_skill_md(&normalized)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let skill_name_from_parse = parsed.manifest.name.clone();

    // Check duplicates and optionally remove the old version under a brief read lock.
    let user_dir = {
        let guard = registry.read().await;

        if guard.has(&skill_name_from_parse) && !force {
            return Ok(Json(skill_duplicate_response(&skill_name_from_parse)));
        }

        guard.install_target_dir().to_path_buf()
    };

    // ── Force-update: remove old version first ─────────────────────────
    // When force=true and the skill exists, remove it atomically so the
    // subsequent install succeeds. This is the "update" path.
    if force {
        let mut guard = registry.write().await;
        if guard.has(&skill_name_from_parse) {
            // Best-effort removal: validate + delete files + commit.
            // If any step fails, fall through — the install will fail with
            // AlreadyExists, which is the correct behavior.
            if let Ok(path) = guard.validate_remove(&skill_name_from_parse) {
                let _ = crate::skills::registry::SkillRegistry::delete_skill_files(&path).await;
                let _ = guard.commit_remove(&skill_name_from_parse);
                tracing::info!(
                    skill = %skill_name_from_parse,
                    "Force-update: removed previous version"
                );
            }
        }
    }

    // Perform async I/O (write to disk, load) with no lock held.
    let (skill_name, loaded_skill) =
        crate::skills::registry::SkillRegistry::prepare_install_to_disk(
            &user_dir,
            &skill_name_from_parse,
            &normalized,
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Commit: brief write lock for in-memory addition.
    // On failure, clean up the orphaned disk files from prepare_install_to_disk.
    let mut guard = registry.write().await;

    match guard.commit_install(&skill_name, loaded_skill) {
        Ok(()) => Ok(Json(skill_install_commit_response(&skill_name, force))),
        Err(e) => {
            // ── TOCTOU cleanup ─────────────────────────────────────────
            // Another concurrent request installed the same skill between
            // prepare_install_to_disk and commit_install. Clean up the
            // orphaned files we wrote to disk.
            let orphan_dir = user_dir.join(&skill_name);
            if orphan_dir.exists() {
                tracing::warn!(
                    skill = %skill_name,
                    "Cleaning up orphaned skill files after failed commit"
                );
                let _ =
                    crate::skills::registry::SkillRegistry::delete_skill_files(&orphan_dir).await;
            }
            Ok(Json(skill_action_error_response(e.to_string())))
        }
    }
}

pub async fn skills_remove_handler(
    State(state): State<Arc<GatewayState>>,
    headers: axum::http::HeaderMap,
    Path(name): Path<String>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    // Require explicit confirmation header to prevent accidental removals.
    if !has_confirm_action_header(&headers) {
        return Err(skill_removal_confirmation_error());
    }

    let registry = state
        .skill_registry
        .as_ref()
        .ok_or_else(skills_system_unavailable_error)?;

    // ── TOCTOU fix ─────────────────────────────────────────────────────
    // Hold the write lock for the entire validate → delete → commit
    // sequence. This prevents concurrent remove+install races where a
    // new install could land files that get incorrectly deleted.
    // The file I/O inside delete_skill_files is fast (single file +
    // rmdir) so lock contention is negligible.
    let mut guard = registry.write().await;

    let skill_path = guard
        .validate_remove(&name)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    crate::skills::registry::SkillRegistry::delete_skill_files(&skill_path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    match guard.commit_remove(&name) {
        Ok(()) => Ok(Json(skill_remove_response(&name))),
        Err(e) => Ok(Json(skill_action_error_response(e.to_string()))),
    }
}

pub async fn skills_trust_handler(
    State(state): State<Arc<GatewayState>>,
    headers: axum::http::HeaderMap,
    Path(name): Path<String>,
    Json(req): Json<SkillTrustRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    // Require explicit confirmation — changing trust is a security-sensitive action.
    if !has_confirm_action_header(&headers) {
        return Err(skill_trust_confirmation_error());
    }

    let registry = state
        .skill_registry
        .as_ref()
        .ok_or_else(skills_system_unavailable_error)?;

    // Parse the target trust level from the request string.
    let target_trust = match req.trust.to_lowercase().as_str() {
        "trusted" => crate::skills::SkillTrust::Trusted,
        "installed" => crate::skills::SkillTrust::Installed,
        other => {
            return Err(invalid_skill_trust_level_error(other));
        }
    };

    let mut guard = registry.write().await;

    match guard.promote_trust(&name, target_trust).await {
        Ok(()) => {
            let label = target_trust.to_string();
            Ok(Json(skill_trust_response(&name, &label)))
        }
        Err(e) => Ok(Json(skill_action_error_response(e.to_string()))),
    }
}

/// POST /api/skills/:name/reload — hot-reload a single skill from disk.
///
/// Re-reads the SKILL.md from its current location and replaces the
/// in-memory entry without touching other skills. Call this after
/// manually editing a skill file on disk.
pub async fn skills_reload_handler(
    State(state): State<Arc<GatewayState>>,
    headers: axum::http::HeaderMap,
    Path(name): Path<String>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    if !has_confirm_action_header(&headers) {
        return Err(skill_reload_confirmation_error());
    }

    let registry = state
        .skill_registry
        .as_ref()
        .ok_or_else(skills_system_unavailable_error)?;

    let mut guard = registry.write().await;
    match guard.reload_skill(&name).await {
        Ok(reloaded) => Ok(Json(skill_reload_response(reloaded))),
        Err(e) => Ok(Json(skill_action_error_response(e.to_string()))),
    }
}

/// POST /api/skills/reload-all — clear and re-discover all skills from disk.
///
/// Use after adding new SKILL.md files on disk (which can't be picked up
/// by the single-skill reload since they aren't in the registry yet).
pub async fn skills_reload_all_handler(
    State(state): State<Arc<GatewayState>>,
    headers: axum::http::HeaderMap,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    if !has_confirm_action_header(&headers) {
        return Err(skill_reload_confirmation_error());
    }

    let registry = state
        .skill_registry
        .as_ref()
        .ok_or_else(skills_system_unavailable_error)?;

    let mut guard = registry.write().await;
    let loaded = guard.reload().await;
    Ok(Json(skill_reload_all_response(&loaded)))
}
