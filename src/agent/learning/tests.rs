use super::*;
use crate::agent::session::{Session, Thread, Turn};
use crate::channels::IncomingMessage;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[cfg(feature = "libsql")]
use tokio::sync::mpsc;

#[cfg(feature = "libsql")]
use crate::agent::routine::{Routine, RoutineAction, RoutineGuardrails, Trigger};
#[cfg(feature = "libsql")]
use crate::agent::routine_engine::RoutineEngine;
#[cfg(feature = "libsql")]
use crate::config::RoutineConfig;
use crate::identity::{ConversationKind, ResolvedIdentity, scope_id_from_key};
#[cfg(feature = "libsql")]
use crate::testing::StubLlm;
#[cfg(feature = "libsql")]
use crate::workspace::Workspace;

#[cfg(feature = "libsql")]
#[derive(Debug)]
struct TestMemoryProvider {
    name: &'static str,
    strict_scoping: bool,
    hits: Vec<ProviderMemoryHit>,
    recalls: Arc<Mutex<Vec<(String, String, usize)>>>,
    exports: Arc<Mutex<Vec<(String, serde_json::Value)>>>,
    health_status: ProviderHealthStatus,
}

#[cfg(feature = "libsql")]
#[async_trait]
impl MemoryProvider for TestMemoryProvider {
    fn name(&self) -> &'static str {
        self.name
    }

    fn supports_strict_subject_scoping(&self) -> bool {
        self.strict_scoping
    }

    async fn health(&self, _settings: &LearningSettings) -> ProviderHealthStatus {
        self.health_status.clone()
    }

    async fn recall(
        &self,
        _settings: &LearningSettings,
        user_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ProviderMemoryHit>, String> {
        self.recalls
            .lock()
            .expect("recall log mutex poisoned")
            .push((user_id.to_string(), query.to_string(), limit));
        Ok(self.hits.iter().take(limit).cloned().collect())
    }

    async fn export_turn(
        &self,
        _settings: &LearningSettings,
        user_id: &str,
        payload: &serde_json::Value,
    ) -> Result<(), String> {
        self.exports
            .lock()
            .expect("export log mutex poisoned")
            .push((user_id.to_string(), payload.clone()));
        Ok(())
    }
}

#[cfg(feature = "libsql")]
fn provider_access(user_id: &str, actor_id: &str) -> thinclaw_identity::AccessContext {
    thinclaw_identity::AccessContext {
        principal_id: user_id.to_string(),
        actor_id: actor_id.to_string(),
        conversation_scope_id: crate::identity::direct_scope_id(user_id, actor_id),
        conversation_kind: ConversationKind::Direct,
        channel: "test".to_string(),
    }
}

#[cfg(feature = "libsql")]
fn direct_learning_identity(user_id: &str, actor_id: &str) -> ResolvedIdentity {
    ResolvedIdentity {
        principal_id: user_id.to_string(),
        actor_id: actor_id.to_string(),
        conversation_scope_id: crate::identity::direct_scope_id(user_id, actor_id),
        conversation_kind: ConversationKind::Direct,
        raw_sender_id: actor_id.to_string(),
        stable_external_conversation_key: String::new(),
    }
}

#[cfg(feature = "libsql")]
fn provider_status(
    name: &str,
    readiness: ProviderReadiness,
    healthy: bool,
    error: Option<&str>,
) -> ProviderHealthStatus {
    ProviderHealthStatus {
        provider: name.to_string(),
        active: false,
        enabled: readiness != ProviderReadiness::Disabled,
        healthy,
        readiness,
        latency_ms: Some(1),
        error: error.map(str::to_string),
        capabilities: Vec::new(),
        metadata: serde_json::json!({}),
    }
}

#[derive(Debug, Clone)]
struct RecordedProviderRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Option<serde_json::Value>,
}

#[derive(Clone)]
struct MockProviderState {
    requests: Arc<Mutex<Vec<RecordedProviderRequest>>>,
}

struct MockProviderServer {
    base_url: String,
    requests: Arc<Mutex<Vec<RecordedProviderRequest>>>,
}

impl MockProviderServer {
    fn requests(&self) -> Vec<RecordedProviderRequest> {
        self.requests
            .lock()
            .expect("mock provider request log")
            .clone()
    }
}

