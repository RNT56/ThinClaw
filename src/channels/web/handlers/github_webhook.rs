use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use base64::Engine as _;
use serde::Serialize;
use uuid::Uuid;

use crate::channels::web::identity_helpers::GatewayRequestIdentity;
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::SseEvent;
use crate::repo_projects::github::{
    GitHubDeliveryDeduper, GitHubWebhookEnvelope, GitHubWebhookError,
    parse_github_webhook_envelope, verify_github_webhook_signature,
};
use crate::repo_projects::supervisor::RepoSupervisorWakeReason;
use thinclaw_repo_projects::RepoWebhookDelivery;

const GITHUB_EVENT_HEADER: &str = "x-github-event";
const GITHUB_DELIVERY_HEADER: &str = "x-github-delivery";
const GITHUB_SIGNATURE_HEADER: &str = "x-hub-signature-256";
const REPO_PROJECT_GITHUB_WEBHOOK_SECRET_ENV: &str = "THINCLAW_REPO_PROJECTS_GITHUB_WEBHOOK_SECRET";

static GITHUB_WEBHOOK_DELIVERY_DEDUPER: OnceLock<GitHubDeliveryDeduper> = OnceLock::new();

#[derive(Debug, Serialize)]
pub(crate) struct GitHubRepoProjectsWebhookResponse {
    pub ok: bool,
    pub event: String,
    pub delivery_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installation_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_full_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_project_id: Option<String>,
    pub duplicate: bool,
    pub supervisor_woken: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct GitHubRepoProjectsWebhookReplayResponse {
    pub ok: bool,
    pub replayed: bool,
    pub event: String,
    pub delivery_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installation_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_full_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_project_id: Option<String>,
    pub supervisor_woken: bool,
    pub raw_payload_replayed: bool,
}

pub(crate) async fn github_repo_projects_webhook_handler(
    State(state): State<Arc<GatewayState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<GitHubRepoProjectsWebhookResponse>, (StatusCode, String)> {
    let secret = resolve_github_webhook_secret(&state).await?;
    let signature_header = header_str(&headers, GITHUB_SIGNATURE_HEADER);
    verify_github_webhook_signature(&secret, &body, signature_header)
        .map_err(github_webhook_error_response)?;

    let event = header_str(&headers, GITHUB_EVENT_HEADER)
        .filter(|event| !event.trim().is_empty())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "missing X-GitHub-Event header".to_string(),
            )
        })?;
    let delivery_id = header_str(&headers, GITHUB_DELIVERY_HEADER)
        .filter(|delivery_id| !delivery_id.trim().is_empty())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "missing X-GitHub-Delivery header".to_string(),
            )
        })?;

    if let Err(error) = github_webhook_delivery_deduper().accept(delivery_id) {
        if error == GitHubWebhookError::DuplicateDelivery {
            tracing::debug!(
                delivery_id,
                event,
                "duplicate GitHub webhook delivery ignored"
            );
            return Ok(Json(GitHubRepoProjectsWebhookResponse {
                ok: true,
                event: event.to_string(),
                delivery_id: delivery_id.to_string(),
                action: None,
                installation_id: None,
                repository_full_name: None,
                matched_project_id: None,
                duplicate: true,
                supervisor_woken: false,
            }));
        }
        return Err(github_webhook_error_response(error));
    }

    let envelope = parse_github_webhook_envelope(event, Some(delivery_id), &body)
        .map_err(github_webhook_error_response)?;

    // Durable, restart-surviving idempotency + audit. The in-memory deduper above
    // is a fast pre-check; this catches redeliveries that span a restart.
    if !record_webhook_delivery(&state, &envelope, &body, signature_header).await {
        tracing::debug!(
            delivery_id = %envelope.delivery_id,
            event = %envelope.event,
            "duplicate GitHub webhook delivery (durable) ignored"
        );
        return Ok(Json(GitHubRepoProjectsWebhookResponse {
            ok: true,
            event: envelope.event,
            delivery_id: envelope.delivery_id,
            action: envelope.action,
            installation_id: envelope.installation_id,
            repository_full_name: envelope.repository_full_name,
            matched_project_id: None,
            duplicate: true,
            supervisor_woken: false,
        }));
    }

    let matched_project_id = find_project_id_for_repo(
        &state,
        envelope.repository_full_name.as_deref(),
        envelope.installation_id,
    )
    .await?;
    let supervisor_woken = wake_repo_project_supervisor(&state, &envelope).await?;

    if let Some(project_id) = matched_project_id {
        broadcast_github_webhook(&state, &envelope, project_id);
    }

    tracing::info!(
        event = envelope.event,
        delivery_id = envelope.delivery_id,
        action = ?envelope.action,
        repository = ?envelope.repository_full_name,
        matched_project_id = ?matched_project_id,
        supervisor_woken,
        "GitHub repo project webhook accepted",
    );

    Ok(Json(GitHubRepoProjectsWebhookResponse {
        ok: true,
        event: envelope.event,
        delivery_id: envelope.delivery_id,
        action: envelope.action,
        installation_id: envelope.installation_id,
        repository_full_name: envelope.repository_full_name,
        matched_project_id: matched_project_id.map(|id| id.to_string()),
        duplicate: false,
        supervisor_woken,
    }))
}

