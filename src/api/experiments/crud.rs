use super::*;

pub(super) async fn ensure_experiments_enabled(
    store: &Arc<dyn Database>,
    user_id: &str,
) -> ApiResult<Settings> {
    let map = store
        .get_all_settings(user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let settings = Settings::from_db_map(&map);
    if !settings.experiments.enabled {
        return Err(ApiError::FeatureDisabled(
            experiments_feature_disabled_message().to_string(),
        ));
    }
    Ok(settings)
}

pub(super) async fn resolve_project_workdir(project: &ExperimentProject) -> ApiResult<PathBuf> {
    let workspace_root = tokio::fs::canonicalize(&project.workspace_path)
        .await
        .map_err(|e| {
            ApiError::InvalidInput(experiment_workspace_path_missing_with_error_message(
                &project.workspace_path,
                e,
            ))
        })?;
    let workdir_fragment =
        validate_project_workdir_fragment(&project.workdir).map_err(ApiError::InvalidInput)?;
    let workdir = workspace_root.join(workdir_fragment);
    let resolved = tokio::fs::canonicalize(&workdir).await.map_err(|e| {
        ApiError::InvalidInput(experiment_project_workdir_missing_message(
            workdir.display(),
            e,
        ))
    })?;
    if !resolved.starts_with(&workspace_root) {
        return Err(ApiError::InvalidInput(
            experiment_project_workdir_outside_workspace_message().to_string(),
        ));
    }
    Ok(resolved)
}

pub(super) async fn validate_project_launch_readiness(
    project: &ExperimentProject,
) -> ApiResult<()> {
    if !Path::new(&project.workspace_path).is_dir() {
        return Err(ApiError::InvalidInput(
            experiment_workspace_path_missing_message(&project.workspace_path),
        ));
    }
    if project.mutable_paths.is_empty() {
        return Err(ApiError::InvalidInput(
            experiment_project_missing_mutable_paths_message().to_string(),
        ));
    }
    if project.run_command.trim().is_empty() {
        return Err(ApiError::InvalidInput(
            experiment_project_run_command_empty_message().to_string(),
        ));
    }

    let _ = resolve_project_workdir(project).await?;

    git_output(&project.workspace_path, &["rev-parse", "--show-toplevel"])
        .await
        .map_err(|error| {
            ApiError::InvalidInput(experiment_workspace_not_git_repository_message(error))
        })?;
    git_output(
        &project.workspace_path,
        &["rev-parse", "--verify", &project.base_branch],
    )
    .await
    .map_err(|error| {
        ApiError::InvalidInput(experiment_base_branch_unavailable_message(
            &project.base_branch,
            error,
        ))
    })?;
    git_output(
        &project.workspace_path,
        &["remote", "get-url", &project.git_remote_name],
    )
    .await
    .map_err(|error| {
        ApiError::InvalidInput(experiment_git_remote_unavailable_message(
            &project.git_remote_name,
            error,
        ))
    })?;

    Ok(())
}

pub(super) async fn resolved_secret_env_pairs(
    user_id: &str,
    runner: &ExperimentRunnerProfile,
) -> Vec<(String, String)> {
    let Some(secrets) = research_secrets_store() else {
        return Vec::new();
    };

    let mut pairs = Vec::new();
    for reference in &runner.secret_references {
        let Some((secret_name, env_names)) = parse_secret_reference(reference) else {
            continue;
        };
        match secrets
            .get_for_injection(
                user_id,
                &secret_name,
                crate::secrets::SecretAccessContext::new(
                    "experiments.api",
                    "runner_env_credential",
                ),
            )
            .await
        {
            Ok(secret) => {
                let value = secret.expose().to_string();
                for env_name in env_names {
                    pairs.push((env_name, value.clone()));
                }
            }
            Err(error) => tracing::debug!(
                secret_name = %secret_name,
                error = %error,
                "Research benchmark secret lookup failed"
            ),
        }
    }
    pairs
}

pub(super) async fn resolved_runner_env_grants(
    user_id: &str,
    runner: &ExperimentRunnerProfile,
) -> serde_json::Value {
    let mut merged = runner.env_grants.as_object().cloned().unwrap_or_default();
    for (env_name, value) in resolved_secret_env_pairs(user_id, runner).await {
        merged
            .entry(env_name)
            .or_insert_with(|| serde_json::json!(value));
    }
    serde_json::Value::Object(merged)
}

pub async fn list_projects(
    store: &Arc<dyn Database>,
    user_id: &str,
) -> ApiResult<ExperimentProjectListResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let projects = store
        .list_experiment_projects()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(ExperimentProjectListResponse { projects })
}

