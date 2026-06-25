use super::*;

pub(super) async fn validate_runner_profile_impl(
    user_id: &str,
    runner: &ExperimentRunnerProfile,
    settings: &Settings,
) -> crate::experiments::adapters::RunnerValidationOutcome {
    let provider_api_key = research_provider_api_key(user_id, runner).await;
    adapters::validate_runner_profile(runner, settings, provider_api_key.as_deref()).await
}

// WS-13: suspected worktree-teardown race. The quarantined E2E
// `autonomous_campaign_runs_planner_mutator_reviewer_and_docker_trial_end_to_end`
// (`#[ignore]`) fails with `Internal("No such file or directory (os error 2)")`.
// The race is between trial-completion cleanup (which restores the worktree to a
// clean committed state) and the next reconcile preparing the worktree here: the
// sequence below does `worktree remove --force` -> `worktree prune` ->
// `remove_dir_all` -> `create_dir_all(parent)`, and a git op can spawn against a
// worktree path that has just vanished mid-trial. Root-cause + de-quarantine is
// owned by WS-13; this WS only annotates the mechanism and does not change behavior.
pub(super) async fn prepare_campaign_worktree(
    project: &ExperimentProject,
    worktree_path: &Path,
) -> ApiResult<()> {
    if !Path::new(&project.workspace_path).exists() {
        return Err(ApiError::InvalidInput(
            experiment_workspace_path_missing_message(&project.workspace_path),
        ));
    }
    if worktree_path.exists() {
        let worktree = worktree_path.to_string_lossy().to_string();
        let _ = git_output(
            &project.workspace_path,
            &["worktree", "remove", "--force", &worktree],
        )
        .await;
        let _ = git_output(&project.workspace_path, &["worktree", "prune"]).await;
        tokio::fs::remove_dir_all(worktree_path)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    }
    if let Some(parent) = worktree_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    }
    Ok(())
}

pub(super) async fn push_experiment_branch(
    project: &ExperimentProject,
    worktree_path: &Path,
    branch: &str,
) -> ApiResult<()> {
    let worktree = worktree_path.to_string_lossy().to_string();
    let _ = git_output(&worktree, &["push", "-u", &project.git_remote_name, branch]).await?;
    Ok(())
}

pub(super) async fn git_changed_files(worktree_path: &str) -> ApiResult<Vec<String>> {
    let output = git_output_raw(worktree_path, &["status", "--porcelain", "-z"]).await?;
    let mut entries = output.split('\0').filter(|entry| !entry.is_empty());
    let mut changed_files = Vec::new();

    while let Some(entry) = entries.next() {
        if entry.len() < 4 {
            continue;
        }
        let status = &entry[..2];
        let primary_path = entry[3..].trim();
        let effective_path = if status.contains('R') || status.contains('C') {
            let _ = entries.next();
            primary_path
        } else {
            primary_path
        };
        if !effective_path.is_empty() {
            changed_files.push(effective_path.to_string());
        }
    }

    Ok(changed_files)
}

pub(super) fn enforce_mutable_paths(
    mutable_paths: &[String],
    changed_files: &[String],
) -> ApiResult<()> {
    enforce_mutable_paths_policy(mutable_paths, changed_files).map_err(ApiError::InvalidInput)
}

pub(super) fn experiment_sandbox_config(settings: &Settings) -> crate::sandbox::SandboxConfig {
    crate::config::SandboxModeConfig::resolve(settings)
        .unwrap_or_else(|_| crate::config::SandboxModeConfig {
            enabled: settings.sandbox.enabled,
            policy: settings.sandbox.policy.clone(),
            timeout_secs: settings.sandbox.timeout_secs,
            memory_limit_mb: settings.sandbox.memory_limit_mb,
            cpu_shares: settings.sandbox.cpu_shares,
            image: settings.sandbox.image.clone(),
            interactive_idle_timeout_secs: settings.sandbox.interactive_idle_timeout_secs,
            auto_pull_image: settings.sandbox.auto_pull_image,
            extra_allowed_domains: settings.sandbox.extra_allowed_domains.clone(),
        })
        .to_sandbox_config()
}

