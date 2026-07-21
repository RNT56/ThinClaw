use tauri::State;
use uuid::Uuid;

use crate::thinclaw::runtime_bridge::ThinClawRuntimeState;

async fn local_repo_project_store(
    ironclaw: &ThinClawRuntimeState,
) -> Result<std::sync::Arc<dyn thinclaw_core::db::Database>, crate::thinclaw::bridge::BridgeError> {
    if ironclaw.remote_proxy().await.is_some() {
        return Err(
            "Repository project commands are not available through remote gateway mode yet"
                .to_string()
                .into(),
        );
    }
    let agent = ironclaw.agent().await?;
    agent.store().cloned().ok_or_else(|| {
        crate::thinclaw::bridge::BridgeError::from("ThinClaw database is not available")
    })
}

type SharedSecrets = std::sync::Arc<dyn thinclaw_core::secrets::SecretsStore + Send + Sync>;

/// Database + secrets together — the connector needs both to mint authenticated
/// GitHub clients for repo discovery and credential storage.
async fn local_repo_project_context(
    ironclaw: &ThinClawRuntimeState,
) -> Result<
    (
        std::sync::Arc<dyn thinclaw_core::db::Database>,
        SharedSecrets,
    ),
    crate::thinclaw::bridge::BridgeError,
> {
    if ironclaw.remote_proxy().await.is_some() {
        return Err(
            "Repository project commands are not available through remote gateway mode yet"
                .to_string()
                .into(),
        );
    }
    let agent = ironclaw.agent().await?;
    let store = agent
        .store()
        .cloned()
        .ok_or_else(|| "ThinClaw database is not available".to_string())?;
    let secrets = agent.secrets_store().cloned().ok_or_else(|| {
        "Secrets store is not available; cannot manage GitHub credentials".to_string()
    })?;
    Ok((store, secrets))
}

fn parse_project_id(project_id: &str) -> Result<Uuid, crate::thinclaw::bridge::BridgeError> {
    Uuid::parse_str(project_id).map_err(|_| crate::thinclaw::bridge::BridgeError::InvalidInput {
        message: "Invalid repository project ID".to_string(),
        field: Some("project_id".to_string()),
    })
}

fn value<T: serde::Serialize>(
    result: Result<T, thinclaw_core::api::ApiError>,
) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
    let output =
        result.map_err(|error| crate::thinclaw::bridge::BridgeError::from(error.to_string()))?;
    serde_json::to_value(output)
        .map_err(|error| crate::thinclaw::bridge::BridgeError::from(error.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_repo_projects_list(
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
    let store = local_repo_project_store(&ironclaw).await?;
    value(thinclaw_core::api::repo_projects::list_projects(&store).await)
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_repo_project_get(
    ironclaw: State<'_, ThinClawRuntimeState>,
    project_id: String,
) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
    let store = local_repo_project_store(&ironclaw).await?;
    let project_id = parse_project_id(&project_id)?;
    value(thinclaw_core::api::repo_projects::get_project(&store, project_id).await)
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_repo_project_create(
    ironclaw: State<'_, ThinClawRuntimeState>,
    input: serde_json::Value,
) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
    let store = local_repo_project_store(&ironclaw).await?;
    let input: thinclaw_core::api::repo_projects::RepoProjectCreateInput =
        serde_json::from_value(input)
            .map_err(|error| crate::thinclaw::bridge::BridgeError::from(error.to_string()))?;
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
) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
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
) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
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
) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
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
) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
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
) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
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
) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
    let store = local_repo_project_store(&ironclaw).await?;
    let project_id = parse_project_id(&project_id)?;
    let input: thinclaw_core::api::repo_projects::RepoApprovalInput = serde_json::from_value(input)
        .map_err(|error| crate::thinclaw::bridge::BridgeError::from(error.to_string()))?;
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
) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
    let store = local_repo_project_store(&ironclaw).await?;
    let project_id = parse_project_id(&project_id)?;
    let item: thinclaw_core::api::repo_projects::RepoBacklogEnqueueInput =
        serde_json::from_value(item)
            .map_err(|error| crate::thinclaw::bridge::BridgeError::from(error.to_string()))?;
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
) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
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
) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
    let store = local_repo_project_store(&ironclaw).await?;
    let project_id = parse_project_id(&project_id)?;
    value(thinclaw_core::api::repo_projects::list_merge_gates(&store, project_id).await)
}