pub async fn get_project(
    store: &Arc<dyn Database>,
    user_id: &str,
    id: Uuid,
) -> ApiResult<ExperimentProject> {
    ensure_experiments_enabled(store, user_id).await?;
    store
        .get_experiment_project(id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(experiment_project_not_found_message(id)))
}

pub async fn create_project(
    store: &Arc<dyn Database>,
    user_id: &str,
    req: CreateExperimentProjectRequest,
) -> ApiResult<ExperimentProject> {
    let settings = ensure_experiments_enabled(store, user_id).await?;
    let now = Utc::now();
    let mut project = ExperimentProject {
        id: Uuid::new_v4(),
        name: req.name,
        workspace_path: req.workspace_path,
        git_remote_name: req.git_remote_name,
        base_branch: req.base_branch,
        preset: req
            .preset
            .unwrap_or(ExperimentPreset::AutoresearchSingleFile),
        strategy_prompt: req.strategy_prompt.unwrap_or_else(default_strategy_prompt),
        workdir: req.workdir,
        prepare_command: req.prepare_command,
        run_command: req.run_command,
        mutable_paths: req.mutable_paths,
        fixed_paths: req.fixed_paths,
        primary_metric: req.primary_metric,
        secondary_metrics: req.secondary_metrics,
        comparison_policy: req.comparison_policy.unwrap_or_default(),
        stop_policy: req.stop_policy.unwrap_or_default(),
        default_runner_profile_id: req.default_runner_profile_id,
        promotion_mode: req
            .promotion_mode
            .unwrap_or(settings.experiments.default_promotion_mode),
        autonomy_mode: req
            .autonomy_mode
            .unwrap_or(ExperimentAutonomyMode::Autonomous),
        status: ExperimentProjectStatus::Draft,
        created_at: now,
        updated_at: now,
    };
    project.status =
        ready_project_status_policy(&project, Path::new(&project.workspace_path).exists());
    store
        .create_experiment_project(&project)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(project)
}

pub async fn update_project(
    store: &Arc<dyn Database>,
    user_id: &str,
    id: Uuid,
    req: UpdateExperimentProjectRequest,
) -> ApiResult<ExperimentProject> {
    ensure_experiments_enabled(store, user_id).await?;
    let mut project = get_project(store, user_id, id).await?;
    if let Some(value) = req.name {
        project.name = value;
    }
    if let Some(value) = req.workspace_path {
        project.workspace_path = value;
    }
    if let Some(value) = req.git_remote_name {
        project.git_remote_name = value;
    }
    if let Some(value) = req.base_branch {
        project.base_branch = value;
    }
    if let Some(value) = req.preset {
        project.preset = value;
    }
    if let Some(value) = req.strategy_prompt {
        project.strategy_prompt = value;
    }
    if let Some(value) = req.workdir {
        project.workdir = value;
    }
    if req.prepare_command.is_some() {
        project.prepare_command = req.prepare_command;
    }
    if let Some(value) = req.run_command {
        project.run_command = value;
    }
    if let Some(value) = req.mutable_paths {
        project.mutable_paths = value;
    }
    if let Some(value) = req.fixed_paths {
        project.fixed_paths = value;
    }
    if let Some(value) = req.primary_metric {
        project.primary_metric = value;
    }
    if let Some(value) = req.secondary_metrics {
        project.secondary_metrics = value;
    }
    if let Some(value) = req.comparison_policy {
        project.comparison_policy = value;
    }
    if let Some(value) = req.stop_policy {
        project.stop_policy = value;
    }
    if req.default_runner_profile_id.is_some() {
        project.default_runner_profile_id = req.default_runner_profile_id;
    }
    if let Some(value) = req.promotion_mode {
        project.promotion_mode = value;
    }
    if let Some(value) = req.autonomy_mode {
        project.autonomy_mode = value;
    }
    project.status = req.status.unwrap_or_else(|| {
        ready_project_status_policy(&project, Path::new(&project.workspace_path).exists())
    });
    project.updated_at = Utc::now();
    store
        .update_experiment_project(&project)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(project)
}

