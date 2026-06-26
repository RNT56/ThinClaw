use super::*;

pub async fn start_campaign(
    store: &Arc<dyn Database>,
    user_id: &str,
    project_id: Uuid,
    req: StartExperimentCampaignRequest,
) -> ApiResult<ExperimentCampaignActionResponse> {
    let settings = ensure_experiments_enabled(store, user_id).await?;
    let project = get_project(store, user_id, project_id).await?;
    validate_project_launch_readiness(&project).await?;
    let active_before = active_campaign_count(store).await?;
    let queue_state = if active_before >= settings.experiments.max_concurrent_campaigns as usize {
        ExperimentCampaignQueueState::Queued
    } else {
        ExperimentCampaignQueueState::NotQueued
    };
    let queue_position = if queue_state == ExperimentCampaignQueueState::Queued {
        next_queue_position(store).await?
    } else {
        0
    };
    let runner_id = req
        .runner_profile_id
        .or(project.default_runner_profile_id)
        .ok_or_else(|| {
            ApiError::InvalidInput(experiment_runner_profile_id_required_message().to_string())
        })?;
    let runner = get_runner(store, user_id, runner_id).await?;
    let validation = validate_runner_profile_impl(user_id, &runner, &settings).await;
    if !validation.valid {
        return Err(ApiError::InvalidInput(format!(
            "Runner profile is not launchable: {}",
            validation.message
        )));
    }
    if queue_state == ExperimentCampaignQueueState::Queued && !validation.launch_eligible {
        return Err(ApiError::InvalidInput(
            "This runner requires operator action and cannot be queued for automatic launch. Wait for a free slot or use a launch-ready runner.".to_string(),
        ));
    }
    let normalized_gateway_url = req
        .gateway_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if req.gateway_url.is_some() && normalized_gateway_url.is_none() {
        return Err(ApiError::InvalidInput(
            "gateway_url must not be empty when provided.".to_string(),
        ));
    }

    let now = Utc::now();
    let campaign_id = Uuid::new_v4();
    let worktree_path = experiments_worktree_path(&project.workspace_path, campaign_id);
    let experiment_branch = format!("codex/experiments/{}", short_id(campaign_id));
    let campaign = ExperimentCampaign {
        id: campaign_id,
        project_id: project.id,
        runner_profile_id: runner.id,
        owner_user_id: user_id.to_string(),
        status: ExperimentCampaignStatus::PendingBaseline,
        baseline_commit: None,
        best_commit: None,
        best_metrics: serde_json::json!({}),
        experiment_branch: Some(experiment_branch.clone()),
        remote_ref: Some(format!("refs/heads/{experiment_branch}")),
        worktree_path: Some(worktree_path.to_string_lossy().to_string()),
        started_at: Some(now),
        ended_at: None,
        trial_count: 0,
        failure_count: 0,
        pause_reason: Some("Pending baseline launch.".to_string()),
        queue_state,
        queue_position,
        active_trial_id: None,
        total_runtime_ms: 0,
        total_cost_usd: 0.0,
        total_llm_cost_usd: 0.0,
        total_runner_cost_usd: 0.0,
        consecutive_non_improving_trials: 0,
        max_trials_override: req.max_trials_override,
        gateway_url: normalized_gateway_url,
        metadata: serde_json::json!({}),
        created_at: now,
        updated_at: now,
    };
    store
        .create_experiment_campaign(&campaign)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    if queue_state == ExperimentCampaignQueueState::Queued {
        let mut queued_campaign = campaign.clone();
        queued_campaign.pause_reason = Some("Queued until a research slot frees up.".to_string());
        queued_campaign.updated_at = Utc::now();
        store
            .update_experiment_campaign(&queued_campaign)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        return Ok(ExperimentCampaignActionResponse {
            campaign: queued_campaign,
            trial: None,
            lease: None,
            launch: None,
            message: format!(
                "Campaign queued. Waiting for one of the {} active campaign slots to free up.",
                settings.experiments.max_concurrent_campaigns
            ),
        });
    }

    match launch_campaign_baseline(
        store,
        user_id,
        &settings,
        &project,
        &runner,
        campaign.clone(),
    )
    .await
    {
        Ok(response) => {
            maybe_launch_next_queued_after_slot_release(store, user_id).await?;
            Ok(response)
        }
        Err(error) => {
            persist_campaign_launch_failure(store, campaign, &error.to_string()).await?;
            Err(error)
        }
    }
}

