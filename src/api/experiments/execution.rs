use super::*;

pub(super) async fn launch_trial(
    store: &Arc<dyn Database>,
    user_id: &str,
    settings: &Settings,
    project: &ExperimentProject,
    runner: &ExperimentRunnerProfile,
    mut campaign: ExperimentCampaign,
    mut trial: ExperimentTrial,
) -> ApiResult<ExperimentCampaignActionResponse> {
    if runner.backend.is_remote() {
        let lease = create_lease(store, user_id, project, runner, &campaign, &trial).await?;
        let provider_api_key = research_provider_api_key(user_id, runner).await;
        let launch_outcome = adapters::try_auto_launch(
            runner,
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
        campaign.queue_state = ExperimentCampaignQueueState::Active;
        campaign.status = ExperimentCampaignStatus::Running;
        campaign.active_trial_id = Some(trial.id);
        campaign.started_at.get_or_insert_with(Utc::now);
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
        return Ok(ExperimentCampaignActionResponse {
            campaign,
            trial: Some(trial),
            lease: Some(lease),
            launch: Some(launch_details_from_outcome(launch_outcome)),
            message: "Remote trial prepared.".to_string(),
        });
    }

    if settings.experiments.max_concurrent_campaigns == 0 {
        return Err(ApiError::Unavailable(
            "experiments.max_concurrent_campaigns is set to 0".to_string(),
        ));
    }
    let completion =
        execute_local_trial(user_id, settings, project, runner, &campaign, &mut trial).await?;
    complete_trial_terminal(store, project, &mut campaign, &mut trial, None, completion).await?;
    Ok(ExperimentCampaignActionResponse {
        campaign,
        trial: Some(trial),
        lease: None,
        launch: None,
        message: "Local trial finished.".to_string(),
    })
}

pub(super) async fn complete_trial_terminal(
    store: &Arc<dyn Database>,
    project: &ExperimentProject,
    campaign: &mut ExperimentCampaign,
    trial: &mut ExperimentTrial,
    lease: Option<&mut ExperimentLease>,
    completion: ExperimentRunnerCompletion,
) -> ApiResult<()> {
    if let Some(lease) = lease.as_ref()
        && let Err(message) = validate_lease_completion_status(lease.status)
    {
        return Err(ApiError::InvalidInput(message.to_string()));
    }

    let completion = normalize_trial_completion(completion);
    finalize_trial(store, project, campaign, trial, completion).await?;

    if let Some(lease) = lease {
        lease.status = ExperimentLeaseStatus::Completed;
        lease.completed_at = Some(Utc::now());
        lease.updated_at = Utc::now();
        store
            .update_experiment_lease(lease)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    }

    Ok(())
}

pub(super) async fn finalize_trial(
    store: &Arc<dyn Database>,
    project: &ExperimentProject,
    campaign: &mut ExperimentCampaign,
    trial: &mut ExperimentTrial,
    completion: ExperimentRunnerCompletion,
) -> ApiResult<()> {
    trial.completed_at = Some(Utc::now());
    let runner_run_artifact = trial_runner_run_artifact(campaign, trial, &completion);
    trial.exit_code = completion.exit_code;
    trial.metrics_json = completion.metrics_json;
    trial.summary = completion.summary;
    trial.log_preview_path = completion.log_preview_path;
    trial.artifact_manifest_json = merge_json(
        &trial.artifact_manifest_json,
        &completion.artifact_manifest_json,
    );
    trial.updated_at = Utc::now();
    push_run_artifact(&mut trial.artifact_manifest_json, runner_run_artifact);
    campaign.active_trial_id = None;
    campaign.queue_state = ExperimentCampaignQueueState::NotQueued;
    trial.runtime_ms = completion.runtime_ms;
    if let Some(runtime_ms) = trial.runtime_ms {
        campaign.total_runtime_ms = campaign.total_runtime_ms.saturating_add(runtime_ms);
    } else if let Some(started_at) = trial.started_at {
        let runtime_ms = (trial.completed_at.unwrap_or_else(Utc::now) - started_at)
            .num_milliseconds()
            .max(0) as u64;
        campaign.total_runtime_ms = campaign.total_runtime_ms.saturating_add(runtime_ms);
        trial.runtime_ms = Some(runtime_ms);
    }
    let llm_cost = attributed_llm_cost_for_trial(store, campaign, trial).await?;
    let runner_cost = runner_cost_breakdown(trial, completion.attributed_cost_usd);
    trial.llm_cost_usd = Some(llm_cost.total_usd);
    trial.runner_cost_usd = Some(runner_cost.total_usd);
    trial.attributed_cost_usd = Some(llm_cost.total_usd + runner_cost.total_usd);
    campaign.total_llm_cost_usd += llm_cost.total_usd;
    campaign.total_runner_cost_usd += runner_cost.total_usd;
    campaign.total_cost_usd += trial.attributed_cost_usd.unwrap_or(0.0);
    trial.artifact_manifest_json = merge_json(
        &trial.artifact_manifest_json,
        &serde_json::json!({
            "cost_breakdown": {
                "total_usd": trial.attributed_cost_usd,
                "llm": llm_cost.details,
                "runner": runner_cost.details,
            }
        }),
    );
    if let Some(provider_overlay) = runner_cost.provider_metadata_overlay {
        trial.provider_job_metadata = merge_json(&trial.provider_job_metadata, &provider_overlay);
    }
    // Surface the runner cost-basis assumptions (e.g. RunPod's
    // `assumed_1_credit_equals_1_usd` normalization) onto the headline campaign
    // `cost_summary` so an operator deciding whether a campaign is within budget
    // can see the runner USD figure is an approximation. The values are lifted
    // verbatim from `runner_cost.details`; they are not recomputed (DP-3: surface,
    // do not gate).
    let runner_cost_basis = serde_json::json!({
        "estimated": runner_cost.details.get("estimated").and_then(|v| v.as_bool()),
        "native_currency": runner_cost.details.get("native_currency").cloned()
            .unwrap_or(serde_json::Value::Null),
        "normalization": runner_cost.details.get("normalization").cloned()
            .unwrap_or(serde_json::Value::Null),
    });
    campaign.metadata = merge_json(
        &campaign.metadata,
        &serde_json::json!({
            "cost_summary": {
                "total_usd": campaign.total_cost_usd,
                "llm_usd": campaign.total_llm_cost_usd,
                "runner_usd": campaign.total_runner_cost_usd,
                "runner_cost_basis": runner_cost_basis,
                "updated_at": Utc::now().to_rfc3339(),
            }
        }),
    );

    let success_exit = completion.exit_code.unwrap_or(1) == 0;
    let has_primary_metric = trial
        .metrics_json
        .get(&project.primary_metric.name)
        .and_then(|value| value.as_f64())
        .is_some();

    let mut non_improving = campaign.consecutive_non_improving_trials;

    if !success_exit {
        let failure_stage = trial
            .artifact_manifest_json
            .get("stage")
            .and_then(|value| value.as_str());
        if matches!(
            failure_stage,
            Some("prepare" | "checkout" | "clone" | "fetch" | "run")
        ) {
            trial.status = ExperimentTrialStatus::InfraFailed;
            trial.decision_reason = Some(format!(
                "{} command exited non-zero.",
                failure_stage.unwrap_or("runner")
            ));
        } else {
            trial.status = ExperimentTrialStatus::Crashed;
            trial.decision_reason = Some("Benchmark command exited non-zero.".to_string());
        }
        campaign.failure_count += 1;
    } else if !has_primary_metric {
        trial.status = ExperimentTrialStatus::InfraFailed;
        trial.decision_reason = Some(experiment_primary_metric_not_found_message(
            &project.primary_metric.name,
        ));
        campaign.failure_count += 1;
    } else if campaign
        .best_metrics
        .as_object()
        .is_none_or(|map| map.is_empty())
    {
        trial.status = ExperimentTrialStatus::Accepted;
        trial.decision_reason = Some("Baseline recorded as the first best result.".to_string());
        campaign.best_commit = trial.candidate_commit.clone();
        campaign.best_metrics = trial.metrics_json.clone();
        campaign.baseline_commit = trial.candidate_commit.clone();
        non_improving = 0;
    } else if compare_metrics(
        &project.primary_metric,
        &project.comparison_policy,
        &trial.metrics_json,
        &campaign.best_metrics,
    ) == Some(true)
    {
        trial.status = ExperimentTrialStatus::Accepted;
        trial.decision_reason = Some(format!(
            "Candidate improved {}.",
            project.primary_metric.name
        ));
        campaign.best_commit = trial.candidate_commit.clone();
        campaign.best_metrics = trial.metrics_json.clone();
        non_improving = 0;
    } else {
        trial.status = ExperimentTrialStatus::Rejected;
        trial.decision_reason = Some(format!(
            "Candidate did not improve {}.",
            project.primary_metric.name
        ));
        non_improving += 1;
    }

    let restore_commit = match trial.status {
        ExperimentTrialStatus::Rejected => campaign.best_commit.as_deref(),
        _ => trial
            .candidate_commit
            .as_deref()
            .or(campaign.best_commit.as_deref()),
    };
    let restore_error =
        if let Err(error) = restore_campaign_worktree_after_trial(campaign, restore_commit).await {
            trial.artifact_manifest_json = merge_json(
                &trial.artifact_manifest_json,
                &serde_json::json!({
                    "worktree_restore_error": error,
                }),
            );
            Some(error)
        } else {
            None
        };

    campaign.consecutive_non_improving_trials = non_improving;
    campaign.trial_count = campaign.trial_count.max(trial.sequence);
    campaign.updated_at = Utc::now();

    let max_trials = campaign
        .max_trials_override
        .or(project.stop_policy.max_trials);
    let plateau_limit = project
        .stop_policy
        .plateau_window
        .unwrap_or(project.stop_policy.non_improving_pause_threshold);
    let runtime_limit_reached = project
        .stop_policy
        .max_total_runtime_secs
        .is_some_and(|limit| (campaign.total_runtime_ms / 1000) >= limit);
    let cost_limit_reached = project
        .stop_policy
        .max_total_cost_usd
        .is_some_and(|limit| campaign.total_cost_usd >= limit);

    if let Some(error) = restore_error {
        campaign.status = ExperimentCampaignStatus::Paused;
        campaign.pause_reason = Some(format!(
            "Campaign paused: failed to restore campaign worktree: {error}"
        ));
    } else {
        let status_decision = CampaignStatusDecisionInput {
            campaign,
            project,
            trial,
            non_improving,
            max_trials,
            plateau_limit,
            runtime_limit_reached,
            cost_limit_reached,
        };
        let pause_reason = campaign_status_message(status_decision);
        let next_status = next_campaign_status(status_decision);
        campaign.pause_reason = Some(pause_reason);
        campaign.status = next_status;
    }
    if matches!(
        campaign.status,
        ExperimentCampaignStatus::Completed
            | ExperimentCampaignStatus::Cancelled
            | ExperimentCampaignStatus::Failed
            | ExperimentCampaignStatus::AwaitingPromotion
    ) {
        campaign.ended_at = Some(Utc::now());
    }

    upsert_local_trial_artifact_refs(store, trial).await?;

    store
        .update_experiment_trial(trial)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    store
        .update_experiment_campaign(campaign)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(())
}

pub(super) async fn upsert_local_trial_artifact_refs(
    store: &Arc<dyn Database>,
    trial: &ExperimentTrial,
) -> ApiResult<()> {
    let mut desired = Vec::new();
    if let Some(path) = trial
        .artifact_manifest_json
        .get("trajectory_json_path")
        .and_then(|value| value.as_str())
    {
        desired.push(("trajectory_json".to_string(), path.to_string()));
    }
    if let Some(path) = trial
        .artifact_manifest_json
        .get("summary_json_path")
        .and_then(|value| value.as_str())
    {
        desired.push(("summary_json".to_string(), path.to_string()));
    }
    if let Some(path) = trial.log_preview_path.as_deref() {
        desired.push(("log_preview".to_string(), path.to_string()));
    }
    if desired.is_empty() {
        return Ok(());
    }

    let mut artifacts = store
        .list_experiment_artifacts(trial.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let mut changed = false;
    for (kind, path) in desired {
        if artifacts
            .iter()
            .any(|artifact| artifact.kind == kind && artifact.uri_or_local_path == path)
        {
            continue;
        }
        let size_bytes = std::fs::metadata(&path).ok().map(|metadata| metadata.len());
        artifacts.push(ExperimentArtifactRef {
            id: Uuid::new_v4(),
            trial_id: trial.id,
            kind,
            uri_or_local_path: path,
            size_bytes,
            fetchable: false,
            metadata: serde_json::json!({
                "source": "local_runner_completion",
            }),
            created_at: Utc::now(),
        });
        changed = true;
    }

    if changed {
        store
            .replace_experiment_artifacts(trial.id, &artifacts)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    }
    Ok(())
}

pub(super) async fn execute_local_trial(
    user_id: &str,
    settings: &Settings,
    project: &ExperimentProject,
    runner: &ExperimentRunnerProfile,
    campaign: &ExperimentCampaign,
    trial: &mut ExperimentTrial,
) -> ApiResult<ExperimentRunnerCompletion> {
    let worktree_root = campaign.worktree_path.as_deref().ok_or_else(|| {
        ApiError::InvalidInput(
            experiment_campaign_missing_worktree_path_field_message().to_string(),
        )
    })?;
    let worktree_root = tokio::fs::canonicalize(worktree_root)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to resolve campaign worktree: {e}")))?;
    let workdir_fragment =
        validate_project_workdir_fragment(&project.workdir).map_err(ApiError::InvalidInput)?;
    let run_root = worktree_root.join(workdir_fragment);
    let run_root = tokio::fs::canonicalize(&run_root)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to resolve campaign workdir: {e}")))?;
    if !run_root.starts_with(&worktree_root) {
        return Err(ApiError::InvalidInput(
            experiment_project_workdir_escapes_campaign_worktree_message().to_string(),
        ));
    }
    let started_at = std::time::Instant::now();
    let experiments_data_dir = crate::platform::resolve_data_dir("experiments");
    let log_dir = experiments_data_dir.join("logs");
    let artifact_dir = experiments_data_dir.join("artifacts");
    tokio::fs::create_dir_all(&log_dir)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    tokio::fs::create_dir_all(&artifact_dir)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let log_path = log_dir.join(format!("{}.log", trial.id.simple()));

    trial.status = ExperimentTrialStatus::Running;
    trial.started_at = Some(Utc::now());
    trial.updated_at = Utc::now();

    if let Some(config) =
        agent_env_benchmark_config(&runner.backend_config).map_err(ApiError::InvalidInput)?
    {
        return execute_agent_env_benchmark_trial(
            config,
            &run_root,
            started_at,
            &log_path,
            &artifact_dir,
            trial,
        )
        .await;
    }

    let env_grants = resolved_runner_env_grants(user_id, runner).await;
    let backend = experiment_execution_backend(settings, runner, user_id);
    let mut log = String::new();
    if let Some(prepare_command) = project.prepare_command.as_deref() {
        let output = run_experiment_shell_command(
            Arc::clone(&backend),
            &run_root,
            prepare_command,
            &env_grants,
        )
        .await?;
        log.push_str("== prepare ==\n");
        log.push_str(&output.output);
        log.push('\n');
        if output.exit_code != 0 {
            let runtime_ms = (started_at.elapsed().as_nanos() / 1_000_000) as u64;
            tokio::fs::write(&log_path, &log)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            return Ok(ExperimentRunnerCompletion {
                exit_code: Some(output.exit_code as i32),
                metrics_json: serde_json::json!({}),
                summary: Some(format!(
                    "Local prepare command failed with exit code {}.",
                    output.exit_code
                )),
                runtime_ms: Some(runtime_ms),
                attributed_cost_usd: None,
                log_preview_path: Some(log_path.to_string_lossy().to_string()),
                artifact_manifest_json: serde_json::json!({
                    "stage": "prepare",
                    "summary_json_path": run_root.join("summary.json").to_string_lossy(),
                }),
            });
        }
    }
    let run_output = run_experiment_shell_command(
        Arc::clone(&backend),
        &run_root,
        &project.run_command,
        &env_grants,
    )
    .await?;
    let runtime_ms = (started_at.elapsed().as_nanos() / 1_000_000) as u64;
    log.push_str("== run ==\n");
    log.push_str(&run_output.output);
    tokio::fs::write(&log_path, &log)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let summary_path = run_root.join("summary.json");
    let persisted_summary_path = artifact_dir.join(format!("{}-summary.json", trial.id.simple()));
    let summary_json = if summary_path.exists() {
        let raw = tokio::fs::read_to_string(&summary_path)
            .await
            .unwrap_or_default();
        tokio::fs::write(&persisted_summary_path, &raw)
            .await
            .map_err(|e| {
                ApiError::Internal(format!("failed to persist local summary.json: {e}"))
            })?;
        serde_json::from_str::<serde_json::Value>(&raw).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };
    let summary_manifest_path = if summary_path.exists() {
        persisted_summary_path.to_string_lossy().to_string()
    } else {
        summary_path.to_string_lossy().to_string()
    };
    let metrics = extract_metrics(
        &project.primary_metric,
        &project.secondary_metrics,
        &log,
        &summary_json,
    );
    let exit_code = run_output.exit_code as i32;
    if exit_code != 0 {
        return Ok(ExperimentRunnerCompletion {
            exit_code: Some(exit_code),
            metrics_json: serde_json::json!({}),
            summary: Some(format!(
                "Local benchmark command failed with exit code {exit_code}."
            )),
            runtime_ms: Some(runtime_ms),
            attributed_cost_usd: None,
            log_preview_path: Some(log_path.to_string_lossy().to_string()),
            artifact_manifest_json: serde_json::json!({
                "stage": "run",
                "summary_json_path": summary_manifest_path,
            }),
        });
    }
    Ok(ExperimentRunnerCompletion {
        exit_code: Some(exit_code),
        metrics_json: metrics,
        summary: Some(format!("Local {} run completed.", backend.kind().as_str())),
        runtime_ms: Some(runtime_ms),
        attributed_cost_usd: None,
        log_preview_path: Some(log_path.to_string_lossy().to_string()),
        artifact_manifest_json: serde_json::json!({
            "stage": "run",
            "summary_json_path": summary_manifest_path,
        }),
    })
}

pub(super) async fn execute_agent_env_benchmark_trial(
    config: AgentEnvBenchmarkConfig,
    run_root: &Path,
    started_at: std::time::Instant,
    log_path: &Path,
    artifact_dir: &Path,
    trial: &ExperimentTrial,
) -> ApiResult<ExperimentRunnerCompletion> {
    let trajectories = match config {
        AgentEnvBenchmarkConfig::TerminalBench { cases, live_agent } => {
            let cases = cases
                .into_iter()
                .map(|mut case| {
                    if case.cwd.is_none() {
                        case.cwd = Some(run_root.to_path_buf());
                    }
                    case
                })
                .collect::<Vec<_>>();
            // The env scores the agent ACTION's output against each case's
            // checks. With `live_agent`, the registered subagent runtime (a
            // real LLM-backed agent) produces the command per case; otherwise
            // each case's own command is the scripted reference action, so
            // the score measures the harness ceiling deterministically.
            let actions: Vec<AgentAction> = if live_agent {
                let prompts = cases
                    .iter()
                    .map(|case| (case.name.clone(), terminal_bench_live_prompt(case)))
                    .collect();
                live_agent_actions(prompts).await?
            } else {
                cases
                    .iter()
                    .map(|case| AgentAction::UserMessage {
                        content: case.command.clone(),
                    })
                    .collect()
            };
            let mut runner = EnvRunner::new(TerminalBenchEnv::new(cases))
                .with_artifact_root(artifact_dir.join("agent_env_runs"));
            runner
                .evaluate(1, move |_| actions.clone())
                .await
                .map_err(|err| ApiError::Internal(err.to_string()))?
        }
        AgentEnvBenchmarkConfig::SkillBench { cases, live_agent } => {
            // With `live_agent`, the agent answers each case applying the
            // skill; otherwise the reference action is the skill content
            // itself, which by construction satisfies the case's required
            // substrings.
            let actions: Vec<AgentAction> = if live_agent {
                let prompts = cases
                    .iter()
                    .map(|case| (case.name.clone(), skill_bench_live_prompt(case)))
                    .collect();
                live_agent_actions(prompts).await?
            } else {
                cases
                    .iter()
                    .map(|case| AgentAction::UserMessage {
                        content: case.skill_content.clone(),
                    })
                    .collect()
            };
            let mut runner = EnvRunner::new(SkillBenchEnv::new(cases))
                .with_artifact_root(artifact_dir.join("agent_env_runs"));
            runner
                .evaluate(1, move |_| actions.clone())
                .await
                .map_err(|err| ApiError::Internal(err.to_string()))?
        }
    };

    let runtime_ms = (started_at.elapsed().as_nanos() / 1_000_000) as u64;
    let score = average_trajectory_score(&trajectories);
    let trajectory_path =
        artifact_dir.join(format!("{}-agent-env-trajectory.json", trial.id.simple()));
    let trajectory_json = serde_json::to_string_pretty(&trajectories)
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    tokio::fs::write(&trajectory_path, &trajectory_json)
        .await
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    let log = render_trajectory_log(&trajectories);
    tokio::fs::write(log_path, &log)
        .await
        .map_err(|err| ApiError::Internal(err.to_string()))?;

    Ok(ExperimentRunnerCompletion {
        exit_code: Some(if score >= 1.0 { 0 } else { 1 }),
        metrics_json: serde_json::json!({
            "score": score,
            "episodes": trajectories.len(),
        }),
        summary: Some(format!(
            "AgentEnv benchmark completed with score {score:.3}."
        )),
        runtime_ms: Some(runtime_ms),
        attributed_cost_usd: None,
        log_preview_path: Some(log_path.to_string_lossy().to_string()),
        artifact_manifest_json: serde_json::json!({
            "stage": "agent_env_benchmark",
            "trajectory_json_path": trajectory_path.to_string_lossy(),
            "trajectory_summary": trajectory_summary(&trajectories),
        }),
    })
}

fn terminal_bench_live_prompt(case: &crate::agent::env::TerminalBenchCase) -> String {
    let mut requirements = Vec::new();
    if !case.expected_stdout_contains.is_empty() {
        requirements.push(format!(
            "its stdout must contain: {}",
            case.expected_stdout_contains.join(", ")
        ));
    }
    if let Some(code) = case.expected_exit_code {
        requirements.push(format!("it must exit with code {code}"));
    }
    let requirements = if requirements.is_empty() {
        String::new()
    } else {
        format!(" Requirements: {}.", requirements.join("; "))
    };
    format!(
        "Produce a single POSIX shell command for the benchmark task named '{}'.{} \
         Reply with ONLY the shell command — no prose, no code fences.",
        case.name, requirements
    )
}

fn skill_bench_live_prompt(case: &crate::agent::env::SkillBenchCase) -> String {
    format!(
        "Apply the following skill to complete the benchmark task named '{}'.\n\n\
         {}\n\n\
         Respond with a concise answer that demonstrates the skill.",
        case.name, case.skill_content
    )
}

/// Strip a single wrapping fenced code block, if present, from an agent
/// response — models often fence commands despite instructions not to.
fn strip_code_fences(response: &str) -> String {
    let trimmed = response.trim();
    let Some(inner) = trimmed
        .strip_prefix("```")
        .and_then(|rest| rest.strip_suffix("```"))
    else {
        return trimmed.to_string();
    };
    // Drop an optional language tag on the opening fence line. Only strip
    // when the token looks like a fence annotation (lowercase alphanumeric,
    // short) — a real first line such as "Done" or "Summary" is content.
    match inner.split_once('\n') {
        Some((first_line, body)) if is_fence_language_tag(first_line.trim()) => {
            body.trim().to_string()
        }
        _ => inner.trim().to_string(),
    }
}

fn is_fence_language_tag(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 12
        && token
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '+' || c == '-')
}

/// Produce one action per bench case from the live agent runtime (the
/// registered subagent executor). A per-case failure yields an empty action,
/// which the env scores as 0.0 — an agent failure is a failed case, not a
/// failed trial.
async fn live_agent_actions(prompts: Vec<(String, String)>) -> ApiResult<Vec<AgentAction>> {
    let Some(executor) = super::types::research_subagent_executor() else {
        return Err(ApiError::Internal(
            "live_agent benchmark requested but no subagent executor is registered".to_string(),
        ));
    };

    let mut actions = Vec::with_capacity(prompts.len());
    for (name, task) in prompts {
        let request: crate::agent::SubagentSpawnRequest =
            serde_json::from_value(serde_json::json!({
                "name": format!("bench:{name}"),
                "task": task,
                "wait": true,
                "timeout_secs": 120,
                "allowed_tools": [],
            }))
            .map_err(|err| ApiError::Internal(err.to_string()))?;

        let content = match executor
            .spawn(
                request,
                "experiments",
                &serde_json::json!({}),
                "experiments",
                None,
                None,
            )
            .await
        {
            Ok(result) if result.success => strip_code_fences(&result.response),
            Ok(result) => {
                tracing::warn!(
                    case = %name,
                    error = ?result.error,
                    "Live-agent bench case failed; scoring as empty action"
                );
                String::new()
            }
            Err(err) => {
                tracing::warn!(
                    case = %name,
                    error = %err,
                    "Live-agent bench spawn failed; scoring as empty action"
                );
                String::new()
            }
        };
        actions.push(AgentAction::UserMessage { content });
    }
    Ok(actions)
}

pub(super) async fn restore_campaign_worktree_after_trial(
    campaign: &ExperimentCampaign,
    restore_commit: Option<&str>,
) -> Result<(), String> {
    let Some(worktree_path) = campaign.worktree_path.as_deref() else {
        return Ok(());
    };

    if let Some(commit) = restore_commit {
        git_output(worktree_path, &["reset", "--hard", commit])
            .await
            .map_err(|error| format!("failed to reset campaign worktree to {commit}: {error}"))?;
    }

    git_output_raw(worktree_path, &["clean", "-fd"])
        .await
        .map_err(|error| format!("failed to clean campaign worktree: {error}"))?;
    Ok(())
}
