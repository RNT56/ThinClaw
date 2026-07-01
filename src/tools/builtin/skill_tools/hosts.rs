//! Skill tool: hosts.

use super::*;

fn tap_to_port(tap: &SkillTapConfig) -> ToolSkillTap {
    ToolSkillTap {
        repo: tap.repo.clone(),
        path: tap.path.clone(),
        branch: tap.branch.clone(),
        trust_level: tap_trust_to_port(tap.trust_level),
    }
}

pub struct RootSkillTapToolHost {
    store: Option<Arc<dyn Database>>,
    remote_hub: Option<SharedRemoteSkillHub>,
}

impl RootSkillTapToolHost {
    pub fn new(store: Option<Arc<dyn Database>>, remote_hub: Option<SharedRemoteSkillHub>) -> Self {
        Self { store, remote_hub }
    }
}

pub fn root_skill_tap_tool_host(
    store: Option<Arc<dyn Database>>,
    remote_hub: Option<SharedRemoteSkillHub>,
) -> Arc<dyn SkillTapToolHostPort> {
    Arc::new(RootSkillTapToolHost::new(store, remote_hub))
}

#[async_trait]
impl SkillTapToolHostPort for RootSkillTapToolHost {
    async fn list_skill_taps(
        &self,
        query: ToolSkillTapQuery,
    ) -> Result<ToolSkillTapList, ToolHostError> {
        let store = require_skill_tap_store(&self.store, "skill_tap_list")
            .map_err(tool_host_error_from_tool)?;
        let settings = load_settings_for_taps(store, tool_scope_user_id(&query.scope))
            .await
            .map_err(tool_host_error_from_tool)?;
        let hub_enabled = if query.include_health {
            match self.remote_hub.as_ref() {
                Some(hub) => Some(hub.is_enabled().await),
                None => Some(false),
            }
        } else {
            None
        };
        Ok(ToolSkillTapList::new(
            settings.skill_taps.iter().map(tap_to_port).collect(),
            hub_enabled,
        ))
    }

    async fn add_skill_tap(
        &self,
        request: ToolSkillTapAddRequest,
    ) -> Result<ToolSkillTapMutationResult, ToolHostError> {
        let store = require_skill_tap_store(&self.store, "skill_tap_add")
            .map_err(tool_host_error_from_tool)?;
        let remote_hub = require_shared_remote_hub(&self.remote_hub, "skill_tap_add")
            .map_err(tool_host_error_from_tool)?;
        let user_id = tool_scope_user_id(&request.scope);
        let trust_level = tap_trust_from_port(request.trust_level);
        let mut settings = load_settings_for_taps(store, user_id)
            .await
            .map_err(tool_host_error_from_tool)?;
        let existing_idx = settings.skill_taps.iter().position(|tap| {
            tap_key_matches(tap, &request.repo, &request.path, request.branch.as_deref())
        });
        match (existing_idx, request.replace) {
            (Some(idx), true) => {
                settings.skill_taps[idx] = SkillTapConfig {
                    repo: request.repo.clone(),
                    path: request.path.clone(),
                    branch: request.branch.clone(),
                    trust_level,
                };
            }
            (Some(_), false) => {
                return Err(ToolHostError::OperationFailed {
                    reason: format!(
                        "Skill tap '{}:{}' already exists; use replace=true to update it",
                        request.repo, request.path
                    ),
                });
            }
            (None, _) => settings.skill_taps.push(SkillTapConfig {
                repo: request.repo.clone(),
                path: request.path.clone(),
                branch: request.branch.clone(),
                trust_level,
            }),
        }
        persist_skill_taps(store, user_id, &settings.skill_taps)
            .await
            .map_err(tool_host_error_from_tool)?;
        let tap_count = refresh_remote_hub_from_settings(store, user_id, remote_hub)
            .await
            .map_err(tool_host_error_from_tool)?;
        let status = if existing_idx.is_some() {
            "replaced"
        } else {
            "added"
        };
        Ok(ToolSkillTapMutationResult {
            status: status.to_string(),
            tap: Some(ToolSkillTap {
                repo: request.repo,
                path: request.path,
                branch: request.branch,
                trust_level: request.trust_level,
            }),
            tap_count,
        })
    }

