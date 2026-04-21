use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};
use rust_decimal::Decimal;
use thinclaw::agent::routine::{
    NotifyConfig, Routine, RoutineAction, RoutineGuardrails, RoutineRun, RunStatus, Trigger,
};
use thinclaw::context::{ActionRecord, JobContext, JobState, StateTransition};
use thinclaw::db::AgentWorkspaceRecord;
use thinclaw::experiments::{
    ExperimentCampaign, ExperimentCampaignQueueState, ExperimentCampaignStatus, ExperimentLease,
    ExperimentLeaseStatus, ExperimentModelUsageRecord, ExperimentPreset, ExperimentProject,
    ExperimentProjectStatus, ExperimentRunnerBackend, ExperimentRunnerProfile,
    ExperimentRunnerStatus, ExperimentTarget, ExperimentTargetKind, ExperimentTargetLink,
    ExperimentTrial, ExperimentTrialStatus,
};
use thinclaw::history::{
    LearningArtifactVersion, LearningCandidate, LearningCodeProposal, LearningEvaluation,
    LearningEvent, OutcomeContract, OutcomeObservation, SandboxJobRecord,
};
use thinclaw::identity::{ActorEndpointRef, ActorStatus, NewActorEndpointRecord, NewActorRecord};
use uuid::Uuid;

use crate::db_contract::support::unique_id;

pub(crate) fn user(prefix: &str) -> String {
    unique_id(prefix)
}

pub(crate) fn actor_name(prefix: &str) -> String {
    format!("{prefix}-actor-{}", Uuid::new_v4().simple())
}

pub(crate) fn new_actor_record(principal_id: &str, display_name: &str) -> NewActorRecord {
    NewActorRecord {
        principal_id: principal_id.to_string(),
        display_name: display_name.to_string(),
        status: ActorStatus::Active,
        preferred_delivery_endpoint: None,
        last_active_direct_endpoint: None,
    }
}

pub(crate) fn new_actor_endpoint_record(
    actor_id: Uuid,
    channel: &str,
    external_user_id: &str,
) -> NewActorEndpointRecord {
    NewActorEndpointRecord {
        endpoint: ActorEndpointRef::new(channel, external_user_id),
        actor_id,
        metadata: serde_json::json!({"source":"contract_test"}),
        approval_status: thinclaw::identity::EndpointApprovalStatus::Approved,
    }
}

pub(crate) fn sandbox_job_record(user_id: &str, actor_id: &str, status: &str) -> SandboxJobRecord {
    let now = Utc::now();
    SandboxJobRecord {
        id: Uuid::new_v4(),
        task: format!("task-{}", Uuid::new_v4().simple()),
        status: status.to_string(),
        user_id: user_id.to_string(),
        actor_id: actor_id.to_string(),
        project_dir: "/tmp/contract".to_string(),
        success: None,
        failure_reason: None,
        created_at: now,
        started_at: None,
        completed_at: None,
        credential_grants_json: "[]".to_string(),
    }
}

pub(crate) fn job_context(user_id: &str, actor_id: &str) -> JobContext {
    let mut ctx = JobContext::with_user_and_actor(
        user_id.to_string(),
        actor_id.to_string(),
        "contract job",
        "job for contract tests",
    );
    ctx.category = Some("contract".to_string());
    ctx.estimated_cost = Some(Decimal::new(125, 2));
    ctx.estimated_duration = Some(Duration::from_secs(90));
    ctx.transitions.push(StateTransition {
        from: JobState::Pending,
        to: JobState::Pending,
        timestamp: Utc::now(),
        reason: Some("fixture".to_string()),
    });
    ctx
}

pub(crate) fn action_record(sequence: u32, tool_name: &str) -> ActionRecord {
    let mut action = ActionRecord::new(sequence, tool_name, serde_json::json!({"x": sequence}));
    action.output_raw = Some("raw output".to_string());
    action.output_sanitized = Some(serde_json::json!({"ok": true}));
    action.sanitization_warnings = vec![];
    action.cost = Some(Decimal::new(12, 3));
    action.duration = Duration::from_millis(80);
    action.success = true;
    action.error = None;
    action.executed_at = Utc::now();
    action
}