pub(super) fn launch_details_from_outcome(outcome: RunnerLaunchOutcome) -> ExperimentLaunchDetails {
    ExperimentLaunchDetails {
        message: outcome.message,
        bootstrap_command: outcome.bootstrap_command,
        provider_template: outcome.provider_template,
        provider_job_id: outcome.provider_job_id,
        provider_job_metadata: outcome.provider_job_metadata,
        auto_launched: outcome.auto_launched,
        requires_operator_action: outcome.requires_operator_action,
    }
}

pub(super) async fn persist_campaign_launch_failure(
    store: &Arc<dyn Database>,
    mut campaign: ExperimentCampaign,
    reason: &str,
) -> ApiResult<()> {
    campaign.status = ExperimentCampaignStatus::Failed;
    campaign.pause_reason = Some(format!("Baseline launch failed: {reason}"));
    campaign.queue_state = ExperimentCampaignQueueState::NotQueued;
    campaign.ended_at = Some(Utc::now());
    campaign.updated_at = Utc::now();
    store
        .update_experiment_campaign(&campaign)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(())
}

pub(super) async fn launch_campaign_baseline(
    store: &Arc<dyn Database>,
    user_id: &str,
    _settings: &Settings,
    project: &ExperimentProject,
    runner: &ExperimentRunnerProfile,
    mut campaign: ExperimentCampaign,
) -> ApiResult<ExperimentCampaignActionResponse> {
    let worktree_path = campaign.worktree_path.clone().ok_or_else(|| {
        ApiError::InvalidInput(
            experiment_campaign_missing_worktree_path_field_message().to_string(),
        )
    })?;
    let branch = campaign.experiment_branch.clone().ok_or_else(|| {
        ApiError::InvalidInput(
            experiment_campaign_missing_experiment_branch_field_message().to_string(),
        )
    })?;

    prepare_campaign_worktree(project, Path::new(&worktree_path)).await?;
    let _ = git_output(
        &project.workspace_path,
        &[
            "worktree",
            "add",
            "--detach",
            &worktree_path,
            &project.base_branch,
        ],
    )
    .await?;
    let _ = git_output(
        &worktree_path,
        &["checkout", "-B", &branch, &project.base_branch],
    )
    .await?;
    let baseline_commit = git_output(&worktree_path, &["rev-parse", "HEAD"]).await?;
    if runner.backend.is_remote() {
        push_experiment_branch(project, Path::new(&worktree_path), &branch).await?;
    }

    campaign.queue_state = ExperimentCampaignQueueState::Active;
    if campaign.started_at.is_none() {
        campaign.started_at = Some(Utc::now());
    }
    let trial_id = Uuid::new_v4();
    campaign.active_trial_id = Some(trial_id);
    campaign.status = ExperimentCampaignStatus::Running;
    campaign.pause_reason = Some("Baseline trial prepared.".to_string());
    campaign.updated_at = Utc::now();
    store
        .update_experiment_campaign(&campaign)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let trial = ExperimentTrial {
        id: trial_id,
        campaign_id: campaign.id,
        sequence: 1,
        candidate_commit: Some(baseline_commit),
        parent_best_commit: None,
        status: ExperimentTrialStatus::Preparing,
        runner_backend: runner.backend,
        exit_code: None,
        metrics_json: serde_json::json!({}),
        summary: Some("Baseline trial prepared".to_string()),
        decision_reason: None,
        log_preview_path: None,
        artifact_manifest_json: serde_json::json!({}),
        runtime_ms: None,
        attributed_cost_usd: None,
        llm_cost_usd: None,
        runner_cost_usd: None,
        hypothesis: Some("Baseline measurement for the configured benchmark.".to_string()),
        mutation_summary: None,
        reviewer_decision: Some("baseline".to_string()),
        provider_job_id: None,
        provider_job_metadata: serde_json::json!({}),
        started_at: None,
        completed_at: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    store
        .create_experiment_trial(&trial)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    launch_trial(store, user_id, _settings, project, runner, campaign, trial).await
}

pub(super) async fn prepare_candidate_trial_from_worktree(
    _store: &Arc<dyn Database>,
    campaign: &ExperimentCampaign,
    project: &ExperimentProject,
    runner: &ExperimentRunnerProfile,
    trial_id: Uuid,
    sequence: u32,
    hypothesis: String,
    mutation_summary: String,
    reviewer_decision: String,
    artifact_manifest_json: serde_json::Value,
) -> ApiResult<ExperimentTrial> {
    let worktree_path = campaign.worktree_path.as_deref().ok_or_else(|| {
        ApiError::InvalidInput(experiment_campaign_has_no_worktree_message().to_string())
    })?;
    let changed_files = filtered_changed_files(git_changed_files(worktree_path).await?);
    if changed_files.is_empty() {
        return Err(ApiError::InvalidInput(
            experiment_no_candidate_changes_message().to_string(),
        ));
    }
    enforce_mutable_paths(&project.mutable_paths, &changed_files)?;
    git_run(
        worktree_path,
        &["add", "--"],
        changed_files
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .as_slice(),
    )
    .await?;
    let message = format!("Experiment trial {sequence}");
    let _ = git_output(worktree_path, &["commit", "-m", &message]).await?;
    let candidate_commit = git_output(worktree_path, &["rev-parse", "HEAD"]).await?;
    if runner.backend.is_remote()
        && let Some(branch) = campaign.experiment_branch.as_deref()
    {
        push_experiment_branch(project, Path::new(worktree_path), branch).await?;
    }

    Ok(ExperimentTrial {
        id: trial_id,
        campaign_id: campaign.id,
        sequence,
        candidate_commit: Some(candidate_commit),
        parent_best_commit: campaign.best_commit.clone(),
        status: ExperimentTrialStatus::Preparing,
        runner_backend: runner.backend,
        exit_code: None,
        metrics_json: serde_json::json!({}),
        summary: Some("Candidate trial prepared".to_string()),
        decision_reason: None,
        log_preview_path: None,
        artifact_manifest_json,
        runtime_ms: None,
        attributed_cost_usd: None,
        llm_cost_usd: None,
        runner_cost_usd: None,
        hypothesis: Some(hypothesis),
        mutation_summary: Some(mutation_summary),
        reviewer_decision: Some(reviewer_decision),
        provider_job_id: None,
        provider_job_metadata: serde_json::json!({}),
        started_at: None,
        completed_at: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    })
}

pub(super) async fn create_experiment_trial_commit(
    store: &Arc<dyn Database>,
    campaign: &ExperimentCampaign,
    project: &ExperimentProject,
    runner: &ExperimentRunnerProfile,
) -> Result<ExperimentTrial, CandidateGenerationError> {
    let sequence = latest_trial(store, campaign.id)
        .await
        .map_err(|error| CandidateGenerationError::new(error.to_string(), Vec::new()))?
        .map(|trial| trial.sequence + 1)
        .unwrap_or(1);
    let trial_id = Uuid::new_v4();
    let planner = match run_planner_subagent(store, campaign, project, Some(trial_id)).await {
        Ok(planner) => planner,
        Err(ResearchSubagentInvocationError::Api(error)) => {
            return Err(CandidateGenerationError::new(error.to_string(), Vec::new()));
        }
        Err(ResearchSubagentInvocationError::Run(error)) => {
            let error = *error;
            return Err(CandidateGenerationError::new(
                error.message,
                vec![error.run_artifact],
            ));
        }
    };
    let mutator =
        match run_mutator_subagent(campaign, project, &planner.value, Some(trial_id)).await {
            Ok(mutator) => mutator,
            Err(ResearchSubagentInvocationError::Api(error)) => {
                return Err(CandidateGenerationError::new(
                    error.to_string(),
                    vec![planner.run_artifact.clone()],
                ));
            }
            Err(ResearchSubagentInvocationError::Run(error)) => {
                let error = *error;
                return Err(CandidateGenerationError::new(
                    error.message,
                    vec![planner.run_artifact.clone(), error.run_artifact],
                ));
            }
        };
    let worktree_path = campaign.worktree_path.as_deref().ok_or_else(|| {
        CandidateGenerationError::new(
            experiment_campaign_has_no_worktree_message(),
            vec![planner.run_artifact.clone(), mutator.run_artifact.clone()],
        )
    })?;
    let changed_files =
        filtered_changed_files(git_changed_files(worktree_path).await.map_err(|e| {
            CandidateGenerationError::new(
                e.to_string(),
                vec![planner.run_artifact.clone(), mutator.run_artifact.clone()],
            )
        })?);
    if changed_files.is_empty() {
        let mut mutator_artifact = mutator.run_artifact.clone();
        mutator_artifact.mark_failed("Autonomous mutator did not produce any candidate changes.");
        return Err(CandidateGenerationError::new(
            "Autonomous mutator did not produce any candidate changes.",
            vec![planner.run_artifact.clone(), mutator_artifact],
        ));
    }
    if let Err(error) = enforce_mutable_paths(&project.mutable_paths, &changed_files) {
        let mut mutator_artifact = mutator.run_artifact.clone();
        mutator_artifact.mark_failed(error.to_string());
        return Err(CandidateGenerationError::new(
            error.to_string(),
            vec![planner.run_artifact.clone(), mutator_artifact],
        ));
    }
    let diff_stat = git_output(worktree_path, &["diff", "--stat", "--", "."])
        .await
        .map_err(|e| {
            CandidateGenerationError::new(
                e.to_string(),
                vec![planner.run_artifact.clone(), mutator.run_artifact.clone()],
            )
        })?;
    let diff_preview = git_output(worktree_path, &["diff", "--", "."])
        .await
        .map_err(|e| {
            CandidateGenerationError::new(
                e.to_string(),
                vec![planner.run_artifact.clone(), mutator.run_artifact.clone()],
            )
        })?;
    let reviewer = match run_reviewer_subagent(
        campaign,
        project,
        &planner.value,
        &diff_stat,
        &diff_preview,
        Some(trial_id),
    )
    .await
    {
        Ok(reviewer) => reviewer,
        Err(ResearchSubagentInvocationError::Api(error)) => {
            return Err(CandidateGenerationError::new(
                error.to_string(),
                vec![planner.run_artifact.clone(), mutator.run_artifact.clone()],
            ));
        }
        Err(ResearchSubagentInvocationError::Run(error)) => {
            let error = *error;
            return Err(CandidateGenerationError::new(
                error.message,
                vec![
                    planner.run_artifact.clone(),
                    mutator.run_artifact.clone(),
                    error.run_artifact,
                ],
            ));
        }
    };
    if !(reviewer.value.approved && reviewer.value.scope_ok && reviewer.value.benchmark_ready) {
        let mut reviewer_artifact = reviewer.run_artifact.clone();
        reviewer_artifact.mark_failed(reviewer.value.reason.clone());
        return Err(CandidateGenerationError::new(
            format!(
                "Reviewer rejected the autonomous candidate: {}",
                reviewer.value.reason
            ),
            vec![
                planner.run_artifact.clone(),
                mutator.run_artifact.clone(),
                reviewer_artifact,
            ],
        ));
    }
    let planner_hypothesis = planner.value.hypothesis.clone();
    let planner_target_ids = planner.value.target_ids.clone();
    let expected_metric_direction = planner.value.expected_metric_direction.clone();
    let mutator_mutation_summary = mutator.value.mutation_summary.clone();
    let mutator_changed_paths = mutator.value.changed_paths.clone();
    let reviewer_reason = reviewer.value.reason.clone();
    prepare_candidate_trial_from_worktree(
        store,
        campaign,
        project,
        runner,
        trial_id,
        sequence,
        planner_hypothesis,
        mutator_mutation_summary,
        reviewer_reason,
        serde_json::json!({
            "candidate_source": "autonomous_subagent",
            "changed_paths": changed_files,
            "planner_target_ids": planner_target_ids,
            "expected_metric_direction": expected_metric_direction,
            "mutator_changed_paths": mutator_changed_paths,
            "workspace": worktree_path,
            "run_artifacts": [
                planner.run_artifact.clone(),
                mutator.run_artifact.clone(),
                reviewer.run_artifact.clone()
            ],
        }),
    )
    .await
    .map_err(|error| {
        CandidateGenerationError::new(
            error.to_string(),
            vec![
                planner.run_artifact.clone(),
                mutator.run_artifact.clone(),
                reviewer.run_artifact.clone(),
            ],
        )
    })
}

pub(super) async fn revoke_lease_with_runner(
    store: &Arc<dyn Database>,
    user_id: &str,
    campaign: &ExperimentCampaign,
    lease: &ExperimentLease,
    action: RemoteLaunchAction,
) -> ApiResult<String> {
    let mut lease = lease.clone();
    lease.status = ExperimentLeaseStatus::Revoked;
    lease.completed_at = Some(Utc::now());
    lease.updated_at = Utc::now();
    store
        .update_experiment_lease(&lease)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let auth = ExperimentLeaseAuthentication {
        lease_id: lease.id,
        token: String::new(),
    };
    let message = if let Some(runner) = store
        .get_experiment_runner_profile(campaign.runner_profile_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        let trial = store
            .get_experiment_trial(lease.trial_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let provider_job_metadata = trial
            .as_ref()
            .map(|entry| entry.provider_job_metadata.clone())
            .unwrap_or_else(|| serde_json::json!({}));
        let provider_api_key = research_provider_api_key(user_id, &runner).await;
        adapters::revoke_remote_launch(
            &runner,
            &auth,
            trial
                .as_ref()
                .and_then(|entry| entry.provider_job_id.as_deref()),
            &provider_job_metadata,
            action,
            provider_api_key.as_deref(),
        )
        .await
        .map_err(ApiError::Internal)?
    } else {
        None
    };

    Ok(message.unwrap_or_else(|| experiment_lease_revoked_action_message().to_string()))
}

pub async fn pause_campaign(
    store: &Arc<dyn Database>,
    user_id: &str,
    campaign_id: Uuid,
) -> ApiResult<ExperimentCampaignActionResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let mut campaign = get_campaign(store, user_id, campaign_id).await?;
    campaign.status = ExperimentCampaignStatus::Paused;
    campaign.queue_state = ExperimentCampaignQueueState::NotQueued;
    campaign.pause_reason = Some(experiment_campaign_paused_by_operator_message().to_string());
    campaign.updated_at = Utc::now();
    store
        .update_experiment_campaign(&campaign)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let mut launch_message = None;
    if let Some(trial) = latest_trial(store, campaign.id).await?
        && let Some(lease) = latest_active_lease(store, trial.id).await?
    {
        launch_message = Some(
            revoke_lease_with_runner(store, user_id, &campaign, &lease, RemoteLaunchAction::Pause)
                .await?,
        );
    }
    maybe_launch_next_queued_after_slot_release(store, user_id).await?;
    Ok(ExperimentCampaignActionResponse {
        campaign,
        trial: None,
        lease: None,
        launch: None,
        message: launch_message.unwrap_or_else(|| experiment_campaign_paused_message().to_string()),
    })
}

pub async fn cancel_campaign(
    store: &Arc<dyn Database>,
    user_id: &str,
    campaign_id: Uuid,
) -> ApiResult<ExperimentCampaignActionResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let mut campaign = get_campaign(store, user_id, campaign_id).await?;
    campaign.status = ExperimentCampaignStatus::Cancelled;
    campaign.queue_state = ExperimentCampaignQueueState::NotQueued;
    campaign.pause_reason = Some(experiment_campaign_cancelled_by_operator_message().to_string());
    campaign.ended_at = Some(Utc::now());
    campaign.updated_at = Utc::now();
    store
        .update_experiment_campaign(&campaign)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let mut launch_message = None;
    if let Some(trial) = latest_trial(store, campaign.id).await?
        && let Some(lease) = latest_active_lease(store, trial.id).await?
    {
        launch_message = Some(
            revoke_lease_with_runner(
                store,
                user_id,
                &campaign,
                &lease,
                RemoteLaunchAction::Cancel,
            )
            .await?,
        );
    }
    maybe_launch_next_queued_after_slot_release(store, user_id).await?;
    Ok(ExperimentCampaignActionResponse {
        campaign,
        trial: None,
        lease: None,
        launch: None,
        message: launch_message
            .unwrap_or_else(|| experiment_campaign_cancelled_message().to_string()),
    })
}

