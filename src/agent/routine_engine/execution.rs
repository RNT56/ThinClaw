//! Routine execution: the lightweight / full-job / heartbeat / subagent runners
//! and notification delivery, driven by an `EngineContext`.

use super::*;

const ROUTINE_COMPLETION_TAIL_TIMEOUT: Duration = Duration::from_secs(10);

fn routine_workspace(ctx: &EngineContext, routine: &Routine) -> AuthorizedWorkspace {
    AuthorizedWorkspace::conversation(&ctx.workspace, &routine_identity(routine), "routine")
}

fn routine_timezone<'a>(ctx: &'a EngineContext, routine: &'a Routine) -> Option<&'a str> {
    routine_timezone_setting(routine, ctx.user_timezone.as_deref())
}

async fn build_routine_daily_context(workspace: &AuthorizedWorkspace) -> String {
    let mut daily_context = String::new();
    let today = workspace.local_today().await;

    if let Ok(document) = workspace.daily_log(today).await
        && !document.content.trim().is_empty()
    {
        let capped = thinclaw_agent::heartbeat::cap_daily_log(&document.content, 3_000);
        daily_context.push_str(&format!(
            "\n\n## Daily Log - {} (today)\n\n{}",
            today.format("%Y-%m-%d"),
            capped
        ));
    }

    if let Some(yesterday) = today.pred_opt()
        && let Ok(document) = workspace.daily_log(yesterday).await
        && !document.content.trim().is_empty()
    {
        let capped = thinclaw_agent::heartbeat::cap_daily_log(&document.content, 2_000);
        daily_context.push_str(&format!(
            "\n\n## Daily Log - {} (yesterday)\n\n{}",
            yesterday.format("%Y-%m-%d"),
            capped
        ));
    }

    daily_context
}