    async fn remove_skill_tap(
        &self,
        request: ToolSkillTapRemoveRequest,
    ) -> Result<ToolSkillTapMutationResult, ToolHostError> {
        let store = require_skill_tap_store(&self.store, "skill_tap_remove")
            .map_err(tool_host_error_from_tool)?;
        let remote_hub = require_shared_remote_hub(&self.remote_hub, "skill_tap_remove")
            .map_err(tool_host_error_from_tool)?;
        let user_id = tool_scope_user_id(&request.scope);
        let mut settings = load_settings_for_taps(store, user_id)
            .await
            .map_err(tool_host_error_from_tool)?;
        let before = settings.skill_taps.len();
        settings.skill_taps.retain(|tap| {
            !tap_key_matches(tap, &request.repo, &request.path, request.branch.as_deref())
        });
        if settings.skill_taps.len() == before {
            return Err(ToolHostError::OperationFailed {
                reason: format!("Skill tap '{}:{}' not found", request.repo, request.path),
            });
        }
        persist_skill_taps(store, user_id, &settings.skill_taps)
            .await
            .map_err(tool_host_error_from_tool)?;
        let tap_count = refresh_remote_hub_from_settings(store, user_id, remote_hub)
            .await
            .map_err(tool_host_error_from_tool)?;
        Ok(ToolSkillTapMutationResult {
            status: "removed".to_string(),
            tap: Some(ToolSkillTap {
                repo: request.repo,
                path: request.path,
                branch: request.branch,
                trust_level: ToolSkillTapTrust::Community,
            }),
            tap_count,
        })
    }

    async fn refresh_skill_taps(
        &self,
        request: ToolSkillTapRefreshRequest,
    ) -> Result<ToolSkillTapRefreshResult, ToolHostError> {
        let store = require_skill_tap_store(&self.store, "skill_tap_refresh")
            .map_err(tool_host_error_from_tool)?;
        let remote_hub = require_shared_remote_hub(&self.remote_hub, "skill_tap_refresh")
            .map_err(tool_host_error_from_tool)?;
        let user_id = tool_scope_user_id(&request.scope);

        if request.repo.is_some() || request.path.is_some() {
            let settings = load_settings_for_taps(store, user_id)
                .await
                .map_err(tool_host_error_from_tool)?;
            let matches = settings.skill_taps.iter().any(|tap| {
                let repo_matches = match request.repo.as_ref() {
                    Some(repo) => tap.repo.eq_ignore_ascii_case(repo),
                    None => true,
                };
                let path_matches = match request.path.as_ref() {
                    Some(path) => normalize_tap_path(&tap.path) == *path,
                    None => true,
                };
                repo_matches && path_matches
            });
            if !matches {
                return Err(ToolHostError::OperationFailed {
                    reason: "No configured skill tap matches the requested refresh filter"
                        .to_string(),
                });
            }
        }

        let tap_count = refresh_remote_hub_from_settings(store, user_id, remote_hub)
            .await
            .map_err(tool_host_error_from_tool)?;
        let hub_enabled = remote_hub.is_enabled().await;
        Ok(ToolSkillTapRefreshResult {
            status: "refreshed".to_string(),
            tap_count,
            repo: request.repo,
            path: request.path,
            hub_enabled,
        })
    }
}

// ── skill_inspect ───────────────────────────────────────────────────────

pub struct RootSkillSearchToolHost {
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
    catalog: Arc<SkillCatalog>,
    remote_hub: Option<SharedRemoteSkillHub>,
}

impl RootSkillSearchToolHost {
    pub fn new(
        registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
        catalog: Arc<SkillCatalog>,
        remote_hub: Option<SharedRemoteSkillHub>,
    ) -> Self {
        Self {
            registry,
            catalog,
            remote_hub,
        }
    }
}

pub fn root_skill_search_tool_host(
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
    catalog: Arc<SkillCatalog>,
    remote_hub: Option<SharedRemoteSkillHub>,
) -> Arc<dyn SkillSearchToolHostPort> {
    Arc::new(RootSkillSearchToolHost::new(registry, catalog, remote_hub))
}

