//! Experiment and learning review commands.
//!
//! These IPC commands keep the alpha `thinclaw_*` command names while routing
//! to either the embedded ThinClaw API modules or the remote gateway HTTP API.

use serde::Serialize;
use serde_json::{json, Value};
use tauri::State;
use uuid::Uuid;

use crate::thinclaw::ironclaw_bridge::IronClawState;

const USER_ID: &str = "local_user";
const DEFAULT_LIMIT: usize = 50;

fn unavailable(reason: impl Into<String>) -> Value {
    json!({
        "available": false,
        "reason": reason.into(),
    })
}

fn api_result_to_json<T: Serialize>(result: ironclaw::api::ApiResult<T>) -> Result<Value, String> {
    match result {
        Ok(value) => serde_json::to_value(value).map_err(|error| error.to_string()),
        Err(ironclaw::api::ApiError::FeatureDisabled(message))
        | Err(ironclaw::api::ApiError::Unavailable(message)) => Ok(unavailable(message)),
        Err(error) => Err(format!("{}: {}", error.error_code(), error)),
    }
}

async fn local_store(
    ironclaw: &State<'_, IronClawState>,
) -> Result<std::sync::Arc<dyn ironclaw::db::Database>, String> {
    let agent = ironclaw.agent().await?;
    agent
        .store()
        .cloned()
        .ok_or_else(|| "ThinClaw database is not available".to_string())
}

async fn learning_orchestrator(
    ironclaw: &State<'_, IronClawState>,
) -> Result<ironclaw::agent::learning::LearningOrchestrator, String> {
    let agent = ironclaw.agent().await?;
    let store = agent
        .store()
        .cloned()
        .ok_or_else(|| "ThinClaw database is not available".to_string())?;
    let orchestrator = ironclaw::agent::learning::LearningOrchestrator::new(
        store,
        agent.workspace().cloned(),
        agent.skill_registry().cloned(),
    )
    .with_routine_engine(ironclaw.routine_engine().await);
    Ok(orchestrator)
}

fn parse_uuid(value: &str, label: &str) -> Result<Uuid, String> {
    Uuid::parse_str(value).map_err(|_| format!("Invalid {label} ID: {value}"))
}

fn limit_or_default(limit: Option<u32>) -> usize {
    limit.unwrap_or(DEFAULT_LIMIT as u32).clamp(1, 500) as usize
}

