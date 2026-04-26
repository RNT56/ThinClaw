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
    SkillPublishTool, SkillTapAddTool, SkillTapListTool, SkillTapRefreshTool, SkillTapRemoveTool,
};

fn confirmed(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get("x-confirm-action")
        .and_then(|v| v.to_str().ok())
        == Some("true")
}

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
    let registry = state.skill_registry.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Skills system not enabled".to_string(),
    ))?;

    let guard = registry.read().await;

    let skills: Vec<SkillInfo> = guard
        .skills()
        .iter()
        .map(|s| SkillInfo {
            name: s.manifest.name.clone(),
            description: s.manifest.description.clone(),
            version: s.manifest.version.clone(),
            trust: s.trust.to_string(),
            source: format!("{:?}", s.source),
            keywords: s.manifest.activation.keywords.clone(),
        })
        .collect();

    let count = skills.len();
    Ok(Json(SkillListResponse { skills, count }))
}

pub async fn skills_search_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<SkillSearchRequest>,
) -> Result<Json<SkillSearchResponse>, (StatusCode, String)> {
    let registry = state.skill_registry.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Skills system not enabled".to_string(),
    ))?;

    let catalog = state.skill_catalog.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Skill catalog not available".to_string(),
    ))?;

    // Search ClawHub catalog
    let catalog_outcome = catalog.search(&req.query).await;
    let catalog_error = catalog_outcome.error.clone();

    // Enrich top results with detail data (stars, downloads, owner)
    let mut entries = catalog_outcome.results;
    catalog.enrich_search_results(&mut entries, 5).await;

    let catalog_json: Vec<serde_json::Value> = entries
        .into_iter()
        .map(|e| {
            serde_json::json!({
                "slug": e.slug,
                "name": e.name,
                "description": e.description,
                "version": e.version,
                "score": e.score,
                "updatedAt": e.updated_at,
                "stars": e.stars,
                "downloads": e.downloads,
                "owner": e.owner,
            })
        })
        .collect();

    // Search local skills
    let query_lower = req.query.to_lowercase();
    let installed: Vec<SkillInfo> = {
        let guard = registry.read().await;
        guard
            .skills()
            .iter()
            .filter(|s| {
                s.manifest.name.to_lowercase().contains(&query_lower)
                    || s.manifest.description.to_lowercase().contains(&query_lower)
            })
            .map(|s| SkillInfo {
                name: s.manifest.name.clone(),
                description: s.manifest.description.clone(),
                version: s.manifest.version.clone(),
                trust: s.trust.to_string(),
                source: format!("{:?}", s.source),
                keywords: s.manifest.activation.keywords.clone(),
            })
            .collect()
    };

    Ok(Json(SkillSearchResponse {
        catalog: catalog_json,
        installed,
        registry_url: catalog.registry_url().to_string(),
        catalog_error,
    }))
}

pub async fn skills_inspect_handler(
    State(state): State<Arc<GatewayState>>,
    Path(name): Path<String>,
    Json(req): Json<SkillInspectRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let registry = state.skill_registry.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Skills system not enabled".to_string(),
    ))?;
    let quarantine = state.skill_quarantine.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Skill quarantine not available".to_string(),
    ))?;

    crate::tools::builtin::skill_tools::inspect_skill_report(
        registry,
        quarantine,
        &name,
        req.include_content.unwrap_or(false),
        req.include_files.unwrap_or(true),
        req.audit.unwrap_or(true),
    )
    .await
    .map(Json)
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
    if remote_write && !confirmed(&headers) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Skill publish remote write requires X-Confirm-Action: true header".to_string(),
        ));
    }

    let registry = state.skill_registry.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Skills system not enabled".to_string(),
    ))?;
    let quarantine = state.skill_quarantine.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Skill quarantine not available".to_string(),
    ))?;

    let tool = SkillPublishTool::new(
        Arc::clone(registry),
        state.skill_remote_hub.clone(),
        Arc::clone(quarantine),
        state.store.clone(),
    );
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
    let tool = SkillTapListTool::new(state.store.clone(), state.skill_remote_hub.clone());
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
    if !confirmed(&headers) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Skill tap add requires X-Confirm-Action: true header".to_string(),
        ));
    }
    let tool = SkillTapAddTool::new(state.store.clone(), state.skill_remote_hub.clone());
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
    if !confirmed(&headers) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Skill tap remove requires X-Confirm-Action: true header".to_string(),
        ));
    }
    let tool = SkillTapRemoveTool::new(state.store.clone(), state.skill_remote_hub.clone());
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
    if !confirmed(&headers) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Skill tap refresh requires X-Confirm-Action: true header".to_string(),
        ));
    }
    let tool = SkillTapRefreshTool::new(state.store.clone(), state.skill_remote_hub.clone());
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
    if headers
        .get("x-confirm-action")
        .and_then(|v| v.to_str().ok())
        != Some("true")
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Skill install requires X-Confirm-Action: true header".to_string(),
        ));
    }

    let registry = state.skill_registry.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Skills system not enabled".to_string(),
    ))?;

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
        return Ok(Json(ActionResponse::fail(
            "Provide 'content' or 'url' to install a skill".to_string(),
        )));
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
            return Ok(Json(ActionResponse::fail(format!(
                "Skill '{}' already exists (use force=true to update)",
                skill_name_from_parse
            ))));
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
        Ok(()) => {
            let action = if force { "updated" } else { "installed" };
            Ok(Json(ActionResponse::ok(format!(
                "Skill '{}' {}",
                skill_name, action
            ))))
        }
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
            Ok(Json(ActionResponse::fail(e.to_string())))
        }
    }
}