pub(crate) async fn github_repo_projects_webhook_replay_handler(
    State(state): State<Arc<GatewayState>>,
    _request_identity: GatewayRequestIdentity,
    Path(delivery_id): Path<String>,
) -> Result<Json<GitHubRepoProjectsWebhookReplayResponse>, (StatusCode, String)> {
    replay_stored_github_webhook_delivery(&state, &delivery_id)
        .await
        .map(Json)
}

async fn resolve_github_webhook_secret(
    state: &GatewayState,
) -> Result<String, (StatusCode, String)> {
    if let Ok(secret) = std::env::var(REPO_PROJECT_GITHUB_WEBHOOK_SECRET_ENV)
        && !secret.trim().is_empty()
    {
        return Ok(secret);
    }

    let Some(store) = state.store.as_ref() else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "GitHub webhook secret is not configured and settings are unavailable".to_string(),
        ));
    };
    let Some(secrets_store) = state.secrets_store.as_ref() else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "GitHub webhook secret is not configured and secrets store is unavailable".to_string(),
        ));
    };

    let settings_map = store
        .get_all_settings(&state.user_id)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let settings = crate::settings::Settings::from_db_map(&settings_map);
    let Some(secret_name) = settings
        .repo_projects
        .github_app
        .webhook_secret_secret
        .as_deref()
        .filter(|name| !name.trim().is_empty())
    else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "GitHub webhook secret is not configured".to_string(),
        ));
    };

    let secret = secrets_store
        .get_for_injection(
            &state.user_id,
            secret_name,
            crate::secrets::SecretAccessContext::new(
                "repo_projects.github_webhook",
                "verify_webhook_signature",
            )
            .target("api.github.com", "/app/hook/deliveries"),
        )
        .await
        .map_err(|error| (StatusCode::SERVICE_UNAVAILABLE, error.to_string()))?;
    if secret.expose().trim().is_empty() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "GitHub webhook secret is empty".to_string(),
        ));
    }
    Ok(secret.expose().to_string())
}