pub(crate) fn routine(user_id: &str, actor_id: &str) -> Routine {
    let now = Utc::now();
    Routine {
        id: Uuid::new_v4(),
        name: format!("routine-{}", Uuid::new_v4().simple()),
        description: "contract routine".to_string(),
        user_id: user_id.to_string(),
        actor_id: actor_id.to_string(),
        enabled: true,
        trigger: Trigger::Manual,
        action: RoutineAction::Lightweight {
            prompt: "Check current status".to_string(),
            context_paths: vec!["MEMORY.md".to_string()],
            max_tokens: 256,
        },
        guardrails: RoutineGuardrails {
            cooldown: Duration::from_secs(60),
            max_concurrent: 1,
            dedup_window: Some(Duration::from_secs(300)),
        },
        notify: NotifyConfig::default(),
        last_run_at: None,
        next_fire_at: None,
        run_count: 0,
        consecutive_failures: 0,
        state: serde_json::json!({}),
        created_at: now,
        updated_at: now,
    }
}

pub(crate) fn routine_run(routine_id: Uuid, status: RunStatus) -> RoutineRun {
    RoutineRun {
        id: Uuid::new_v4(),
        routine_id,
        trigger_type: "manual".to_string(),
        trigger_detail: Some("contract".to_string()),
        started_at: Utc::now(),
        completed_at: None,
        status,
        result_summary: None,
        tokens_used: None,
        job_id: None,
        created_at: Utc::now(),
    }
}

pub(crate) fn agent_workspace(agent_id: &str) -> AgentWorkspaceRecord {
    let now = Utc::now();
    AgentWorkspaceRecord {
        id: Uuid::new_v4(),
        agent_id: agent_id.to_string(),
        display_name: "Contract Agent".to_string(),
        system_prompt: Some("You are a contract test agent.".to_string()),
        model: Some("openai/gpt-5-mini".to_string()),
        bound_channels: vec!["repl".to_string()],
        trigger_keywords: vec!["contract".to_string()],
        allowed_tools: Some(vec!["memory_read".to_string()]),
        allowed_skills: Some(vec!["github:github".to_string()]),
        tool_profile: None,
        is_default: false,
        created_at: now,
        updated_at: now,
    }
}

pub(crate) fn experiment_project() -> ExperimentProject {
    let now = Utc::now();
    ExperimentProject {
        id: Uuid::new_v4(),
        name: format!("project-{}", Uuid::new_v4().simple()),
        workspace_path: ".".to_string(),
        git_remote_name: "origin".to_string(),
        base_branch: "main".to_string(),
        preset: ExperimentPreset::AutoresearchSingleFile,
        strategy_prompt: "Improve benchmark".to_string(),
        workdir: ".".to_string(),
        prepare_command: None,
        run_command: "cargo test".to_string(),
        mutable_paths: vec!["src/".to_string()],
        fixed_paths: vec!["Cargo.toml".to_string()],
        primary_metric: Default::default(),
        secondary_metrics: vec![],
        comparison_policy: Default::default(),
        stop_policy: Default::default(),
        default_runner_profile_id: None,
        promotion_mode: "branch_pr_draft".to_string(),
        autonomy_mode: thinclaw::experiments::ExperimentAutonomyMode::Autonomous,
        status: ExperimentProjectStatus::Draft,
        created_at: now,
        updated_at: now,
    }
}

pub(crate) fn experiment_runner_profile() -> ExperimentRunnerProfile {
    let now = Utc::now();
    ExperimentRunnerProfile {
        id: Uuid::new_v4(),
        name: format!("runner-{}", Uuid::new_v4().simple()),
        backend: ExperimentRunnerBackend::LocalDocker,
        backend_config: serde_json::json!({}),
        image_or_runtime: Some("rust:latest".to_string()),
        gpu_requirements: serde_json::json!({}),
        env_grants: serde_json::json!({}),
        secret_references: vec![],
        cache_policy: serde_json::json!({}),
        status: ExperimentRunnerStatus::Draft,
        readiness_class: thinclaw::experiments::ExperimentRunnerReadinessClass::ManualOnly,
        launch_eligible: false,
        created_at: now,
        updated_at: now,
    }
}

pub(crate) fn experiment_campaign(project_id: Uuid, runner_profile_id: Uuid) -> ExperimentCampaign {
    let now = Utc::now();
    ExperimentCampaign {
        id: Uuid::new_v4(),
        project_id,
        runner_profile_id,
        owner_user_id: "default".to_string(),
        status: ExperimentCampaignStatus::PendingBaseline,
        baseline_commit: Some("abc123".to_string()),
        best_commit: None,
        best_metrics: serde_json::json!({}),
        experiment_branch: Some("exp/contract".to_string()),
        remote_ref: None,
        worktree_path: None,
        started_at: None,
        ended_at: None,
        trial_count: 0,
        failure_count: 0,
        pause_reason: None,
        queue_state: ExperimentCampaignQueueState::NotQueued,
        queue_position: 0,
        active_trial_id: None,
        total_runtime_ms: 0,
        total_cost_usd: 0.0,
        total_llm_cost_usd: 0.0,
        total_runner_cost_usd: 0.0,
        consecutive_non_improving_trials: 0,
        max_trials_override: None,
        gateway_url: None,
        metadata: serde_json::json!({}),
        created_at: now,
        updated_at: now,
    }
}

