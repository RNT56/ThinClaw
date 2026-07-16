//! Long-running tasks owned by the embedded desktop agent.

use std::sync::Arc;

use thinclaw_core::agent::subagent_executor::SubagentResultMessage;
use thinclaw_core::agent::{Agent, BackgroundTasksHandle, RoutineEngine};

pub(super) fn spawn_subagent_result_injector(
    agent: &Arc<Agent>,
    subagent_result_rx: tokio::sync::mpsc::Receiver<SubagentResultMessage>,
) {
    // ── 6b. Sub-agent result injector ───────────────────────────────
    // Polls the SubagentExecutor's result channel and re-injects
    // completed sub-agent results back into the main agent as new
    // user-invisible turns. This is the "fire-and-forget → re-inject"
    // pattern that enables true parallelism.
    {
        let agent_for_subagent = Arc::clone(agent);
        let mut rx = subagent_result_rx;
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                let result = &msg.result;
                let synthetic_content = if result.success {
                    format!(
                        "[Sub-agent '{}' completed ({} iterations, {:.1}s)]\n\n{}",
                        result.name,
                        result.iterations,
                        result.duration_ms as f64 / 1000.0,
                        result.response
                    )
                } else {
                    format!(
                        "[Sub-agent '{}' failed ({:.1}s)]\n\nError: {}",
                        result.name,
                        result.duration_ms as f64 / 1000.0,
                        result.error.as_deref().unwrap_or("unknown"),
                    )
                };

                // Handle status/ledger updates are performed by the
                // executor's own finalization block when the subagent task
                // completes; no external completion call is needed here.
                tracing::info!(
                    agent_id = %result.agent_id,
                    name = %result.name,
                    success = result.success,
                    iterations = result.iterations,
                    duration_ms = result.duration_ms,
                    "Sub-agent result received, injecting into main agent"
                );

                // Build an IncomingMessage that goes through the normal pipeline
                let incoming = thinclaw_core::channels::IncomingMessage::new(
                    "subagent",
                    "system",
                    &synthetic_content,
                )
                .with_thread(&msg.parent_thread_id)
                .with_metadata(msg.channel_metadata.clone());

                match agent_for_subagent.handle_message_external(&incoming).await {
                    Ok(Some(response)) if !response.is_empty() => {
                        tracing::debug!(
                            "Main agent response to sub-agent result: {} chars",
                            response.len()
                        );
                        // The response goes through TauriChannel automatically
                        // via the normal respond() path in handle_message
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!("Failed to inject sub-agent result: {}", e);
                    }
                }
            }
            tracing::debug!("[subagent] Result injector task ended");
        });
    }
}