pub async fn delete_project(store: &Arc<dyn Database>, user_id: &str, id: Uuid) -> ApiResult<bool> {
    ensure_experiments_enabled(store, user_id).await?;
    store
        .delete_experiment_project(id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))
}

pub async fn list_runners(
    store: &Arc<dyn Database>,
    user_id: &str,
) -> ApiResult<ExperimentRunnerListResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let runners = store
        .list_experiment_runner_profiles()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(ExperimentRunnerListResponse { runners })
}

pub async fn get_runner(
    store: &Arc<dyn Database>,
    user_id: &str,
    id: Uuid,
) -> ApiResult<ExperimentRunnerProfile> {
    ensure_experiments_enabled(store, user_id).await?;
    store
        .get_experiment_runner_profile(id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(experiment_runner_not_found_message(id)))
}

pub async fn create_runner(
    store: &Arc<dyn Database>,
    user_id: &str,
    req: CreateExperimentRunnerProfileRequest,
) -> ApiResult<ExperimentRunnerProfile> {
    ensure_experiments_enabled(store, user_id).await?;
    let now = Utc::now();
    let runner = ExperimentRunnerProfile {
        id: Uuid::new_v4(),
        name: req.name,
        backend: req.backend,
        backend_config: req.backend_config,
        image_or_runtime: req.image_or_runtime,
        gpu_requirements: req.gpu_requirements,
        env_grants: req.env_grants,
        secret_references: req.secret_references,
        cache_policy: req.cache_policy,
        status: ExperimentRunnerStatus::Draft,
        readiness_class: crate::experiments::ExperimentRunnerReadinessClass::ManualOnly,
        launch_eligible: false,
        created_at: now,
        updated_at: now,
    };
    store
        .create_experiment_runner_profile(&runner)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(runner)
}

pub async fn update_runner(
    store: &Arc<dyn Database>,
    user_id: &str,
    id: Uuid,
    req: UpdateExperimentRunnerProfileRequest,
) -> ApiResult<ExperimentRunnerProfile> {
    ensure_experiments_enabled(store, user_id).await?;
    let mut runner = get_runner(store, user_id, id).await?;
    if let Some(value) = req.name {
        runner.name = value;
    }
    if let Some(value) = req.backend {
        runner.backend = value;
    }
    if let Some(value) = req.backend_config {
        runner.backend_config = value;
    }
    if req.image_or_runtime.is_some() {
        runner.image_or_runtime = req.image_or_runtime;
    }
    if let Some(value) = req.gpu_requirements {
        runner.gpu_requirements = value;
    }
    if let Some(value) = req.env_grants {
        runner.env_grants = value;
    }
    if let Some(value) = req.secret_references {
        runner.secret_references = value;
    }
    if let Some(value) = req.cache_policy {
        runner.cache_policy = value;
    }
    if let Some(value) = req.status {
        runner.status = value;
    }
    runner.updated_at = Utc::now();
    store
        .update_experiment_runner_profile(&runner)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(runner)
}

pub async fn delete_runner(store: &Arc<dyn Database>, user_id: &str, id: Uuid) -> ApiResult<bool> {
    ensure_experiments_enabled(store, user_id).await?;
    store
        .delete_experiment_runner_profile(id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))
}