pub(super) fn experiment_execution_backend(
    settings: &Settings,
    runner: &ExperimentRunnerProfile,
) -> Arc<dyn ExecutionBackend> {
    match runner.backend {
        ExperimentRunnerBackend::LocalDocker => {
            let mut sandbox_config = experiment_sandbox_config(settings);
            sandbox_config.enabled = true;
            sandbox_config.policy = crate::sandbox::SandboxPolicy::WorkspaceWrite;
            if let Some(image) = runner
                .image_or_runtime
                .as_ref()
                .filter(|value| !value.trim().is_empty())
            {
                sandbox_config.image = image.trim().to_string();
            }
            DockerSandboxExecutionBackend::from_sandbox(
                Arc::new(crate::sandbox::SandboxManager::new(sandbox_config)),
                crate::sandbox::SandboxPolicy::WorkspaceWrite,
            )
        }
        _ => LocalHostExecutionBackend::shared(),
    }
}

pub(super) async fn run_experiment_shell_command(
    backend: Arc<dyn ExecutionBackend>,
    cwd: &Path,
    command: &str,
    env_grants: &serde_json::Value,
) -> ApiResult<ExecutionResult> {
    backend
        .run_shell(CommandExecutionRequest {
            command: command.to_string(),
            workdir: cwd.to_path_buf(),
            timeout: TokioDuration::from_secs(600),
            extra_env: env_pairs_from_json(env_grants).into_iter().collect(),
            allow_network: false,
        })
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))
}

pub(super) async fn run_command_capture(
    cwd: Option<&Path>,
    binary: &str,
    args: &[&str],
    env: &[(String, String)],
) -> ApiResult<String> {
    let output = LocalHostExecutionBackend::shared()
        .run_script(ScriptExecutionRequest {
            program: binary.to_string(),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
            workdir: cwd
                .map(Path::to_path_buf)
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))),
            timeout: TokioDuration::from_secs(600),
            extra_env: env.iter().cloned().collect(),
            allow_network: true,
        })
        .await
        .map_err(|e| ApiError::Internal(format!("failed to run {binary}: {e}")))?;
    let mut text = output.stdout.clone();
    if !output.stderr.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&output.stderr);
    }
    if output.exit_code != 0 {
        return Err(ApiError::Internal(format!(
            "{binary} exited with status {}{}",
            output.exit_code,
            if text.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", text.trim())
            }
        )));
    }
    Ok(text)
}

pub(super) async fn git_output(cwd: &str, args: &[&str]) -> ApiResult<String> {
    let output = run_command_capture(Some(Path::new(cwd)), "git", args, &[]).await?;
    Ok(output.trim().to_string())
}

pub(super) async fn git_output_raw(cwd: &str, args: &[&str]) -> ApiResult<String> {
    run_command_capture(Some(Path::new(cwd)), "git", args, &[]).await
}

pub(super) async fn git_run(cwd: &str, prefix_args: &[&str], extra_args: &[&str]) -> ApiResult<()> {
    let mut args = prefix_args.to_vec();
    args.extend_from_slice(extra_args);
    let _ = git_output(cwd, &args).await?;
    Ok(())
}

pub(super) async fn attributed_llm_cost_for_trial(
    store: &Arc<dyn Database>,
    campaign: &ExperimentCampaign,
    trial: &ExperimentTrial,
) -> ApiResult<LlmCostAttribution> {
    let exact = store
        .list_experiment_model_usage_for_trial(trial.id, 2_000)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !exact.is_empty() {
        return Ok(summarize_llm_usage(&exact, "trial_id"));
    }

    let campaign_records = store
        .list_experiment_model_usage_for_campaign(campaign.id, 5_000)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let trials = store
        .list_experiment_trials(campaign.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let lower_bound = trials
        .iter()
        .filter(|candidate| candidate.sequence < trial.sequence)
        .max_by_key(|candidate| candidate.sequence)
        .map(|candidate| {
            candidate
                .completed_at
                .or(candidate.started_at)
                .unwrap_or(candidate.created_at)
        })
        .unwrap_or(campaign.created_at);
    let fallback = campaign_records
        .into_iter()
        .filter(|record| {
            metadata_string_field(&record.metadata, "experiment_trial_id").is_none()
                && record.created_at >= lower_bound
                && record.created_at <= trial.created_at
        })
        .collect::<Vec<_>>();
    Ok(summarize_llm_usage(&fallback, "campaign_window"))
}

pub(super) fn ensure_unique_target_signature(
    kind: ExperimentTargetKind,
    metadata: &serde_json::Value,
    skip_target_id: Option<Uuid>,
    targets: &[ExperimentTarget],
) -> ApiResult<()> {
    ensure_unique_target_signature_policy(kind, metadata, skip_target_id, targets)
        .map_err(ApiError::InvalidInput)
}
