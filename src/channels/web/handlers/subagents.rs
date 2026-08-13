use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use thinclaw_types::subagent::{
    SUBAGENT_RUN_STATUS_CANCELLED, SUBAGENT_RUN_STATUS_RUNNING, is_subagent_run_status,
};
use uuid::Uuid;

use crate::agent::subagent_executor::SubagentSpawnRequest;
use crate::channels::web::identity_helpers::GatewayRequestIdentity;
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::{
    SubagentCancelApiResponse, SubagentExecutionResult, SubagentRunApiResponse,
    SubagentRunsApiResponse, SubagentSpawnApiRequest, SubagentSpawnApiResponse,
};

const MAX_SUBAGENT_RUNS: usize = 500;
const MAX_SUBAGENT_NAME_BYTES: usize = 128;
const MAX_SUBAGENT_TASK_BYTES: usize = 256 * 1024;

#[derive(Debug, Deserialize)]
pub(crate) struct SubagentListQuery {
    status: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    100
}

fn owner_scope(identity: &GatewayRequestIdentity) -> Option<(&str, &str)> {
    if identity.is_legacy_primary_bearer() {
        None
    } else {
        Some((&identity.principal_id, &identity.actor_id))
    }
}

fn validate_spawn_request(request: &SubagentSpawnApiRequest) -> Result<(), (StatusCode, String)> {
    let name = request.name.trim();
    if name.is_empty() || name.len() > MAX_SUBAGENT_NAME_BYTES {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("sub-agent name must be 1..={MAX_SUBAGENT_NAME_BYTES} bytes"),
        ));
    }
    let objective = request
        .task_packet
        .as_ref()
        .map(|packet| packet.objective.trim())
        .filter(|objective| !objective.is_empty())
        .unwrap_or_else(|| request.task.trim());
    if objective.is_empty() || objective.len() > MAX_SUBAGENT_TASK_BYTES {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("sub-agent task must be 1..={MAX_SUBAGENT_TASK_BYTES} bytes"),
        ));
    }
    if request.timeout_secs.is_some_and(|seconds| seconds == 0) {
        return Err((
            StatusCode::BAD_REQUEST,
            "sub-agent timeout must be greater than zero".to_string(),
        ));
    }
    Ok(())
}

pub(crate) async fn subagents_spawn_handler(
    State(state): State<Arc<GatewayState>>,
    identity: GatewayRequestIdentity,
    Json(request): Json<SubagentSpawnApiRequest>,
) -> Result<Json<SubagentSpawnApiResponse>, (StatusCode, String)> {
    validate_spawn_request(&request)?;
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "sub-agent durable run ledger is unavailable".to_string(),
    ))?;
    let executor = state.subagent_executor().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "sub-agent runtime is not ready".to_string(),
    ))?;
    let thread_id = request
        .parent_thread_id
        .clone()
        .unwrap_or_else(|| format!("cli:{}", Uuid::new_v4()));
    let resolved_identity = identity.resolved_identity(Some(&thread_id));
    let wait = request.wait;
    let runtime_request = SubagentSpawnRequest {
        name: request.name,
        task: request.task,
        system_prompt: request.system_prompt,
        model: request.model,
        task_packet: request.task_packet,
        memory_mode: request.memory_mode,
        tool_mode: request.tool_mode,
        skill_mode: request.skill_mode,
        tool_profile: request.tool_profile,
        allowed_tools: request.allowed_tools,
        allowed_skills: request.allowed_skills,
        principal_id: None,
        actor_id: None,
        agent_workspace_id: None,
        timeout_secs: request.timeout_secs,
        wait,
    };
    let result = executor
        .spawn(
            runtime_request,
            "gateway",
            &serde_json::json!({"reinject_result": false, "source": "cli"}),
            &identity.principal_id,
            Some(&resolved_identity),
            Some(&thread_id),
        )
        .await
        .map_err(|error| {
            (
                StatusCode::CONFLICT,
                format!("sub-agent admission failed: {error}"),
            )
        })?;
    let run = store
        .get_subagent_run(result.agent_id, owner_scope(&identity))
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
        .ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "sub-agent spawned without a durable ledger record".to_string(),
        ))?;
    let result = wait.then_some(SubagentExecutionResult {
        agent_id: result.agent_id,
        name: result.name,
        response: result.response,
        iterations: result.iterations,
        duration_ms: result.duration_ms,
        success: result.success,
        error: result.error,
    });
    Ok(Json(SubagentSpawnApiResponse { run, result }))
}

pub(crate) async fn subagents_list_handler(
    State(state): State<Arc<GatewayState>>,
    identity: GatewayRequestIdentity,
    Query(query): Query<SubagentListQuery>,
) -> Result<Json<SubagentRunsApiResponse>, (StatusCode, String)> {
    if !(1..=MAX_SUBAGENT_RUNS).contains(&query.limit) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("sub-agent list limit must be between 1 and {MAX_SUBAGENT_RUNS}"),
        ));
    }
    if query
        .status
        .as_deref()
        .is_some_and(|status| !is_subagent_run_status(status))
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "invalid sub-agent run status".to_string(),
        ));
    }
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "sub-agent durable run ledger is unavailable".to_string(),
    ))?;
    let runs = store
        .list_subagent_runs(
            owner_scope(&identity),
            query.status.as_deref(),
            query.limit as i64,
        )
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    Ok(Json(SubagentRunsApiResponse { runs }))
}