async fn replay_stored_github_webhook_delivery(
    state: &GatewayState,
    delivery_id: &str,
) -> Result<GitHubRepoProjectsWebhookReplayResponse, (StatusCode, String)> {
    let delivery_id = delivery_id.trim();
    if delivery_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "GitHub webhook delivery id is required".to_string(),
        ));
    }
    let Some(store) = state.store.as_ref() else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Repository project database is not available".to_string(),
        ));
    };
    let Some(delivery) = store
        .get_repo_webhook_delivery(delivery_id)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
    else {
        return Err((
            StatusCode::NOT_FOUND,
            format!("GitHub webhook delivery '{delivery_id}' was not found"),
        ));
    };

    let (envelope, raw_payload_replayed) = replay_envelope_from_delivery(&delivery)?;

    let matched_project_id = find_project_id_for_repo(
        state,
        envelope.repository_full_name.as_deref(),
        envelope.installation_id,
    )
    .await?;
    let supervisor_woken = wake_repo_project_supervisor(state, &envelope).await?;

    if let Some(project_id) = matched_project_id {
        broadcast_github_webhook_replay(state, &envelope, project_id);
    }

    tracing::info!(
        event = envelope.event,
        delivery_id = envelope.delivery_id,
        repository = ?envelope.repository_full_name,
        matched_project_id = ?matched_project_id,
        supervisor_woken,
        "GitHub repo project webhook delivery replayed",
    );

    Ok(GitHubRepoProjectsWebhookReplayResponse {
        ok: true,
        replayed: true,
        event: envelope.event,
        delivery_id: envelope.delivery_id,
        action: envelope.action,
        installation_id: envelope.installation_id,
        repository_full_name: envelope.repository_full_name,
        matched_project_id: matched_project_id.map(|id| id.to_string()),
        supervisor_woken,
        raw_payload_replayed,
    })
}

async fn find_project_id_for_repo(
    state: &GatewayState,
    repository_full_name: Option<&str>,
    installation_id: Option<i64>,
) -> Result<Option<Uuid>, (StatusCode, String)> {
    let Some(repository_full_name) = repository_full_name else {
        return Ok(None);
    };
    let Some((owner, repo_name)) = repository_full_name.split_once('/') else {
        return Ok(None);
    };
    let Some(store) = state.store.as_ref() else {
        return Ok(None);
    };

    match_and_backfill_repo(store, owner, repo_name, installation_id)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))
}

/// Find the project enrolling `owner/repo_name` and, when the webhook envelope
/// carries an `installation_id` that differs from the stored one, backfill it
/// onto the matched repo row (best-effort — a failed upsert is logged, not
/// surfaced, so the webhook never fails on a stored-state write).
async fn match_and_backfill_repo(
    store: &Arc<dyn crate::db::Database>,
    owner: &str,
    repo_name: &str,
    installation_id: Option<i64>,
) -> Result<Option<Uuid>, String> {
    let projects = store
        .list_repo_projects()
        .await
        .map_err(|error| error.to_string())?;
    for project in projects {
        let repos = store
            .list_repo_project_repos(project.id)
            .await
            .map_err(|error| error.to_string())?;
        if let Some(repo) = repos.iter().find(|repo| {
            repo.owner.eq_ignore_ascii_case(owner) && repo.repo.eq_ignore_ascii_case(repo_name)
        }) {
            // Backfill the per-repo installation id discovered from the webhook
            // envelope so multi-installation App auth can pin the correct
            // installation instead of falling back to the global default.
            if let Some(installation_id) = installation_id
                && repo.installation_id != Some(installation_id)
            {
                let mut updated = repo.clone();
                updated.installation_id = Some(installation_id);
                updated.updated_at = chrono::Utc::now();
                if let Err(error) = store.upsert_repo_project_repo(&updated).await {
                    tracing::warn!(
                        project_id = %project.id,
                        repo_id = %repo.id,
                        error = %error,
                        "failed to persist webhook-discovered installation_id"
                    );
                }
            }
            return Ok(Some(project.id));
        }
    }
    Ok(None)
}

/// Persist the delivery durably. Returns `true` when the delivery is new (or no
/// store is available), `false` when it was already recorded (a duplicate). A
/// transient DB error fails open (treats the delivery as new) so a real event is
/// never dropped on infrastructure trouble.
async fn record_webhook_delivery(
    state: &GatewayState,
    envelope: &GitHubWebhookEnvelope,
    raw_body: &[u8],
    signature_header: Option<&str>,
) -> bool {
    let Some(store) = state.store.as_ref() else {
        return true;
    };
    let delivery = RepoWebhookDelivery {
        delivery_id: envelope.delivery_id.clone(),
        event: envelope.event.clone(),
        action: envelope.action.clone(),
        repository_full_name: envelope.repository_full_name.clone(),
        installation_id: envelope.installation_id,
        raw_payload_base64: Some(base64::engine::general_purpose::STANDARD.encode(raw_body)),
        signature_header: signature_header.map(str::to_string),
        received_at: chrono::Utc::now(),
    };
    match store.record_repo_webhook_delivery(&delivery).await {
        Ok(is_new) => is_new,
        Err(error) => {
            tracing::warn!(error = %error, "failed to durably record GitHub webhook delivery");
            true
        }
    }
}