/// Execute a routine run. Handles both lightweight and full_job modes.
pub(super) async fn execute_routine(ctx: EngineContext, routine: Routine, run: RoutineRun) {
    // Broadcast routine start event
    ctx.broadcast_sse(SseEvent::RoutineLifecycle {
        routine_name: routine.name.clone(),
        event: "started".to_string(),
        run_id: Some(run.id.to_string()),
        result_summary: None,
    });

    let result = match &routine.action {
        RoutineAction::Lightweight {
            prompt,
            context_paths,
            max_tokens,
        } => {
            execute_lightweight(
                &ctx,
                &routine,
                prompt,
                context_paths,
                *max_tokens,
                run.trigger_detail.as_deref(),
            )
            .await
        }
        RoutineAction::FullJob {
            title,
            description,
            max_iterations,
            allowed_tools,
            allowed_skills,
            tool_profile,
        } => {
            // Append any trigger payload (e.g. signed webhook body) into the
            // job description as a delimited, untrusted-data block.
            let payload_block = render_trigger_payload_block(run.trigger_detail.as_deref());
            let effective_description = if payload_block.is_empty() {
                description.clone()
            } else {
                format!("{description}{payload_block}")
            };
            if ctx.subagent_executor.is_some() {
                execute_as_subagent(
                    &ctx,
                    &routine,
                    &run,
                    title,
                    &effective_description,
                    allowed_tools.as_deref(),
                    allowed_skills.as_deref(),
                    *tool_profile,
                )
                .await
            } else {
                execute_full_job(
                    &ctx,
                    &routine,
                    &run,
                    title,
                    &effective_description,
                    *max_iterations,
                    allowed_tools.as_deref(),
                    allowed_skills.as_deref(),
                    *tool_profile,
                )
                .await
            }
        }
        RoutineAction::Heartbeat {
            light_context,
            prompt,
            include_reasoning,
            active_start_hour,
            active_end_hour,
            target,
            max_iterations,
            ..
        } => {
            execute_heartbeat(
                &ctx,
                &routine,
                &run,
                *light_context,
                prompt.as_deref(),
                *include_reasoning,
                *active_start_hour,
                *active_end_hour,
                target,
                *max_iterations,
            )
            .await
        }
        RoutineAction::ExperimentCampaign {
            project_id,
            runner_profile_id,
            max_trials_override,
        } => Ok(
            match experiments_api::start_campaign(
                &ctx.store,
                &routine.user_id,
                *project_id,
                experiments_api::StartExperimentCampaignRequest {
                    runner_profile_id: *runner_profile_id,
                    max_trials_override: *max_trials_override,
                    gateway_url: None,
                },
            )
            .await
            {
                Ok(response) => (
                    RunStatus::Attention,
                    Some(format!(
                        "Experiment campaign {} started: {}",
                        response.campaign.id, response.message
                    )),
                    None,
                ),
                Err(error) => (RunStatus::Failed, Some(error.to_string()), None),
            },
        ),
    };

    // Process result
    let (status, summary, tokens) = match result {
        Ok(execution) => execution,
        Err(e) => {
            tracing::error!(routine = %routine.name, "Execution failed: {}", e);
            (RunStatus::Failed, Some(e.to_string()), None)
        }
    };

    // RunStatus::Running means the job was dispatched to a worker or subagent.
    // The worker/subagent handles its own DB completion + SSE lifecycle event,
    // so skip all post-processing here to avoid conflicts.
    if status == RunStatus::Running {
        return;
    }

    match finalize_routine_run_record(&ctx.store, run.id, status, summary.as_deref(), tokens).await
    {
        Ok(true) => {}
        Ok(false) => {
            tracing::debug!(
                routine = %routine.name,
                run_id = %run.id,
                "Routine run was already terminal; skipping duplicate completion tails"
            );
            return;
        }
        Err(e) => {
            tracing::error!(routine = %routine.name, "Failed to complete run record: {}", e);
            return;
        }
    }

    // IC-CRON-STAGGER: notify an optional external webhook that this run
    // finished. Keep delivery in the tracked routine tail so runtime shutdown
    // cannot silently detach it; the HTTP helper and this outer tail are both
    // bounded and failures never change the run outcome.
    let webhook_url = StaggerConfig::from_env().finished_webhook_url;
    let webhook_payload = crate::agent::cron_stagger::FinishedRunPayload {
        routine_id: routine.id.to_string(),
        routine_name: routine.name.clone(),
        success: status != RunStatus::Failed,
        duration_ms: Utc::now()
            .signed_duration_since(run.started_at)
            .num_milliseconds()
            .max(0) as u64,
        error: summary.clone().filter(|_| status == RunStatus::Failed),
        completed_at: Utc::now().to_rfc3339(),
    };
    let webhook_delivery = async move {
        if let Some(webhook_url) = webhook_url {
            crate::agent::cron_stagger::notify_finished_run(&webhook_url, &webhook_payload).await;
        }
    };

    let mut completed_run = run.clone();
    completed_run.status = status;
    completed_run.result_summary = summary.clone();
    completed_run.tokens_used = tokens;
    completed_run.completed_at = Some(Utc::now());
    if let Err(err) =
        outcomes::maybe_create_routine_contract(&ctx.store, &routine, &completed_run).await
    {
        tracing::debug!(routine = %routine.name, error = %err, "Outcome routine contract hook skipped");
    }
    let run_artifact = AgentRunArtifact::new(
        "routine_run",
        match status {
            RunStatus::Failed => AgentRunStatus::Failed,
            RunStatus::Ok | RunStatus::Attention | RunStatus::Running => AgentRunStatus::Completed,
        },
        run.started_at,
        completed_run.completed_at,
    )
    .with_failure_reason(
        summary
            .as_ref()
            .filter(|_| status == RunStatus::Failed)
            .cloned(),
    )
    .with_runtime_descriptor(Some(&crate::agent::run_artifact::run_runtime_descriptor(
        &routine_engine_runtime_descriptor(),
    )))
    .with_metadata(serde_json::json!({
        "event": "routine_run_completed",
        "routine_id": routine.id,
        "routine_name": routine.name.clone(),
        "run_id": completed_run.id,
        "status": status.to_string(),
        "result_summary": completed_run.result_summary.clone(),
        "tokens_used": completed_run.tokens_used,
    }));
    let provider_store = Arc::clone(&ctx.store);
    let mut run_artifact = run_artifact;
    run_artifact.user_id = Some(routine.user_id.clone());
    run_artifact.actor_id = Some(routine.owner_actor_id().to_string());
    run_artifact.conversation_scope_id = Some(thinclaw_identity::direct_scope_id(
        &routine.user_id,
        routine.owner_actor_id(),
    ));
    run_artifact.conversation_kind = Some("direct".to_string());
    run_artifact.channel = Some("system".to_string());
    let artifact_persistence = async move {
        let harness = crate::agent::AgentRunHarness::new(None);
        if let Err(err) = harness.append_artifact(&run_artifact).await {
            tracing::debug!(error = %err, "Failed to append routine run artifact");
        }
        let manager = crate::agent::learning::MemoryProviderManager::new(provider_store);
        if let Some(access) =
            crate::agent::learning::provider_access_context_from_artifact(&run_artifact)
        {
            manager.session_end_extract(&access, &run_artifact).await;
        }
    };
    let bounded_artifact_persistence = async move {
        if tokio::time::timeout(ROUTINE_COMPLETION_TAIL_TIMEOUT, artifact_persistence)
            .await
            .is_err()
        {
            tracing::warn!(
                timeout_secs = ROUTINE_COMPLETION_TAIL_TIMEOUT.as_secs(),
                "Routine run artifact/learning tail timed out"
            );
        }
    };
    let bounded_webhook_delivery = async move {
        if tokio::time::timeout(ROUTINE_COMPLETION_TAIL_TIMEOUT, webhook_delivery)
            .await
            .is_err()
        {
            tracing::warn!(
                timeout_secs = ROUTINE_COMPLETION_TAIL_TIMEOUT.as_secs(),
                "Routine completion webhook tail timed out"
            );
        }
    };
    tokio::join!(bounded_artifact_persistence, bounded_webhook_delivery);

    // Send notifications based on config
    send_notification(
        &ctx.notify_tx,
        &routine.notify,
        &routine.name,
        status,
        summary.as_deref(),
    )
    .await;

    let event_type = match status {
        RunStatus::Ok => "completed",
        RunStatus::Attention => "attention",
        RunStatus::Failed => "failed",
        RunStatus::Running => {
            tracing::error!(
                routine = %routine.name,
                run_id = %run.id,
                "Dispatched routine unexpectedly reached terminal notification handling"
            );
            return;
        }
    };
    ctx.broadcast_sse(SseEvent::RoutineLifecycle {
        routine_name: routine.name.clone(),
        event: event_type.to_string(),
        run_id: Some(run.id.to_string()),
        result_summary: summary.clone(),
    });
}