async fn mock_provider_handler(
    axum::extract::State(state): axum::extract::State<MockProviderState>,
    method: axum::http::Method,
    uri: axum::http::Uri,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let path = uri.path().to_string();
    let body_json = if body.is_empty() {
        None
    } else {
        serde_json::from_slice::<serde_json::Value>(&body).ok()
    };
    let recorded_headers = headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_ascii_lowercase(), value.to_string()))
        })
        .collect::<HashMap<_, _>>();
    state
        .requests
        .lock()
        .expect("mock provider request log")
        .push(RecordedProviderRequest {
            method: method.to_string(),
            path: path.clone(),
            headers: recorded_headers,
            body: body_json,
        });

    if path.contains("/fail") {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({"error": "mock failure"})),
        )
            .into_response();
    }

    let value = if path == "/" || path == "/health" || path.contains("heartbeat") {
        serde_json::json!({"ok": true})
    } else if path == "/embed" {
        serde_json::json!({"embedding": [0.1, 0.2, 0.3]})
    } else if path.contains("/chroma/") && path.ends_with("/query") {
        serde_json::json!({
            "ids": [["chroma-1"]],
            "documents": [["uses chroma for vector recall"]],
            "distances": [[0.12]],
            "metadatas": [[{"source": "mock"}]]
        })
    } else if path.contains("/qdrant/") && path.ends_with("/query") {
        serde_json::json!({
            "result": {
                "points": [{
                    "id": "qdrant-1",
                    "score": 0.91,
                    "payload": {"text": "uses qdrant for durable recall"}
                }]
            }
        })
    } else if method == axum::http::Method::GET && path.contains("/letta/") {
        serde_json::json!({
            "data": [{
                "id": "letta-1",
                "score": 0.74,
                "text": "letta remembers archival facts"
            }]
        })
    } else if path.contains("/custom/recall") {
        serde_json::json!({
            "results": [{
                "id": "custom-1",
                "summary": "custom http recalls preferences",
                "score": 0.88
            }]
        })
    } else if path.contains("/mem0/search") {
        serde_json::json!({
            "results": [{
                "id": "mem0-1",
                "memory": "mem0 recalls preferences",
                "score": 0.82
            }]
        })
    } else if path.contains("/openmemory/search") {
        serde_json::json!({
            "memories": [{
                "id": "openmemory-1",
                "content": "openmemory recalls local facts",
                "score": 0.79
            }]
        })
    } else {
        serde_json::json!({"ok": true})
    };

    (axum::http::StatusCode::OK, axum::Json(value)).into_response()
}

async fn spawn_mock_provider_server() -> MockProviderServer {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let state = MockProviderState {
        requests: Arc::clone(&requests),
    };
    let app = axum::Router::new()
        .fallback(axum::routing::any(mock_provider_handler))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock provider server");
    let addr = listener.local_addr().expect("mock provider server address");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("mock provider server");
    });
    MockProviderServer {
        base_url: format!("http://{addr}"),
        requests,
    }
}

fn configured_provider_settings(
    provider_name: &str,
    base_url: &str,
    enabled: bool,
) -> LearningSettings {
    let mut settings = LearningSettings::default();
    let mut provider = crate::settings::LearningProviderSettings {
        enabled,
        ..crate::settings::LearningProviderSettings::default()
    };
    provider
        .config
        .insert("base_url".to_string(), base_url.to_string());
    provider
        .config
        .insert("api_key".to_string(), "secret-token".to_string());
    match provider_name {
        "mem0" => {
            provider
                .config
                .insert("search_path".to_string(), "/mem0/search".to_string());
            provider
                .config
                .insert("sync_path".to_string(), "/mem0/sync".to_string());
        }
        "openmemory" => {
            provider
                .config
                .insert("search_path".to_string(), "/openmemory/search".to_string());
            provider
                .config
                .insert("sync_path".to_string(), "/openmemory/sync".to_string());
        }
        "letta" => {
            provider
                .config
                .insert("agent_id".to_string(), "agent-123".to_string());
            provider.config.insert(
                "search_path".to_string(),
                "/letta/{agent_id}/search".to_string(),
            );
            provider.config.insert(
                "sync_path".to_string(),
                "/letta/{agent_id}/sync".to_string(),
            );
        }
        "chroma" => {
            provider
                .config
                .insert("collection_id".to_string(), "collection-123".to_string());
            provider
                .config
                .insert("embedding_url".to_string(), format!("{base_url}/embed"));
            provider.config.insert(
                "query_path".to_string(),
                "/chroma/{collection_id}/query".to_string(),
            );
            provider.config.insert(
                "sync_path".to_string(),
                "/chroma/{collection_id}/upsert".to_string(),
            );
        }
        "qdrant" => {
            provider
                .config
                .insert("collection".to_string(), "memories".to_string());
            provider
                .config
                .insert("embedding_url".to_string(), format!("{base_url}/embed"));
            provider.config.insert(
                "query_path".to_string(),
                "/qdrant/{collection}/query".to_string(),
            );
            provider.config.insert(
                "sync_path".to_string(),
                "/qdrant/{collection}/points".to_string(),
            );
        }
        "custom_http" => {
            provider.config.insert(
                "recall_url".to_string(),
                format!("{base_url}/custom/recall"),
            );
            provider
                .config
                .insert("sync_url".to_string(), format!("{base_url}/custom/sync"));
        }
        _ => {}
    }
    *settings.providers.provider_mut(provider_name) = provider;
    settings
}

fn configured_provider_cases() -> Vec<(
    &'static str,
    Arc<dyn MemoryProvider>,
    &'static str,
    &'static str,
)> {
    vec![
        (
            "mem0",
            Arc::new(Mem0Provider),
            "authorization",
            "Token secret-token",
        ),
        (
            "openmemory",
            Arc::new(OpenMemoryProvider),
            "x-api-key",
            "secret-token",
        ),
        (
            "letta",
            Arc::new(LettaProvider),
            "authorization",
            "Bearer secret-token",
        ),
        (
            "chroma",
            Arc::new(ChromaProvider),
            "x-chroma-token",
            "secret-token",
        ),
        (
            "qdrant",
            Arc::new(QdrantProvider),
            "api-key",
            "secret-token",
        ),
        (
            "custom_http",
            Arc::new(CustomHttpProvider),
            "authorization",
            "Bearer secret-token",
        ),
    ]
}