// ── Learning reads ──────────────────────────────────────────────────────────

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_learning_status(
    ironclaw: State<'_, IronClawState>,
    limit: Option<u32>,
) -> Result<Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .get_json(&format!(
                "/api/learning/status?limit={}",
                limit_or_default(limit)
            ))
            .await;
    }
    let store = local_store(&ironclaw).await?;
    let orchestrator = learning_orchestrator(&ironclaw).await?;
    api_result_to_json(
        ironclaw::api::learning::status(&store, &orchestrator, USER_ID, limit_or_default(limit))
            .await,
    )
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_learning_history(
    ironclaw: State<'_, IronClawState>,
    limit: Option<u32>,
) -> Result<Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .get_json(&format!(
                "/api/learning/history?limit={}",
                limit_or_default(limit)
            ))
            .await;
    }
    let store = local_store(&ironclaw).await?;
    api_result_to_json(
        ironclaw::api::learning::history(
            &store,
            USER_ID,
            Some(USER_ID),
            None,
            None,
            limit_or_default(limit),
        )
        .await,
    )
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_learning_candidates(
    ironclaw: State<'_, IronClawState>,
    limit: Option<u32>,
) -> Result<Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .get_json(&format!(
                "/api/learning/candidates?limit={}",
                limit_or_default(limit)
            ))
            .await;
    }
    let store = local_store(&ironclaw).await?;
    api_result_to_json(
        ironclaw::api::learning::candidates(&store, USER_ID, None, None, limit_or_default(limit))
            .await,
    )
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_learning_artifact_versions(
    ironclaw: State<'_, IronClawState>,
    limit: Option<u32>,
) -> Result<Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .get_json(&format!(
                "/api/learning/artifact-versions?limit={}",
                limit_or_default(limit)
            ))
            .await;
    }
    let store = local_store(&ironclaw).await?;
    api_result_to_json(
        ironclaw::api::learning::artifact_versions(
            &store,
            USER_ID,
            None,
            None,
            limit_or_default(limit),
        )
        .await,
    )
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_learning_provider_health(
    ironclaw: State<'_, IronClawState>,
) -> Result<Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.get_json("/api/learning/provider-health").await;
    }
    let orchestrator = learning_orchestrator(&ironclaw).await?;
    api_result_to_json(ironclaw::api::learning::provider_health(&orchestrator, USER_ID).await)
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_learning_code_proposals(
    ironclaw: State<'_, IronClawState>,
    status: Option<String>,
    limit: Option<u32>,
) -> Result<Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let status_query = status
            .as_deref()
            .filter(|value| !value.is_empty())
            .map(|value| format!("&status={value}"))
            .unwrap_or_default();
        return proxy
            .get_json(&format!(
                "/api/learning/code-proposals?limit={}{}",
                limit_or_default(limit),
                status_query
            ))
            .await;
    }
    let store = local_store(&ironclaw).await?;
    api_result_to_json(
        ironclaw::api::learning::code_proposals(
            &store,
            USER_ID,
            status.as_deref(),
            limit_or_default(limit),
        )
        .await,
    )
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_learning_outcomes(
    ironclaw: State<'_, IronClawState>,
    status: Option<String>,
    limit: Option<u32>,
) -> Result<Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let status_query = status
            .as_deref()
            .filter(|value| !value.is_empty())
            .map(|value| format!("&status={value}"))
            .unwrap_or_default();
        return proxy
            .get_json(&format!(
                "/api/learning/outcomes?limit={}{}",
                limit_or_default(limit),
                status_query
            ))
            .await;
    }
    let store = local_store(&ironclaw).await?;
    api_result_to_json(
        ironclaw::api::learning::outcomes(
            &store,
            USER_ID,
            Some(USER_ID),
            status.as_deref(),
            None,
            None,
            None,
            limit_or_default(limit),
        )
        .await,
    )
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_learning_rollbacks(
    ironclaw: State<'_, IronClawState>,
    limit: Option<u32>,
) -> Result<Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .get_json(&format!(
                "/api/learning/rollbacks?limit={}",
                limit_or_default(limit)
            ))
            .await;
    }
    let store = local_store(&ironclaw).await?;
    api_result_to_json(
        ironclaw::api::learning::rollbacks(&store, USER_ID, None, None, limit_or_default(limit))
            .await,
    )
}

