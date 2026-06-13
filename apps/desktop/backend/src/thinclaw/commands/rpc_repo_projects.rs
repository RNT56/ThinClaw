use tauri::State;
use uuid::Uuid;

use crate::thinclaw::runtime_bridge::ThinClawRuntimeState;

async fn local_repo_project_store(
    ironclaw: &ThinClawRuntimeState,
) -> Result<std::sync::Arc<dyn thinclaw_core::db::Database>, String> {
    if ironclaw.remote_proxy().await.is_some() {
        return Err(
            "Repository project commands are not available through remote gateway mode yet"
                .to_string(),
        );
    }
    let agent = ironclaw.agent().await?;
    agent
        .store()
        .cloned()
        .ok_or_else(|| "ThinClaw database is not available".to_string())
}

fn parse_project_id(project_id: &str) -> Result<Uuid, String> {
    Uuid::parse_str(project_id).map_err(|_| "Invalid repository project ID".to_string())
}

fn value<T: serde::Serialize>(result: Result<T, thinclaw_core::api::ApiError>) -> Result<serde_json::Value, String> {
    let output = result.map_err(|error| error.to_string())?;
    serde_json::to_value(output).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_repo_projects_list(
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<serde_json::Value, String> {
    let store = local_repo_project_store(&ironclaw).await?;
    value(thinclaw_core::api::repo_projects::list_projects(&store).await)
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_repo_project_get(
    ironclaw: State<'_, ThinClawRuntimeState>,
    project_id: String,
) -> Result<serde_json::Value, String> {
    let store = local_repo_project_store(&ironclaw).await?;
    let project_id = parse_project_id(&project_id)?;
    value(thinclaw_core::api::repo_projects::get_project(&store, project_id).await)
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_repo_project_create(
    ironclaw: State<'_, ThinClawRuntimeState>,
    input: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let store = local_repo_project_store(&ironclaw).await?;
    let input: thinclaw_core::api::repo_projects::RepoProjectCreateInput =
        serde_json::from_value(input).map_err(|error| error.to_string())?;
    value(
        thinclaw_core::api::repo_projects::create_project(
            &store,
            thinclaw_core::api::repo_projects::default_user_id(),
            input,
        )
        .await,
    )
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_repo_project_start(
    ironclaw: State<'_, ThinClawRuntimeState>,
    project_id: String,
) -> Result<serde_json::Value, String> {
    let store = local_repo_project_store(&ironclaw).await?;
    let project_id = parse_project_id(&project_id)?;
    value(
        thinclaw_core::api::repo_projects::start_project(
            &store,
            thinclaw_core::api::repo_projects::default_user_id(),
            project_id,
        )
        .await,
    )
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_repo_project_plan(
    ironclaw: State<'_, ThinClawRuntimeState>,
    project_id: String,
) -> Result<serde_json::Value, String> {
    let store = local_repo_project_store(&ironclaw).await?;
    let project_id = parse_project_id(&project_id)?;
    value(
        thinclaw_core::api::repo_projects::plan_project(
            &store,
            thinclaw_core::api::repo_projects::default_user_id(),
            project_id,
        )
        .await,
    )
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_repo_project_pause(
    ironclaw: State<'_, ThinClawRuntimeState>,
    project_id: String,
) -> Result<serde_json::Value, String> {
    let store = local_repo_project_store(&ironclaw).await?;
    let project_id = parse_project_id(&project_id)?;
    value(
        thinclaw_core::api::repo_projects::pause_project(
            &store,
            thinclaw_core::api::repo_projects::default_user_id(),
            project_id,
        )
        .await,
    )
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_repo_project_resume(
    ironclaw: State<'_, ThinClawRuntimeState>,
    project_id: String,
) -> Result<serde_json::Value, String> {
    let store = local_repo_project_store(&ironclaw).await?;
    let project_id = parse_project_id(&project_id)?;
    value(
        thinclaw_core::api::repo_projects::resume_project(
            &store,
            thinclaw_core::api::repo_projects::default_user_id(),
            project_id,
        )
        .await,
    )
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_repo_project_cancel(
    ironclaw: State<'_, ThinClawRuntimeState>,
    project_id: String,
    run_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let _ = run_id;
    let store = local_repo_project_store(&ironclaw).await?;
    let project_id = parse_project_id(&project_id)?;
    value(
        thinclaw_core::api::repo_projects::cancel_project(
            &store,
            thinclaw_core::api::repo_projects::default_user_id(),
            project_id,
        )
        .await,
    )
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_repo_project_approve(
    ironclaw: State<'_, ThinClawRuntimeState>,
    project_id: String,
    input: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let store = local_repo_project_store(&ironclaw).await?;
    let project_id = parse_project_id(&project_id)?;
    let input: thinclaw_core::api::repo_projects::RepoApprovalInput =
        serde_json::from_value(input).map_err(|error| error.to_string())?;
    value(
        thinclaw_core::api::repo_projects::approve_project(
            &store,
            thinclaw_core::api::repo_projects::default_user_id(),
            project_id,
            input,
        )
        .await,
    )
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_repo_project_enqueue(
    ironclaw: State<'_, ThinClawRuntimeState>,
    project_id: String,
    item: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let store = local_repo_project_store(&ironclaw).await?;
    let project_id = parse_project_id(&project_id)?;
    let item: thinclaw_core::api::repo_projects::RepoBacklogEnqueueInput =
        serde_json::from_value(item).map_err(|error| error.to_string())?;
    value(
        thinclaw_core::api::repo_projects::enqueue_task(
            &store,
            thinclaw_core::api::repo_projects::default_user_id(),
            project_id,
            item,
        )
        .await,
    )
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_repo_project_events(
    ironclaw: State<'_, ThinClawRuntimeState>,
    project_id: String,
    limit: Option<i64>,
) -> Result<serde_json::Value, String> {
    let store = local_repo_project_store(&ironclaw).await?;
    let project_id = parse_project_id(&project_id)?;
    value(
        thinclaw_core::api::repo_projects::list_events(
            &store,
            project_id,
            limit.unwrap_or(100).clamp(1, 500),
        )
        .await,
    )
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_repo_project_merge_gates(
    ironclaw: State<'_, ThinClawRuntimeState>,
    project_id: String,
) -> Result<serde_json::Value, String> {
    let store = local_repo_project_store(&ironclaw).await?;
    let project_id = parse_project_id(&project_id)?;
    value(thinclaw_core::api::repo_projects::list_merge_gates(&store, project_id).await)
}