#[async_trait]
impl SkillSearchToolHostPort for RootSkillSearchToolHost {
    async fn search_skills(
        &self,
        request: ToolSkillSearchRequest,
    ) -> Result<ToolSkillSearchResult, ToolHostError> {
        skill_policy::ensure_skill_admin_available(&request.scope.metadata, "skill_search")
            .map_err(tool_host_error_from_tool)?;

        let catalog_outcome = self.catalog.search(&request.query).await;
        let catalog_error = catalog_outcome.error.clone();
        let mut catalog_entries = catalog_outcome.results;
        self.catalog
            .enrich_search_results(&mut catalog_entries, 5)
            .await;

        let (installed_names, local) = {
            let guard = self.registry.read().await;
            let names = guard
                .skills()
                .iter()
                .map(|skill| skill.manifest.name.clone())
                .collect::<Vec<_>>();
            let query_lower = request.query.to_lowercase();
            let local = guard
                .skills()
                .iter()
                .filter(|skill| {
                    skill.manifest.name.to_lowercase().contains(&query_lower)
                        || skill
                            .manifest
                            .description
                            .to_lowercase()
                            .contains(&query_lower)
                        || skill
                            .manifest
                            .activation
                            .keywords
                            .iter()
                            .any(|keyword| keyword.to_lowercase().contains(&query_lower))
                })
                .map(|skill| ToolSkillSearchLocalEntry {
                    name: skill.manifest.name.clone(),
                    description: skill.manifest.description.clone(),
                    trust: skill.trust.to_string(),
                    source_tier: skill.source_tier.to_string(),
                })
                .collect::<Vec<_>>();
            (names, local)
        };

        let catalog = catalog_entries
            .into_iter()
            .map(|entry| {
                let installed = installed_names
                    .iter()
                    .any(|name| entry.slug.ends_with(name.as_str()) || entry.name == *name);
                ToolSkillSearchCatalogEntry {
                    slug: entry.slug,
                    name: entry.name,
                    description: entry.description,
                    version: entry.version,
                    score: entry.score,
                    installed,
                    stars: entry.stars,
                    downloads: entry.downloads,
                    owner: entry.owner,
                }
            })
            .collect::<Vec<_>>();

        let remote = if let Some(hub) = self.remote_hub.as_ref() {
            hub.search(&request.query)
                .await
                .into_iter()
                .map(|entry| ToolSkillSearchRemoteEntry {
                    slug: entry.slug,
                    name: entry.name,
                    description: entry.description,
                    version: entry.version,
                    source: entry.source_adapter,
                    source_label: entry.source_label,
                    source_ref: entry.source_ref,
                    manifest_url: entry.manifest_url,
                    manifest_digest: entry.manifest_digest,
                    repo: entry.repo,
                    path: entry.path,
                    branch: entry.branch,
                    trust_level: format!("{:?}", entry.trust_level).to_lowercase(),
                })
                .collect()
        } else {
            Vec::new()
        };

        Ok(ToolSkillSearchResult {
            catalog,
            remote,
            local,
            registry_url: self.catalog.registry_url().to_string(),
            catalog_error,
        })
    }
}

pub struct RootSkillInstallToolHost {
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
    catalog: Arc<SkillCatalog>,
    remote_hub: Option<SharedRemoteSkillHub>,
    quarantine: Arc<QuarantineManager>,
}

impl RootSkillInstallToolHost {
    pub fn new(
        registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
        catalog: Arc<SkillCatalog>,
        remote_hub: Option<SharedRemoteSkillHub>,
        quarantine: Arc<QuarantineManager>,
    ) -> Self {
        Self {
            registry,
            catalog,
            remote_hub,
            quarantine,
        }
    }
}

pub fn root_skill_install_tool_host(
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
    catalog: Arc<SkillCatalog>,
    remote_hub: Option<SharedRemoteSkillHub>,
    quarantine: Arc<QuarantineManager>,
) -> Arc<dyn SkillInstallToolHostPort> {
    Arc::new(RootSkillInstallToolHost::new(
        registry, catalog, remote_hub, quarantine,
    ))
}