/// Execute a non-heartbeat automation as a subagent.
///
/// Routes through the SubagentExecutor for UI isolation (dedicated split pane),
/// fresh context per run, and proper cancellation support. The subagent executor
/// handles its own SSE lifecycle events via SubagentSpawned / SubagentProgress /
/// SubagentCompleted status updates. Returns `RunStatus::Running` so the calling
/// `execute_routine` skips premature `complete_routine_run`.
async fn execute_as_subagent(
    ctx: &EngineContext,
    routine: &Routine,
    run: &RoutineRun,
    title: &str,
    description: &str,
    allowed_tools: Option<&[String]>,
    allowed_skills: Option<&[String]>,
    tool_profile: Option<ToolProfile>,
) -> Result<(RunStatus, Option<String>, Option<i32>), RoutineError> {
    let executor = ctx
        .subagent_executor
        .as_ref()
        .ok_or_else(|| RoutineError::ExecutionFailed {
            reason: "SubagentExecutor not available".into(),
        })?;

    let request = SubagentSpawnRequest {
        name: format!("Automation: {}", routine.name),
        task: description.to_string(),
        system_prompt: Some(format!(
            "You are executing the automation '{}'. \
             Complete the task thoroughly and report results via `emit_user_message`. \
             Use tools as needed. When finished, return a clear summary.\n\n\
             Title: {}\n\nDescription: {}",
            routine.name, title, description
        )),
        model: None,
        task_packet: None,
        memory_mode: None,
        tool_mode: None,
        skill_mode: None,
        tool_profile,
        allowed_tools: allowed_tools.map(|tools| tools.to_vec()),
        allowed_skills: allowed_skills.map(|skills| skills.to_vec()),
        principal_id: Some(routine.user_id.clone()),
        actor_id: Some(routine.owner_actor_id().to_string()),
        agent_workspace_id: None,
        timeout_secs: Some(300),
        wait: false,
    };

    // Pass routine metadata through channel_metadata so SubagentExecutor
    // can finalize the routine_run on completion.
    let channel_metadata = serde_json::json!({
        "thread_id": "agent:main",
        "principal_id": routine.user_id,
        "actor_id": routine.owner_actor_id(),
        "conversation_kind": "direct",
        "conversation_scope_id": thinclaw_identity::direct_scope_id(
            &routine.user_id,
            routine.owner_actor_id(),
        ).to_string(),
        "stable_external_conversation_key": thinclaw_identity::direct_conversation_key(
            &routine.user_id,
            routine.owner_actor_id(),
        ),
        "user_timezone": routine_timezone(ctx, routine),
        "routine_id": routine.id.to_string(),
        "routine_name": routine.name,
        "routine_run_id": run.id.to_string(),
        "reinject_result": false,
    });

    match executor
        .spawn(
            request,
            "tauri",
            &channel_metadata,
            routine.owner_actor_id(),
            None,
            Some("agent:main"),
        )
        .await
    {
        Ok(result) => {
            // Broadcast "dispatched" SSE so the UI shows the subagent panel
            ctx.broadcast_sse(SseEvent::RoutineLifecycle {
                routine_name: routine.name.clone(),
                event: "dispatched".to_string(),
                run_id: Some(run.id.to_string()),
                result_summary: Some(format!(
                    "Subagent spawned (id: {}) — {}",
                    result.agent_id,
                    summarize_runtime_capabilities(
                        tool_profile.unwrap_or(ToolProfile::ExplicitOnly),
                        allowed_tools,
                        allowed_skills,
                    )
                )),
            });

            Ok((
                RunStatus::Running,
                Some(format!("Subagent spawned (id: {})", result.agent_id)),
                None,
            ))
        }
        Err(e) => Err(RoutineError::ExecutionFailed {
            reason: format!("Failed to spawn subagent: {}", e),
        }),
    }
}