pub(crate) fn experiment_trial(campaign_id: Uuid, sequence: u32) -> ExperimentTrial {
    let now = Utc::now();
    ExperimentTrial {
        id: Uuid::new_v4(),
        campaign_id,
        sequence,
        candidate_commit: Some("def456".to_string()),
        parent_best_commit: Some("abc123".to_string()),
        status: ExperimentTrialStatus::Preparing,
        runner_backend: ExperimentRunnerBackend::LocalDocker,
        exit_code: None,
        metrics_json: serde_json::json!({}),
        summary: None,
        decision_reason: None,
        log_preview_path: None,
        artifact_manifest_json: serde_json::json!({}),
        runtime_ms: None,
        attributed_cost_usd: None,
        llm_cost_usd: None,
        runner_cost_usd: None,
        hypothesis: Some("faster compile".to_string()),
        mutation_summary: Some("changed cache config".to_string()),
        reviewer_decision: None,
        provider_job_id: None,
        provider_job_metadata: serde_json::json!({}),
        started_at: None,
        completed_at: None,
        created_at: now,
        updated_at: now,
    }
}

pub(crate) fn experiment_target() -> ExperimentTarget {
    let now = Utc::now();
    ExperimentTarget {
        id: Uuid::new_v4(),
        name: format!("target-{}", Uuid::new_v4().simple()),
        kind: ExperimentTargetKind::PromptAsset,
        location: Some("prompts/system.md".to_string()),
        metadata: serde_json::json!({}),
        created_at: now,
        updated_at: now,
    }
}

pub(crate) fn experiment_target_link(target_id: Uuid) -> ExperimentTargetLink {
    let now = Utc::now();
    ExperimentTargetLink {
        id: Uuid::new_v4(),
        target_id,
        kind: ExperimentTargetKind::PromptAsset,
        provider: "openai".to_string(),
        model: "gpt-5-mini".to_string(),
        route_key: Some("default".to_string()),
        logical_role: Some("assistant".to_string()),
        metadata: serde_json::json!({}),
        created_at: now,
        updated_at: now,
    }
}

pub(crate) fn experiment_model_usage() -> ExperimentModelUsageRecord {
    ExperimentModelUsageRecord {
        id: Uuid::new_v4(),
        provider: "openai".to_string(),
        model: "gpt-5-mini".to_string(),
        route_key: Some("default".to_string()),
        logical_role: Some("assistant".to_string()),
        endpoint_type: Some("chat".to_string()),
        workload_tag: Some("contract".to_string()),
        latency_ms: Some(120),
        cost_usd: Some(0.002),
        success: true,
        prompt_asset_ids: vec![],
        retrieval_asset_ids: vec![],
        tool_policy_ids: vec![],
        evaluator_ids: vec![],
        parser_ids: vec![],
        metadata: serde_json::json!({}),
        created_at: Utc::now(),
    }
}

pub(crate) fn experiment_lease(
    campaign_id: Uuid,
    trial_id: Uuid,
    runner_profile_id: Uuid,
) -> ExperimentLease {
    let now = Utc::now();
    ExperimentLease {
        id: Uuid::new_v4(),
        campaign_id,
        trial_id,
        runner_profile_id,
        status: ExperimentLeaseStatus::Pending,
        token_hash: "hash".to_string(),
        job_payload: serde_json::json!({}),
        credentials_payload: serde_json::json!({}),
        expires_at: now + ChronoDuration::hours(1),
        claimed_at: None,
        completed_at: None,
        created_at: now,
        updated_at: now,
    }
}

pub(crate) fn learning_event(
    user_id: &str,
    conversation_id: Option<Uuid>,
    message_id: Option<Uuid>,
) -> LearningEvent {
    LearningEvent {
        id: Uuid::new_v4(),
        user_id: user_id.to_string(),
        actor_id: None,
        channel: Some("repl".to_string()),
        thread_id: Some("contract-thread".to_string()),
        conversation_id,
        message_id,
        job_id: None,
        event_type: "observation".to_string(),
        source: "contract".to_string(),
        payload: serde_json::json!({"detail":"something happened"}),
        metadata: Some(serde_json::json!({"scope":"tests"})),
        created_at: Utc::now(),
    }
}

