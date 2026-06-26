use super::*;

pub(super) async fn latest_active_lease(
    store: &Arc<dyn Database>,
    trial_id: Uuid,
) -> ApiResult<Option<ExperimentLease>> {
    let lease = store
        .get_experiment_lease_for_trial(trial_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(lease.filter(|lease| {
        matches!(
            lease.status,
            ExperimentLeaseStatus::Pending | ExperimentLeaseStatus::Claimed
        )
    }))
}

pub async fn lease_job(
    store: &Arc<dyn Database>,
    user_id: &str,
    lease_id: Uuid,
    token: &str,
) -> ApiResult<ExperimentLeaseJobResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let mut lease = verified_lease(store, lease_id, token).await?;
    if lease.status == ExperimentLeaseStatus::Revoked {
        return Err(ApiError::Unavailable(
            experiment_lease_revoked_message().to_string(),
        ));
    }
    if lease.status == ExperimentLeaseStatus::Pending {
        lease.status = ExperimentLeaseStatus::Claimed;
        lease.claimed_at = Some(Utc::now());
        lease.updated_at = Utc::now();
        store
            .update_experiment_lease(&lease)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    }
    let job: ExperimentRunnerJob =
        serde_json::from_value(lease.job_payload.clone()).map_err(ApiError::Serialization)?;
    Ok(ExperimentLeaseJobResponse { job })
}

pub async fn lease_credentials(
    store: &Arc<dyn Database>,
    user_id: &str,
    lease_id: Uuid,
    token: &str,
) -> ApiResult<ExperimentLeaseCredentialsResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let lease = verified_lease(store, lease_id, token).await?;
    Ok(ExperimentLeaseCredentialsResponse {
        credentials: lease.credentials_payload,
    })
}

pub async fn lease_status(
    store: &Arc<dyn Database>,
    user_id: &str,
    lease_id: Uuid,
    token: &str,
    req: ExperimentLeaseStatusRequest,
) -> ApiResult<ExperimentCampaignActionResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let lease = verified_lease(store, lease_id, token).await?;
    let mut trial = store
        .get_experiment_trial(lease.trial_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| {
            ApiError::SessionNotFound(experiment_trial_not_found_message(lease.trial_id))
        })?;
    trial.summary = Some(req.status.clone());
    trial.status = lease_runner_trial_status(&req.status, trial.status);
    if matches!(
        trial.status,
        ExperimentTrialStatus::Running | ExperimentTrialStatus::Evaluating
    ) && trial.started_at.is_none()
    {
        trial.started_at = Some(Utc::now());
    }
    if let Some(metadata) = req.metadata {
        trial.artifact_manifest_json = merge_json(&trial.artifact_manifest_json, &metadata);
    }
    trial.updated_at = Utc::now();
    store
        .update_experiment_trial(&trial)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let campaign = get_campaign(store, user_id, lease.campaign_id).await?;
    Ok(ExperimentCampaignActionResponse {
        campaign,
        trial: Some(trial),
        lease: None,
        launch: None,
        message: "Lease status recorded.".to_string(),
    })
}

pub async fn lease_event(
    store: &Arc<dyn Database>,
    user_id: &str,
    lease_id: Uuid,
    token: &str,
    req: ExperimentLeaseEventRequest,
) -> ApiResult<ExperimentCampaignActionResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let lease = verified_lease(store, lease_id, token).await?;
    let mut trial = store
        .get_experiment_trial(lease.trial_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| {
            ApiError::SessionNotFound(experiment_trial_not_found_message(lease.trial_id))
        })?;
    let mut manifest = if trial.artifact_manifest_json.is_object() {
        trial.artifact_manifest_json.clone()
    } else {
        serde_json::json!({})
    };
    let event_entry = serde_json::json!({
        "message": req.message,
        "metadata": req.metadata,
        "at": Utc::now().to_rfc3339(),
    });
    let events = manifest
        .as_object_mut()
        .expect("manifest initialized as object")
        .entry("events".to_string())
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    if let Some(array) = events.as_array_mut() {
        array.push(event_entry);
    }
    trial.artifact_manifest_json = manifest;
    trial.updated_at = Utc::now();
    store
        .update_experiment_trial(&trial)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let campaign = get_campaign(store, user_id, lease.campaign_id).await?;
    Ok(ExperimentCampaignActionResponse {
        campaign,
        trial: Some(trial),
        lease: None,
        launch: None,
        message: "Lease event recorded.".to_string(),
    })
}