/// Execute a full-job routine by dispatching to the scheduler.
///
/// Uses `dispatch_job_for_routine` so the spawned worker carries routine
/// metadata and can emit a real `RoutineLifecycle` SSE event on actual
/// completion — not just on dispatch. Returns `RunStatus::Running` so
/// `execute_routine` knows NOT to emit a premature "completed" event.
async fn execute_full_job(
    ctx: &EngineContext,
    routine: &Routine,
    run: &RoutineRun,
    title: &str,
    description: &str,
    max_iterations: u32,
    allowed_tools: Option<&[String]>,
    allowed_skills: Option<&[String]>,
    tool_profile: Option<ToolProfile>,
) -> Result<(RunStatus, Option<String>, Option<i32>), RoutineError> {
    let scheduler = ctx
        .scheduler
        .as_ref()
        .ok_or_else(|| RoutineError::JobDispatchFailed {
            reason: "scheduler not available".to_string(),
        })?;

    if let Some(manager) = ctx.desktop_autonomy_manager.as_ref() {
        if routine_requests_desktop_capabilities(allowed_tools) {
            manager
                .ensure_can_run()
                .await
                .map_err(|reason| RoutineError::ExecutionFailed { reason })?;
        } else if manager.emergency_stop_active() {
            return Err(RoutineError::ExecutionFailed {
                reason: "desktop autonomy emergency stop is active".to_string(),
            });
        }
    }

    let desktop = ctx.desktop_autonomy_manager.as_ref().map(|manager| {
        serde_json::json!({
            "desktop_session": manager.default_session_id(),
            "deployment_mode": manager.config().deployment_mode.as_str(),
            "desktop_run_id": run.id.to_string(),
            "recovery_count": 0,
            "last_verified_snapshot": serde_json::Value::Null,
            "managed_build_id": manager.current_build_id(),
            "autonomy_profile": manager.config().profile.as_str(),
        })
    });
    let metadata = full_job_metadata(
        routine,
        run.id,
        max_iterations,
        FullJobRuntimeMetadata {
            allowed_tools: allowed_tools.map(|tools| tools.to_vec()),
            allowed_skills: allowed_skills.map(|skills| skills.to_vec()),
            tool_profile,
            desktop,
            user_timezone: routine_timezone(ctx, routine).map(str::to_string),
        },
    );

    let job_id = scheduler
        .dispatch_job_for_routine(
            &routine.user_id,
            routine.owner_actor_id(),
            title,
            description,
            Some(metadata),
            routine.id,
            routine.name.clone(),
            run.id.to_string(),
            Some(ctx.notify_tx.clone()),
        )
        .await
        .map_err(|e| RoutineError::JobDispatchFailed {
            reason: format!("failed to dispatch job: {e}"),
        })?;

    // Link the routine run to the dispatched job
    if let Err(e) = ctx.store.link_routine_run_to_job(run.id, job_id).await {
        tracing::error!(
            routine = %routine.name,
            "Failed to link run to job: {}", e
        );
    }

    // Broadcast "dispatched" SSE so the UI shows a queued state, NOT success
    ctx.broadcast_sse(SseEvent::RoutineLifecycle {
        routine_name: routine.name.clone(),
        event: "dispatched".to_string(),
        run_id: Some(run.id.to_string()),
        result_summary: Some(format!(
            "Job {job_id} queued — {}",
            summarize_runtime_capabilities(
                tool_profile.unwrap_or(ToolProfile::Restricted),
                allowed_tools,
                allowed_skills,
            )
        )),
    });

    // Also broadcast the generic job started event for job view
    ctx.broadcast_sse(SseEvent::JobStarted {
        job_id: job_id.to_string(),
        title: format!("Routine '{}': {}", routine.name, title),
        browse_url: String::new(),
    });

    tracing::info!(
        routine = %routine.name,
        job_id = %job_id,
        max_iterations = max_iterations,
        "Dispatched full job for routine — worker will emit completion SSE"
    );

    let summary = format!(
        "Dispatched job {job_id} for full execution ({}, max_iterations: {max_iterations})",
        summarize_runtime_capabilities(
            tool_profile.unwrap_or(ToolProfile::Restricted),
            allowed_tools,
            allowed_skills,
        )
    );
    // Return RunStatus::Running — execute_routine will skip emitting "completed"
    // for this case; the worker emits the real event via WorkerDeps::sse_tx.
    Ok((RunStatus::Running, Some(summary), None))
}