pub async fn resume_campaign(
    store: &Arc<dyn Database>,
    user_id: &str,
    campaign_id: Uuid,
) -> ApiResult<ExperimentCampaignActionResponse> {
    let settings = ensure_experiments_enabled(store, user_id).await?;
    let mut campaign = get_campaign(store, user_id, campaign_id).await?;
    let project = get_project(store, user_id, campaign.project_id).await?;
    let runner = get_runner(store, user_id, campaign.runner_profile_id).await?;

    if let Some(active) = active_trial(store, campaign.id).await? {
        return Err(ApiError::InvalidInput(format!(
            "Campaign already has an active trial ({})",
            active.id
        )));
    }

    if project.autonomy_mode != ExperimentAutonomyMode::ManualCandidate {
        campaign.status = ExperimentCampaignStatus::Running;
        campaign.queue_state = ExperimentCampaignQueueState::NotQueued;
        campaign.queue_position = 0;
        campaign.pause_reason = None;
        campaign.updated_at = Utc::now();
        store
            .update_experiment_campaign(&campaign)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        launch_next_trial_if_ready(store, user_id, &settings, &project, &runner, &mut campaign)
            .await?;
        let refreshed = get_campaign(store, user_id, campaign.id).await?;
        let trial = latest_trial(store, campaign.id).await?;
        return Ok(ExperimentCampaignActionResponse {
            campaign: refreshed,
            trial,
            lease: None,
            launch: None,
            message: "Campaign resumed.".to_string(),
        });
    }

    let worktree_path = campaign.worktree_path.clone().ok_or_else(|| {
        ApiError::InvalidInput(experiment_campaign_has_no_worktree_message().to_string())
    })?;
    let filtered_changed_files = filtered_changed_files(git_changed_files(&worktree_path).await?);
    let sequence = latest_trial(store, campaign.id)
        .await?
        .map(|trial| trial.sequence + 1)
        .unwrap_or(1);
    let trial_id = Uuid::new_v4();
    let trial = prepare_candidate_trial_from_worktree(
        store,
        &campaign,
        &project,
        &runner,
        trial_id,
        sequence,
        "Manual candidate submitted for evaluation.".to_string(),
        format!(
            "Candidate diff staged from campaign worktree ({} changed paths).",
            filtered_changed_files.len()
        ),
        "manual_candidate".to_string(),
        serde_json::json!({
            "candidate_source": "manual_candidate",
            "changed_paths": filtered_changed_files,
            "workspace": worktree_path,
        }),
    )
    .await?;
    campaign.status = ExperimentCampaignStatus::Running;
    campaign.queue_state = ExperimentCampaignQueueState::Active;
    campaign.queue_position = 0;
    campaign.pause_reason = None;
    campaign.updated_at = Utc::now();
    store
        .update_experiment_campaign(&campaign)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    store
        .create_experiment_trial(&trial)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let response = launch_trial(
        store, user_id, &settings, &project, &runner, campaign, trial,
    )
    .await?;
    maybe_launch_next_queued_after_slot_release(store, user_id).await?;
    Ok(response)
}