pub async fn lease_artifact(
    store: &Arc<dyn Database>,
    user_id: &str,
    lease_id: Uuid,
    token: &str,
    artifact: ExperimentRunnerArtifactUpload,
) -> ApiResult<ExperimentCampaignActionResponse> {
    let artifact_store = LocalArtifactStore::shared_default();
    lease_artifact_with_store(store, &artifact_store, user_id, lease_id, token, artifact).await
}

/// Core of [`lease_artifact`] parameterized over the durable [`ArtifactStore`] so
/// tests can inject a temp-rooted store. When the runner attaches inline
/// `content_base64`, the bytes are persisted to durable host storage and the
/// recorded `ExperimentArtifactRef` points at the durable path with
/// `fetchable: true`; otherwise the upload is recorded as posted (pod-local
/// breadcrumb only).
pub(super) async fn lease_artifact_with_store(
    store: &Arc<dyn Database>,
    artifact_store: &Arc<dyn ArtifactStore>,
    user_id: &str,
    lease_id: Uuid,
    token: &str,
    artifact: ExperimentRunnerArtifactUpload,
) -> ApiResult<ExperimentCampaignActionResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let lease = verified_lease(store, lease_id, token).await?;
    let mut artifacts = store
        .list_experiment_artifacts(lease.trial_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let artifact_id = Uuid::new_v4();
    let mut uri_or_local_path = artifact.uri_or_local_path;
    let mut size_bytes = artifact.size_bytes;
    let mut fetchable = artifact.fetchable;
    if let Some(content_base64) = artifact.content_base64 {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(content_base64.as_bytes())
            .map_err(|e| ApiError::InvalidInput(format!("invalid artifact content_base64: {e}")))?;
        let durable = artifact_store
            .put(lease.trial_id, artifact_id, &artifact.kind, &bytes)
            .await
            .map_err(|e| ApiError::Internal(format!("failed to persist artifact: {e}")))?;
        size_bytes = Some(bytes.len() as u64);
        uri_or_local_path = durable;
        fetchable = true;
    }

    artifacts.push(ExperimentArtifactRef {
        id: artifact_id,
        trial_id: lease.trial_id,
        kind: artifact.kind,
        uri_or_local_path,
        size_bytes,
        fetchable,
        metadata: artifact.metadata,
        created_at: Utc::now(),
    });
    store
        .replace_experiment_artifacts(lease.trial_id, &artifacts)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let campaign = get_campaign(store, user_id, lease.campaign_id).await?;
    Ok(ExperimentCampaignActionResponse {
        campaign,
        trial: None,
        lease: None,
        launch: None,
        message: "Artifact recorded.".to_string(),
    })
}

pub async fn lease_complete(
    store: &Arc<dyn Database>,
    user_id: &str,
    lease_id: Uuid,
    token: &str,
    completion: ExperimentRunnerCompletion,
) -> ApiResult<ExperimentCampaignActionResponse> {
    ensure_experiments_enabled(store, user_id).await?;
    let mut lease = verified_lease(store, lease_id, token).await?;
    let mut campaign = get_campaign(store, user_id, lease.campaign_id).await?;
    let project = get_project(store, user_id, campaign.project_id).await?;
    let mut trial = get_trial(store, user_id, lease.trial_id).await?;
    complete_trial_terminal(
        store,
        &project,
        &mut campaign,
        &mut trial,
        Some(&mut lease),
        completion,
    )
    .await?;
    maybe_launch_next_queued_after_slot_release(store, user_id).await?;
    Ok(ExperimentCampaignActionResponse {
        campaign,
        trial: Some(trial),
        lease: None,
        launch: None,
        message: "Lease completed.".to_string(),
    })
}