#[cfg(feature = "libsql")]
fn generated_skill_test_content(skill_name: &str) -> String {
    synthesize_generated_skill_markdown(
        skill_name,
        "Help the user collect a file summary and write it down.",
        &[crate::agent::session::TurnToolCall {
            id: "call-shell".to_string(),
            name: "shell".to_string(),
            parameters: serde_json::json!({"cmd": "echo hi"}),
            result: Some(serde_json::json!({"stdout": "hi"})),
            error: None,
        }],
        GeneratedSkillLifecycle::Shadow,
        3,
        Some("shadow_candidate".to_string()),
    )
    .expect("generated skill markdown should parse")
}

#[cfg(feature = "libsql")]
fn generated_skill_candidate(
    user_id: &str,
    skill_name: &str,
    skill_content: &str,
    created_at: DateTime<Utc>,
) -> DbLearningCandidate {
    DbLearningCandidate {
        id: Uuid::new_v4(),
        learning_event_id: None,
        user_id: user_id.to_string(),
        candidate_type: "skill".to_string(),
        risk_tier: "medium".to_string(),
        confidence: Some(0.92),
        target_type: Some("skill".to_string()),
        target_name: Some(skill_name.to_string()),
        summary: Some("Generated procedural skill".to_string()),
        proposal: serde_json::json!({
            "workflow_digest": "sha256:test-workflow",
            "provenance": "generated",
            "lifecycle_status": GeneratedSkillLifecycle::Shadow.as_str(),
            "reuse_count": 3,
            "outcome_score": 0.92,
            "activation_reason": "shadow_candidate",
            "skill_content": skill_content,
            "last_transition_at": created_at,
            "state_history": [generated_skill_transition_entry(
                GeneratedSkillLifecycle::Shadow,
                Some("shadow_candidate"),
                None,
                None,
                None,
                created_at,
            )],
        }),
        created_at,
    }
}

#[test]
fn prompt_validator_rejects_transcript_residue() {
    assert!(validate_prompt_content("# Header\nrole: user\nfoo").is_err());
    assert!(validate_prompt_content("# Header\nNormal content").is_ok());
}

#[test]
fn prompt_candidate_patch_materializes_content() {
    let current = "# USER.md\n\n## Preferences\n- concise\n";
    let proposal = serde_json::json!({
        "prompt_patch": {
            "operation": "upsert_section",
            "heading": "Outcome-Backed Guidance",
            "section_content": "- finish the requested implementation before concluding"
        }
    });

    let next = materialize_prompt_candidate_content(current, &proposal, paths::USER)
        .expect("prompt patch should materialize");

    assert!(next.contains("## Preferences\n- concise"));
    assert!(next.contains("## Outcome-Backed Guidance"));
    assert!(next.contains("finish the requested implementation"));
}

#[test]
fn prompt_candidate_patch_materializes_valid_canonical_soul_when_empty() {
    let proposal = serde_json::json!({
        "prompt_patch": {
            "operation": "upsert_section",
            "heading": "Outcome-Backed Guidance",
            "section_content": "- call out bad ideas early"
        }
    });

    let next = materialize_prompt_candidate_content("", &proposal, paths::SOUL)
        .expect("prompt patch should materialize");

    assert!(crate::identity::soul::validate_canonical_soul(&next).is_ok());
    assert!(next.contains("## Outcome-Backed Guidance"));
}

#[test]
fn prompt_candidate_patch_materializes_valid_local_overlay_when_empty() {
    let proposal = serde_json::json!({
        "prompt_patch": {
            "operation": "upsert_section",
            "heading": "Tone Adjustments",
            "section_content": "- stay extra terse for this workspace"
        }
    });

    let next = materialize_prompt_candidate_content("", &proposal, paths::SOUL_LOCAL)
        .expect("prompt patch should materialize");

    assert!(crate::identity::soul::validate_local_overlay(&next).is_ok());
    assert!(next.contains("## Tone Adjustments"));
}

#[test]
fn classify_event_prefers_code_when_diff_present() {
    let event = DbLearningEvent {
        id: Uuid::new_v4(),
        user_id: "u".to_string(),
        actor_id: None,
        channel: None,
        thread_id: None,
        conversation_id: None,
        message_id: None,
        job_id: None,
        event_type: "feedback".to_string(),
        source: "test".to_string(),
        payload: serde_json::json!({"diff": "--- a\n+++ b"}),
        metadata: None,
        created_at: Utc::now(),
    };
    assert_eq!(classify_event(&event), ImprovementClass::Code);
}

#[test]
fn proposal_fingerprint_is_stable_for_identical_input() {
    let files = vec!["src/lib.rs".to_string(), "src/main.rs".to_string()];
    let first = proposal_fingerprint("Fix bug", "rationale", &files, "--- a\n+++ b");
    let second = proposal_fingerprint("Fix bug", "rationale", &files, "--- a\n+++ b");
    assert_eq!(first, second);
}

#[test]
fn proposal_fingerprint_changes_when_diff_changes() {
    let files = vec!["src/lib.rs".to_string()];
    let first = proposal_fingerprint("Fix bug", "rationale", &files, "--- a\n+++ b");
    let second = proposal_fingerprint("Fix bug", "rationale", &files, "--- a\n+++ c");
    assert_ne!(first, second);
}