pub(crate) fn learning_evaluation(user_id: &str, learning_event_id: Uuid) -> LearningEvaluation {
    LearningEvaluation {
        id: Uuid::new_v4(),
        learning_event_id,
        user_id: user_id.to_string(),
        evaluator: "contract-evaluator".to_string(),
        status: "approved".to_string(),
        score: Some(0.9),
        details: serde_json::json!({"notes":"looks good"}),
        created_at: Utc::now(),
    }
}

pub(crate) fn learning_candidate(user_id: &str, learning_event_id: Uuid) -> LearningCandidate {
    LearningCandidate {
        id: Uuid::new_v4(),
        learning_event_id: Some(learning_event_id),
        user_id: user_id.to_string(),
        candidate_type: "prompt_change".to_string(),
        risk_tier: "low".to_string(),
        confidence: Some(0.8),
        target_type: Some("prompt".to_string()),
        target_name: Some("system".to_string()),
        summary: Some("Improve routing".to_string()),
        proposal: serde_json::json!({"delta":"small"}),
        created_at: Utc::now(),
    }
}

pub(crate) fn learning_artifact_version(
    user_id: &str,
    candidate_id: Uuid,
) -> LearningArtifactVersion {
    LearningArtifactVersion {
        id: Uuid::new_v4(),
        candidate_id: Some(candidate_id),
        user_id: user_id.to_string(),
        artifact_type: "prompt".to_string(),
        artifact_name: "system".to_string(),
        version_label: Some("v1".to_string()),
        status: "proposed".to_string(),
        diff_summary: Some("Tweaked instruction order".to_string()),
        before_content: Some("before".to_string()),
        after_content: Some("after".to_string()),
        provenance: serde_json::json!({"source":"contract"}),
        created_at: Utc::now(),
    }
}

pub(crate) fn learning_code_proposal(
    user_id: &str,
    learning_event_id: Uuid,
) -> LearningCodeProposal {
    let now = Utc::now();
    LearningCodeProposal {
        id: Uuid::new_v4(),
        learning_event_id: Some(learning_event_id),
        user_id: user_id.to_string(),
        status: "proposed".to_string(),
        title: "Contract proposal".to_string(),
        rationale: "Improve reliability".to_string(),
        target_files: vec!["src/db/mod.rs".to_string()],
        diff: "diff --git".to_string(),
        validation_results: serde_json::json!({"tests":"pending"}),
        rollback_note: None,
        confidence: Some(0.7),
        branch_name: Some("contract/proposal".to_string()),
        pr_url: None,
        metadata: serde_json::json!({}),
        created_at: now,
        updated_at: now,
    }
}

pub(crate) fn outcome_contract(user_id: &str) -> OutcomeContract {
    let now = Utc::now();
    OutcomeContract {
        id: Uuid::new_v4(),
        user_id: user_id.to_string(),
        actor_id: Some(actor_name("outcome")),
        channel: Some("web".to_string()),
        thread_id: Some(format!("thread-{}", Uuid::new_v4().simple())),
        source_kind: "learning_event".to_string(),
        source_id: Uuid::new_v4().to_string(),
        contract_type: "turn_usefulness".to_string(),
        status: "open".to_string(),
        summary: Some("contract outcome".to_string()),
        due_at: now,
        expires_at: now + ChronoDuration::hours(72),
        final_verdict: None,
        final_score: None,
        evaluation_details: serde_json::json!({}),
        metadata: serde_json::json!({"pattern_key":"contract:test"}),
        dedupe_key: format!("dedupe-{}", Uuid::new_v4().simple()),
        claimed_at: None,
        evaluated_at: None,
        created_at: now,
        updated_at: now,
    }
}

pub(crate) fn outcome_observation(contract_id: Uuid) -> OutcomeObservation {
    let now = Utc::now();
    OutcomeObservation {
        id: Uuid::new_v4(),
        contract_id,
        observation_kind: "explicit_approval".to_string(),
        polarity: "positive".to_string(),
        weight: 0.6,
        summary: Some("Looks good".to_string()),
        evidence: serde_json::json!({"source":"contract_test"}),
        fingerprint: format!("fp-{}", Uuid::new_v4().simple()),
        observed_at: now,
        created_at: now,
    }
}