pub(super) async fn start(
    agent: &Arc<Agent>,
) -> (BackgroundTasksHandle, Option<Arc<RoutineEngine>>) {
    // ── 7. Start background tasks ───────────────────────────────────
    let bg_handle = agent.start_background_tasks().await;

    // Extract routine engine Arc for easy access (parity with run() loop's
    // routine_engine_for_loop). The same Arc stays in bg_handle too.
    let routine_engine = bg_handle.routine_engine().map(Arc::clone);

    // ── 7a. System event consumer (heartbeat → livechat) ─────────────
    // In standalone mode, agent.run() reads from system_event_rx in its
    // main select! loop. In Tauri mode, there IS no message loop — each
    // user message is processed on-demand via handle_message_external().
    // Without this consumer, heartbeat messages pile up in the channel
    // buffer (capacity 16) and are silently dropped.
    {
        let mut bg_lock = bg_handle.lock_system_events().await;
        if let Some(mut system_rx) = bg_lock.take() {
            let agent_for_sys = Arc::clone(agent);
            tokio::spawn(async move {
                tracing::info!(
                    "[thinclaw-runtime] System event consumer started (heartbeat → livechat)"
                );
                while let Some(msg) = system_rx.recv().await {
                    tracing::info!(
                        channel = %msg.channel,
                        "[thinclaw-runtime] Processing system event in Tauri mode"
                    );

                    match agent_for_sys.handle_message_external(&msg).await {
                        Ok(Some(response)) if !response.is_empty() => {
                            // Deliver via broadcast_all (→ TauriChannel → thinclaw-event)
                            // We use broadcast_all instead of respond() because the
                            // message's channel is "heartbeat" which isn't a registered
                            // channel — TauriChannel registers as "tauri".
                            let results = agent_for_sys
                                .channels()
                                .broadcast_all(
                                    &msg.user_id,
                                    thinclaw_core::channels::OutgoingResponse::text(response),
                                )
                                .await;
                            for (ch, result) in results {
                                if let Err(e) = result {
                                    tracing::error!(
                                        "[thinclaw-runtime] System event broadcast to {} failed: {}",
                                        ch,
                                        e
                                    );
                                }
                            }
                        }
                        Ok(_) => {
                            tracing::debug!(
                                "[thinclaw-runtime] System event processed (no visible response)"
                            );
                        }
                        Err(e) => {
                            tracing::error!(
                                "[thinclaw-runtime] System event processing failed: {}",
                                e
                            );
                        }
                    }
                }
                tracing::info!("[thinclaw-runtime] System event consumer ended");
            });
        }
    }

    // ── 7b. Job TTL reaper — force-cancel zombie jobs ────────────────
    // Prevents the "Maximum parallel jobs (5) exceeded" cascade.
    // If a job is active for longer than JOB_MAX_TTL, we force-cancel it
    // to free the slot. The existing cleanup tasks in scheduler.rs only
    // remove finished handles from the jobs HashMap — they don't touch
    // the ContextManager, which is where the slot-counting happens.
    {
        const JOB_MAX_TTL_SECS: i64 = 600; // 10 minutes
        const REAPER_INTERVAL_SECS: u64 = 60; // check every minute

        let agent_for_reaper = Arc::clone(agent);
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(REAPER_INTERVAL_SECS));
            // Skip immediate first tick
            interval.tick().await;

            loop {
                interval.tick().await;

                let cm = agent_for_reaper.context_manager();
                let active = cm.active_jobs().await;

                if active.is_empty() {
                    continue;
                }

                let now = chrono::Utc::now();
                let mut reaped = 0usize;

                for job_id in active {
                    if let Ok(ctx) = cm.get_context(job_id).await {
                        // Only reap InProgress or Pending jobs (not Stuck — self-repair handles those)
                        if !matches!(
                            ctx.state,
                            thinclaw_core::context::JobState::InProgress
                                | thinclaw_core::context::JobState::Pending
                        ) {
                            continue;
                        }

                        let age = now.signed_duration_since(ctx.created_at);
                        if age.num_seconds() > JOB_MAX_TTL_SECS {
                            tracing::warn!(
                                job_id = %job_id,
                                age_secs = age.num_seconds(),
                                title = %ctx.title,
                                "[reaper] Force-cancelling zombie job (exceeded {}s TTL)",
                                JOB_MAX_TTL_SECS
                            );

                            // Try to cancel via scheduler first (sends Stop + abort)
                            agent_for_reaper.scheduler().stop(job_id).await.ok();

                            // Also force the ContextManager state to terminal
                            // in case the scheduler didn't clean it up
                            let _ = cm
                                .update_context(job_id, |c| {
                                    let _ = c.transition_to(
                                        thinclaw_core::context::JobState::Failed,
                                        Some(format!(
                                            "Force-cancelled by TTL reaper (alive {}s, limit {}s)",
                                            age.num_seconds(),
                                            JOB_MAX_TTL_SECS
                                        )),
                                    );
                                })
                                .await;

                            reaped += 1;
                        }
                    }
                }

                if reaped > 0 {
                    tracing::info!(
                        "[reaper] Force-cancelled {} zombie job(s), freeing slots",
                        reaped
                    );
                }
            }
        });
    }

    // ── 7c. BeforeAgentStart hook ────────────────────────────────────
    // Parity with run() loop — allows hooks to inspect startup config.
    {
        let event = thinclaw_core::hooks::HookEvent::AgentStart {
            model: "tauri-direct".to_string(),
            provider: "ironclaw".to_string(),
        };
        match agent.hooks().run(&event).await {
            Err(thinclaw_core::hooks::HookError::Rejected { reason }) => {
                tracing::error!("BeforeAgentStart hook rejected startup: {}", reason);
                // Don't fail the engine start — just log. The hook can still
                // do pre-flight checks, but we don't want to prevent the UI.
            }
            Err(err) => {
                tracing::warn!("BeforeAgentStart hook error (fail-open): {}", err);
            }
            Ok(_) => {}
        }
    }

    (bg_handle, routine_engine)
}