#[tokio::test]
async fn trajectory_logger_appends_jsonl_records() {
    let root = std::env::temp_dir().join(format!("thinclaw-trajectories-{}", Uuid::new_v4()));
    let logger = TrajectoryLogger::with_root(&root);
    let record = TrajectoryTurnRecord {
        session_id: Uuid::new_v4(),
        thread_id: Uuid::new_v4(),
        user_id: "user-123".to_string(),
        actor_id: "actor-123".to_string(),
        channel: "cli".to_string(),
        conversation_scope_id: Uuid::new_v4(),
        conversation_kind: "direct".to_string(),
        external_thread_id: Some("thread-1".to_string()),
        turn_number: 0,
        user_message: "hello".to_string(),
        assistant_response: Some("hi".to_string()),
        tool_calls: vec![],
        started_at: Utc::now(),
        completed_at: Some(Utc::now()),
        turn_status: TrajectoryTurnStatus::Completed,
        outcome: TrajectoryOutcome::Success,
        failure_reason: None,
        execution_backend: Some("interactive_chat".to_string()),
        llm_provider: None,
        llm_model: None,
        prompt_snapshot_hash: None,
        ephemeral_overlay_hash: None,
        provider_context_refs: Vec::new(),
        user_feedback: None,
        assessment: Some(TrajectoryAssessment {
            outcome: TrajectoryOutcome::Success,
            score: 0.95,
            source: "test".to_string(),
            reasoning: "positive".to_string(),
        }),
    };

    let path = logger.append_turn(&record).await.expect("append_turn");
    let contents = tokio::fs::read_to_string(path).await.expect("read jsonl");
    assert!(contents.contains("\"user_message\":\"hello\""));
    assert!(contents.contains("\"assistant_response\":\"hi\""));
}

#[test]
fn trajectory_turn_record_prefers_incoming_actor_identity() {
    let session = Session::new_scoped(
        "user-shared",
        "phone",
        scope_id_from_key("principal:user-shared"),
        ConversationKind::Direct,
    );
    let thread = Thread::new(session.id);
    let incoming =
        IncomingMessage::new("gateway", "user-shared", "hello").with_identity(ResolvedIdentity {
            principal_id: "user-shared".to_string(),
            actor_id: "desktop".to_string(),
            conversation_scope_id: scope_id_from_key("principal:user-shared"),
            conversation_kind: ConversationKind::Direct,
            raw_sender_id: "user-shared".to_string(),
            stable_external_conversation_key:
                "gateway://direct/user-shared/actor/desktop/thread/thread-a".to_string(),
        });
    let turn = Turn::new(0, "hello", false);

    let record =
        TrajectoryTurnRecord::from_turn(&session, Uuid::new_v4(), &thread, &incoming, &turn);

    assert_eq!(record.actor_id, "desktop");
}