#[async_trait]
impl SkillInstallToolHostPort for RootSkillInstallToolHost {
    async fn install_skill_action(
        &self,
        request: ToolSkillInstallActionRequest,
    ) -> Result<ToolSkillMutationActionResult, ToolHostError> {
        skill_policy::ensure_skill_admin_available(&request.scope.metadata, "skill_install")
            .map_err(tool_host_error_from_tool)?;
        let mut params = serde_json::json!({
            "name": request.name,
            "force": request.force,
            "approve_risky": request.approve_risky,
        });
        if let Some(url) = request.url {
            params["url"] = serde_json::Value::String(url);
        }
        if let Some(content) = request.content {
            params["content"] = serde_json::Value::String(content);
        }
        let ctx = job_context_from_tool_scope(request.scope, "skill_install");
        let tool = SkillInstallTool::new(
            Arc::clone(&self.registry),
            Arc::clone(&self.catalog),
            self.remote_hub.clone(),
            Arc::clone(&self.quarantine),
        );
        let output = tool
            .execute(params, &ctx)
            .await
            .map_err(tool_host_error_from_tool)?;
        Ok(ToolSkillMutationActionResult {
            output: output.result,
        })
    }

    async fn update_skill_action(
        &self,
        request: ToolSkillUpdateActionRequest,
    ) -> Result<ToolSkillMutationActionResult, ToolHostError> {
        skill_policy::ensure_skill_admin_available(&request.scope.metadata, "skill_update")
            .map_err(tool_host_error_from_tool)?;
        let ctx = job_context_from_tool_scope(request.scope, "skill_update");
        let tool = SkillUpdateTool::new(
            Arc::clone(&self.registry),
            Arc::clone(&self.catalog),
            self.remote_hub.clone(),
            Arc::clone(&self.quarantine),
        );
        let output = tool
            .execute(
                serde_json::json!({
                    "name": request.name,
                    "approve_risky": request.approve_risky,
                }),
                &ctx,
            )
            .await
            .map_err(tool_host_error_from_tool)?;
        Ok(ToolSkillMutationActionResult {
            output: output.result,
        })
    }
}

pub struct RootSkillToolHost {
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
    quarantine: Arc<QuarantineManager>,
}

impl RootSkillToolHost {
    pub fn new(
        registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
        quarantine: Arc<QuarantineManager>,
    ) -> Self {
        Self {
            registry,
            quarantine,
        }
    }
}

pub fn root_skill_tool_host(
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
    quarantine: Arc<QuarantineManager>,
) -> Arc<dyn SkillToolHostPort> {
    Arc::new(RootSkillToolHost::new(registry, quarantine))
}

#[async_trait]
impl SkillToolHostPort for RootSkillToolHost {
    async fn list_skills(
        &self,
        query: ToolSkillQuery,
    ) -> Result<Vec<ToolSkillSummary>, ToolHostError> {
        let allowed_skills = skill_policy::restricted_skill_names(&query.scope.metadata);
        let query_text = query.query.as_ref().map(|value| value.to_ascii_lowercase());
        let guard = self.registry.read().await;
        Ok(guard
            .skills()
            .iter()
            .filter(|skill| {
                allowed_skills
                    .as_ref()
                    .is_none_or(|allowed| allowed.contains(skill.manifest.name.as_str()))
            })
            .filter(|skill| {
                query_text.as_ref().is_none_or(|query| {
                    skill.manifest.name.to_ascii_lowercase().contains(query)
                        || skill
                            .manifest
                            .description
                            .to_ascii_lowercase()
                            .contains(query)
                })
            })
            .map(|skill| {
                let mut metadata = serde_json::json!({
                    "source_tier": skill.source_tier.to_string(),
                    "source": format!("{:?}", skill.source),
                    "keywords": skill.manifest.activation.keywords,
                    "version": skill.manifest.version,
                    "tags": skill.manifest.activation.tags,
                    "content_hash": skill.content_hash,
                    "max_context_tokens": skill.manifest.activation.max_context_tokens,
                });
                if let Some(openclaw) = skill
                    .manifest
                    .metadata
                    .as_ref()
                    .and_then(|metadata| metadata.openclaw.as_ref())
                    && let Some(object) = metadata.as_object_mut()
                {
                    object.insert(
                        "provenance".to_string(),
                        serde_json::json!(openclaw.provenance.clone()),
                    );
                    object.insert(
                        "lifecycle_status".to_string(),
                        serde_json::json!(openclaw.lifecycle_status.clone()),
                    );
                    object.insert(
                        "outcome_score".to_string(),
                        serde_json::json!(openclaw.outcome_score),
                    );
                    object.insert(
                        "reuse_count".to_string(),
                        serde_json::json!(openclaw.reuse_count),
                    );
                    object.insert(
                        "activation_reason".to_string(),
                        serde_json::json!(openclaw.activation_reason.clone()),
                    );
                }

                ToolSkillSummary {
                    name: skill.manifest.name.clone(),
                    description: Some(skill.manifest.description.clone()),
                    trust: skill_trust_to_port(skill.trust),
                    enabled: true,
                    metadata,
                }
            })
            .collect())
    }