pub async fn reissue_lease(
    store: &Arc<dyn Database>,
    user_id: &str,
    campaign_id: Uuid,
) -> ApiResult<ExperimentCampaignActionResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let mut campaign = get_campaign(store, user_id, campaign_id).await?;
    let project = get_project(store, user_id, campaign.project_id).await?;
    let runner = get_runner(store, user_id, campaign.runner_profile_id).await?;
    if !runner.backend.is_remote() {
        return Err(ApiError::InvalidInput(
            experiment_lease_reissue_remote_only_message().to_string(),
        ));
    }
    let mut trial = latest_trial(store, campaign.id).await?.ok_or_else(|| {
        ApiError::InvalidInput(experiment_campaign_has_no_trial_to_reissue_message().to_string())
    })?;
    if matches!(
        trial.status,
        ExperimentTrialStatus::Accepted
            | ExperimentTrialStatus::Rejected
            | ExperimentTrialStatus::Crashed
            | ExperimentTrialStatus::TimedOut
            | ExperimentTrialStatus::InfraFailed
    ) {
        return Err(ApiError::InvalidInput(
            experiment_remote_trial_reissue_in_flight_only_message().to_string(),
        ));
    }

    if let Some(lease) = latest_active_lease(store, trial.id).await? {
        let _ = revoke_lease_with_runner(
            store,
            user_id,
            &campaign,
            &lease,
            RemoteLaunchAction::Reissue,
        )
        .await?;
    }
    let lease = create_lease(store, user_id, &project, &runner, &campaign, &trial).await?;
    let provider_api_key = research_provider_api_key(user_id, &runner).await;
    let launch_outcome = adapters::try_auto_launch(
        &runner,
        campaign_gateway_url(&campaign).as_deref(),
        &lease,
        provider_api_key.as_deref(),
    )
    .await
    .unwrap_or_else(|err| RunnerLaunchOutcome {
        message: err,
        bootstrap_command: campaign_gateway_url(&campaign)
            .as_deref()
            .map(|gateway| adapters::build_bootstrap_command(gateway, &lease)),
        provider_template: None,
        provider_job_id: None,
        provider_job_metadata: serde_json::json!({}),
        auto_launched: false,
        requires_operator_action: true,
    });

    trial.status = if launch_outcome.auto_launched {
        ExperimentTrialStatus::Running
    } else {
        ExperimentTrialStatus::Preparing
    };
    if launch_outcome.auto_launched {
        trial.started_at = Some(Utc::now());
    }
    trial.summary = Some(launch_outcome.message.clone());
    trial.provider_job_id = launch_outcome.provider_job_id.clone();
    trial.provider_job_metadata = launch_outcome.provider_job_metadata.clone();
    trial.updated_at = Utc::now();
    campaign.status = ExperimentCampaignStatus::Running;
    campaign.queue_state = ExperimentCampaignQueueState::Active;
    campaign.pause_reason = Some(launch_outcome.message.clone());
    campaign.updated_at = Utc::now();
    store
        .update_experiment_trial(&trial)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    store
        .update_experiment_campaign(&campaign)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(ExperimentCampaignActionResponse {
        campaign,
        trial: Some(trial),
        lease: Some(lease),
        launch: Some(launch_details_from_outcome(launch_outcome)),
        message: "Lease reissued.".to_string(),
    })
}