pub async fn validate_runner(
    store: &Arc<dyn Database>,
    user_id: &str,
    id: Uuid,
) -> ApiResult<ExperimentRunnerValidationResponse> {
    let settings = ensure_experiments_enabled(store, user_id).await?;
    let mut runner = get_runner(store, user_id, id).await?;
    let validation = validate_runner_profile_impl(user_id, &runner, &settings).await;
    runner.status = if validation.valid {
        ExperimentRunnerStatus::Validated
    } else {
        ExperimentRunnerStatus::Unavailable
    };
    runner.readiness_class = validation.readiness_class;
    runner.launch_eligible = validation.launch_eligible;
    runner.updated_at = Utc::now();
    store
        .update_experiment_runner_profile(&runner)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(ExperimentRunnerValidationResponse {
        runner,
        valid: validation.valid,
        readiness_class: validation.readiness_class,
        launch_eligible: validation.launch_eligible,
        message: validation.message,
    })
}

pub async fn list_campaigns(
    store: &Arc<dyn Database>,
    user_id: &str,
) -> ApiResult<ExperimentCampaignListResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let campaigns = store
        .list_experiment_campaigns_for_owner(user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(ExperimentCampaignListResponse { campaigns })
}

pub async fn get_campaign(
    store: &Arc<dyn Database>,
    user_id: &str,
    id: Uuid,
) -> ApiResult<ExperimentCampaign> {
    ensure_experiments_enabled(store, user_id).await?;
    let campaign = store
        .get_experiment_campaign_for_owner(id, user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(experiment_campaign_not_found_message(id)))?;
    Ok(campaign)
}

pub async fn list_trials(
    store: &Arc<dyn Database>,
    user_id: &str,
    campaign_id: Uuid,
) -> ApiResult<ExperimentTrialListResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let campaign = get_campaign(store, user_id, campaign_id).await?;
    let trials = store
        .list_experiment_trials_for_owner(campaign.id, user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(ExperimentTrialListResponse { trials })
}

pub async fn get_trial(
    store: &Arc<dyn Database>,
    user_id: &str,
    id: Uuid,
) -> ApiResult<ExperimentTrial> {
    ensure_experiments_enabled(store, user_id).await?;
    let trial = store
        .get_experiment_trial_for_owner(id, user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(experiment_trial_not_found_message(id)))?;
    Ok(trial)
}

pub async fn list_artifacts(
    store: &Arc<dyn Database>,
    user_id: &str,
    trial_id: Uuid,
) -> ApiResult<ExperimentArtifactListResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let artifacts = store
        .list_experiment_artifacts_for_owner(trial_id, user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(ExperimentArtifactListResponse { artifacts })
}

pub async fn list_targets(
    store: &Arc<dyn Database>,
    user_id: &str,
) -> ApiResult<ExperimentTargetListResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let targets = store
        .list_experiment_targets()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(ExperimentTargetListResponse { targets })
}

pub async fn create_target(
    store: &Arc<dyn Database>,
    user_id: &str,
    req: CreateExperimentTargetRequest,
) -> ApiResult<ExperimentTarget> {
    ensure_experiments_enabled(store, user_id).await?;
    let kind = req.kind;
    let metadata = if req.metadata.is_object() {
        req.metadata
    } else {
        serde_json::json!({})
    };
    let targets = store
        .list_experiment_targets()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    ensure_unique_target_signature(kind, &metadata, None, &targets)?;
    let now = Utc::now();
    let target = ExperimentTarget {
        id: Uuid::new_v4(),
        name: req.name,
        kind,
        location: req.location,
        metadata,
        created_at: now,
        updated_at: now,
    };
    store
        .create_experiment_target(&target)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(target)
}

