use super::*;

pub async fn start_experiment_controller_loop(store: Arc<dyn Database>) {
    let (_shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    start_experiment_controller_loop_with_shutdown(store, shutdown_rx).await;
}

pub async fn start_experiment_controller_loop_with_shutdown(
    store: Arc<dyn Database>,
    mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
) {
    let mut interval = interval(TokioDuration::from_secs(
        DEFAULT_EXPERIMENT_CONTROLLER_TICK_SECS,
    ));
    interval.tick().await;
    loop {
        tokio::select! {
            _ = &mut shutdown_rx => {
                tracing::debug!("Experiment controller loop stopped");
                break;
            }
            _ = reconcile_experiment_controller_pass(&store) => {}
        }
        tokio::select! {
            _ = &mut shutdown_rx => {
                tracing::debug!("Experiment controller loop stopped");
                break;
            }
            _ = interval.tick() => {}
        }
    }
}

/// Background loop that enforces `experiments.default_artifact_retention_days` by
/// pruning `experiment_artifact_refs` (and best-effort the underlying durable
/// files) older than the retention window. Mirrors the controller loop shape:
/// tick-first `interval`, `tracing::warn!` on error, never panics the task.
///
/// A `retention_days` of `0` disables reaping (the loop still runs but each pass
/// is a no-op), so an operator can opt out without tearing down the task.
pub async fn start_experiment_artifact_reaper_loop(store: Arc<dyn Database>, retention_days: u32) {
    let (_shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    start_experiment_artifact_reaper_loop_with_shutdown(store, retention_days, shutdown_rx).await;
}

pub async fn start_experiment_artifact_reaper_loop_with_shutdown(
    store: Arc<dyn Database>,
    retention_days: u32,
    mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
) {
    let mut interval = interval(TokioDuration::from_secs(DEFAULT_ARTIFACT_REAPER_TICK_SECS));
    interval.tick().await;
    loop {
        tokio::select! {
            _ = &mut shutdown_rx => {
                tracing::debug!("Experiment artifact reaper loop stopped");
                break;
            }
            _ = experiment_artifact_reaper_pass(&store, retention_days) => {}
        }
        tokio::select! {
            _ = &mut shutdown_rx => {
                tracing::debug!("Experiment artifact reaper loop stopped");
                break;
            }
            _ = interval.tick() => {}
        }
    }
}

async fn reconcile_experiment_controller_pass(store: &Arc<dyn Database>) {
    match reconcile_experiments_once(store).await {
        Ok(()) => {}
        Err(error) => match error {
            ApiError::FeatureDisabled(_) => {
                tracing::debug!("Experiment controller loop skipped: experiments are disabled");
            }
            _ => tracing::warn!("Experiment controller reconcile failed: {error}"),
        },
    }
}

async fn experiment_artifact_reaper_pass(store: &Arc<dyn Database>, retention_days: u32) {
    match reap_expired_artifacts_once(store, retention_days).await {
        Ok(pruned) => {
            if pruned > 0 {
                tracing::info!("Experiment artifact reaper pruned {pruned} expired artifact(s)");
            }
        }
        Err(error) => {
            tracing::warn!("Experiment artifact reaper failed: {error}");
        }
    }
}

/// Run a single reaper pass: prune every artifact whose `created_at` is older than
/// `now - retention_days`, best-effort deleting the on-disk file when it lives
/// under the durable artifact root. Returns the number of pruned artifact rows.
///
/// Deletion goes through the `Database` trait (no backend-specific code) and
/// re-persists the surviving set via `replace_experiment_artifacts`, mirroring the
/// list/mutate/replace round-trip used by ingest.
pub(super) async fn reap_expired_artifacts_once(
    store: &Arc<dyn Database>,
    retention_days: u32,
) -> ApiResult<usize> {
    if retention_days == 0 {
        return Ok(0);
    }
    let cutoff = Utc::now() - chrono::Duration::days(retention_days as i64);
    let artifact_root = crate::experiments::artifact_store::default_artifact_root();
    let campaigns = store
        .list_experiment_campaigns()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let mut pruned_total = 0usize;
    for campaign in campaigns {
        let trials = store
            .list_experiment_trials(campaign.id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        for trial in trials {
            let artifacts = store
                .list_experiment_artifacts(trial.id)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            let (expired, surviving): (Vec<_>, Vec<_>) = artifacts
                .into_iter()
                .partition(|artifact| artifact.created_at < cutoff);
            if expired.is_empty() {
                continue;
            }
            for artifact in &expired {
                remove_durable_artifact_file(&artifact_root, &artifact.uri_or_local_path).await;
            }
            store
                .replace_experiment_artifacts(trial.id, &surviving)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            pruned_total += expired.len();
        }
    }
    Ok(pruned_total)
}

/// Best-effort delete a durable artifact file, but only when its canonicalized
/// path is contained within `artifact_root`. Runner-supplied pod-local paths and
/// any path outside the operator-controlled root are left untouched — never delete
/// based on an unvalidated runner path (see WS-07 pitfalls).
pub(super) async fn remove_durable_artifact_file(artifact_root: &Path, raw_path: &str) {
    let path = PathBuf::from(raw_path);
    let canonical_root = match tokio::fs::canonicalize(artifact_root).await {
        Ok(root) => root,
        Err(_) => return,
    };
    let canonical_path = match tokio::fs::canonicalize(&path).await {
        Ok(p) => p,
        Err(_) => return,
    };
    if !canonical_path.starts_with(&canonical_root) {
        return;
    }
    if let Err(error) = tokio::fs::remove_file(&canonical_path).await {
        tracing::debug!(
            "Experiment artifact reaper could not remove {}: {error}",
            canonical_path.display()
        );
    }
}

pub(super) async fn reconcile_experiments_once(store: &Arc<dyn Database>) -> ApiResult<()> {
    let campaigns = store
        .list_experiment_campaigns()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let mut owners = HashSet::new();

    for mut campaign in campaigns {
        owners.insert(campaign.owner_user_id.clone());
        if matches!(
            campaign.status,
            ExperimentCampaignStatus::Completed
                | ExperimentCampaignStatus::Cancelled
                | ExperimentCampaignStatus::Failed
        ) {
            campaign.queue_state = ExperimentCampaignQueueState::NotQueued;
            continue;
        }

        if campaign.status == ExperimentCampaignStatus::PendingBaseline
            && campaign.queue_state == ExperimentCampaignQueueState::Queued
        {
            continue;
        }

        if campaign.status == ExperimentCampaignStatus::Running
            || (campaign.status == ExperimentCampaignStatus::PendingBaseline
                && campaign.queue_state != ExperimentCampaignQueueState::Queued)
        {
            let owner_user_id = campaign.owner_user_id.clone();
            reconcile_active_campaign(store, &owner_user_id, &mut campaign).await?;
        }
    }

    for owner_user_id in owners {
        maybe_launch_next_queued_after_slot_release(store, &owner_user_id).await?;
    }
    Ok(())
}

pub(super) async fn reconcile_active_campaign(
    store: &Arc<dyn Database>,
    user_id: &str,
    campaign: &mut ExperimentCampaign,
) -> ApiResult<()> {
    let project = get_project(store, user_id, campaign.project_id).await?;
    let runner = get_runner(store, user_id, campaign.runner_profile_id).await?;
    let settings = ensure_experiments_enabled(store, user_id).await?;
    let latest = latest_trial(store, campaign.id).await?;

    let max_trials = campaign
        .max_trials_override
        .or(project.stop_policy.max_trials);
    let max_trials_reached = latest
        .as_ref()
        .map(|trial| max_trials.is_some_and(|limit| trial.sequence >= limit))
        .unwrap_or(false);
    let runtime_budget_reached = project
        .stop_policy
        .max_total_runtime_secs
        .is_some_and(|limit| campaign.total_runtime_ms / 1000 >= limit);
    let cost_budget_reached = project
        .stop_policy
        .max_total_cost_usd
        .is_some_and(|limit| campaign.total_cost_usd >= limit);
    let infra_failure_threshold_reached =
        campaign.failure_count >= project.stop_policy.infra_failure_pause_threshold;
    let plateau_window = project
        .stop_policy
        .plateau_window
        .unwrap_or(project.stop_policy.non_improving_pause_threshold);
    let non_improving_threshold_reached =
        campaign.consecutive_non_improving_trials >= plateau_window;

    if let Some(mut trial) = latest {
        if matches!(
            trial.status,
            ExperimentTrialStatus::Preparing
                | ExperimentTrialStatus::Running
                | ExperimentTrialStatus::Evaluating
        ) {
            if let Some(lease) = latest_active_lease(store, trial.id).await? {
                if is_stale_lease(&lease, Utc::now()) {
                    trial.status = ExperimentTrialStatus::TimedOut;
                    trial.decision_reason = Some(
                        "Tracked lease was stale while trial was in-flight. Campaign paused for operator review.".to_string(),
                    );
                    trial.updated_at = Utc::now();
                    campaign.active_trial_id = None;
                    campaign.failure_count = campaign.failure_count.saturating_add(1);
                    campaign.status = ExperimentCampaignStatus::Paused;
                    campaign.queue_state = ExperimentCampaignQueueState::NotQueued;
                    campaign.pause_reason = Some(
                        "Tracked lease was stale and could not be confirmed. Reissue lease or resume manually."
                            .to_string(),
                    );
                    campaign.updated_at = Utc::now();
                    store
                        .update_experiment_trial(&trial)
                        .await
                        .map_err(|e| ApiError::Internal(e.to_string()))?;
                    store
                        .update_experiment_campaign(campaign)
                        .await
                        .map_err(|e| ApiError::Internal(e.to_string()))?;
                    maybe_launch_next_queued_after_slot_release(store, user_id).await?;
                    return Ok(());
                }

                return Ok(());
            }

            if runner.backend.is_remote() {
                campaign.active_trial_id = None;
                campaign.failure_count = campaign.failure_count.saturating_add(1);
                campaign.status = ExperimentCampaignStatus::Paused;
                campaign.queue_state = ExperimentCampaignQueueState::NotQueued;
                campaign.pause_reason = Some(
                    "Running remote trial is missing a claimed lease after restart. Reissue the lease or retry manually."
                        .to_string(),
                );
                campaign.updated_at = Utc::now();
                campaign.trial_count = campaign.trial_count.max(trial.sequence);
                store
                    .update_experiment_campaign(campaign)
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?;
                maybe_launch_next_queued_after_slot_release(store, user_id).await?;
            }
            return Ok(());
        }

        if campaign.status == ExperimentCampaignStatus::Running {
            if max_trials_reached {
                campaign.status = ExperimentCampaignStatus::AwaitingPromotion;
                campaign.pause_reason = Some(format!(
                    "Reached max_trials={limit}. Promote the best commit when ready.",
                    limit = max_trials.unwrap_or(0)
                ));
                campaign.updated_at = Utc::now();
                store
                    .update_experiment_campaign(campaign)
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?;
                maybe_launch_next_queued_after_slot_release(store, user_id).await?;
                return Ok(());
            }

            if runtime_budget_reached {
                campaign.status = if campaign.best_commit.is_some() {
                    ExperimentCampaignStatus::AwaitingPromotion
                } else {
                    ExperimentCampaignStatus::Failed
                };
                campaign.pause_reason = Some(
                    "Reached the campaign runtime budget. Promote the best commit when ready."
                        .to_string(),
                );
                campaign.updated_at = Utc::now();
                store
                    .update_experiment_campaign(campaign)
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?;
                maybe_launch_next_queued_after_slot_release(store, user_id).await?;
                return Ok(());
            }

            if cost_budget_reached {
                campaign.status = if campaign.best_commit.is_some() {
                    ExperimentCampaignStatus::AwaitingPromotion
                } else {
                    ExperimentCampaignStatus::Failed
                };
                campaign.pause_reason = Some(
                    "Reached the campaign cost budget. Promote the best commit when ready."
                        .to_string(),
                );
                campaign.updated_at = Utc::now();
                store
                    .update_experiment_campaign(campaign)
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?;
                maybe_launch_next_queued_after_slot_release(store, user_id).await?;
                return Ok(());
            }

            if infra_failure_threshold_reached || non_improving_threshold_reached {
                campaign.status = ExperimentCampaignStatus::Paused;
                campaign.pause_reason = Some(format!(
                    "Campaign paused after hitting configured thresholds (infra failures: {}, non-improving trials: {}).",
                    campaign.failure_count, campaign.consecutive_non_improving_trials
                ));
                campaign.updated_at = Utc::now();
                store
                    .update_experiment_campaign(campaign)
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?;
                maybe_launch_next_queued_after_slot_release(store, user_id).await?;
                return Ok(());
            }

            return launch_next_trial_if_ready(
                store, user_id, &settings, &project, &runner, campaign,
            )
            .await
            .map(|_| ());
        }

        if campaign.status == ExperimentCampaignStatus::Running {
            return launch_next_trial_if_ready(
                store, user_id, &settings, &project, &runner, campaign,
            )
            .await
            .map(|_| ());
        }
    }

    if campaign.status == ExperimentCampaignStatus::Running {
        campaign.status = ExperimentCampaignStatus::Paused;
        campaign.queue_state = ExperimentCampaignQueueState::NotQueued;
        campaign.pause_reason = Some(
            "Campaign state recovery could not find a valid trial record. Resume manually."
                .to_string(),
        );
        campaign.updated_at = Utc::now();
        store
            .update_experiment_campaign(campaign)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        maybe_launch_next_queued_after_slot_release(store, user_id).await?;
    }

    Ok(())
}

pub(super) fn is_stale_lease(lease: &ExperimentLease, now: DateTime<Utc>) -> bool {
    is_stale_lease_policy(lease, now, STALE_LEASE_GRACE_MINUTES)
}

pub(super) async fn launch_next_trial_if_ready(
    store: &Arc<dyn Database>,
    user_id: &str,
    settings: &Settings,
    project: &ExperimentProject,
    runner: &ExperimentRunnerProfile,
    campaign: &mut ExperimentCampaign,
) -> ApiResult<()> {
    if campaign.queue_state == ExperimentCampaignQueueState::Active {
        return Ok(());
    }

    match project.autonomy_mode {
        ExperimentAutonomyMode::ManualCandidate => {
            campaign.status = ExperimentCampaignStatus::Paused;
            campaign.queue_state = ExperimentCampaignQueueState::NotQueued;
            campaign.pause_reason =
                Some("Awaiting manual candidate changes in the campaign worktree.".to_string());
            campaign.updated_at = Utc::now();
            store
                .update_experiment_campaign(campaign)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            maybe_launch_next_queued_after_slot_release(store, user_id).await?;
            return Ok(());
        }
        ExperimentAutonomyMode::SuggestOnly => {
            let planner = match run_planner_subagent(store, campaign, project, None).await {
                Ok(planner) => planner,
                Err(ResearchSubagentInvocationError::Api(error)) => return Err(error),
                Err(ResearchSubagentInvocationError::Run(error)) => {
                    let error = *error;
                    campaign.status = ExperimentCampaignStatus::Paused;
                    campaign.queue_state = ExperimentCampaignQueueState::NotQueued;
                    campaign.pause_reason =
                        Some(format!("Suggestion generation paused: {}", error.message));
                    record_campaign_candidate_generation(
                        campaign,
                        "suggest_only",
                        "failed",
                        &error.message,
                        &[error.run_artifact],
                    );
                    campaign.updated_at = Utc::now();
                    store
                        .update_experiment_campaign(campaign)
                        .await
                        .map_err(|e| ApiError::Internal(e.to_string()))?;
                    maybe_launch_next_queued_after_slot_release(store, user_id).await?;
                    return Ok(());
                }
            };
            campaign.status = ExperimentCampaignStatus::Paused;
            campaign.queue_state = ExperimentCampaignQueueState::NotQueued;
            campaign.pause_reason = Some(format!(
                "Suggestion ready: {}",
                truncate_for_prompt(&planner.value.mutation_brief, 500)
            ));
            record_campaign_candidate_generation(
                campaign,
                "suggest_only",
                "completed",
                &planner.value.mutation_brief,
                std::slice::from_ref(&planner.run_artifact),
            );
            campaign.updated_at = Utc::now();
            store
                .update_experiment_campaign(campaign)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            maybe_launch_next_queued_after_slot_release(store, user_id).await?;
            return Ok(());
        }
        ExperimentAutonomyMode::Autonomous => {}
    }

    let trial = match create_experiment_trial_commit(store, campaign, project, runner).await {
        Ok(trial) => trial,
        Err(error) => {
            campaign.status = ExperimentCampaignStatus::Paused;
            campaign.queue_state = ExperimentCampaignQueueState::NotQueued;
            campaign.pause_reason = Some(format!(
                "Autonomous candidate generation paused: {}",
                error.message
            ));
            record_campaign_candidate_generation(
                campaign,
                "autonomous",
                "failed",
                &error.message,
                &error.run_artifacts,
            );
            campaign.updated_at = Utc::now();
            store
                .update_experiment_campaign(campaign)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            maybe_launch_next_queued_after_slot_release(store, user_id).await?;
            return Ok(());
        }
    };
    store
        .create_experiment_trial(&trial)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let response = launch_trial(
        store,
        user_id,
        settings,
        project,
        runner,
        campaign.clone(),
        trial,
    )
    .await?;
    *campaign = response.campaign;
    Ok(())
}

pub(super) async fn active_campaign_count(store: &Arc<dyn Database>) -> ApiResult<usize> {
    let campaigns = store
        .list_experiment_campaigns()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(campaigns
        .into_iter()
        .filter(|campaign| {
            campaign.status == ExperimentCampaignStatus::Running
                || (campaign.status == ExperimentCampaignStatus::PendingBaseline
                    && campaign.queue_state != ExperimentCampaignQueueState::Queued)
        })
        .count())
}

pub(super) async fn next_queue_position(store: &Arc<dyn Database>) -> ApiResult<u32> {
    let campaigns = store
        .list_experiment_campaigns()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(campaigns
        .into_iter()
        .filter(|campaign| {
            campaign.status == ExperimentCampaignStatus::PendingBaseline
                && campaign.queue_state == ExperimentCampaignQueueState::Queued
        })
        .map(|campaign| campaign.queue_position)
        .max()
        .unwrap_or(0)
        .saturating_add(1))
}

pub(super) async fn next_queued_campaign_for_owner(
    store: &Arc<dyn Database>,
    owner_user_id: Option<&str>,
) -> ApiResult<Option<ExperimentCampaign>> {
    let mut queued: Vec<_> = store
        .list_experiment_campaigns()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .into_iter()
        .filter(|campaign| {
            campaign.status == ExperimentCampaignStatus::PendingBaseline
                && campaign.queue_state == ExperimentCampaignQueueState::Queued
                && owner_user_id.is_none_or(|owner| campaign.owner_user_id == owner)
        })
        .collect();
    queued.sort_by_key(|campaign| (campaign.queue_position, campaign.created_at));
    Ok(queued.into_iter().next())
}

pub(super) async fn maybe_launch_next_queued_campaign(
    store: &Arc<dyn Database>,
    user_id: &str,
) -> ApiResult<Option<ExperimentCampaignActionResponse>> {
    let settings = ensure_experiments_enabled(store, user_id).await?;
    let active_count = active_campaign_count(store).await?;
    if active_count >= settings.experiments.max_concurrent_campaigns as usize {
        return Ok(None);
    }

    let Some(mut campaign) = next_queued_campaign_for_owner(store, Some(user_id)).await? else {
        return Ok(None);
    };
    let campaign_owner_user_id = campaign.owner_user_id.clone();
    let project = get_project(store, &campaign_owner_user_id, campaign.project_id).await?;
    let runner = get_runner(store, &campaign_owner_user_id, campaign.runner_profile_id).await?;
    if let Err(error) = validate_project_launch_readiness(&project).await {
        campaign.status = ExperimentCampaignStatus::Failed;
        campaign.pause_reason = Some(format!(
            "Queued launch failed project validation: {}",
            error
        ));
        campaign.ended_at = Some(Utc::now());
        campaign.updated_at = Utc::now();
        store
            .update_experiment_campaign(&campaign)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        return Ok(Some(ExperimentCampaignActionResponse {
            campaign,
            trial: None,
            lease: None,
            launch: None,
            message: "Queued campaign failed project validation before launch.".to_string(),
        }));
    }

    let validation =
        validate_runner_profile_impl(&campaign_owner_user_id, &runner, &settings).await;
    if !validation.valid {
        campaign.status = ExperimentCampaignStatus::Failed;
        campaign.pause_reason = Some(format!(
            "Queued launch failed because runner '{}' is not valid: {}",
            runner.name, validation.message
        ));
        campaign.ended_at = Some(Utc::now());
        campaign.updated_at = Utc::now();
        store
            .update_experiment_campaign(&campaign)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        return Ok(Some(ExperimentCampaignActionResponse {
            campaign,
            trial: None,
            lease: None,
            launch: None,
            message: "Queued campaign failed validation before launch.".to_string(),
        }));
    }
    if !validation.launch_eligible {
        campaign.status = ExperimentCampaignStatus::Paused;
        campaign.queue_state = ExperimentCampaignQueueState::NotQueued;
        campaign.pause_reason = Some(format!(
            "Queued launch requires operator action because runner '{}' is {}.",
            runner.name,
            match validation.readiness_class {
                crate::experiments::ExperimentRunnerReadinessClass::ManualOnly => "manual_only",
                crate::experiments::ExperimentRunnerReadinessClass::BootstrapReady =>
                    "bootstrap_ready",
                crate::experiments::ExperimentRunnerReadinessClass::LaunchReady => "launch_ready",
            }
        ));
        campaign.updated_at = Utc::now();
        store
            .update_experiment_campaign(&campaign)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        return Ok(Some(ExperimentCampaignActionResponse {
            campaign,
            trial: None,
            lease: None,
            launch: None,
            message: "Queued campaign paused until an operator starts the runner manually."
                .to_string(),
        }));
    }

    match launch_campaign_baseline(
        store,
        &campaign_owner_user_id,
        &settings,
        &project,
        &runner,
        campaign.clone(),
    )
    .await
    {
        Ok(response) => Ok(Some(response)),
        Err(error) => {
            persist_campaign_launch_failure(store, campaign, &error.to_string()).await?;
            Err(error)
        }
    }
}

pub(super) async fn maybe_launch_next_queued_after_slot_release(
    store: &Arc<dyn Database>,
    user_id: &str,
) -> ApiResult<()> {
    loop {
        match maybe_launch_next_queued_campaign(store, user_id).await {
            Ok(Some(_)) => continue,
            Ok(None) => break,
            Err(error) => {
                tracing::warn!("failed to launch queued experiment campaign: {error}");
                break;
            }
        }
    }
    Ok(())
}