#[test]
fn trajectory_stats_handle_empty_roots() {
    let root = std::env::temp_dir().join(format!("thinclaw-trajectories-{}", Uuid::new_v4()));
    let logger = TrajectoryLogger::with_root(&root);
    let stats = logger.stats().expect("stats");
    assert_eq!(stats.record_count, 0);
    assert_eq!(stats.file_count, 0);
    assert_eq!(stats.session_count, 0);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn hydrate_trajectory_record_prefers_learning_evaluation() {
    let (db, _guard) = crate::testing::test_db().await;
    let session_id = Uuid::new_v4();
    let thread_id = Uuid::new_v4();
    let mut record = TrajectoryTurnRecord {
        session_id,
        thread_id,
        user_id: "user-123".to_string(),
        actor_id: "actor-123".to_string(),
        channel: "cli".to_string(),
        conversation_scope_id: Uuid::new_v4(),
        conversation_kind: "direct".to_string(),
        external_thread_id: Some("thread-1".to_string()),
        turn_number: 7,
        user_message: "hello".to_string(),
        assistant_response: Some("hi".to_string()),
        tool_calls: vec![],
        started_at: Utc::now(),
        completed_at: Some(Utc::now()),
        turn_status: TrajectoryTurnStatus::Completed,
        outcome: TrajectoryOutcome::Success,
        failure_reason: None,
        execution_backend: Some("interactive_chat".to_string()),
        llm_provider: None,
        llm_model: None,
        prompt_snapshot_hash: None,
        ephemeral_overlay_hash: None,
        provider_context_refs: Vec::new(),
        user_feedback: None,
        assessment: Some(TrajectoryAssessment {
            outcome: TrajectoryOutcome::Success,
            score: 0.9,
            source: "heuristic_turn_eval_v1".to_string(),
            reasoning: "fallback".to_string(),
        }),
    };
    let target_id = record.target_id();
    let event = DbLearningEvent {
        id: Uuid::new_v4(),
        user_id: record.user_id.clone(),
        actor_id: Some(record.actor_id.clone()),
        channel: Some(record.channel.clone()),
        thread_id: Some(record.thread_id.to_string()),
        conversation_id: None,
        message_id: None,
        job_id: None,
        event_type: "trajectory_review".to_string(),
        source: "trajectory_test".to_string(),
        payload: serde_json::json!({
            "target_type": "trajectory_turn",
            "target_id": target_id,
            "thread_id": record.thread_id.to_string(),
            "session_id": record.session_id.to_string(),
            "turn_number": record.turn_number,
        }),
        metadata: None,
        created_at: Utc::now(),
    };
    db.insert_learning_event(&event)
        .await
        .expect("insert learning event");
    db.insert_learning_evaluation(&DbLearningEvaluation {
        id: Uuid::new_v4(),
        learning_event_id: event.id,
        user_id: record.user_id.clone(),
        evaluator: "learning_orchestrator_v1".to_string(),
        status: "poor".to_string(),
        score: Some(0.1),
        details: serde_json::json!({
            "quality_score": 10.0
        }),
        created_at: Utc::now(),
    })
    .await
    .expect("insert learning evaluation");

    hydrate_trajectory_record(&mut record, Some(&db)).await;

    assert_eq!(record.outcome, TrajectoryOutcome::Failure);
    let assessment = record.assessment.expect("assessment");
    assert_eq!(assessment.outcome, TrajectoryOutcome::Failure);
    assert_eq!(
        assessment.source,
        "learning_evaluation:learning_orchestrator_v1"
    );
    assert!(assessment.score <= 0.1);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn auto_apply_routine_records_artifact_version_and_outcome_contract() {
    let (db, _guard) = crate::testing::test_db().await;
    let user_id = "routine-auto-apply-user";

    db.set_setting(user_id, "learning.enabled", &serde_json::json!(true))
        .await
        .expect("set learning.enabled");
    db.set_setting(
        user_id,
        "learning.outcomes.enabled",
        &serde_json::json!(true),
    )
    .await
    .expect("set learning.outcomes.enabled");

    let workspace = Arc::new(Workspace::new_with_db(user_id, Arc::clone(&db)));
    let (notify_tx, _notify_rx) = mpsc::channel(4);
    let routine_engine = Arc::new(RoutineEngine::new(
        RoutineConfig::default(),
        Arc::clone(&db),
        Arc::new(StubLlm::new("ok")),
        Arc::clone(&workspace),
        notify_tx,
        None,
    ));

    let now = Utc::now();
    let routine = Routine {
        id: Uuid::new_v4(),
        name: "Daily outcome digest".to_string(),
        description: "Summarize outcomes".to_string(),
        user_id: user_id.to_string(),
        actor_id: user_id.to_string(),
        enabled: true,
        trigger: Trigger::Manual,
        action: RoutineAction::Lightweight {
            prompt: "Summarize the latest outcome-backed learning signals.".to_string(),
            context_paths: Vec::new(),
            max_tokens: 128,
        },
        guardrails: RoutineGuardrails::default(),
        notify: crate::agent::routine::NotifyConfig {
            user: user_id.to_string(),
            on_success: true,
            ..crate::agent::routine::NotifyConfig::default()
        },
        policy: Default::default(),
        last_run_at: None,
        next_fire_at: None,
        run_count: 0,
        consecutive_failures: 0,
        state: serde_json::json!({}),
        config_version: 1,
        created_at: now,
        updated_at: now,
    };
    db.create_routine(&routine).await.expect("create routine");

    let event = DbLearningEvent {
        id: Uuid::new_v4(),
        user_id: user_id.to_string(),
        actor_id: Some(user_id.to_string()),
        channel: Some("gateway".to_string()),
        thread_id: Some("thread-routine".to_string()),
        conversation_id: None,
        message_id: None,
        job_id: None,
        event_type: "outcome_candidate".to_string(),
        source: "test".to_string(),
        payload: serde_json::json!({}),
        metadata: None,
        created_at: Utc::now(),
    };
    db.insert_learning_event(&event)
        .await
        .expect("insert learning event");
    let candidate = DbLearningCandidate {
        id: Uuid::new_v4(),
        learning_event_id: Some(event.id),
        user_id: user_id.to_string(),
        candidate_type: "routine_patch".to_string(),
        risk_tier: "medium".to_string(),
        confidence: Some(0.91),
        target_type: Some("routine".to_string()),
        target_name: Some(routine.name.clone()),
        summary: Some("Disable noisy success notifications for this routine".to_string()),
        proposal: serde_json::json!({
            "routine_patch": {
                "type": "notification_noise_reduction",
                "routine_id": routine.id.to_string(),
                "changes": {
                    "notify": {
                        "on_success": false
                    }
                }
            }
        }),
        created_at: Utc::now(),
    };
    db.insert_learning_candidate(&candidate)
        .await
        .expect("insert learning candidate");

    let orchestrator = LearningOrchestrator::new(Arc::clone(&db), Some(workspace), None::<Arc<_>>)
        .with_routine_engine(Some(routine_engine));

    let applied = orchestrator
        .auto_apply_routine(&candidate)
        .await
        .expect("auto_apply_routine should succeed");
    assert!(applied, "routine patch should auto-apply");

    let updated_routine = db
        .get_routine(routine.id)
        .await
        .expect("get routine")
        .expect("routine should exist");
    assert!(
        !updated_routine.notify.on_success,
        "routine success notifications should be disabled"
    );

    let artifact_versions = db
        .list_learning_artifact_versions(user_id, Some("routine"), Some(&routine.name), 10)
        .await
        .expect("list learning artifact versions");
    assert_eq!(
        artifact_versions.len(),
        1,
        "routine mutation should be ledgered"
    );
    let version = &artifact_versions[0];
    assert_eq!(version.status, "applied");
    assert_eq!(version.artifact_type, "routine");
    assert!(
        version
            .provenance
            .get("patch_type")
            .and_then(|value| value.as_str())
            == Some("notification_noise_reduction")
    );

    let contracts = db
        .list_outcome_contracts(&crate::history::OutcomeContractQuery {
            user_id: user_id.to_string(),
            actor_id: Some(user_id.to_string()),
            status: Some("open".to_string()),
            contract_type: Some("tool_durability".to_string()),
            source_kind: Some("artifact_version".to_string()),
            source_id: Some(version.id.to_string()),
            thread_id: None,
            limit: 10,
        })
        .await
        .expect("list outcome contracts");
    assert_eq!(
        contracts.len(),
        1,
        "routine artifact auto-apply should create a durability contract"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn auto_apply_memory_uses_authoritative_actor_scope() {
    let (db, _guard) = crate::testing::test_db().await;
    let user_id = "memory-auto-apply-user";
    let actor_id = "alice";
    let workspace = Arc::new(Workspace::new_with_db(user_id, Arc::clone(&db)));
    let orchestrator = LearningOrchestrator::new(
        Arc::clone(&db),
        Some(Arc::clone(&workspace)),
        None::<Arc<_>>,
    );
    let candidate = DbLearningCandidate {
        id: Uuid::new_v4(),
        learning_event_id: None,
        user_id: user_id.to_string(),
        candidate_type: "memory".to_string(),
        risk_tier: "low".to_string(),
        confidence: Some(0.9),
        target_type: Some("memory".to_string()),
        target_name: Some(paths::MEMORY.to_string()),
        summary: Some("Remember an actor preference".to_string()),
        proposal: proposal_with_resolved_identity(
            serde_json::json!({
                "memory_entry": "Alice prefers compact summaries.",
                // These proposal fields are deliberately hostile; the reserved
                // identity envelope must remain the only authority.
                "actor_id": "bob",
                "conversation_scope_id": Uuid::new_v4().to_string(),
            }),
            &direct_learning_identity(user_id, actor_id),
            "test",
            None,
        ),
        created_at: Utc::now(),
    };
    db.insert_learning_candidate(&candidate)
        .await
        .expect("persist actor-scoped memory candidate");

    assert!(
        orchestrator
            .auto_apply_memory(&candidate)
            .await
            .expect("authorized actor memory should apply")
    );
    let actor_memory = workspace
        .read(&paths::actor_memory(actor_id))
        .await
        .expect("actor memory should be written");
    assert!(actor_memory.content.contains("compact summaries"));
    assert!(workspace.read(&paths::actor_memory("bob")).await.is_err());
    assert!(workspace.read(paths::MEMORY).await.is_err());

    let versions = db
        .list_learning_artifact_versions(user_id, Some("memory"), None, 10)
        .await
        .expect("list memory artifacts");
    assert_eq!(versions.len(), 1);
    assert!(versions[0].before_content.is_none());
    assert!(versions[0].after_content.is_none());
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn auto_apply_memory_rejects_unverified_group_scope() {
    let (db, _guard) = crate::testing::test_db().await;
    let user_id = "memory-auto-apply-group";
    let workspace = Arc::new(Workspace::new_with_db(user_id, Arc::clone(&db)));
    let orchestrator = LearningOrchestrator::new(
        Arc::clone(&db),
        Some(Arc::clone(&workspace)),
        None::<Arc<_>>,
    );
    let group_scope = Uuid::new_v4();
    let group_identity = ResolvedIdentity {
        principal_id: user_id.to_string(),
        actor_id: "alice".to_string(),
        conversation_scope_id: group_scope,
        conversation_kind: ConversationKind::Group,
        raw_sender_id: "alice".to_string(),
        stable_external_conversation_key: "group:test".to_string(),
    };
    let candidate = DbLearningCandidate {
        id: Uuid::new_v4(),
        learning_event_id: None,
        user_id: user_id.to_string(),
        candidate_type: "memory".to_string(),
        risk_tier: "low".to_string(),
        confidence: Some(0.9),
        target_type: Some("memory".to_string()),
        target_name: Some(paths::MEMORY.to_string()),
        summary: None,
        proposal: proposal_with_resolved_identity(
            serde_json::json!({"memory_entry": "must not be written"}),
            &group_identity,
            "test",
            Some(Uuid::new_v4()),
        ),
        created_at: Utc::now(),
    };

    let error = orchestrator
        .auto_apply_memory(&candidate)
        .await
        .expect_err("unknown group conversation must fail closed");
    assert!(error.contains("does not belong"));
    assert!(
        workspace
            .read(&paths::conversation_memory(group_scope))
            .await
            .is_err()
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn auto_apply_prompt_materializes_patch_for_actor_user_targets() {
    let (db, _guard) = crate::testing::test_db().await;
    let user_id = "prompt-auto-apply-user";
    let actor_target = paths::actor_user("alice");
    let workspace = Arc::new(Workspace::new_with_db(user_id, Arc::clone(&db)));
    let orchestrator = LearningOrchestrator::new(
        Arc::clone(&db),
        Some(Arc::clone(&workspace)),
        None::<Arc<_>>,
    );

    let candidate = DbLearningCandidate {
        id: Uuid::new_v4(),
        learning_event_id: None,
        user_id: user_id.to_string(),
        candidate_type: "prompt".to_string(),
        risk_tier: "medium".to_string(),
        confidence: Some(0.88),
        target_type: Some("prompt".to_string()),
        target_name: Some(actor_target.clone()),
        summary: Some("Add outcome-backed prompt guidance".to_string()),
        proposal: proposal_with_resolved_identity(
            serde_json::json!({
                "target": actor_target,
                "prompt_patch": {
                    "operation": "upsert_section",
                    "heading": "Outcome-Backed Guidance",
                    "section_content": "- prefer direct implementation and verification"
                }
            }),
            &direct_learning_identity(user_id, "alice"),
            "test",
            None,
        ),
        created_at: Utc::now(),
    };
    db.insert_learning_candidate(&candidate)
        .await
        .expect("insert prompt learning candidate");

    let applied = orchestrator
        .auto_apply_prompt(&candidate)
        .await
        .expect("auto_apply_prompt should succeed");
    assert!(applied, "prompt patch should auto-apply");

    let content = workspace
        .read(&actor_target)
        .await
        .expect("read actor USER.md")
        .content;
    assert!(content.contains("## Outcome-Backed Guidance"));
    assert!(content.contains("prefer direct implementation and verification"));

    let versions = db
        .list_learning_artifact_versions(user_id, Some("prompt"), Some(&actor_target), 10)
        .await
        .expect("list prompt artifact versions");
    assert_eq!(versions.len(), 1, "prompt auto-apply should be ledgered");
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn auto_apply_prompt_cannot_target_a_sibling_actor() {
    let (db, _guard) = crate::testing::test_db().await;
    let user_id = "prompt-auto-apply-cross-actor";
    let workspace = Arc::new(Workspace::new_with_db(user_id, Arc::clone(&db)));
    let orchestrator = LearningOrchestrator::new(
        Arc::clone(&db),
        Some(Arc::clone(&workspace)),
        None::<Arc<_>>,
    );
    let candidate = DbLearningCandidate {
        id: Uuid::new_v4(),
        learning_event_id: None,
        user_id: user_id.to_string(),
        candidate_type: "prompt".to_string(),
        risk_tier: "medium".to_string(),
        confidence: Some(0.9),
        target_type: Some("prompt".to_string()),
        target_name: Some(paths::actor_user("bob")),
        summary: None,
        proposal: proposal_with_resolved_identity(
            serde_json::json!({"content": "# USER.md\n\n- **Name:** Bob\n"}),
            &direct_learning_identity(user_id, "alice"),
            "test",
            None,
        ),
        created_at: Utc::now(),
    };

    let error = orchestrator
        .auto_apply_prompt(&candidate)
        .await
        .expect_err("cross-actor prompt mutation must fail");
    assert!(error.contains("different actor"));
    assert!(workspace.read(&paths::actor_user("bob")).await.is_err());
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn auto_apply_prompt_routes_canonical_soul_to_home_store() {
    let (db, _guard) = crate::testing::test_db().await;
    let temp_home = tempfile::tempdir().expect("temp home");
    let _home = crate::testing::ScopedEnvVar::set("THINCLAW_HOME", temp_home.path());

    let user_id = "prompt-auto-apply-soul";
    let workspace = Arc::new(Workspace::new_with_db(user_id, Arc::clone(&db)));
    crate::identity::soul_store::write_home_soul(
        &crate::identity::soul::compose_seeded_soul("balanced").unwrap(),
    )
    .expect("write initial home soul");
    workspace
        .write(paths::SOUL, "# stale workspace soul should not change")
        .await
        .expect("write stale legacy workspace soul");

    let orchestrator = LearningOrchestrator::new(
        Arc::clone(&db),
        Some(Arc::clone(&workspace)),
        None::<Arc<_>>,
    );

    let candidate = DbLearningCandidate {
        id: Uuid::new_v4(),
        learning_event_id: None,
        user_id: user_id.to_string(),
        candidate_type: "prompt".to_string(),
        risk_tier: "medium".to_string(),
        confidence: Some(0.9),
        target_type: Some("prompt".to_string()),
        target_name: Some(paths::SOUL.to_string()),
        summary: Some("Sharpen canonical soul guidance".to_string()),
        proposal: proposal_with_resolved_identity(
            serde_json::json!({
                "target": paths::SOUL,
                "prompt_patch": {
                    "operation": "upsert_section",
                    "heading": "Outcome-Backed Guidance",
                    "section_content": "- be direct and finish the job"
                }
            }),
            &direct_learning_identity(user_id, user_id),
            "test",
            None,
        ),
        created_at: Utc::now(),
    };
    db.insert_learning_candidate(&candidate)
        .await
        .expect("insert prompt learning candidate");

    let applied = orchestrator
        .auto_apply_prompt(&candidate)
        .await
        .expect("auto_apply_prompt should succeed");
    assert!(applied, "canonical soul patch should auto-apply");

    let home = crate::identity::soul_store::read_home_soul().expect("read home soul");
    assert!(home.contains("## Outcome-Backed Guidance"));
    assert!(home.contains("be direct and finish the job"));

    let workspace_soul = workspace
        .read(paths::SOUL)
        .await
        .expect("read stale workspace soul");
    assert!(
        !workspace_soul
            .content
            .contains("be direct and finish the job"),
        "auto-apply should not write canonical soul changes into workspace SOUL.md"
    );

    let versions = db
        .list_learning_artifact_versions(user_id, Some("prompt"), Some(paths::SOUL), 10)
        .await
        .expect("list prompt artifact versions");
    assert_eq!(
        versions.len(),
        1,
        "canonical soul auto-apply should be ledgered"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn create_code_proposal_from_candidate_rejects_empty_diff() {
    let (db, _guard) = crate::testing::test_db().await;
    let user_id = "empty-diff-outcome-user";
    let orchestrator = LearningOrchestrator::new(Arc::clone(&db), None, None::<Arc<_>>);

    let candidate = DbLearningCandidate {
        id: Uuid::new_v4(),
        learning_event_id: None,
        user_id: user_id.to_string(),
        candidate_type: "code".to_string(),
        risk_tier: "critical".to_string(),
        confidence: Some(0.92),
        target_type: Some("code".to_string()),
        target_name: Some("Fix missing diff handling".to_string()),
        summary: Some("Repeated negative durability outcomes".to_string()),
        proposal: serde_json::json!({
            "title": "Fix missing diff handling",
            "rationale": "Outcome-backed durability fix",
            "target_files": ["src/agent/learning/mod.rs"],
            "diff": ""
        }),
        created_at: Utc::now(),
    };

    let err = orchestrator
        .create_code_proposal_from_candidate(&candidate)
        .await
        .expect_err("empty diff should be rejected");
    assert!(err.contains("missing diff"));
}

#[test]
fn learning_publish_mode_and_git_ref_validation_fail_closed() {
    for mode in [
        "branch_pr_draft",
        "branch_only",
        "bundle_only",
        "local_autorollout",
    ] {
        assert_eq!(validate_learning_publish_mode(mode).unwrap(), mode);
    }
    assert!(validate_learning_publish_mode("typo_push_mode").is_err());

    for valid in ["main", "feature/agent-fix", "release-1.2_3"] {
        assert!(validate_learning_git_ref(valid).is_ok());
    }
    for invalid in [
        "-branch",
        "HEAD",
        "foo..bar",
        "foo//bar",
        "foo/.bar",
        "foo/bar.lock",
        "foo/@{bar",
    ] {
        assert!(validate_learning_git_ref(invalid).is_err());
    }
}

#[test]
fn generated_skill_lifecycle_requires_shadow_before_activation() {
    let draft = generated_skill_lifecycle_for_reuse(1);
    assert_eq!(draft.0.as_str(), "draft");
    assert_eq!(draft.1, None);
    assert!(!draft.2);

    let shadow = generated_skill_lifecycle_for_reuse(2);
    assert_eq!(shadow.0.as_str(), "shadow");
    assert_eq!(shadow.1.as_deref(), Some("shadow_candidate"));
    assert!(!shadow.2);

    let second_shadow_match = generated_skill_lifecycle_for_reuse(3);
    assert_eq!(second_shadow_match.0.as_str(), "shadow");
    assert_eq!(second_shadow_match.1.as_deref(), Some("shadow_candidate"));
    assert!(!second_shadow_match.2);

    let proposed_threshold = generated_skill_lifecycle_for_reuse(4);
    assert_eq!(proposed_threshold.0.as_str(), "proposed");
    assert_eq!(
        proposed_threshold.1.as_deref(),
        Some("proposal_reuse_threshold")
    );
    assert!(!proposed_threshold.2);
}

#[test]
fn generated_skill_feedback_polarity_maps_positive_and_negative_verdicts() {
    assert_eq!(generated_skill_feedback_polarity("helpful"), 1);
    assert_eq!(generated_skill_feedback_polarity("APPROVED"), 1);
    assert_eq!(generated_skill_feedback_polarity("reject"), -1);
    assert_eq!(generated_skill_feedback_polarity("dont_learn"), -1);
    assert_eq!(generated_skill_feedback_polarity("unclear"), 0);
}

#[test]
fn generated_workflow_digest_distinguishes_parameters_and_outcomes() {
    let first = vec![crate::agent::session::TurnToolCall {
        id: "call-1".to_string(),
        name: "shell".to_string(),
        parameters: serde_json::json!({"cmd": "echo one"}),
        result: Some(serde_json::json!({"stdout": "one"})),
        error: None,
    }];
    let second = vec![crate::agent::session::TurnToolCall {
        id: "call-2".to_string(),
        name: "shell".to_string(),
        parameters: serde_json::json!({"cmd": "echo two"}),
        result: Some(serde_json::json!({"stdout": "two"})),
        error: None,
    }];

    assert_ne!(
        generated_workflow_digest("run the shell command", &first),
        generated_workflow_digest("run the shell command", &second)
    );
}

#[test]
fn generated_workflow_digest_is_stable_for_reordered_object_keys() {
    let first = vec![crate::agent::session::TurnToolCall {
        id: "call-1".to_string(),
        name: "http".to_string(),
        parameters: serde_json::json!({"url": "https://example.com", "method": "GET"}),
        result: Some(serde_json::json!({"status": 200, "ok": true})),
        error: None,
    }];
    let second = vec![crate::agent::session::TurnToolCall {
        id: "call-2".to_string(),
        name: "http".to_string(),
        parameters: serde_json::json!({"method": "GET", "url": "https://example.com"}),
        result: Some(serde_json::json!({"ok": true, "status": 200})),
        error: None,
    }];

    assert_eq!(
        generated_workflow_digest("fetch the endpoint", &first),
        generated_workflow_digest("fetch the endpoint", &second)
    );
}

mod providers;
