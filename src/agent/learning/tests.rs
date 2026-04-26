use super::*;
use crate::agent::session::{Session, Thread, Turn};
use crate::channels::IncomingMessage;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use crate::agent::routine::{Routine, RoutineAction, RoutineGuardrails, Trigger};
use crate::agent::routine_engine::RoutineEngine;
use crate::config::RoutineConfig;
use crate::identity::{ConversationKind, ResolvedIdentity, scope_id_from_key};
use crate::testing::StubLlm;
use crate::workspace::Workspace;

#[derive(Debug)]
struct TestMemoryProvider {
    name: &'static str,
    hits: Vec<ProviderMemoryHit>,
    recalls: Arc<Mutex<Vec<(String, String, usize)>>>,
    exports: Arc<Mutex<Vec<(String, serde_json::Value)>>>,
    health_status: ProviderHealthStatus,
}

#[async_trait]
impl MemoryProvider for TestMemoryProvider {
    fn name(&self) -> &'static str {
        self.name
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

fn generated_skill_test_content(skill_name: &str) -> String {
    synthesize_generated_skill_markdown(
        skill_name,
        "Help the user collect a file summary and write it down.",
        &[crate::agent::session::TurnToolCall {
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
        proposal: serde_json::json!({
            "target": actor_target,
            "prompt_patch": {
                "operation": "upsert_section",
                "heading": "Outcome-Backed Guidance",
                "section_content": "- prefer direct implementation and verification"
            }
        }),
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
async fn auto_apply_prompt_routes_canonical_soul_to_home_store() {
    let (db, _guard) = crate::testing::test_db().await;
    let temp_home = tempfile::tempdir().expect("temp home");
    let previous_home = std::env::var_os("THINCLAW_HOME");
    unsafe {
        std::env::set_var("THINCLAW_HOME", temp_home.path());
    }

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
        proposal: serde_json::json!({
            "target": paths::SOUL,
            "prompt_patch": {
                "operation": "upsert_section",
                "heading": "Outcome-Backed Guidance",
                "section_content": "- be direct and finish the job"
            }
        }),
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

    if let Some(previous_home) = previous_home {
        unsafe {
            std::env::set_var("THINCLAW_HOME", previous_home);
        }
    } else {
        unsafe {
            std::env::remove_var("THINCLAW_HOME");
        }
    }
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
            "target_files": ["src/agent/learning.rs"],
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
        name: "shell".to_string(),
        parameters: serde_json::json!({"cmd": "echo one"}),
        result: Some(serde_json::json!({"stdout": "one"})),
        error: None,
    }];
    let second = vec![crate::agent::session::TurnToolCall {
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
        name: "http".to_string(),
        parameters: serde_json::json!({"url": "https://example.com", "method": "GET"}),
        result: Some(serde_json::json!({"status": 200, "ok": true})),
        error: None,
    }];
    let second = vec![crate::agent::session::TurnToolCall {
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

#[cfg(feature = "libsql")]
#[tokio::test]
async fn prefetch_provider_context_uses_only_the_active_provider() {
    let (db, _guard) = crate::testing::test_db().await;
    let user_id = "provider-prefetch-user";
    db.set_setting(
        user_id,
        "learning.providers.active",
        &serde_json::json!("honcho"),
    )
    .await
    .expect("set active provider");

    let honcho_recalls = Arc::new(Mutex::new(Vec::new()));
    let zep_recalls = Arc::new(Mutex::new(Vec::new()));
    let orchestrator = LearningOrchestrator {
        store: Arc::clone(&db),
        workspace: None,
        skill_registry: None,
        routine_engine: None,
        provider_manager: Arc::new(MemoryProviderManager::with_providers(
            Arc::clone(&db),
            vec![
                Arc::new(TestMemoryProvider {
                    name: "honcho",
                    hits: vec![ProviderMemoryHit {
                        provider: "honcho".to_string(),
                        summary: "Remembered preference".to_string(),
                        score: Some(0.91),
                        provenance: serde_json::json!({"id": "honcho:1"}),
                    }],
                    recalls: Arc::clone(&honcho_recalls),
                    exports: Arc::new(Mutex::new(Vec::new())),
                    health_status: provider_status("honcho", ProviderReadiness::Ready, true, None),
                }),
                Arc::new(TestMemoryProvider {
                    name: "zep",
                    hits: vec![ProviderMemoryHit {
                        provider: "zep".to_string(),
                        summary: "Should not be used".to_string(),
                        score: Some(0.32),
                        provenance: serde_json::json!({"id": "zep:1"}),
                    }],
                    recalls: Arc::clone(&zep_recalls),
                    exports: Arc::new(Mutex::new(Vec::new())),
                    health_status: provider_status("zep", ProviderReadiness::Ready, true, None),
                }),
            ],
        )),
    };

    let context = orchestrator
        .prefetch_provider_context(user_id, "summarize my preferences", 3)
        .await
        .expect("active provider should return prefetch context");

    assert_eq!(context.provider, "honcho");
    assert_eq!(context.context_refs, vec!["honcho:1"]);
    assert!(context.rendered_context.contains("honcho"));
    assert_eq!(
        honcho_recalls.lock().expect("honcho recall log").len(),
        1,
        "the selected provider should be queried exactly once"
    );
    assert!(
        zep_recalls.lock().expect("zep recall log").is_empty(),
        "inactive providers must not be queried"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn unhealthy_active_provider_fails_closed_for_prefetch_and_tool_surface() {
    let (db, _guard) = crate::testing::test_db().await;
    let user_id = "provider-health-gating-user";
    db.set_setting(
        user_id,
        "learning.providers.active",
        &serde_json::json!("honcho"),
    )
    .await
    .expect("set active provider");

    let honcho_recalls = Arc::new(Mutex::new(Vec::new()));
    let zep_recalls = Arc::new(Mutex::new(Vec::new()));
    let orchestrator = LearningOrchestrator {
        store: Arc::clone(&db),
        workspace: None,
        skill_registry: None,
        routine_engine: None,
        provider_manager: Arc::new(MemoryProviderManager::with_providers(
            Arc::clone(&db),
            vec![
                Arc::new(TestMemoryProvider {
                    name: "honcho",
                    hits: vec![ProviderMemoryHit {
                        provider: "honcho".to_string(),
                        summary: "Should not be recalled".to_string(),
                        score: Some(0.11),
                        provenance: serde_json::json!({"id": "honcho:down"}),
                    }],
                    recalls: Arc::clone(&honcho_recalls),
                    exports: Arc::new(Mutex::new(Vec::new())),
                    health_status: provider_status(
                        "honcho",
                        ProviderReadiness::Unhealthy,
                        false,
                        Some("provider health check failed"),
                    ),
                }),
                Arc::new(TestMemoryProvider {
                    name: "zep",
                    hits: vec![ProviderMemoryHit {
                        provider: "zep".to_string(),
                        summary: "Inactive backup".to_string(),
                        score: Some(0.88),
                        provenance: serde_json::json!({"id": "zep:1"}),
                    }],
                    recalls: Arc::clone(&zep_recalls),
                    exports: Arc::new(Mutex::new(Vec::new())),
                    health_status: provider_status("zep", ProviderReadiness::Ready, true, None),
                }),
            ],
        )),
    };

    let statuses = orchestrator.provider_health(user_id).await;
    let active = statuses
        .iter()
        .find(|status| status.provider == "honcho")
        .expect("active provider status");
    assert!(active.active, "honcho should be marked active");
    assert_eq!(active.readiness, ProviderReadiness::Unhealthy);

    assert!(
        orchestrator
            .prefetch_provider_context(user_id, "remember my preferences", 3)
            .await
            .is_none(),
        "unhealthy providers should not surface prompt recall"
    );
    assert!(
        orchestrator
            .provider_recall(user_id, "remember my preferences", 3)
            .await
            .is_empty(),
        "unhealthy providers should not execute recall calls"
    );
    assert!(
        orchestrator
            .provider_tool_extensions(user_id)
            .await
            .is_empty(),
        "tool extensions should disappear when the active provider is unhealthy"
    );
    assert!(
        honcho_recalls.lock().expect("honcho recall log").is_empty(),
        "prefetch/recall must fail closed before dispatching to an unhealthy provider"
    );
    assert!(
        zep_recalls.lock().expect("zep recall log").is_empty(),
        "inactive backups must not be used automatically"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn export_provider_payload_uses_only_ready_active_provider() {
    let (db, _guard) = crate::testing::test_db().await;
    let user_id = "provider-export-user";
    db.set_setting(
        user_id,
        "learning.providers.active",
        &serde_json::json!("honcho"),
    )
    .await
    .expect("set active provider");

    let honcho_exports = Arc::new(Mutex::new(Vec::new()));
    let zep_exports = Arc::new(Mutex::new(Vec::new()));
    let orchestrator = LearningOrchestrator {
        store: Arc::clone(&db),
        workspace: None,
        skill_registry: None,
        routine_engine: None,
        provider_manager: Arc::new(MemoryProviderManager::with_providers(
            Arc::clone(&db),
            vec![
                Arc::new(TestMemoryProvider {
                    name: "honcho",
                    hits: Vec::new(),
                    recalls: Arc::new(Mutex::new(Vec::new())),
                    exports: Arc::clone(&honcho_exports),
                    health_status: provider_status("honcho", ProviderReadiness::Ready, true, None),
                }),
                Arc::new(TestMemoryProvider {
                    name: "zep",
                    hits: Vec::new(),
                    recalls: Arc::new(Mutex::new(Vec::new())),
                    exports: Arc::clone(&zep_exports),
                    health_status: provider_status("zep", ProviderReadiness::Ready, true, None),
                }),
            ],
        )),
    };

    let provider = orchestrator
        .export_provider_payload(
            user_id,
            &serde_json::json!({"content": "prefers concise docs"}),
        )
        .await
        .expect("export should use active provider");

    assert_eq!(provider, "honcho");
    let exports = honcho_exports.lock().expect("honcho export log");
    assert_eq!(exports.len(), 1);
    assert_eq!(exports[0].0, user_id);
    assert_eq!(exports[0].1["content"], "prefers concise docs");
    assert!(
        zep_exports.lock().expect("zep export log").is_empty(),
        "inactive providers must not receive explicit exports"
    );
}

#[test]
fn provider_hit_parser_handles_memory_service_and_vector_shapes() {
    let mem0_hits = parse_provider_hits(
        serde_json::json!({
            "results": [
                {"id": "m1", "memory": "likes terse changelogs", "score": 0.88}
            ]
        }),
        "mem0",
    );
    assert_eq!(mem0_hits[0].summary, "likes terse changelogs");
    assert_eq!(mem0_hits[0].score, Some(0.88));

    let chroma_hits = parse_provider_hits(
        serde_json::json!({
            "ids": [["doc-1"]],
            "documents": [["uses qdrant for high-recall vector search"]],
            "distances": [[0.12]],
            "metadatas": [[{"source": "test"}]]
        }),
        "chroma",
    );
    assert_eq!(
        chroma_hits[0].summary,
        "uses qdrant for high-recall vector search"
    );
    assert_eq!(chroma_hits[0].score, Some(0.12));

    let qdrant_hits = parse_provider_hits(
        serde_json::json!({
            "result": {
                "points": [
                    {
                        "id": "point-1",
                        "score": 0.77,
                        "payload": {"text": "keeps OpenMemory local"}
                    }
                ]
            }
        }),
        "qdrant",
    );
    assert_eq!(qdrant_hits[0].summary, "keeps OpenMemory local");
    assert_eq!(qdrant_hits[0].score, Some(0.77));
}

#[tokio::test]
async fn vector_provider_health_requires_embedding_wiring() {
    let mut settings = LearningSettings::default();
    let mut qdrant = crate::settings::LearningProviderSettings {
        enabled: true,
        ..crate::settings::LearningProviderSettings::default()
    };
    qdrant
        .config
        .insert("collection".to_string(), "memories".to_string());
    *settings.providers.provider_mut("qdrant") = qdrant;

    let status = QdrantProvider.health(&settings).await;
    assert_eq!(status.readiness, ProviderReadiness::NotConfigured);
    assert!(
        status
            .error
            .as_deref()
            .is_some_and(|error| error.contains("embedding_url")),
        "vector memory providers should report missing embedding wiring"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn positive_feedback_promotes_generated_skill_and_updates_candidate_proposal() {
    let (db, _guard) = crate::testing::test_db().await;
    let user_id = "generated-skill-positive-feedback";
    let created_at = Utc::now();
    let skill_name = "workflow-generated-positive";
    let skill_content = generated_skill_test_content(skill_name);
    let candidate = generated_skill_candidate(user_id, skill_name, &skill_content, created_at);
    db.insert_learning_candidate(&candidate)
        .await
        .expect("insert learning candidate");

    let user_dir = tempfile::tempdir().expect("temporary user dir for generated skill registry");
    let installed_dir =
        tempfile::tempdir().expect("temporary installed dir for generated skill registry");
    let registry = Arc::new(tokio::sync::RwLock::new(
        SkillRegistry::new(user_dir.path().to_path_buf())
            .with_installed_dir(installed_dir.path().to_path_buf()),
    ));
    let orchestrator =
        LearningOrchestrator::new(Arc::clone(&db), None, Some(Arc::clone(&registry)));

    orchestrator
        .submit_feedback(
            user_id,
            "skill",
            skill_name,
            "helpful",
            Some("this saved time"),
            None,
        )
        .await
        .expect("positive feedback should activate generated skill");

    assert!(
        registry.read().await.has(skill_name),
        "positive feedback should install the generated skill"
    );

    let persisted = db
        .list_learning_candidates(user_id, Some("skill"), None, 10)
        .await
        .expect("list learning candidates")
        .into_iter()
        .find(|entry| entry.id == candidate.id)
        .expect("updated candidate");
    assert_eq!(
        persisted
            .proposal
            .get("lifecycle_status")
            .and_then(|value| value.as_str()),
        Some("active")
    );
    assert_eq!(
        persisted
            .proposal
            .get("activation_reason")
            .and_then(|value| value.as_str()),
        Some("explicit_positive_feedback")
    );
    assert_eq!(
        persisted
            .proposal
            .get("last_feedback")
            .and_then(|value| value.get("verdict"))
            .and_then(|value| value.as_str()),
        Some("helpful")
    );
    assert!(
        persisted
            .proposal
            .get("state_history")
            .and_then(|value| value.as_array())
            .is_some_and(|entries| entries.len() >= 2),
        "candidate proposal should retain lifecycle history on the canonical record"
    );

    let versions = db
        .list_learning_artifact_versions(user_id, Some("skill"), Some(skill_name), 10)
        .await
        .expect("list learning artifact versions");
    let active_version = versions
        .iter()
        .find(|version| version.status == "active")
        .expect("active artifact version");
    assert_eq!(active_version.candidate_id, Some(candidate.id));
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn negative_feedback_rolls_back_generated_skill_and_updates_candidate_proposal() {
    let (db, _guard) = crate::testing::test_db().await;
    let user_id = "generated-skill-negative-feedback";
    let created_at = Utc::now();
    let skill_name = "workflow-generated-negative";
    let skill_content = generated_skill_test_content(skill_name);
    let candidate = generated_skill_candidate(user_id, skill_name, &skill_content, created_at);
    db.insert_learning_candidate(&candidate)
        .await
        .expect("insert learning candidate");

    let user_dir = tempfile::tempdir().expect("temporary user dir for generated skill registry");
    let installed_dir =
        tempfile::tempdir().expect("temporary installed dir for generated skill registry");
    let registry = Arc::new(tokio::sync::RwLock::new(
        SkillRegistry::new(user_dir.path().to_path_buf())
            .with_installed_dir(installed_dir.path().to_path_buf()),
    ));
    registry
        .write()
        .await
        .install_skill(&skill_content)
        .await
        .expect("preinstall generated skill");
    let orchestrator =
        LearningOrchestrator::new(Arc::clone(&db), None, Some(Arc::clone(&registry)));

    orchestrator
        .submit_feedback(
            user_id,
            "skill",
            skill_name,
            "reject",
            Some("this introduced drift"),
            None,
        )
        .await
        .expect("negative feedback should update generated skill lifecycle");

    assert!(
        !registry.read().await.has(skill_name),
        "negative feedback should remove the installed generated skill"
    );

    let persisted = db
        .list_learning_candidates(user_id, Some("skill"), None, 10)
        .await
        .expect("list learning candidates")
        .into_iter()
        .find(|entry| entry.id == candidate.id)
        .expect("updated candidate");
    assert_eq!(
        persisted
            .proposal
            .get("lifecycle_status")
            .and_then(|value| value.as_str()),
        Some("rolled_back")
    );
    assert_eq!(
        persisted
            .proposal
            .get("last_feedback")
            .and_then(|value| value.get("verdict"))
            .and_then(|value| value.as_str()),
        Some("reject")
    );
    assert!(
        persisted
            .proposal
            .get("rolled_back_at")
            .and_then(|value| value.as_str())
            .is_some(),
        "candidate proposal should record rollback timing"
    );

    let versions = db
        .list_learning_artifact_versions(user_id, Some("skill"), Some(skill_name), 10)
        .await
        .expect("list learning artifact versions");
    let rollback_version = versions
        .iter()
        .find(|version| version.status == "rolled_back")
        .expect("rollback artifact version");
    assert_eq!(rollback_version.candidate_id, Some(candidate.id));
}

#[test]
fn custom_http_provider_parses_common_recall_shapes() {
    let hits = parse_custom_http_hits(
        serde_json::json!({
            "results": [
                { "id": "m1", "summary": "prefers concise answers", "score": 0.82 },
                { "id": "m2", "content": "likes examples" },
                { "id": "m3", "summary": "" }
            ]
        }),
        "custom_http",
    );
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].provider, "custom_http");
    assert_eq!(hits[0].score, Some(0.82));
    assert_eq!(hits[1].summary, "likes examples");
}