/// Execute a heartbeat routine.
///
/// In `light_context` mode (default), dispatches as a full worker job with
/// HEARTBEAT.md + daily logs as the prompt — isolated from the main session
/// but with full tool access.
///
/// When `light_context` is false, injects the heartbeat prompt into the main
/// session via `system_event_tx` for full conversational context.
async fn execute_heartbeat(
    ctx: &EngineContext,
    routine: &Routine,
    run: &RoutineRun,
    light_context: bool,
    custom_prompt: Option<&str>,
    include_reasoning: bool,
    active_start_hour: Option<u8>,
    active_end_hour: Option<u8>,
    target: &str,
    max_iterations: u32,
) -> Result<(RunStatus, Option<String>, Option<i32>), RoutineError> {
    // 0. Active hours check
    if let (Some(s), Some(e)) = (active_start_hour, active_end_hour) {
        let tz = crate::timezone::resolve_effective_timezone(
            Some(&routine.user_id),
            routine_timezone(ctx, routine),
        );
        let now_hour = crate::timezone::now_in_tz(tz).hour() as u8;
        if !active_hour_allows(now_hour, s, e) {
            tracing::debug!(
                routine = %routine.name,
                hour = now_hour,
                active = %format!("{:02}:00-{:02}:00", s, e),
                "Heartbeat outside active hours — skipping"
            );
            return Ok((
                RunStatus::Ok,
                Some("Skipped — outside active hours".to_string()),
                None,
            ));
        }
    }

    // 1. Read HEARTBEAT.md
    let workspace = routine_workspace(ctx, routine);
    let checklist = match workspace.heartbeat_checklist().await {
        Ok(Some(content)) if !crate::agent::heartbeat::is_effectively_empty(&content) => content,
        Ok(_) => {
            tracing::debug!(routine = %routine.name, "HEARTBEAT.md is empty or missing — skipping");
            return Ok((
                RunStatus::Ok,
                Some("Checklist empty; no action required.".to_string()),
                None,
            ));
        }
        Err(e) => {
            return Err(RoutineError::ExecutionFailed {
                reason: format!("Failed to read HEARTBEAT.md: {}", e),
            });
        }
    };

    // IC-013: Use shared function to build daily log context
    let daily_context = build_routine_daily_context(&workspace).await;

    // ── Self-critique feedback: inject previous run's evaluation ─────
    // If the previous heartbeat was flagged by the post-completion
    // evaluator, inject that feedback so the agent can learn from it.
    let critique_key = heartbeat_critique_setting_key(routine.owner_actor_id());
    let critique_context = match ctx.store.get_setting(&routine.user_id, &critique_key).await {
        Ok(Some(critique)) if !critique.is_null() => {
            let reasoning = critique
                .get("reasoning")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown issue");
            let quality = critique
                .get("quality")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            format!(
                "\n\n## Previous Heartbeat Feedback (Self-Critique)\n\n\
                 ⚠️ The previous heartbeat run scored {}/100. \
                 Evaluator feedback: {}\n\n\
                 Take this into account and avoid repeating the same mistake.",
                quality, reasoning
            )
        }
        _ => String::new(),
    };

    // 3. Build the full prompt
    // The existing aggregate is principal-wide. Until the persistence port
    // exposes actor-filtered stats, only the principal actor may receive it;
    // household actors must not learn counts derived from sibling activity.
    let outcome_summary = if routine.owner_actor_id() == routine.user_id {
        match crate::agent::outcomes::heartbeat_review_summary(&ctx.store, &routine.user_id).await {
            Ok(Some(summary)) => Some(summary),
            _ => None,
        }
    } else {
        None
    };
    let mut full_prompt = build_heartbeat_prompt(
        custom_prompt,
        &checklist,
        &daily_context,
        &critique_context,
        outcome_summary.as_deref(),
        include_reasoning,
    );
    // Forward any trigger payload (e.g. a signed webhook body) into the
    // heartbeat prompt as a delimited, untrusted-data block.
    full_prompt.push_str(&render_trigger_payload_block(run.trigger_detail.as_deref()));

    if !light_context {
        // ── Main-session injection mode ──────────────────────────────
        // Inject the heartbeat prompt into the main session via system_event_tx.
        // The dispatcher processes it as a normal turn with full session history
        // and tool access. The response flows through normal SSE → chat.
        if let Some(ref tx) = ctx.system_event_tx {
            let heartbeat_target = HeartbeatTarget::parse(target);
            let identity = routine_identity(routine);
            let message = IncomingMessage::new("heartbeat", "system", &full_prompt)
                .with_metadata(serde_json::json!({
                    "source": "heartbeat",
                    "conversation_kind": "direct",
                    "conversation_scope_id": identity.conversation_scope_id.to_string(),
                    "stable_external_conversation_key": identity.stable_external_conversation_key,
                    "user_timezone": routine_timezone(ctx, routine),
                    "routine_name": routine.name,
                    "run_id": run.id.to_string(),
                    "include_reasoning": include_reasoning,
                    "suppress_output": heartbeat_target.suppresses_output(),
                    "notify_channel": heartbeat_target.channel_override(),
                }))
                .with_identity(identity);

            if let Err(e) = tx.send(message).await {
                return Err(RoutineError::ExecutionFailed {
                    reason: format!("Failed to inject heartbeat into main session: {}", e),
                });
            }

            tracing::info!(
                routine = %routine.name,
                "Injected heartbeat into main session — dispatcher will process with full context"
            );

            // Complete the run now, as `Ok`. This run's job is delivering the
            // heartbeat prompt into the main session — that delivery just
            // succeeded. The dispatcher turn that follows is a normal
            // conversational turn on the main session, not part of this
            // routine run, and nothing else ever calls complete_routine_run
            // for it. Previously this returned `RunStatus::Running` on the
            // (incorrect) assumption that "the dispatcher handles
            // completion" — it doesn't, so every main-session heartbeat run
            // was eventually reaped as a failure by the zombie cleanup,
            // poisoning run history and downstream learning signals.
            return Ok((
                RunStatus::Ok,
                Some("Injected into main session".to_string()),
                None,
            ));
        } else {
            tracing::warn!(
                routine = %routine.name,
                "No system_event_tx available — falling back to light_context mode"
            );
            // Fall through to light_context mode below
        }
    }

    // ── Light-context mode: dispatch as isolated worker job ──────────
    // Uses the reserved overflow slot so heartbeats never get blocked
    // by "Maximum parallel jobs exceeded" when user jobs fill all slots.
    let title = format!("Heartbeat: {}", routine.name);
    let scheduler = ctx
        .scheduler
        .as_ref()
        .ok_or_else(|| RoutineError::JobDispatchFailed {
            reason: "scheduler not available".to_string(),
        })?;

    let metadata = heartbeat_job_metadata(
        routine,
        max_iterations,
        target,
        include_reasoning,
        routine_timezone(ctx, routine),
    );

    let job_id = scheduler
        .dispatch_job_reserved_for_routine(
            &routine.user_id,
            routine.owner_actor_id(),
            &title,
            &full_prompt,
            Some(metadata),
            routine.id,
            routine.name.clone(),
            run.id.to_string(),
            Some(ctx.notify_tx.clone()),
        )
        .await
        .map_err(|e| RoutineError::JobDispatchFailed {
            reason: format!("failed to dispatch heartbeat job: {e}"),
        })?;

    // Link the routine run to the dispatched job
    if let Err(e) = ctx.store.link_routine_run_to_job(run.id, job_id).await {
        tracing::error!(
            routine = %routine.name,
            "Failed to link heartbeat run to job: {}", e
        );
    }

    ctx.broadcast_sse(SseEvent::RoutineLifecycle {
        routine_name: routine.name.clone(),
        event: "dispatched".to_string(),
        run_id: Some(run.id.to_string()),
        result_summary: Some(format!("Heartbeat job {job_id} dispatched (reserved slot)")),
    });

    tracing::info!(
        routine = %routine.name,
        job_id = %job_id,
        "Dispatched heartbeat via reserved slot"
    );

    Ok((
        RunStatus::Running,
        Some(format!("Dispatched heartbeat job {job_id} (reserved slot)")),
        None,
    ))
}