// ── Learning review actions ────────────────────────────────────────────────

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_learning_review_code_proposal(
    ironclaw: State<'_, IronClawState>,
    proposal_id: String,
    decision: String,
    note: Option<String>,
) -> Result<Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .post_json(
                &format!("/api/learning/code-proposals/{proposal_id}/review"),
                &json!({ "decision": decision, "note": note }),
            )
            .await;
    }
    let orchestrator = learning_orchestrator(&ironclaw).await?;
    let proposal_id = parse_uuid(&proposal_id, "proposal")?;
    api_result_to_json(
        ironclaw::api::learning::review_code_proposal(
            &orchestrator,
            USER_ID,
            proposal_id,
            &decision,
            note.as_deref(),
        )
        .await,
    )
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_learning_review_outcome(
    ironclaw: State<'_, IronClawState>,
    outcome_id: String,
    decision: String,
    verdict: Option<String>,
) -> Result<Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .post_json(
                &format!("/api/learning/outcomes/{outcome_id}/review"),
                &json!({ "decision": decision, "verdict": verdict }),
            )
            .await;
    }
    let store = local_store(&ironclaw).await?;
    let outcome_id = parse_uuid(&outcome_id, "outcome")?;
    api_result_to_json(
        ironclaw::api::learning::review_outcome(
            &store,
            USER_ID,
            outcome_id,
            &decision,
            verdict.as_deref(),
        )
        .await,
    )
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_learning_record_rollback(
    ironclaw: State<'_, IronClawState>,
    artifact_type: String,
    artifact_name: String,
    artifact_version_id: Option<String>,
    reason: String,
) -> Result<Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .post_json(
                "/api/learning/rollbacks",
                &json!({
                    "artifact_type": artifact_type,
                    "artifact_name": artifact_name,
                    "artifact_version_id": artifact_version_id,
                    "reason": reason,
                }),
            )
            .await;
    }
    let store = local_store(&ironclaw).await?;
    let version_id = artifact_version_id
        .as_deref()
        .map(|id| parse_uuid(id, "artifact version"))
        .transpose()?;
    api_result_to_json(
        ironclaw::api::learning::record_rollback(
            &store,
            USER_ID,
            &artifact_type,
            &artifact_name,
            version_id,
            &reason,
            None,
        )
        .await,
    )
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_learning_evaluate_outcomes(
    ironclaw: State<'_, IronClawState>,
) -> Result<Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .post_json("/api/learning/outcomes/evaluate-now", &json!({}))
            .await;
    }
    Ok(unavailable(
        "Manual outcome evaluation requires the gateway outcome service; the Desktop local engine exposes outcome review state but not direct evaluator execution.",
    ))
}

// ── Experiments reads ──────────────────────────────────────────────────────

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_experiments_projects(
    ironclaw: State<'_, IronClawState>,
) -> Result<Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.get_json("/api/experiments/projects").await;
    }
    let store = local_store(&ironclaw).await?;
    api_result_to_json(ironclaw::api::experiments::list_projects(&store, USER_ID).await)
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_experiments_campaigns(
    ironclaw: State<'_, IronClawState>,
) -> Result<Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.get_json("/api/experiments/campaigns").await;
    }
    let store = local_store(&ironclaw).await?;
    api_result_to_json(ironclaw::api::experiments::list_campaigns(&store, USER_ID).await)
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_experiments_runners(
    ironclaw: State<'_, IronClawState>,
) -> Result<Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.get_json("/api/experiments/runners").await;
    }
    let store = local_store(&ironclaw).await?;
    api_result_to_json(ironclaw::api::experiments::list_runners(&store, USER_ID).await)
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_experiments_targets(
    ironclaw: State<'_, IronClawState>,
) -> Result<Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.get_json("/api/experiments/targets").await;
    }
    let store = local_store(&ironclaw).await?;
    api_result_to_json(ironclaw::api::experiments::list_targets(&store, USER_ID).await)
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_experiments_trials(
    ironclaw: State<'_, IronClawState>,
    campaign_id: String,
) -> Result<Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .get_json(&format!("/api/experiments/campaigns/{campaign_id}/trials"))
            .await;
    }
    let store = local_store(&ironclaw).await?;
    let campaign_id = parse_uuid(&campaign_id, "campaign")?;
    api_result_to_json(ironclaw::api::experiments::list_trials(&store, USER_ID, campaign_id).await)
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_experiments_trial_artifacts(
    ironclaw: State<'_, IronClawState>,
    trial_id: String,
) -> Result<Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .get_json(&format!("/api/experiments/trials/{trial_id}/artifacts"))
            .await;
    }
    let store = local_store(&ironclaw).await?;
    let trial_id = parse_uuid(&trial_id, "trial")?;
    api_result_to_json(ironclaw::api::experiments::list_artifacts(&store, USER_ID, trial_id).await)
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_experiments_model_usage(
    ironclaw: State<'_, IronClawState>,
    limit: Option<u32>,
) -> Result<Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .get_json(&format!(
                "/api/experiments/model-usage?limit={}",
                limit_or_default(limit)
            ))
            .await;
    }
    let store = local_store(&ironclaw).await?;
    api_result_to_json(
        ironclaw::api::experiments::list_model_usage(&store, USER_ID, limit_or_default(limit))
            .await,
    )
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_experiments_opportunities(
    ironclaw: State<'_, IronClawState>,
    limit: Option<u32>,
) -> Result<Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .get_json(&format!(
                "/api/experiments/opportunities?limit={}",
                limit_or_default(limit)
            ))
            .await;
    }
    let store = local_store(&ironclaw).await?;
    api_result_to_json(
        ironclaw::api::experiments::list_opportunities(&store, USER_ID, limit_or_default(limit))
            .await,
    )
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_experiments_gpu_clouds(
    ironclaw: State<'_, IronClawState>,
) -> Result<Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .get_json("/api/experiments/providers/gpu-clouds")
            .await;
    }
    let store = local_store(&ironclaw).await?;
    api_result_to_json(ironclaw::api::experiments::list_gpu_cloud_providers(&store, USER_ID).await)
}