pub async fn promote_campaign(
    store: &Arc<dyn Database>,
    user_id: &str,
    campaign_id: Uuid,
) -> ApiResult<ExperimentCampaignActionResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let mut campaign = get_campaign(store, user_id, campaign_id).await?;
    let project = get_project(store, user_id, campaign.project_id).await?;
    let best_commit = campaign
        .best_commit
        .clone()
        .or(campaign.baseline_commit.clone())
        .ok_or_else(|| {
            ApiError::InvalidInput(experiment_campaign_has_no_accepted_commit_message().to_string())
        })?;
    let promotion_branch = format!("codex/experiment-review/{}", short_id(campaign.id));
    let _ = git_output(
        &project.workspace_path,
        &["branch", "-f", &promotion_branch, &best_commit],
    )
    .await?;

    let mut message = format!("Created review branch {promotion_branch} at {best_commit}.");
    if project.promotion_mode == "branch_pr_draft" {
        let push_result = git_output(
            &project.workspace_path,
            &["push", "-u", &project.git_remote_name, &promotion_branch],
        )
        .await;
        if push_result.is_ok() {
            let title = format!("Experiment promotion: {}", project.name);
            let body = experiment_promotion_pr_body(
                campaign.id,
                &best_commit,
                &project.primary_metric.name,
            );
            let pr_result = run_command_capture(
                Some(Path::new(&project.workspace_path)),
                "gh",
                &[
                    "pr",
                    "create",
                    "--draft",
                    "--base",
                    &project.base_branch,
                    "--head",
                    &promotion_branch,
                    "--title",
                    &title,
                    "--body",
                    &body,
                ],
                &[],
            )
            .await;
            if let Ok(output) = pr_result
                && !output.trim().is_empty()
            {
                message.push(' ');
                message.push_str(output.trim());
            }
        }
    }
    campaign.status = ExperimentCampaignStatus::AwaitingPromotion;
    campaign.pause_reason = Some(message.clone());
    campaign.updated_at = Utc::now();
    store
        .update_experiment_campaign(&campaign)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(ExperimentCampaignActionResponse {
        campaign,
        trial: None,
        lease: None,
        launch: None,
        message,
    })
}