/// Execute a lightweight routine (single LLM call).
async fn execute_lightweight(
    ctx: &EngineContext,
    routine: &Routine,
    prompt: &str,
    context_paths: &[String],
    max_tokens: u32,
    trigger_detail: Option<&str>,
) -> Result<(RunStatus, Option<String>, Option<i32>), RoutineError> {
    let workspace = routine_workspace(ctx, routine);
    // Load context from workspace
    let mut context_parts = Vec::new();
    for path in context_paths {
        match workspace.read(path).await {
            Ok(doc) => {
                context_parts.push(format!("## {}\n\n{}", path, doc.content));
            }
            Err(e) => {
                tracing::debug!(
                    routine = %routine.name,
                    "Failed to read context path {}: {}", path, e
                );
            }
        }
    }

    // Load routine state from workspace (name sanitized to prevent path traversal)
    let safe_name = sanitize_routine_name(&routine.name);
    let state_path = format!("routines/{safe_name}/state.md");
    let state_content = match workspace.read(&state_path).await {
        Ok(doc) => Some(doc.content),
        Err(_) => None,
    };

    // Get system prompt
    let system_prompt = match workspace.trusted_system_prompt(false).await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(routine = %routine.name, "Failed to get system prompt: {}", e);
            String::new()
        }
    };

    let model_context_length = ctx
        .llm
        .model_metadata()
        .await
        .ok()
        .and_then(|meta| meta.context_length)
        .filter(|length| *length > 0);
    // Honor the configured output cap while ensuring it cannot consume more
    // than half of a smaller provider window.
    let effective_max_tokens = effective_lightweight_max_tokens(max_tokens, model_context_length);

    let fixed_messages = lightweight_routine_fixed_messages(&system_prompt, prompt);
    let evidence =
        lightweight_routine_evidence(&context_parts, state_content.as_deref(), trigger_detail);
    let monitor = crate::agent::context_monitor::ContextMonitor::new().with_limit(
        model_context_length.map_or_else(
            || crate::agent::context_monitor::ContextMonitor::new().limit(),
            |length| length as usize,
        ),
    );
    let Some(bounded_evidence) = thinclaw_agent::context_monitor::bound_recent_untrusted_context(
        &monitor,
        &fixed_messages,
        "lightweight_routine_evidence",
        "workspace_state_and_trigger",
        &evidence,
        effective_max_tokens as usize,
        thinclaw_agent::context_monitor::AUXILIARY_CONTEXT_SAFETY_MARGIN_PERCENT,
    ) else {
        return Err(RoutineError::LlmFailed {
            reason: format!(
                "routine policy and prompt exceed the active model context window ({} tokens)",
                monitor.limit()
            ),
        });
    };
    if bounded_evidence.was_truncated {
        tracing::warn!(
            routine = %routine.name,
            context_limit = monitor.limit(),
            retained_chars = bounded_evidence.retained_chars,
            "Lightweight routine evidence was truncated to the active model window"
        );
    }
    let mut messages = fixed_messages;
    messages.push(bounded_evidence.message);

    let request = CompletionRequest::new(messages)
        .with_max_tokens(effective_max_tokens)
        .with_temperature(0.3);

    let response = ctx
        .llm
        .complete(request)
        .await
        .map_err(|e| RoutineError::LlmFailed {
            reason: e.to_string(),
        })?;

    classify_lightweight_routine_response(
        &response.content,
        response.finish_reason,
        response.input_tokens,
        response.output_tokens,
    )
}

/// Send a notification based on the routine's notify config and run status.
async fn send_notification(
    tx: &mpsc::Sender<OutgoingResponse>,
    notify: &NotifyConfig,
    routine_name: &str,
    status: RunStatus,
    summary: Option<&str>,
) {
    let Some(notification) = build_routine_notification(notify, routine_name, status, summary)
    else {
        return;
    };

    let response = OutgoingResponse {
        content: notification.content,
        thread_id: None,
        metadata: notification.metadata,
        attachments: Vec::new(),
    };

    if let Err(e) = tx.send(response).await {
        tracing::error!(routine = %routine_name, "Failed to send notification: {}", e);
    }
}