pub async fn link_target(
    store: &Arc<dyn Database>,
    user_id: &str,
    req: LinkExperimentTargetRequest,
) -> ApiResult<ExperimentTarget> {
    ensure_experiments_enabled(store, user_id).await?;
    if req.target_id.trim().is_empty() {
        return Err(ApiError::InvalidInput(
            experiment_target_id_required_message().to_string(),
        ));
    }

    let usage = store
        .list_experiment_model_usage(250)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let targets = store
        .list_experiment_targets()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let target_links = store
        .list_experiment_target_links()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let opportunity = derive_opportunities(&usage, &targets, &target_links)
        .into_iter()
        .find(|entry| entry.id == req.opportunity_id)
        .ok_or_else(|| {
            ApiError::SessionNotFound(experiment_opportunity_not_found_message(
                &req.opportunity_id,
            ))
        })?;

    let mut metadata = if req.metadata.is_object() {
        req.metadata
    } else {
        serde_json::json!({})
    };
    if let Some(obj) = metadata.as_object_mut() {
        obj.insert(
            "asset_id".to_string(),
            serde_json::json!(req.target_id.trim()),
        );
        obj.insert(
            "opportunity_id".to_string(),
            serde_json::json!(opportunity.id),
        );
        obj.insert(
            "provider".to_string(),
            serde_json::json!(opportunity.provider),
        );
        obj.insert("model".to_string(), serde_json::json!(opportunity.model));
        if let Some(route_key) = opportunity.route_key.clone() {
            obj.insert("route_key".to_string(), serde_json::json!(route_key));
        }
        if let Some(logical_role) = opportunity.logical_role.clone() {
            obj.insert("logical_role".to_string(), serde_json::json!(logical_role));
        }
        obj.insert(
            "suggested_preset".to_string(),
            serde_json::json!(opportunity.suggested_preset),
        );
        obj.insert(
            "gpu_requirement".to_string(),
            serde_json::json!(opportunity.gpu_requirement),
        );
    }

    let now = Utc::now();
    ensure_unique_target_signature(req.target_type, &metadata, None, &targets)?;

    if let Some(mut target) = targets.into_iter().find(|target| {
        target.kind == req.target_type
            && target
                .metadata
                .get("asset_id")
                .and_then(|value| value.as_str())
                .map(|value| value == req.target_id.trim())
                .unwrap_or(false)
    }) {
        target.name = req
            .target_name
            .clone()
            .unwrap_or_else(|| target.name.clone());
        if req.location.is_some() {
            target.location = req.location.clone();
        }
        target.metadata = merge_json(&target.metadata, &metadata);
        target.updated_at = now;
        store
            .update_experiment_target(&target)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let link = ExperimentTargetLink {
            id: Uuid::new_v4(),
            target_id: target.id,
            kind: req.target_type,
            provider: opportunity.provider.clone(),
            model: opportunity.model.clone(),
            route_key: opportunity.route_key.clone(),
            logical_role: opportunity.logical_role.clone(),
            metadata: serde_json::json!({
                "opportunity_id": opportunity.id,
                "suggested_preset": opportunity.suggested_preset,
                "gpu_requirement": opportunity.gpu_requirement,
            }),
            created_at: now,
            updated_at: now,
        };
        store
            .upsert_experiment_target_link(&link)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        return Ok(target);
    }

    let target = ExperimentTarget {
        id: Uuid::new_v4(),
        name: req
            .target_name
            .unwrap_or_else(|| req.target_id.trim().to_string()),
        kind: req.target_type,
        location: req.location,
        metadata,
        created_at: now,
        updated_at: now,
    };
    store
        .create_experiment_target(&target)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let link = ExperimentTargetLink {
        id: Uuid::new_v4(),
        target_id: target.id,
        kind: req.target_type,
        provider: opportunity.provider,
        model: opportunity.model,
        route_key: opportunity.route_key,
        logical_role: opportunity.logical_role,
        metadata: serde_json::json!({
            "opportunity_id": req.opportunity_id,
            "suggested_preset": opportunity.suggested_preset,
            "gpu_requirement": opportunity.gpu_requirement,
        }),
        created_at: now,
        updated_at: now,
    };
    store
        .upsert_experiment_target_link(&link)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(target)
}