// ── Experiments actions ────────────────────────────────────────────────────

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_experiments_validate_runner(
    ironclaw: State<'_, IronClawState>,
    runner_id: String,
) -> Result<Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .post_json(
                &format!("/api/experiments/runners/{runner_id}/validate"),
                &json!({}),
            )
            .await;
    }
    let store = local_store(&ironclaw).await?;
    let runner_id = parse_uuid(&runner_id, "runner")?;
    api_result_to_json(
        ironclaw::api::experiments::validate_runner(&store, USER_ID, runner_id).await,
    )
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_experiments_campaign_action(
    ironclaw: State<'_, IronClawState>,
    campaign_id: String,
    action: String,
) -> Result<Value, String> {
    let action = action.trim().to_ascii_lowercase();
    if !matches!(
        action.as_str(),
        "pause" | "resume" | "cancel" | "promote" | "reissue-lease"
    ) {
        return Err(format!("Unsupported experiment campaign action: {action}"));
    }

    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .post_json(
                &format!("/api/experiments/campaigns/{campaign_id}/{action}"),
                &json!({}),
            )
            .await;
    }

    let store = local_store(&ironclaw).await?;
    let campaign_id = parse_uuid(&campaign_id, "campaign")?;
    let result = match action.as_str() {
        "pause" => ironclaw::api::experiments::pause_campaign(&store, USER_ID, campaign_id).await,
        "resume" => ironclaw::api::experiments::resume_campaign(&store, USER_ID, campaign_id).await,
        "cancel" => ironclaw::api::experiments::cancel_campaign(&store, USER_ID, campaign_id).await,
        "promote" => {
            ironclaw::api::experiments::promote_campaign(&store, USER_ID, campaign_id).await
        }
        "reissue-lease" => {
            ironclaw::api::experiments::reissue_lease(&store, USER_ID, campaign_id).await
        }
        _ => unreachable!(),
    };
    api_result_to_json(result)
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_experiments_gpu_validate(
    ironclaw: State<'_, IronClawState>,
    provider: String,
) -> Result<Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .post_json(
                &format!("/api/experiments/providers/gpu-clouds/{provider}/validate"),
                &json!({}),
            )
            .await;
    }
    Ok(unavailable(
        "GPU cloud credential validation is only available through a ThinClaw gateway because it requires the gateway secrets service.",
    ))
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_experiments_gpu_launch_test(
    ironclaw: State<'_, IronClawState>,
    provider: String,
) -> Result<Value, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .post_json(
                &format!("/api/experiments/providers/gpu-clouds/{provider}/launch-test"),
                &json!({}),
            )
            .await;
    }
    Ok(unavailable(
        "GPU cloud launch tests are only available through a ThinClaw gateway because Desktop local mode does not expose provider credentials for remote compute launch.",
    ))
}
