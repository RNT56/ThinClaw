//! RPC commands - jobs and desktop autonomy control surfaces.

use tauri::State;
use uuid::Uuid;

use crate::thinclaw::ironclaw_bridge::IronClawState;

fn local_unavailable(capability: &str, reason: impl AsRef<str>) -> String {
    format!(
        "unavailable: local ThinClaw desktop does not support {}: {}",
        capability,
        reason.as_ref()
    )
}

fn autonomy_unavailable(reason: impl AsRef<str>) -> String {
    local_unavailable(
        "desktop autonomy execution",
        format!(
            "{}. Enable reckless desktop autonomy in the ThinClaw host config and satisfy host permission checks first",
            reason.as_ref()
        ),
    )
}

async fn local_direct_jobs(
    ironclaw: &IronClawState,
) -> Result<Vec<ironclaw::context::JobContext>, String> {
    let agent = ironclaw.agent().await?;
    let context_manager = agent.context_manager();
    let mut jobs = Vec::new();
    for job_id in context_manager.all_jobs().await {
        if let Ok(job) = context_manager.get_context(job_id).await {
            jobs.push(job);
        }
    }
    jobs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(jobs)
}

fn job_summary(jobs: &[ironclaw::context::JobContext]) -> serde_json::Value {
    let mut summary = serde_json::json!({
        "total": jobs.len(),
        "pending": 0,
        "in_progress": 0,
        "completed": 0,
        "failed": 0,
        "cancelled": 0,
        "interrupted": 0,
        "stuck": 0,
    });

    for job in jobs {
        let key = match job.state {
            ironclaw::context::JobState::Pending => "pending",
            ironclaw::context::JobState::InProgress => "in_progress",
            ironclaw::context::JobState::Completed
            | ironclaw::context::JobState::Submitted
            | ironclaw::context::JobState::Accepted => "completed",
            ironclaw::context::JobState::Failed | ironclaw::context::JobState::Abandoned => {
                "failed"
            }
            ironclaw::context::JobState::Cancelled => "cancelled",
            ironclaw::context::JobState::Stuck => "stuck",
        };
        if let Some(value) = summary.get_mut(key).and_then(|v| v.as_u64()) {
            summary[key] = serde_json::json!(value + 1);
        }
    }

    summary
}

fn job_list_item(job: &ironclaw::context::JobContext) -> serde_json::Value {
    serde_json::json!({
        "id": job.job_id,
        "title": job.title,
        "state": job.state.to_string(),
        "user_id": job.user_id,
        "created_at": job.created_at.to_rfc3339(),
        "started_at": job.started_at.map(|dt| dt.to_rfc3339()),
        "execution_backend": "local_host",
        "runtime_family": "local",
        "runtime_mode": "desktop",
    })
}

fn job_detail(job: &ironclaw::context::JobContext) -> serde_json::Value {
    let elapsed_secs = job.started_at.map(|started| {
        let end = job.completed_at.unwrap_or_else(chrono::Utc::now);
        (end - started).num_seconds().max(0) as u64
    });
    let transitions: Vec<serde_json::Value> = job
        .transitions
        .iter()
        .map(|transition| {
            serde_json::json!({
                "from": transition.from.to_string(),
                "to": transition.to.to_string(),
                "timestamp": transition.timestamp.to_rfc3339(),
                "reason": transition.reason,
            })
        })
        .collect();

    serde_json::json!({
        "id": job.job_id,
        "title": job.title,
        "description": job.description,
        "state": job.state.to_string(),
        "user_id": job.user_id,
        "created_at": job.created_at.to_rfc3339(),
        "started_at": job.started_at.map(|dt| dt.to_rfc3339()),
        "completed_at": job.completed_at.map(|dt| dt.to_rfc3339()),
        "elapsed_secs": elapsed_secs,
        "execution_backend": "local_host",
        "runtime_family": "local",
        "runtime_mode": "desktop",
        "runtime_capabilities": ["cancel", "events"],
        "interactive": false,
        "transitions": transitions,
    })
}