    async fn inspect_skill(
        &self,
        _scope: ToolOperationScope,
        name: String,
        include_content: bool,
        include_files: bool,
        audit: bool,
    ) -> Result<serde_json::Value, ToolHostError> {
        inspect_skill_report(
            &self.registry,
            &self.quarantine,
            &name,
            include_content,
            include_files,
            audit,
        )
        .await
        .map_err(tool_host_error_from_tool)
    }

    async fn read_skill(
        &self,
        _scope: ToolOperationScope,
        name: String,
    ) -> Result<ToolSkillRead, ToolHostError> {
        let guard = self.registry.read().await;
        let skill = guard
            .skills()
            .iter()
            .find(|skill| skill.manifest.name.eq_ignore_ascii_case(&name));

        match skill {
            Some(skill) => Ok(ToolSkillRead {
                name: skill.manifest.name.clone(),
                version: skill.manifest.version.clone(),
                description: skill.manifest.description.clone(),
                trust: skill_trust_to_port(skill.trust),
                source_tier: skill.source_tier.to_string(),
                content: skill.prompt_content.clone(),
            }),
            None => {
                let available = guard
                    .skills()
                    .iter()
                    .map(|skill| skill.manifest.name.clone())
                    .collect::<Vec<_>>();
                Err(ToolHostError::OperationFailed {
                    reason: format!(
                        "Skill '{}' not found. Available skills: {}",
                        name,
                        if available.is_empty() {
                            "none".to_string()
                        } else {
                            available.join(", ")
                        }
                    ),
                })
            }
        }
    }

    async fn install_skill(
        &self,
        _request: ToolSkillInstallRequest,
    ) -> Result<ToolSkillSummary, ToolHostError> {
        Err(ToolHostError::Unavailable {
            service: "skill_install".to_string(),
        })
    }

    async fn check_skill(
        &self,
        request: ToolSkillCheckRequest,
    ) -> Result<ToolSkillCheckResult, ToolHostError> {
        skill_policy::ensure_skill_admin_available(&request.scope.metadata, "skill_check")
            .map_err(tool_host_error_from_tool)?;
        let input = skill_policy::SkillCheckInput::from(request.source);
        let output = skill_check_output_for_input(&self.quarantine, input)
            .await
            .map_err(tool_host_error_from_tool)?;
        Ok(ToolSkillCheckResult { output })
    }

    async fn remove_skill(
        &self,
        scope: ToolOperationScope,
        name: String,
    ) -> Result<ToolSkillRemoveResult, ToolHostError> {
        skill_policy::ensure_skill_admin_available(&scope.metadata, "skill_remove")
            .map_err(tool_host_error_from_tool)?;
        remove_skill_from_registry(&self.registry, &name)
            .await
            .map_err(tool_host_error_from_tool)?;
        Ok(ToolSkillRemoveResult { name })
    }