pub async fn update_target(
    store: &Arc<dyn Database>,
    user_id: &str,
    id: Uuid,
    req: UpdateExperimentTargetRequest,
) -> ApiResult<ExperimentTarget> {
    ensure_experiments_enabled(store, user_id).await?;
    let mut target = store
        .get_experiment_target(id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(experiment_target_not_found_message(id)))?;
    let new_kind = req.kind.unwrap_or(target.kind);
    let new_metadata = req
        .metadata
        .as_ref()
        .map(|metadata| {
            if metadata.is_object() {
                metadata.clone()
            } else {
                serde_json::json!({})
            }
        })
        .or_else(|| Some(target.metadata.clone()));
    let existing_targets = store
        .list_experiment_targets()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if let Some(ref metadata) = new_metadata {
        ensure_unique_target_signature(new_kind, metadata, Some(id), &existing_targets)?;
        target.metadata = metadata.clone();
    } else {
        ensure_unique_target_signature(new_kind, &target.metadata, Some(id), &existing_targets)?;
    }

    if let Some(name) = req.name {
        target.name = name;
    }
    target.kind = new_kind;
    if req.location.is_some() {
        target.location = req.location;
    }

    target.updated_at = Utc::now();
    store
        .update_experiment_target(&target)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(target)
}

pub async fn delete_target(store: &Arc<dyn Database>, user_id: &str, id: Uuid) -> ApiResult<bool> {
    ensure_experiments_enabled(store, user_id).await?;
    store
        .delete_experiment_target_links_for_target(id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    store
        .delete_experiment_target(id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))
}

pub async fn list_model_usage(
    store: &Arc<dyn Database>,
    user_id: &str,
    limit: usize,
) -> ApiResult<ExperimentModelUsageListResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let usage = store
        .list_experiment_model_usage(limit)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(ExperimentModelUsageListResponse { usage })
}

pub async fn list_opportunities(
    store: &Arc<dyn Database>,
    user_id: &str,
    limit: usize,
) -> ApiResult<ExperimentOpportunityListResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let usage = store
        .list_experiment_model_usage(limit)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let targets = store
        .list_experiment_targets()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let target_links = store
        .list_experiment_target_links()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let outcome_contracts = store
        .list_outcome_contracts(&OutcomeContractQuery {
            user_id: user_id.to_string(),
            actor_id: None,
            status: Some("evaluated".to_string()),
            contract_type: None,
            source_kind: None,
            source_id: None,
            thread_id: None,
            limit: ((limit.max(25)) * 8) as i64,
        })
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let mut opportunities = derive_opportunities(&usage, &targets, &target_links);
    opportunities.extend(derive_outcome_opportunities(
        &outcome_contracts,
        &targets,
        limit,
        crate::workspace::paths::USER,
    ));
    sort_experiment_opportunities(&mut opportunities);
    opportunities.truncate(limit.max(1));
    Ok(ExperimentOpportunityListResponse { opportunities })
}

pub async fn list_gpu_cloud_providers(
    store: &Arc<dyn Database>,
    user_id: &str,
) -> ApiResult<ExperimentGpuCloudProviderListResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let providers = [
        ExperimentRunnerBackend::Runpod,
        ExperimentRunnerBackend::Vast,
        ExperimentRunnerBackend::Lambda,
    ]
    .into_iter()
    .map(|backend| ExperimentGpuCloudProviderInfo {
        slug: backend.slug().to_string(),
        display_name: adapters::gpu_cloud_display_name(backend).to_string(),
        backend,
        description: format!(
            "{} setup for outbound ThinClaw experiment runners.",
            adapters::gpu_cloud_display_name(backend)
        ),
        signup_url: adapters::gpu_cloud_signup_url(backend)
            .unwrap_or_default()
            .to_string(),
        docs_url: adapters::gpu_cloud_docs_url(backend)
            .unwrap_or_default()
            .to_string(),
        secret_name: adapters::gpu_cloud_secret_name(backend)
            .unwrap_or_default()
            .to_string(),
        connected: false,
        template_hint: Some(adapters::gpu_cloud_template_hint(backend)),
    })
    .collect();
    Ok(ExperimentGpuCloudProviderListResponse { providers })
}