pub async fn lease_owner_user_id(
    store: &Arc<dyn Database>,
    lease_id: Uuid,
    token: &str,
) -> ApiResult<String> {
    let lease = verified_lease(store, lease_id, token).await?;
    let campaign = store
        .get_experiment_campaign(lease.campaign_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| {
            ApiError::SessionNotFound(experiment_campaign_not_found_message(lease.campaign_id))
        })?;
    Ok(campaign.owner_user_id)
}

pub(super) async fn create_lease(
    store: &Arc<dyn Database>,
    user_id: &str,
    project: &ExperimentProject,
    runner: &ExperimentRunnerProfile,
    campaign: &ExperimentCampaign,
    trial: &ExperimentTrial,
) -> ApiResult<ExperimentLeaseAuthentication> {
    let token = format!("exp_{}_{}", short_id(campaign.id), Uuid::new_v4().simple());
    let repo_url = git_output(
        &project.workspace_path,
        &["remote", "get-url", &project.git_remote_name],
    )
    .await?;
    let resolved_env_grants = resolved_runner_env_grants(user_id, runner).await;
    let git_ref = campaign.experiment_branch.clone().ok_or_else(|| {
        ApiError::InvalidInput(experiment_campaign_missing_experiment_branch_message().to_string())
    })?;
    let job = ExperimentRunnerJob {
        lease_id: Uuid::new_v4(),
        trial_id: trial.id,
        campaign_id: campaign.id,
        project_id: project.id,
        runner_profile_id: runner.id,
        backend: runner.backend,
        repo_url,
        git_ref,
        workdir: project.workdir.clone(),
        prepare_command: project.prepare_command.clone(),
        run_command: project.run_command.clone(),
        primary_metric: project.primary_metric.clone(),
        secondary_metrics: project.secondary_metrics.clone(),
        env_grants: resolved_env_grants.clone(),
        artifact_paths: vec!["run.log".to_string(), "summary.json".to_string()],
    };
    let lease = ExperimentLease {
        id: job.lease_id,
        campaign_id: campaign.id,
        trial_id: trial.id,
        runner_profile_id: runner.id,
        status: ExperimentLeaseStatus::Pending,
        token_hash: hash_lease_token(&token),
        job_payload: serde_json::to_value(&job).map_err(|e| ApiError::Internal(e.to_string()))?,
        credentials_payload: serde_json::json!({
            "env": resolved_env_grants,
            "secret_references": runner.secret_references,
        }),
        expires_at: Utc::now() + chrono::Duration::minutes(DEFAULT_REMOTE_LEASE_MINUTES),
        claimed_at: None,
        completed_at: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    store
        .create_experiment_lease(&lease)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(ExperimentLeaseAuthentication {
        lease_id: lease.id,
        token,
    })
}

pub(super) async fn verified_lease(
    store: &Arc<dyn Database>,
    lease_id: Uuid,
    token: &str,
) -> ApiResult<ExperimentLease> {
    let lease = store
        .get_experiment_lease(lease_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(experiment_lease_not_found_message(lease_id)))?;
    if lease.expires_at < Utc::now() {
        return Err(ApiError::Unavailable(
            experiment_lease_expired_message().to_string(),
        ));
    }
    if lease.token_hash != hash_lease_token(token) {
        return Err(ApiError::InvalidInput(
            invalid_experiment_lease_token_message().to_string(),
        ));
    }
    Ok(lease)
}

pub(super) async fn latest_trial(
    store: &Arc<dyn Database>,
    campaign_id: Uuid,
) -> ApiResult<Option<ExperimentTrial>> {
    let mut trials = store
        .list_experiment_trials(campaign_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(trials.pop())
}

pub(super) async fn active_trial(
    store: &Arc<dyn Database>,
    campaign_id: Uuid,
) -> ApiResult<Option<ExperimentTrial>> {
    let trials = store
        .list_experiment_trials(campaign_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(trials.into_iter().find(|trial| {
        matches!(
            trial.status,
            ExperimentTrialStatus::Preparing
                | ExperimentTrialStatus::Running
                | ExperimentTrialStatus::Evaluating
        )
    }))
}