fn replay_envelope_from_delivery(
    delivery: &RepoWebhookDelivery,
) -> Result<(GitHubWebhookEnvelope, bool), (StatusCode, String)> {
    let Some(raw_payload_base64) = delivery.raw_payload_base64.as_deref() else {
        return Ok((legacy_replay_envelope_from_delivery(delivery), false));
    };
    let raw_payload = base64::engine::general_purpose::STANDARD
        .decode(raw_payload_base64.as_bytes())
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!(
                    "GitHub webhook delivery '{}' has invalid stored raw payload: {error}",
                    delivery.delivery_id
                ),
            )
        })?;
    parse_github_webhook_envelope(&delivery.event, Some(&delivery.delivery_id), &raw_payload)
        .map(|envelope| (envelope, true))
        .map_err(github_webhook_error_response)
}

fn legacy_replay_envelope_from_delivery(delivery: &RepoWebhookDelivery) -> GitHubWebhookEnvelope {
    GitHubWebhookEnvelope {
        event: delivery.event.clone(),
        delivery_id: delivery.delivery_id.clone(),
        installation_id: delivery.installation_id,
        repository_full_name: delivery.repository_full_name.clone(),
        action: delivery.action.clone(),
        payload: serde_json::json!({
            "replayed_from_delivery_record": true,
            "delivery_id": delivery.delivery_id,
            "received_at": delivery.received_at.to_rfc3339(),
        }),
    }
}

async fn wake_repo_project_supervisor(
    state: &GatewayState,
    envelope: &GitHubWebhookEnvelope,
) -> Result<bool, (StatusCode, String)> {
    let supervisor = state.repo_project_supervisor.read().await.clone();
    let Some(supervisor) = supervisor else {
        return Ok(false);
    };
    supervisor
        .wake(
            None,
            RepoSupervisorWakeReason::GitHubWebhook {
                delivery_id: envelope.delivery_id.clone(),
            },
        )
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
    Ok(true)
}

fn broadcast_github_webhook(
    state: &GatewayState,
    envelope: &GitHubWebhookEnvelope,
    project_id: Uuid,
) {
    let event_type = format!("github.{}", envelope.event);
    let message = match (
        envelope.repository_full_name.as_deref(),
        envelope.action.as_deref(),
    ) {
        (Some(repository), Some(action)) => {
            format!("GitHub {repository} {event_type} webhook: {action}")
        }
        (Some(repository), None) => format!("GitHub {repository} {event_type} webhook"),
        (None, Some(action)) => format!("GitHub {event_type} webhook: {action}"),
        (None, None) => format!("GitHub {event_type} webhook"),
    };
    state.sse.broadcast(SseEvent::RepoProjectEvent {
        project_id: project_id.to_string(),
        event_type,
        message,
    });
}

fn broadcast_github_webhook_replay(
    state: &GatewayState,
    envelope: &GitHubWebhookEnvelope,
    project_id: Uuid,
) {
    let event_type = format!("github.{}.replay", envelope.event);
    let message = match (
        envelope.repository_full_name.as_deref(),
        envelope.action.as_deref(),
    ) {
        (Some(repository), Some(action)) => {
            format!(
                "Replayed GitHub {repository} {} webhook: {action}",
                envelope.event
            )
        }
        (Some(repository), None) => {
            format!("Replayed GitHub {repository} {} webhook", envelope.event)
        }
        (None, Some(action)) => format!("Replayed GitHub {} webhook: {action}", envelope.event),
        (None, None) => format!("Replayed GitHub {} webhook", envelope.event),
    };
    state.sse.broadcast(SseEvent::RepoProjectEvent {
        project_id: project_id.to_string(),
        event_type,
        message,
    });
}