    async fn promote_skill_trust(
        &self,
        request: ToolSkillTrustMutationRequest,
    ) -> Result<ToolSkillTrustMutationResult, ToolHostError> {
        skill_policy::ensure_skill_admin_available(&request.scope.metadata, "skill_trust_promote")
            .map_err(tool_host_error_from_tool)?;
        let target_trust = match request.target_trust {
            ToolSkillTrust::Installed => SkillTrust::Installed,
            ToolSkillTrust::Trusted => SkillTrust::Trusted,
            ToolSkillTrust::Community => {
                return Err(ToolHostError::InvalidRequest {
                    reason: "target_trust must be installed or trusted".to_string(),
                });
            }
        };
        let source_tier =
            promote_skill_trust_in_registry(&self.registry, &request.name, target_trust)
                .await
                .map_err(tool_host_error_from_tool)?;
        Ok(ToolSkillTrustMutationResult {
            name: request.name,
            trust: request.target_trust,
            source_tier,
        })
    }

    async fn audit_skills(
        &self,
        scope: ToolOperationScope,
        name: Option<String>,
    ) -> Result<Vec<serde_json::Value>, ToolHostError> {
        skill_policy::ensure_skill_admin_available(&scope.metadata, "skill_audit")
            .map_err(tool_host_error_from_tool)?;
        audit_skills_for_registry(&self.registry, &self.quarantine, name.as_deref())
            .await
            .map_err(tool_host_error_from_tool)
    }

    async fn reload_skills(
        &self,
        _scope: ToolOperationScope,
        name: Option<String>,
    ) -> Result<Vec<ToolSkillSummary>, ToolHostError> {
        let mut guard = self.registry.write().await;
        if let Some(name) = name {
            let reloaded_name =
                guard
                    .reload_skill(&name)
                    .await
                    .map_err(|err| ToolHostError::OperationFailed {
                        reason: format!("Failed to reload skill '{}': {}", name, err),
                    })?;
            let skill = guard.find_by_name(&reloaded_name).ok_or_else(|| {
                ToolHostError::OperationFailed {
                    reason: format!(
                        "Skill '{}' was reloaded but is not available",
                        reloaded_name
                    ),
                }
            })?;
            return Ok(vec![ToolSkillSummary {
                name: skill.manifest.name.clone(),
                description: Some(skill.manifest.description.clone()),
                trust: skill_trust_to_port(skill.trust),
                enabled: true,
                metadata: serde_json::Value::Null,
            }]);
        }

        Ok(guard
            .reload()
            .await
            .into_iter()
            .map(|name| ToolSkillSummary {
                name,
                description: None,
                trust: ToolSkillTrust::Community,
                enabled: true,
                metadata: serde_json::Value::Null,
            })
            .collect())
    }

    async fn snapshot_skills(
        &self,
        _scope: ToolOperationScope,
    ) -> Result<ToolSkillSnapshotResult, ToolHostError> {
        let guard = self.registry.read().await;
        let snapshot = skill_policy::skill_snapshot_document(
            Utc::now().to_rfc3339(),
            guard
                .skills()
                .iter()
                .map(|skill| {
                    skill_policy::skill_snapshot_entry(
                        &skill.manifest.name,
                        &skill.manifest.version,
                        &skill.trust.to_string(),
                        &skill.source_tier.to_string(),
                        &skill.content_hash,
                        source_path_for_skill(skill).map(|path| path.display().to_string()),
                    )
                })
                .collect::<Vec<_>>(),
        );

        let snapshot_dir = crate::platform::state_paths().skills_dir.join(".hub");
        tokio::fs::create_dir_all(&snapshot_dir)
            .await
            .map_err(|err| ToolHostError::OperationFailed {
                reason: err.to_string(),
            })?;
        let snapshot_path = snapshot_dir.join(format!(
            "snapshot-{}.json",
            Utc::now().format("%Y%m%dT%H%M%SZ")
        ));
        tokio::fs::write(
            &snapshot_path,
            serde_json::to_vec_pretty(&snapshot).map_err(|err| ToolHostError::OperationFailed {
                reason: err.to_string(),
            })?,
        )
        .await
        .map_err(|err| ToolHostError::OperationFailed {
            reason: err.to_string(),
        })?;

        Ok(ToolSkillSnapshotResult {
            path: snapshot_path.display().to_string(),
            count: guard.count(),
        })
    }
}

// ── skill_read ──────────────────────────────────────────────────────────