async fn local_job_by_id(
    ironclaw: &IronClawState,
    job_id: &str,
) -> Result<ironclaw::context::JobContext, String> {
    let parsed = Uuid::parse_str(job_id).map_err(|_| "Invalid job ID".to_string())?;
    let agent = ironclaw.agent().await?;
    agent
        .context_manager()
        .get_context(parsed)
        .await
        .map_err(|_| "Job not found".to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_jobs_list(
    ironclaw: State<'_, IronClawState>,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.get_jobs().await;
    }

    let jobs = local_direct_jobs(&ironclaw).await?;
    Ok(serde_json::json!({
        "jobs": jobs.iter().map(job_list_item).collect::<Vec<_>>(),
        "capabilities": {
            "detail": true,
            "events": true,
            "cancel": true,
            "restart": false,
            "prompt": false,
            "files": false,
        },
        "unavailable": {
            "restart": "Local desktop direct jobs are not restartable through the sandbox restart endpoint.",
            "prompt": "Local desktop direct jobs are not interactive sandbox jobs.",
            "files": "Local desktop direct jobs do not expose a sandbox project directory."
        }
    }))
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_jobs_summary(
    ironclaw: State<'_, IronClawState>,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.get_jobs_summary().await;
    }

    let jobs = local_direct_jobs(&ironclaw).await?;
    Ok(job_summary(&jobs))
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_job_detail(
    ironclaw: State<'_, IronClawState>,
    job_id: String,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.get_job_detail(&job_id).await;
    }

    let job = local_job_by_id(&ironclaw, &job_id).await?;
    Ok(job_detail(&job))
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_job_cancel(
    ironclaw: State<'_, IronClawState>,
    job_id: String,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.cancel_job(&job_id).await;
    }

    let parsed = Uuid::parse_str(&job_id).map_err(|_| "Invalid job ID".to_string())?;
    let agent = ironclaw.agent().await?;
    let current = agent
        .context_manager()
        .get_context(parsed)
        .await
        .map_err(|_| "Job not found".to_string())?;
    if !current.state.is_active() {
        return Err(format!("Cannot cancel job in state '{}'", current.state));
    }

    if agent.scheduler().is_running(parsed).await {
        agent
            .scheduler()
            .stop(parsed)
            .await
            .map_err(|e| e.to_string())?;
    }

    agent
        .context_manager()
        .update_context(parsed, |ctx| {
            ctx.transition_to(
                ironclaw::context::JobState::Cancelled,
                Some("Cancelled by user".to_string()),
            )
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;

    if let Some(store) = agent.store() {
        if let Ok(snapshot) = agent.context_manager().get_context(parsed).await {
            store.save_job(&snapshot).await.map_err(|e| e.to_string())?;
        }
    }

    Ok(serde_json::json!({ "status": "cancelled", "job_id": parsed }))
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_job_restart(
    ironclaw: State<'_, IronClawState>,
    job_id: String,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.restart_job(&job_id).await;
    }

    Err(local_unavailable(
        "job restart",
        "only remote gateway sandbox jobs expose restartable stored job specs",
    ))
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_job_prompt(
    ironclaw: State<'_, IronClawState>,
    job_id: String,
    content: Option<String>,
    done: Option<bool>,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .prompt_job(&job_id, content, done.unwrap_or(false))
            .await;
    }

    Err(local_unavailable(
        "job prompt",
        "only remote gateway interactive sandbox jobs accept follow-up prompts",
    ))
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_job_events(
    ironclaw: State<'_, IronClawState>,
    job_id: String,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.get_job_events(&job_id).await;
    }

    let parsed = Uuid::parse_str(&job_id).map_err(|_| "Invalid job ID".to_string())?;
    let agent = ironclaw.agent().await?;
    if agent.context_manager().get_context(parsed).await.is_err() {
        return Err("Job not found".to_string());
    }
    let events = if let Some(store) = agent.store() {
        store
            .list_job_events(parsed, None)
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|event| {
                serde_json::json!({
                    "id": event.id,
                    "event_type": event.event_type,
                    "data": event.data,
                    "created_at": event.created_at.to_rfc3339(),
                })
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    Ok(serde_json::json!({
        "job_id": parsed,
        "events": events,
        "events_available": agent.store().is_some(),
        "unavailable_reason": if agent.store().is_some() {
            serde_json::Value::Null
        } else {
            serde_json::json!("Local job event history requires the ThinClaw database store.")
        }
    }))
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_job_files_list(
    ironclaw: State<'_, IronClawState>,
    job_id: String,
    path: Option<String>,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.list_job_files(&job_id, path.as_deref()).await;
    }

    Err(local_unavailable(
        "job files",
        "only remote gateway sandbox jobs expose project file browsing",
    ))
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_job_file_read(
    ironclaw: State<'_, IronClawState>,
    job_id: String,
    path: String,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.read_job_file(&job_id, &path).await;
    }

    Err(local_unavailable(
        "job file read",
        "only remote gateway sandbox jobs expose project file reads",
    ))
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_autonomy_status(
    ironclaw: State<'_, IronClawState>,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.get_autonomy_status().await;
    }

    ironclaw::tauri_commands::autonomy_status()
        .await
        .and_then(|status| serde_json::to_value(status).map_err(|e| e.to_string()))
        .map_err(autonomy_unavailable)
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_autonomy_bootstrap(
    ironclaw: State<'_, IronClawState>,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.bootstrap_autonomy().await;
    }

    ironclaw::tauri_commands::autonomy_bootstrap()
        .await
        .and_then(|report| serde_json::to_value(report).map_err(|e| e.to_string()))
        .map_err(autonomy_unavailable)
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_autonomy_pause(
    ironclaw: State<'_, IronClawState>,
    reason: Option<String>,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.pause_autonomy(reason).await;
    }

    ironclaw::tauri_commands::autonomy_pause(reason)
        .await
        .map_err(autonomy_unavailable)
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_autonomy_resume(
    ironclaw: State<'_, IronClawState>,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.resume_autonomy().await;
    }

    ironclaw::tauri_commands::autonomy_resume()
        .await
        .map_err(autonomy_unavailable)
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_autonomy_permissions(
    ironclaw: State<'_, IronClawState>,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.get_autonomy_permissions().await;
    }

    ironclaw::tauri_commands::desktop_permission_status()
        .await
        .map_err(autonomy_unavailable)
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_desktop_permission_status(
    ironclaw: State<'_, IronClawState>,
) -> Result<serde_json::Value, String> {
    thinclaw_autonomy_permissions(ironclaw).await
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_autonomy_rollback(
    ironclaw: State<'_, IronClawState>,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.rollback_autonomy().await;
    }

    ironclaw::tauri_commands::autonomy_rollback()
        .await
        .map_err(autonomy_unavailable)
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_autonomy_rollouts(
    ironclaw: State<'_, IronClawState>,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.get_autonomy_rollouts().await;
    }

    let Some(manager) = ironclaw::desktop_autonomy::desktop_autonomy_manager() else {
        return Err(autonomy_unavailable(
            "desktop autonomy manager is not active",
        ));
    };
    manager
        .rollout_summary()
        .await
        .and_then(|summary| serde_json::to_value(summary).map_err(|e| e.to_string()))
        .map_err(autonomy_unavailable)
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_autonomy_checks(
    ironclaw: State<'_, IronClawState>,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.get_autonomy_checks().await;
    }

    let Some(manager) = ironclaw::desktop_autonomy::desktop_autonomy_manager() else {
        return Err(autonomy_unavailable(
            "desktop autonomy manager is not active",
        ));
    };
    manager
        .checks_summary()
        .await
        .and_then(|summary| serde_json::to_value(summary).map_err(|e| e.to_string()))
        .map_err(autonomy_unavailable)
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_autonomy_evidence(
    ironclaw: State<'_, IronClawState>,
) -> Result<serde_json::Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.get_autonomy_evidence().await;
    }

    let Some(manager) = ironclaw::desktop_autonomy::desktop_autonomy_manager() else {
        return Err(autonomy_unavailable(
            "desktop autonomy manager is not active",
        ));
    };
    manager
        .evidence_summary()
        .await
        .and_then(|summary| serde_json::to_value(summary).map_err(|e| e.to_string()))
        .map_err(autonomy_unavailable)
}