fn github_webhook_delivery_deduper() -> &'static GitHubDeliveryDeduper {
    GITHUB_WEBHOOK_DELIVERY_DEDUPER
        .get_or_init(|| GitHubDeliveryDeduper::new(Duration::from_secs(24 * 60 * 60)))
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn github_webhook_error_response(error: GitHubWebhookError) -> (StatusCode, String) {
    let status = match error {
        GitHubWebhookError::MissingSignature | GitHubWebhookError::SignatureMismatch => {
            StatusCode::UNAUTHORIZED
        }
        GitHubWebhookError::InvalidSignatureFormat
        | GitHubWebhookError::InvalidSecret
        | GitHubWebhookError::MissingDeliveryId => StatusCode::BAD_REQUEST,
        GitHubWebhookError::DuplicateDelivery => StatusCode::CONFLICT,
    };
    (status, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_webhook_errors_map_to_http_statuses() {
        assert_eq!(
            github_webhook_error_response(GitHubWebhookError::MissingSignature).0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            github_webhook_error_response(GitHubWebhookError::SignatureMismatch).0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            github_webhook_error_response(GitHubWebhookError::MissingDeliveryId).0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            github_webhook_error_response(GitHubWebhookError::DuplicateDelivery).0,
            StatusCode::CONFLICT
        );
    }
}

#[cfg(all(test, feature = "libsql"))]
mod backfill_tests {
    use super::*;
    use crate::channels::web::sse::SseManager;
    use crate::db::Database;
    use crate::repo_projects::supervisor::{
        ProjectSupervisor, RepoSupervisorDecision, RepoSupervisorStore, RepoSupervisorWakeReason,
    };
    use crate::testing::test_db;
    use chrono::Utc;
    use std::time::Duration;
    use thinclaw_repo_projects::{
        GitHubAuthMode, ProjectPolicy, RepoProject, RepoProjectRepo, RepoProjectState,
        RepoWebhookDelivery,
    };

    #[derive(Default)]
    struct NoopSupervisorStore;

    #[async_trait::async_trait]
    impl RepoSupervisorStore for NoopSupervisorStore {
        async fn reconcile_project(
            &self,
            _project_id: Option<Uuid>,
            _reason: RepoSupervisorWakeReason,
        ) -> Result<Vec<RepoSupervisorDecision>, String> {
            Ok(vec![RepoSupervisorDecision::Idle])
        }
    }

    fn gateway_state(
        store: Arc<dyn Database>,
        supervisor: Option<ProjectSupervisor>,
    ) -> GatewayState {
        GatewayState {
            msg_tx: tokio::sync::RwLock::new(None),
            sse: SseManager::new(),
            workspace: None,
            session_manager: None,
            log_broadcaster: None,
            log_level_handle: None,
            extension_manager: None,
            tool_registry: None,
            store: Some(store),
            job_manager: None,
            prompt_queue: None,
            context_manager: None,
            scheduler: tokio::sync::RwLock::new(None),
            user_id: "default".to_string(),
            actor_id: "default".to_string(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: None,
            llm_provider: None,
            llm_runtime: None,
            skill_registry: None,
            skill_catalog: None,
            skill_remote_hub: None,
            skill_quarantine: None,
            chat_rate_limiter: crate::channels::web::rate_limiter::RateLimiter::new(30, 60),
            pair_complete_rate_limiter: crate::channels::web::rate_limiter::RateLimiter::new(
                10, 300,
            ),
            device_registry: crate::channels::web::server::test_device_registry(),
            pending_approvals: Arc::new(
                crate::channels::web::server::PendingApprovalsStore::in_memory(),
            ),
            registry_entries: Vec::new(),
            cost_guard: None,
            cost_tracker: None,
            metrics_registry: None,
            response_cache: None,
            routine_engine: None,
            repo_project_supervisor: Arc::new(tokio::sync::RwLock::new(supervisor)),
            startup_time: std::time::Instant::now(),
            restart_requested: std::sync::atomic::AtomicBool::new(false),
            secrets_store: None,
            channel_manager: None,
            hooks: None,
        }
    }

    fn project() -> RepoProject {
        let now = Utc::now();
        RepoProject {
            id: Uuid::new_v4(),
            slug: "proj".to_string(),
            name: "Proj".to_string(),
            state: RepoProjectState::Active,
            policy: ProjectPolicy::default(),
            description: None,
            current_run_id: None,
            created_at: now,
            updated_at: now,
            started_at: Some(now),
            completed_at: None,
        }
    }

    fn repo(project_id: Uuid, installation_id: Option<i64>) -> RepoProjectRepo {
        let now = Utc::now();
        RepoProjectRepo {
            id: Uuid::new_v4(),
            project_id,
            owner: "acme".to_string(),
            repo: "widgets".to_string(),
            github_repo_id: None,
            installation_id,
            default_branch: "main".to_string(),
            base_branch: Some("main".to_string()),
            enrolled: true,
            local_path: None,
            auth_mode: GitHubAuthMode::GitHubApp,
            metadata: serde_json::json!({}),
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn webhook_backfills_installation_id_onto_matched_repo() {
        let (db, _guard) = test_db().await;
        let project = project();
        let repo = repo(project.id, None);
        let repo_id = repo.id;
        db.create_repo_project(&project).await.unwrap();
        db.upsert_repo_project_repo(&repo).await.unwrap();

        let matched = match_and_backfill_repo(&db, "acme", "widgets", Some(4242))
            .await
            .unwrap();
        assert_eq!(matched, Some(project.id));

        let stored = db
            .list_repo_project_repos(project.id)
            .await
            .unwrap()
            .into_iter()
            .find(|repo| repo.id == repo_id)
            .unwrap();
        assert_eq!(
            stored.installation_id,
            Some(4242),
            "installation id is backfilled from the webhook envelope"
        );

        // Re-delivery with the same id is idempotent (no spurious change).
        let matched_again = match_and_backfill_repo(&db, "ACME", "Widgets", Some(4242))
            .await
            .unwrap();
        assert_eq!(matched_again, Some(project.id), "case-insensitive match");
        let stored_again = db
            .list_repo_project_repos(project.id)
            .await
            .unwrap()
            .into_iter()
            .find(|repo| repo.id == repo_id)
            .unwrap();
        assert_eq!(stored_again.installation_id, Some(4242));
    }

    #[tokio::test]
    async fn record_webhook_delivery_persists_raw_payload_for_exact_replay() {
        let (db, _guard) = test_db().await;
        let state = gateway_state(Arc::clone(&db), None);
        let raw_payload = br#"{"action":"opened","repository":{"full_name":"acme/widgets"},"installation":{"id":4242}}"#;
        let envelope = parse_github_webhook_envelope(
            "pull_request",
            Some("delivery-raw-store-1"),
            raw_payload,
        )
        .unwrap();

        assert!(
            record_webhook_delivery(
                &state,
                &envelope,
                raw_payload,
                Some("sha256=stored-signature"),
            )
            .await
        );

        let stored = db
            .get_repo_webhook_delivery("delivery-raw-store-1")
            .await
            .unwrap()
            .expect("delivery should be stored");
        let encoded = base64::engine::general_purpose::STANDARD.encode(raw_payload);
        assert_eq!(stored.raw_payload_base64.as_deref(), Some(encoded.as_str()));
        assert_eq!(
            stored.signature_header.as_deref(),
            Some("sha256=stored-signature")
        );
    }

    #[tokio::test]
    async fn unmatched_repo_returns_none() {
        let (db, _guard) = test_db().await;
        let project = project();
        db.create_repo_project(&project).await.unwrap();
        db.upsert_repo_project_repo(&repo(project.id, None))
            .await
            .unwrap();

        let matched = match_and_backfill_repo(&db, "other", "thing", Some(1))
            .await
            .unwrap();
        assert_eq!(matched, None);
    }

    #[tokio::test]
    async fn replay_stored_delivery_matches_repo_and_wakes_supervisor() {
        let (db, _guard) = test_db().await;
        let project = project();
        db.create_repo_project(&project).await.unwrap();
        db.upsert_repo_project_repo(&repo(project.id, Some(4242)))
            .await
            .unwrap();
        let delivery = RepoWebhookDelivery {
            delivery_id: "delivery-replay-1".to_string(),
            event: "pull_request".to_string(),
            action: Some("synchronize".to_string()),
            repository_full_name: Some("acme/widgets".to_string()),
            installation_id: Some(4242),
            raw_payload_base64: None,
            signature_header: None,
            received_at: Utc::now(),
        };
        db.record_repo_webhook_delivery(&delivery).await.unwrap();

        let (supervisor, mut wake_rx) = ProjectSupervisor::new(Arc::new(NoopSupervisorStore), 4);
        let state = gateway_state(Arc::clone(&db), Some(supervisor));

        let response = replay_stored_github_webhook_delivery(&state, "delivery-replay-1")
            .await
            .unwrap();

        assert!(response.ok);
        assert!(response.replayed);
        assert_eq!(response.delivery_id, "delivery-replay-1");
        assert_eq!(response.event, "pull_request");
        assert_eq!(response.action.as_deref(), Some("synchronize"));
        assert_eq!(response.matched_project_id, Some(project.id.to_string()));
        assert!(response.supervisor_woken);
        assert!(!response.raw_payload_replayed);

        let wake = tokio::time::timeout(Duration::from_secs(2), wake_rx.recv())
            .await
            .expect("supervisor wake should arrive")
            .expect("supervisor wake channel should stay open");
        assert_eq!(wake.project_id, None);
        assert_eq!(
            wake.reason,
            RepoSupervisorWakeReason::GitHubWebhook {
                delivery_id: "delivery-replay-1".to_string()
            }
        );
    }

    #[tokio::test]
    async fn replay_stored_delivery_prefers_raw_payload_over_derived_metadata() {
        let (db, _guard) = test_db().await;
        let project = project();
        db.create_repo_project(&project).await.unwrap();
        db.upsert_repo_project_repo(&repo(project.id, Some(4242)))
            .await
            .unwrap();
        let raw_payload = serde_json::json!({
            "action": "closed",
            "repository": {
                "full_name": "acme/widgets"
            },
            "installation": {
                "id": 4242
            }
        })
        .to_string();
        let delivery = RepoWebhookDelivery {
            delivery_id: "delivery-replay-raw-1".to_string(),
            event: "pull_request".to_string(),
            action: Some("stale-derived-action".to_string()),
            repository_full_name: Some("wrong/repo".to_string()),
            installation_id: Some(1111),
            raw_payload_base64: Some(base64::engine::general_purpose::STANDARD.encode(raw_payload)),
            signature_header: Some("sha256=stored-signature".to_string()),
            received_at: Utc::now(),
        };
        db.record_repo_webhook_delivery(&delivery).await.unwrap();

        let (supervisor, _wake_rx) = ProjectSupervisor::new(Arc::new(NoopSupervisorStore), 4);
        let state = gateway_state(Arc::clone(&db), Some(supervisor));

        let response = replay_stored_github_webhook_delivery(&state, "delivery-replay-raw-1")
            .await
            .unwrap();

        assert!(response.ok);
        assert!(response.raw_payload_replayed);
        assert_eq!(response.action.as_deref(), Some("closed"));
        assert_eq!(
            response.repository_full_name.as_deref(),
            Some("acme/widgets")
        );
        assert_eq!(response.installation_id, Some(4242));
        assert_eq!(response.matched_project_id, Some(project.id.to_string()));
    }

    #[tokio::test]
    async fn replay_missing_delivery_returns_not_found() {
        let (db, _guard) = test_db().await;
        let state = gateway_state(db, None);

        let error = replay_stored_github_webhook_delivery(&state, "missing-delivery")
            .await
            .unwrap_err();

        assert_eq!(error.0, StatusCode::NOT_FOUND);
        assert!(error.1.contains("missing-delivery"));
    }
}