pub async fn skills_remove_handler(
    State(state): State<Arc<GatewayState>>,
    headers: axum::http::HeaderMap,
    Path(name): Path<String>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    // Require explicit confirmation header to prevent accidental removals.
    if headers
        .get("x-confirm-action")
        .and_then(|v| v.to_str().ok())
        != Some("true")
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Skill removal requires X-Confirm-Action: true header".to_string(),
        ));
    }

    let registry = state.skill_registry.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Skills system not enabled".to_string(),
    ))?;

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
        Ok(()) => Ok(Json(ActionResponse::ok(format!(
            "Skill '{}' removed",
            name
        )))),
        Err(e) => Ok(Json(ActionResponse::fail(e.to_string()))),
    }
}

pub async fn skills_trust_handler(
    State(state): State<Arc<GatewayState>>,
    headers: axum::http::HeaderMap,
    Path(name): Path<String>,
    Json(req): Json<SkillTrustRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    // Require explicit confirmation — changing trust is a security-sensitive action.
    if headers
        .get("x-confirm-action")
        .and_then(|v| v.to_str().ok())
        != Some("true")
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Trust changes require X-Confirm-Action: true header".to_string(),
        ));
    }

    let registry = state.skill_registry.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Skills system not enabled".to_string(),
    ))?;

    // Parse the target trust level from the request string.
    let target_trust = match req.trust.to_lowercase().as_str() {
        "trusted" => crate::skills::SkillTrust::Trusted,
        "installed" => crate::skills::SkillTrust::Installed,
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!(
                    "Invalid trust level '{}'. Must be 'trusted' or 'installed'.",
                    other
                ),
            ));
        }
    };

    let mut guard = registry.write().await;

    match guard.promote_trust(&name, target_trust).await {
        Ok(()) => {
            let label = target_trust.to_string();
            Ok(Json(ActionResponse::ok(format!(
                "Skill '{}' is now {}",
                name, label
            ))))
        }
        Err(e) => Ok(Json(ActionResponse::fail(e.to_string()))),
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
    if headers
        .get("x-confirm-action")
        .and_then(|v| v.to_str().ok())
        != Some("true")
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Skill reload requires X-Confirm-Action: true header".to_string(),
        ));
    }

    let registry = state.skill_registry.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Skills system not enabled".to_string(),
    ))?;

    let mut guard = registry.write().await;
    match guard.reload_skill(&name).await {
        Ok(reloaded) => Ok(Json(ActionResponse::ok(format!(
            "Skill '{}' reloaded from disk",
            reloaded
        )))),
        Err(e) => Ok(Json(ActionResponse::fail(e.to_string()))),
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
    if headers
        .get("x-confirm-action")
        .and_then(|v| v.to_str().ok())
        != Some("true")
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Skill reload requires X-Confirm-Action: true header".to_string(),
        ));
    }

    let registry = state.skill_registry.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Skills system not enabled".to_string(),
    ))?;

    let mut guard = registry.write().await;
    let loaded = guard.reload().await;
    Ok(Json(ActionResponse::ok(format!(
        "Reloaded {} skill(s): {}",
        loaded.len(),
        loaded.join(", ")
    ))))
}