// ── Setup / credentials / GitHub connector ──────────────────────────────

fn user() -> &'static str {
    thinclaw_core::api::repo_projects::default_user_id()
}

/// Report supervisor readiness: feature flag, credential mode, GitHub App
/// config, derived install URL, and per-secret presence.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_repo_projects_readiness(
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
    let store = local_repo_project_store(&ironclaw).await?;
    let secrets = ironclaw.agent().await?.secrets_store().cloned();
    value(
        thinclaw_core::api::repo_projects::repo_projects_readiness(
            &store,
            secrets.as_ref(),
            user(),
        )
        .await,
    )
}

/// Enable + configure the supervisor (feature flag, GitHub App config, policy).
/// Secret VALUES are never written here — only the names of secrets.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_repo_projects_setup(
    ironclaw: State<'_, ThinClawRuntimeState>,
    input: serde_json::Value,
) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
    let store = local_repo_project_store(&ironclaw).await?;
    let secrets = ironclaw.agent().await?.secrets_store().cloned();
    let input: thinclaw_core::api::repo_projects::RepoProjectsConfigureInput =
        serde_json::from_value(input)
            .map_err(|error| crate::thinclaw::bridge::BridgeError::from(error.to_string()))?;
    value(
        thinclaw_core::api::repo_projects::configure_supervisor(
            &store,
            secrets.as_ref(),
            user(),
            input,
        )
        .await,
    )
}

/// Securely store a GitHub credential (PAT or GitHub App PEM key) in the
/// encrypted secrets store. The value never touches settings, events, or logs.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_repo_projects_set_credential(
    ironclaw: State<'_, ThinClawRuntimeState>,
    name: String,
    value_secret: String,
) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
    let (_store, secrets) = local_repo_project_context(&ironclaw).await?;
    value(
        thinclaw_core::api::repo_projects::store_repo_credential(
            &secrets,
            user(),
            name,
            value_secret,
        )
        .await,
    )
}

/// List the repositories the connected GitHub credential can act on — the
/// connector repo picker. Each repo is marked with whether it is already
/// under supervision.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_repo_projects_connectable_repos(
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
    let (store, secrets) = local_repo_project_context(&ironclaw).await?;
    value(thinclaw_core::api::repo_projects::list_connectable_repos(&store, &secrets, user()).await)
}

/// Bring selected repositories under supervision (a project per repo). Provide
/// `repos: ["owner/repo", …]` or `all: true`.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_repo_projects_connect(
    ironclaw: State<'_, ThinClawRuntimeState>,
    input: serde_json::Value,
) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
    let (store, secrets) = local_repo_project_context(&ironclaw).await?;
    let input: thinclaw_core::api::repo_projects::RepoConnectInput = serde_json::from_value(input)
        .map_err(|error| crate::thinclaw::bridge::BridgeError::from(error.to_string()))?;
    value(thinclaw_core::api::repo_projects::connect_repos(&store, &secrets, user(), input).await)
}

/// Enroll an additional repository into an existing project.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_repo_project_enroll(
    ironclaw: State<'_, ThinClawRuntimeState>,
    project_id: String,
    input: serde_json::Value,
) -> Result<serde_json::Value, crate::thinclaw::bridge::BridgeError> {
    let store = local_repo_project_store(&ironclaw).await?;
    let project_id = parse_project_id(&project_id)?;
    let input: thinclaw_core::api::repo_projects::RepoEnrollInput =
        serde_json::from_value(input)
            .map_err(|error| crate::thinclaw::bridge::BridgeError::from(error.to_string()))?;
    value(thinclaw_core::api::repo_projects::enroll_repo(&store, user(), project_id, input).await)
}