pub(crate) async fn subagents_status_handler(
    State(state): State<Arc<GatewayState>>,
    identity: GatewayRequestIdentity,
    Path(id): Path<Uuid>,
) -> Result<Json<SubagentRunApiResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "sub-agent durable run ledger is unavailable".to_string(),
    ))?;
    let run = store
        .get_subagent_run(id, owner_scope(&identity))
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "sub-agent run not found".to_string()))?;
    Ok(Json(SubagentRunApiResponse { run }))
}

pub(crate) async fn subagents_cancel_handler(
    State(state): State<Arc<GatewayState>>,
    identity: GatewayRequestIdentity,
    Path(id): Path<Uuid>,
) -> Result<Json<SubagentCancelApiResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "sub-agent durable run ledger is unavailable".to_string(),
    ))?;
    let owner = owner_scope(&identity);
    let current = store
        .get_subagent_run(id, owner)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "sub-agent run not found".to_string()))?;
    if current.status != SUBAGENT_RUN_STATUS_RUNNING {
        return Ok(Json(SubagentCancelApiResponse {
            agent_id: id,
            cancelled: false,
            status: "not_found_or_already_done".to_string(),
            run: current,
        }));
    }

    let executor = state.subagent_executor().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "sub-agent runtime is not ready".to_string(),
    ))?;
    if !executor.cancel(id).await {
        let run = store
            .get_subagent_run(id, owner)
            .await
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
            .unwrap_or(current);
        return Ok(Json(SubagentCancelApiResponse {
            agent_id: id,
            cancelled: false,
            status: "not_found_or_already_done".to_string(),
            run,
        }));
    }
    // The executor owns normal completion. This first-write-wins call closes
    // the ledger if its bounded finalization timed out or the store briefly
    // failed during cancellation.
    store
        .complete_subagent_run(id, SUBAGENT_RUN_STATUS_CANCELLED, None)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let run = store
        .get_subagent_run(id, owner)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "sub-agent run not found".to_string()))?;
    Ok(Json(SubagentCancelApiResponse {
        agent_id: id,
        cancelled: true,
        status: "cancelled".to_string(),
        run,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "libsql")]
    use crate::channels::web::server::test_gateway_state_with_store;
    #[cfg(feature = "libsql")]
    use crate::db::{Database, SubagentRunStore, libsql::LibSqlBackend};
    use thinclaw_gateway::web::identity::GatewayAuthSource;
    #[cfg(feature = "libsql")]
    use thinclaw_types::subagent::SUBAGENT_RUN_STATUS_COMPLETED;

    #[test]
    fn configured_principals_are_exactly_owner_scoped() {
        let identity = GatewayRequestIdentity::new(
            "principal-a",
            "actor-a",
            GatewayAuthSource::BearerHeader,
            false,
        );
        assert_eq!(owner_scope(&identity), Some(("principal-a", "actor-a")));
    }

    #[test]
    fn only_legacy_primary_bearer_gets_global_compatibility_scope() {
        let identity = GatewayRequestIdentity::new(
            "default",
            "default",
            GatewayAuthSource::BearerHeader,
            true,
        )
        .with_legacy_primary_binding();
        assert_eq!(owner_scope(&identity), None);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn list_and_status_handlers_use_completed_owner_scoped_ledger_rows() {
        let directory = tempfile::tempdir().unwrap();
        let backend = Arc::new(
            LibSqlBackend::new_local(&directory.path().join("subagents.db"))
                .await
                .unwrap(),
        );
        backend.run_migrations().await.unwrap();
        let owned_id = Uuid::new_v4();
        let other_id = Uuid::new_v4();
        for (id, actor_id) in [(owned_id, "actor-a"), (other_id, "actor-b")] {
            backend
                .insert_subagent_run(&thinclaw_types::subagent::SubagentRunRecord::new_running(
                    id,
                    "reviewer",
                    "review",
                    "principal-a",
                    actor_id,
                    None,
                    None,
                    chrono::Utc::now(),
                ))
                .await
                .unwrap();
            backend
                .complete_subagent_run(id, SUBAGENT_RUN_STATUS_COMPLETED, None)
                .await
                .unwrap();
        }
        let store: Arc<dyn Database> = backend;
        let state = Arc::new(test_gateway_state_with_store(
            "principal-a",
            "actor-a",
            Some(store),
        ));
        let identity = GatewayRequestIdentity::new(
            "principal-a",
            "actor-a",
            GatewayAuthSource::BearerHeader,
            false,
        );

        let Json(listed) = subagents_list_handler(
            State(Arc::clone(&state)),
            identity.clone(),
            Query(SubagentListQuery {
                status: None,
                limit: 100,
            }),
        )
        .await
        .unwrap();
        assert_eq!(listed.runs.len(), 1);
        assert_eq!(listed.runs[0].id, owned_id);
        assert_eq!(listed.runs[0].status, SUBAGENT_RUN_STATUS_COMPLETED);

        let Json(found) =
            subagents_status_handler(State(Arc::clone(&state)), identity.clone(), Path(owned_id))
                .await
                .unwrap();
        assert_eq!(found.run.id, owned_id);
        let error = subagents_status_handler(State(state), identity, Path(other_id))
            .await
            .unwrap_err();
        assert_eq!(error.0, StatusCode::NOT_FOUND);
    }
}
