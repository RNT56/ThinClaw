//! Behavioral tests for the experiments API (relocated from the former
//! `src/api/experiments.rs` god-file; unchanged in content).

use super::{git_changed_files, record_campaign_candidate_generation};
use crate::agent::subagent_executor::{SubagentConfig, SubagentExecutor};
use crate::agent::{AgentRunArtifact, AgentRunStatus};
use crate::channels::ChannelManager;
use crate::experiments::{
    ExperimentAutonomyMode, ExperimentCampaign, ExperimentCampaignQueueState,
    ExperimentCampaignStatus, ExperimentLease, ExperimentLeaseStatus, ExperimentMetricComparator,
    ExperimentMetricDefinition, ExperimentProject, ExperimentProjectStatus,
    ExperimentRunnerBackend, ExperimentRunnerCompletion, ExperimentRunnerProfile,
    ExperimentRunnerStatus, ExperimentTrial, ExperimentTrialStatus,
};
use crate::llm::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, Role, ToolCall,
    ToolCompletionRequest, ToolCompletionResponse,
};
use crate::tools::ToolRegistry;
use crate::tools::builtin::{
    ApplyPatchTool, ListDirTool, ReadFileTool, SearchFilesTool, WriteFileTool,
};
use async_trait::async_trait;
use base64::Engine as _;
use chrono::Utc;
use rust_decimal::Decimal;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use tempfile::TempDir;
use uuid::Uuid;

struct AutonomousResearchTestLlm;

impl AutonomousResearchTestLlm {
    fn response_for_messages(
        &self,
        messages: &[ChatMessage],
    ) -> (Option<String>, Vec<ToolCall>, FinishReason) {
        let joined = messages
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        if joined.contains("planning role for ThinClaw Research") {
            return (
                Some(
                    serde_json::json!({
                        "hypothesis": "Switch app.txt to the candidate configuration to improve score.",
                        "target_ids": ["app-config"],
                        "allowed_paths": ["app.txt"],
                        "expected_metric_direction": "increase",
                        "mutation_brief": "Rewrite app.txt with the candidate configuration."
                    })
                    .to_string(),
                ),
                Vec::new(),
                FinishReason::Stop,
            );
        }

        if joined.contains("mutator role for ThinClaw Research") {
            let wrote_file = messages.iter().any(|message| {
                message.role == Role::Tool && message.name.as_deref() == Some("write_file")
            });
            if !wrote_file {
                return (
                    None,
                    vec![ToolCall {
                        id: "mutator_write_app".to_string(),
                        name: "write_file".to_string(),
                        arguments: serde_json::json!({
                            "path": "app.txt",
                            "content": "candidate\n",
                        }),
                    }],
                    FinishReason::ToolUse,
                );
            }
            return (
                Some(
                    serde_json::json!({
                        "changed_paths": ["app.txt"],
                        "mutation_summary": "Updated app.txt to the candidate configuration."
                    })
                    .to_string(),
                ),
                Vec::new(),
                FinishReason::Stop,
            );
        }

        if joined.contains("reviewer role for ThinClaw Research") {
            return (
                Some(
                    serde_json::json!({
                        "approved": true,
                        "scope_ok": true,
                        "benchmark_ready": true,
                        "reason": "approved"
                    })
                    .to_string(),
                ),
                Vec::new(),
                FinishReason::Stop,
            );
        }

        (Some("{}".to_string()), Vec::new(), FinishReason::Stop)
    }
}

#[async_trait]
impl LlmProvider for AutonomousResearchTestLlm {
    fn model_name(&self) -> &str {
        "autonomous-research-test"
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, crate::error::LlmError> {
        let (content, _, finish_reason) = self.response_for_messages(&request.messages);
        Ok(CompletionResponse {
            content: content.unwrap_or_default(),
            provider_model: Some(self.model_name().to_string()),
            cost_usd: None,
            thinking_content: None,
            input_tokens: 32,
            output_tokens: 24,
            finish_reason,
            token_capture: None,
        })
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, crate::error::LlmError> {
        let (content, tool_calls, finish_reason) = self.response_for_messages(&request.messages);
        Ok(ToolCompletionResponse {
            content,
            provider_model: Some(self.model_name().to_string()),
            cost_usd: None,
            tool_calls,
            thinking_content: None,
            input_tokens: 32,
            output_tokens: 24,
            finish_reason,
            token_capture: None,
        })
    }
}

async fn ensure_test_research_subagent_executor() {
    if super::research_subagent_executor().is_some() {
        return;
    }

    let llm = Arc::new(AutonomousResearchTestLlm);
    let safety = Arc::new(crate::safety::SafetyLayer::new(
        &crate::config::SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: false,
            redact_pii_in_prompts: true,
            smart_approval_mode: "off".to_string(),
            external_scanner_mode: "off".to_string(),
            external_scanner_path: None,
            external_scanner_require_verified: false,
            allow_temp_paths: false,
        },
    ));
    let tools = Arc::new(ToolRegistry::new());
    tools.register_sync(Arc::new(ReadFileTool::new()));
    tools.register_sync(Arc::new(WriteFileTool::new()));
    tools.register_sync(Arc::new(ListDirTool::new()));
    tools.register_sync(Arc::new(ApplyPatchTool::new()));
    tools.register_sync(Arc::new(SearchFilesTool::new()));

    let channels = Arc::new(ChannelManager::new());
    let (executor, _result_rx) =
        SubagentExecutor::new(llm, safety, tools, channels, SubagentConfig::default());
    super::register_experiment_subagent_executor(Arc::new(executor));
}

#[test]
fn record_campaign_candidate_generation_tracks_last_failure_and_artifacts() {
    let mut campaign = ExperimentCampaign {
        id: Uuid::new_v4(),
        project_id: Uuid::new_v4(),
        runner_profile_id: Uuid::new_v4(),
        owner_user_id: "default".to_string(),
        status: ExperimentCampaignStatus::Paused,
        baseline_commit: None,
        best_commit: None,
        best_metrics: serde_json::json!({}),
        experiment_branch: None,
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
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let artifact = AgentRunArtifact::new(
        "experiment_subagent:planner",
        AgentRunStatus::Failed,
        Utc::now(),
        Some(Utc::now()),
    )
    .with_failure_reason(Some("planner failed".to_string()));

    record_campaign_candidate_generation(
        &mut campaign,
        "autonomous",
        "failed",
        "planner failed",
        &[artifact.clone()],
    );

    let artifacts = campaign
        .metadata
        .get("run_artifacts")
        .and_then(|value| value.as_array())
        .expect("run artifacts should be recorded");
    assert_eq!(artifacts.len(), 1);
    assert_eq!(
        campaign.metadata["candidate_generation"]["mode"].as_str(),
        Some("autonomous")
    );
    assert_eq!(
        campaign.metadata["candidate_generation"]["status"].as_str(),
        Some("failed")
    );
    assert_eq!(
        campaign.metadata["candidate_generation"]["artifact_run_ids"][0].as_str(),
        Some(artifact.run_id.as_str())
    );
}

#[test]
fn research_subagent_tool_denylist_blocks_memory_and_session_recall() {
    for tool_name in ["memory_read", "memory_search", "session_search"] {
        assert!(
            super::RESEARCH_SHARED_TOOL_DENYLIST.contains(&tool_name),
            "{tool_name} should be denied for research subagents"
        );
    }
}

#[tokio::test]
async fn git_changed_files_reports_modified_path() {
    let repo = TempDir::new().expect("temp repo");
    git(repo.path(), &["init"]);
    git(repo.path(), &["config", "user.email", "tests@example.com"]);
    git(repo.path(), &["config", "user.name", "ThinClaw Tests"]);
    std::fs::write(repo.path().join("app.txt"), "baseline\n").expect("write app file");
    git(repo.path(), &["add", "app.txt"]);
    git(repo.path(), &["commit", "-m", "initial"]);
    std::fs::write(repo.path().join("app.txt"), "candidate\n").expect("rewrite app file");

    let changed = git_changed_files(&repo.path().to_string_lossy())
        .await
        .expect("changed files");
    assert_eq!(changed, vec!["app.txt".to_string()]);
}

#[tokio::test]
async fn git_changed_files_reports_rename_destination_path() {
    let repo = TempDir::new().expect("temp repo");
    git(repo.path(), &["init"]);
    git(repo.path(), &["config", "user.email", "tests@example.com"]);
    git(repo.path(), &["config", "user.name", "ThinClaw Tests"]);
    std::fs::write(repo.path().join("before.txt"), "hello\n").expect("write before file");
    git(repo.path(), &["add", "before.txt"]);
    git(repo.path(), &["commit", "-m", "initial"]);
    git(repo.path(), &["mv", "before.txt", "after.txt"]);

    let changed = git_changed_files(&repo.path().to_string_lossy())
        .await
        .expect("changed files");
    assert_eq!(changed, vec!["after.txt".to_string()]);
}

#[tokio::test]
async fn launch_campaign_baseline_runs_local_docker_trial_end_to_end() {
    let mut settings = crate::settings::Settings::default();
    settings.sandbox.enabled = true;
    let sandbox = crate::sandbox::SandboxManager::new(super::experiment_sandbox_config(&settings));
    if !sandbox.is_available().await {
        eprintln!("skipping docker-backed experiment test because sandbox is unavailable");
        return;
    }

    let (store, _guard) = crate::testing::test_db().await;
    let repo = TempDir::new().expect("temp repo");
    git(repo.path(), &["init"]);
    git(repo.path(), &["checkout", "-b", "main"]);
    git(repo.path(), &["config", "user.email", "tests@example.com"]);
    git(repo.path(), &["config", "user.name", "ThinClaw Tests"]);
    std::fs::write(repo.path().join("app.txt"), "baseline\n").expect("write repo file");
    git(repo.path(), &["add", "app.txt"]);
    git(repo.path(), &["commit", "-m", "initial"]);

    let now = Utc::now();
    let project = ExperimentProject {
        id: Uuid::new_v4(),
        name: "docker-baseline".to_string(),
        workspace_path: repo.path().to_string_lossy().to_string(),
        git_remote_name: "origin".to_string(),
        base_branch: "main".to_string(),
        preset: Default::default(),
        strategy_prompt: "Validate baseline execution".to_string(),
        workdir: ".".to_string(),
        prepare_command: None,
        run_command: "printf '{\"score\":1}\\n' > summary.json && echo benchmark-ok".to_string(),
        mutable_paths: vec!["app.txt".to_string()],
        fixed_paths: Vec::new(),
        primary_metric: ExperimentMetricDefinition {
            name: "score".to_string(),
            regex: None,
            json_path: Some("score".to_string()),
            comparator: ExperimentMetricComparator::HigherIsBetter,
        },
        secondary_metrics: Vec::new(),
        comparison_policy: Default::default(),
        stop_policy: Default::default(),
        default_runner_profile_id: None,
        promotion_mode: "manual".to_string(),
        autonomy_mode: Default::default(),
        status: ExperimentProjectStatus::Ready,
        created_at: now,
        updated_at: now,
    };
    store
        .create_experiment_project(&project)
        .await
        .expect("store project");

    let runner = ExperimentRunnerProfile {
        id: Uuid::new_v4(),
        name: "local-docker".to_string(),
        backend: ExperimentRunnerBackend::LocalDocker,
        backend_config: serde_json::json!({}),
        image_or_runtime: Some("alpine:3.20".to_string()),
        gpu_requirements: serde_json::json!({}),
        env_grants: serde_json::json!({}),
        secret_references: Vec::new(),
        cache_policy: serde_json::json!({}),
        status: ExperimentRunnerStatus::Validated,
        readiness_class: crate::experiments::ExperimentRunnerReadinessClass::LaunchReady,
        launch_eligible: true,
        created_at: now,
        updated_at: now,
    };
    store
        .create_experiment_runner_profile(&runner)
        .await
        .expect("store runner");

    let campaign_id = Uuid::new_v4();
    let worktree_path = super::experiments_worktree_path(&project.workspace_path, campaign_id);
    let campaign = ExperimentCampaign {
        id: campaign_id,
        project_id: project.id,
        runner_profile_id: runner.id,
        owner_user_id: "owner-a".to_string(),
        status: ExperimentCampaignStatus::PendingBaseline,
        baseline_commit: None,
        best_commit: None,
        best_metrics: serde_json::json!({}),
        experiment_branch: Some(format!(
            "codex/experiments/{}",
            super::short_id(campaign_id)
        )),
        remote_ref: None,
        worktree_path: Some(worktree_path.to_string_lossy().to_string()),
        started_at: Some(now),
        ended_at: None,
        trial_count: 0,
        failure_count: 0,
        pause_reason: Some("Pending baseline launch.".to_string()),
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
    };
    store
        .create_experiment_campaign(&campaign)
        .await
        .expect("store campaign");

    let response =
        super::launch_campaign_baseline(&store, "owner-a", &settings, &project, &runner, campaign)
            .await
            .expect("launch baseline");

    let trial = response.trial.expect("trial should be recorded");
    assert_eq!(trial.exit_code, Some(0));
    assert_eq!(trial.metrics_json["score"], 1.0);
    let worktree_path = Path::new(
        response
            .campaign
            .worktree_path
            .as_deref()
            .expect("worktree path"),
    );
    let summary_path = Path::new(
        trial.artifact_manifest_json["summary_json_path"]
            .as_str()
            .expect("summary json path"),
    );
    assert!(summary_path.exists(), "summary.json should exist");
    assert!(
        !summary_path.starts_with(worktree_path),
        "summary artifact should be persisted outside the campaign worktree"
    );
    let run_log =
        std::fs::read_to_string(trial.log_preview_path.as_deref().expect("log preview path"))
            .expect("read run log");
    assert!(
        run_log.contains("benchmark-ok"),
        "unexpected run log: {run_log}"
    );
    assert!(worktree_path.exists(), "campaign worktree should exist");
    assert!(
        super::git_changed_files(&worktree_path.to_string_lossy())
            .await
            .expect("list changed files after baseline")
            .is_empty(),
        "baseline run should restore the campaign worktree to a clean state"
    );
}

#[tokio::test]
async fn agent_env_terminal_bench_completion_writes_metrics_and_artifact() {
    let dir = TempDir::new().expect("tempdir");
    let run_root = dir.path().join("run");
    let artifact_dir = dir.path().join("artifacts");
    std::fs::create_dir_all(&run_root).expect("run root");
    std::fs::create_dir_all(&artifact_dir).expect("artifact root");
    let log_path = dir.path().join("bench.log");
    let now = Utc::now();
    let trial = ExperimentTrial {
        id: Uuid::new_v4(),
        campaign_id: Uuid::new_v4(),
        sequence: 1,
        candidate_commit: None,
        parent_best_commit: None,
        status: ExperimentTrialStatus::Running,
        runner_backend: ExperimentRunnerBackend::LocalDocker,
        exit_code: None,
        metrics_json: serde_json::json!({}),
        summary: None,
        decision_reason: None,
        artifact_manifest_json: serde_json::json!({}),
        log_preview_path: None,
        reviewer_decision: None,
        runtime_ms: None,
        attributed_cost_usd: None,
        llm_cost_usd: None,
        runner_cost_usd: None,
        hypothesis: None,
        mutation_summary: None,
        provider_job_id: None,
        provider_job_metadata: serde_json::json!({}),
        started_at: Some(now),
        completed_at: None,
        created_at: now,
        updated_at: now,
    };

    let completion = super::execute_agent_env_benchmark_trial(
        super::AgentEnvBenchmarkConfig::TerminalBench {
            live_agent: false,
            cases: vec![crate::agent::env::TerminalBenchCase {
                name: "echo".to_string(),
                command: "printf agent-env-ok".to_string(),
                cwd: None,
                expected_stdout_contains: vec!["agent-env-ok".to_string()],
                expected_exit_code: Some(0),
                timeout_secs: 5,
            }],
        },
        &run_root,
        std::time::Instant::now(),
        &log_path,
        &artifact_dir,
        &trial,
    )
    .await
    .expect("agent env benchmark completion");

    assert_eq!(completion.exit_code, Some(0));
    assert_eq!(completion.metrics_json["score"], 1.0);
    assert_eq!(
        completion.artifact_manifest_json["stage"],
        serde_json::json!("agent_env_benchmark")
    );
    assert_eq!(
        completion.artifact_manifest_json["trajectory_summary"]["env_names"][0],
        serde_json::json!("terminal_bench")
    );
    assert_eq!(
        completion.artifact_manifest_json["trajectory_summary"]["token_capture_steps"],
        serde_json::json!(1)
    );
    let trajectory_path = Path::new(
        completion.artifact_manifest_json["trajectory_json_path"]
            .as_str()
            .expect("trajectory path"),
    );
    assert!(trajectory_path.exists());
    let log = std::fs::read_to_string(log_path).expect("read log");
    assert!(log.contains("agent-env-ok"));
}

#[tokio::test]
async fn agent_env_skill_bench_completion_writes_metrics_and_artifact() {
    let dir = TempDir::new().expect("tempdir");
    let run_root = dir.path().join("run");
    let artifact_dir = dir.path().join("artifacts");
    std::fs::create_dir_all(&run_root).expect("run root");
    std::fs::create_dir_all(&artifact_dir).expect("artifact root");
    let log_path = dir.path().join("skill-bench.log");
    let now = Utc::now();
    let trial = ExperimentTrial {
        id: Uuid::new_v4(),
        campaign_id: Uuid::new_v4(),
        sequence: 1,
        candidate_commit: None,
        parent_best_commit: None,
        status: ExperimentTrialStatus::Running,
        runner_backend: ExperimentRunnerBackend::LocalDocker,
        exit_code: None,
        metrics_json: serde_json::json!({}),
        summary: None,
        decision_reason: None,
        artifact_manifest_json: serde_json::json!({}),
        log_preview_path: None,
        reviewer_decision: None,
        runtime_ms: None,
        attributed_cost_usd: None,
        llm_cost_usd: None,
        runner_cost_usd: None,
        hypothesis: None,
        mutation_summary: None,
        provider_job_id: None,
        provider_job_metadata: serde_json::json!({}),
        started_at: Some(now),
        completed_at: None,
        created_at: now,
        updated_at: now,
    };

    let completion = super::execute_agent_env_benchmark_trial(
        super::AgentEnvBenchmarkConfig::SkillBench {
            live_agent: false,
            cases: vec![crate::agent::env::SkillBenchCase {
                name: "minimal-skill".to_string(),
                skill_content: "# Skill\n\nUse this skill carefully.".to_string(),
                required_substrings: vec!["carefully".to_string()],
            }],
        },
        &run_root,
        std::time::Instant::now(),
        &log_path,
        &artifact_dir,
        &trial,
    )
    .await
    .expect("agent env skill benchmark completion");

    assert_eq!(completion.exit_code, Some(0));
    assert_eq!(completion.metrics_json["score"], 1.0);
    assert_eq!(
        completion.artifact_manifest_json["stage"],
        serde_json::json!("agent_env_benchmark")
    );
    assert_eq!(
        completion.artifact_manifest_json["trajectory_summary"]["env_names"][0],
        serde_json::json!("skill_bench")
    );
    let trajectory_path = Path::new(
        completion.artifact_manifest_json["trajectory_json_path"]
            .as_str()
            .expect("trajectory path"),
    );
    assert!(trajectory_path.exists());
    let trajectory_json = std::fs::read_to_string(trajectory_path).expect("read trajectory json");
    assert!(trajectory_json.contains("skill_bench"));
    let log = std::fs::read_to_string(log_path).expect("read log");
    assert!(log.contains("minimal-skill"));
}

#[tokio::test]
async fn local_trial_artifact_refs_include_agent_env_paths() {
    let (store, _guard) = crate::testing::test_db().await;
    let dir = TempDir::new().expect("tempdir");
    let trajectory_path = dir.path().join("trajectory.json");
    let log_path = dir.path().join("trial.log");
    std::fs::write(&trajectory_path, "[]").expect("write trajectory");
    std::fs::write(&log_path, "log").expect("write log");
    let now = Utc::now();
    let project = ExperimentProject {
        id: Uuid::new_v4(),
        name: "artifact-ref-project".to_string(),
        workspace_path: dir.path().to_string_lossy().to_string(),
        git_remote_name: "origin".to_string(),
        base_branch: "main".to_string(),
        preset: Default::default(),
        strategy_prompt: "Verify artifact refs".to_string(),
        workdir: ".".to_string(),
        prepare_command: None,
        run_command: "true".to_string(),
        mutable_paths: Vec::new(),
        fixed_paths: Vec::new(),
        primary_metric: ExperimentMetricDefinition {
            name: "score".to_string(),
            regex: None,
            json_path: Some("score".to_string()),
            comparator: ExperimentMetricComparator::HigherIsBetter,
        },
        secondary_metrics: Vec::new(),
        comparison_policy: Default::default(),
        stop_policy: Default::default(),
        default_runner_profile_id: None,
        promotion_mode: "manual".to_string(),
        autonomy_mode: Default::default(),
        status: ExperimentProjectStatus::Ready,
        created_at: now,
        updated_at: now,
    };
    store
        .create_experiment_project(&project)
        .await
        .expect("store project");
    let runner = ExperimentRunnerProfile {
        id: Uuid::new_v4(),
        name: "artifact-ref-runner".to_string(),
        backend: ExperimentRunnerBackend::AgentEnv,
        backend_config: serde_json::json!({}),
        image_or_runtime: None,
        gpu_requirements: serde_json::json!({}),
        env_grants: serde_json::json!({}),
        secret_references: Vec::new(),
        cache_policy: serde_json::json!({}),
        status: ExperimentRunnerStatus::Validated,
        readiness_class: crate::experiments::ExperimentRunnerReadinessClass::LaunchReady,
        launch_eligible: true,
        created_at: now,
        updated_at: now,
    };
    store
        .create_experiment_runner_profile(&runner)
        .await
        .expect("store runner");
    let campaign = ExperimentCampaign {
        id: Uuid::new_v4(),
        project_id: project.id,
        runner_profile_id: runner.id,
        owner_user_id: "owner-a".to_string(),
        status: ExperimentCampaignStatus::Running,
        baseline_commit: None,
        best_commit: None,
        best_metrics: serde_json::json!({}),
        experiment_branch: None,
        remote_ref: None,
        worktree_path: None,
        started_at: Some(now),
        ended_at: None,
        trial_count: 1,
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
    };
    store
        .create_experiment_campaign(&campaign)
        .await
        .expect("store campaign");
    let trial = ExperimentTrial {
        id: Uuid::new_v4(),
        campaign_id: campaign.id,
        sequence: 1,
        candidate_commit: None,
        parent_best_commit: None,
        status: ExperimentTrialStatus::Running,
        runner_backend: ExperimentRunnerBackend::AgentEnv,
        exit_code: Some(0),
        metrics_json: serde_json::json!({}),
        summary: None,
        decision_reason: None,
        artifact_manifest_json: serde_json::json!({
            "trajectory_json_path": trajectory_path.to_string_lossy(),
        }),
        log_preview_path: Some(log_path.to_string_lossy().to_string()),
        reviewer_decision: None,
        runtime_ms: None,
        attributed_cost_usd: None,
        llm_cost_usd: None,
        runner_cost_usd: None,
        hypothesis: None,
        mutation_summary: None,
        provider_job_id: None,
        provider_job_metadata: serde_json::json!({}),
        started_at: Some(now),
        completed_at: Some(now),
        created_at: now,
        updated_at: now,
    };
    store
        .create_experiment_trial(&trial)
        .await
        .expect("store trial");

    super::upsert_local_trial_artifact_refs(&store, &trial)
        .await
        .expect("upsert local refs");
    let artifacts = store
        .list_experiment_artifacts(trial.id)
        .await
        .expect("list refs");
    let kinds = artifacts
        .iter()
        .map(|artifact| artifact.kind.as_str())
        .collect::<std::collections::HashSet<_>>();
    assert!(kinds.contains("trajectory_json"));
    assert!(kinds.contains("log_preview"));
}

// Flaky under the main-only `--all-features --lib` coverage job: this heavy
// autonomous Docker E2E (planner -> mutator -> reviewer -> two real
// local-Docker trials over a git worktree) intermittently fails with
// `Internal("No such file or directory (os error 2)")` when a worktree git
// op spawns after the worktree path has gone missing mid-trial — a timing
// race that only manifests under heavy parallel CI load and passes on a
// plain re-run. It is not deterministically reproducible outside that
// environment, so it is run explicitly (`cargo test -- --ignored`) rather
// than gating CI. The simpler `launch_campaign_baseline_runs_local_docker_
// trial_end_to_end` keeps the Docker-trial path under continuous coverage.
#[tokio::test]
#[ignore = "flaky worktree/Docker race under parallel CI; run explicitly with --ignored"]
async fn autonomous_campaign_runs_planner_mutator_reviewer_and_docker_trial_end_to_end() {
    let mut settings = crate::settings::Settings::default();
    settings.sandbox.enabled = true;
    let sandbox = crate::sandbox::SandboxManager::new(super::experiment_sandbox_config(&settings));
    if !sandbox.is_available().await {
        eprintln!(
            "skipping autonomous docker-backed experiment test because sandbox is unavailable"
        );
        return;
    }

    ensure_test_research_subagent_executor().await;

    let (store, _guard) = crate::testing::test_db().await;
    let repo = TempDir::new().expect("temp repo");
    git(repo.path(), &["init"]);
    git(repo.path(), &["checkout", "-b", "main"]);
    git(repo.path(), &["config", "user.email", "tests@example.com"]);
    git(repo.path(), &["config", "user.name", "ThinClaw Tests"]);
    std::fs::write(repo.path().join("app.txt"), "baseline\n").expect("write repo file");
    git(repo.path(), &["add", "app.txt"]);
    git(repo.path(), &["commit", "-m", "initial"]);

    let now = Utc::now();
    let project = ExperimentProject {
        id: Uuid::new_v4(),
        name: "autonomous-docker".to_string(),
        workspace_path: repo.path().to_string_lossy().to_string(),
        git_remote_name: "origin".to_string(),
        base_branch: "main".to_string(),
        preset: Default::default(),
        strategy_prompt: "Autonomously improve app.txt".to_string(),
        workdir: ".".to_string(),
        prepare_command: None,
        run_command: "if grep -q candidate app.txt; then printf '{\"score\":2}\\n' > summary.json && echo improved; else printf '{\"score\":1}\\n' > summary.json && echo baseline; fi".to_string(),
        mutable_paths: vec!["app.txt".to_string()],
        fixed_paths: Vec::new(),
        primary_metric: ExperimentMetricDefinition {
            name: "score".to_string(),
            regex: None,
            json_path: Some("score".to_string()),
            comparator: ExperimentMetricComparator::HigherIsBetter,
        },
        secondary_metrics: Vec::new(),
        comparison_policy: Default::default(),
        stop_policy: Default::default(),
        default_runner_profile_id: None,
        promotion_mode: "manual".to_string(),
        autonomy_mode: ExperimentAutonomyMode::Autonomous,
        status: ExperimentProjectStatus::Ready,
        created_at: now,
        updated_at: now,
    };
    store
        .create_experiment_project(&project)
        .await
        .expect("store project");

    let runner = ExperimentRunnerProfile {
        id: Uuid::new_v4(),
        name: "local-docker".to_string(),
        backend: ExperimentRunnerBackend::LocalDocker,
        backend_config: serde_json::json!({}),
        image_or_runtime: Some("alpine:3.20".to_string()),
        gpu_requirements: serde_json::json!({}),
        env_grants: serde_json::json!({}),
        secret_references: Vec::new(),
        cache_policy: serde_json::json!({}),
        status: ExperimentRunnerStatus::Validated,
        readiness_class: crate::experiments::ExperimentRunnerReadinessClass::LaunchReady,
        launch_eligible: true,
        created_at: now,
        updated_at: now,
    };
    store
        .create_experiment_runner_profile(&runner)
        .await
        .expect("store runner");

    let campaign_id = Uuid::new_v4();
    let worktree_path = super::experiments_worktree_path(&project.workspace_path, campaign_id);
    let campaign = ExperimentCampaign {
        id: campaign_id,
        project_id: project.id,
        runner_profile_id: runner.id,
        owner_user_id: "owner-a".to_string(),
        status: ExperimentCampaignStatus::PendingBaseline,
        baseline_commit: None,
        best_commit: None,
        best_metrics: serde_json::json!({}),
        experiment_branch: Some(format!(
            "codex/experiments/{}",
            super::short_id(campaign_id)
        )),
        remote_ref: None,
        worktree_path: Some(worktree_path.to_string_lossy().to_string()),
        started_at: Some(now),
        ended_at: None,
        trial_count: 0,
        failure_count: 0,
        pause_reason: Some("Pending baseline launch.".to_string()),
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
    };
    store
        .create_experiment_campaign(&campaign)
        .await
        .expect("store campaign");

    let baseline_response =
        super::launch_campaign_baseline(&store, "owner-a", &settings, &project, &runner, campaign)
            .await
            .expect("launch baseline");
    let baseline_trial = baseline_response.trial.expect("baseline trial");
    assert_eq!(baseline_trial.metrics_json["score"], 1.0);

    let mut active_campaign = baseline_response.campaign;
    assert!(
        super::git_changed_files(
            active_campaign
                .worktree_path
                .as_deref()
                .expect("campaign worktree path"),
        )
        .await
        .expect("list changed files after baseline")
        .is_empty(),
        "baseline run should leave the campaign worktree clean before autonomous mutation"
    );
    super::launch_next_trial_if_ready(
        &store,
        "owner-a",
        &settings,
        &project,
        &runner,
        &mut active_campaign,
    )
    .await
    .expect("autonomous follow-up trial should succeed");

    let trials = store
        .list_experiment_trials(active_campaign.id)
        .await
        .expect("list trials");
    assert_eq!(
        trials.len(),
        2,
        "unexpected trial count; campaign_status={:?}; pause_reason={:?}; metadata={}",
        active_campaign.status,
        active_campaign.pause_reason,
        active_campaign.metadata
    );

    let autonomous_trial = trials.last().expect("autonomous trial");
    assert_eq!(autonomous_trial.sequence, 2);
    assert_eq!(autonomous_trial.status, ExperimentTrialStatus::Accepted);
    assert_eq!(autonomous_trial.metrics_json["score"], 2.0);
    assert_eq!(
        autonomous_trial.reviewer_decision.as_deref(),
        Some("approved")
    );
    assert_eq!(
        autonomous_trial.mutation_summary.as_deref(),
        Some("Updated app.txt to the candidate configuration.")
    );
    assert_ne!(
        autonomous_trial.candidate_commit,
        baseline_trial.candidate_commit
    );

    let run_artifacts = autonomous_trial
        .artifact_manifest_json
        .get("run_artifacts")
        .and_then(|value| value.as_array())
        .expect("run artifacts should be present");
    let sources = run_artifacts
        .iter()
        .filter_map(|artifact| artifact.get("source").and_then(|value| value.as_str()))
        .collect::<Vec<_>>();
    assert!(sources.contains(&"experiment_subagent:planner"));
    assert!(sources.contains(&"experiment_subagent:mutator"));
    assert!(sources.contains(&"experiment_subagent:reviewer"));
    assert!(sources.contains(&"experiment_runner"));

    assert_eq!(active_campaign.best_metrics["score"], 2.0);
    assert!(
        super::git_changed_files(
            active_campaign
                .worktree_path
                .as_deref()
                .expect("campaign worktree path"),
        )
        .await
        .expect("list changed files after autonomous trial")
        .is_empty(),
        "autonomous trial should also leave the campaign worktree clean"
    );
}

#[test]
fn normalize_trial_completion_adds_default_stage() {
    let completion = ExperimentRunnerCompletion {
        exit_code: Some(0),
        metrics_json: serde_json::json!({}),
        summary: Some("ok".to_string()),
        runtime_ms: Some(42),
        attributed_cost_usd: None,
        log_preview_path: None,
        artifact_manifest_json: serde_json::Value::Null,
    };

    let normalized = super::normalize_trial_completion(completion);
    assert_eq!(
        normalized
            .artifact_manifest_json
            .get("stage")
            .and_then(|value| value.as_str()),
        Some("complete")
    );
}

#[tokio::test]
async fn complete_trial_terminal_rejects_repeated_completed_lease() {
    let (store, _guard) = crate::testing::test_db().await;
    let now = Utc::now();
    let project = ExperimentProject {
        id: Uuid::new_v4(),
        name: "demo".to_string(),
        workspace_path: ".".to_string(),
        git_remote_name: "origin".to_string(),
        base_branch: "main".to_string(),
        preset: Default::default(),
        strategy_prompt: "demo".to_string(),
        workdir: ".".to_string(),
        prepare_command: None,
        run_command: "echo ok".to_string(),
        mutable_paths: vec!["src".to_string()],
        fixed_paths: Vec::new(),
        primary_metric: ExperimentMetricDefinition::default(),
        secondary_metrics: Vec::new(),
        comparison_policy: Default::default(),
        stop_policy: Default::default(),
        default_runner_profile_id: None,
        promotion_mode: "manual".to_string(),
        autonomy_mode: Default::default(),
        status: ExperimentProjectStatus::Ready,
        created_at: now,
        updated_at: now,
    };
    let mut campaign = ExperimentCampaign {
        id: Uuid::new_v4(),
        project_id: project.id,
        runner_profile_id: Uuid::new_v4(),
        owner_user_id: "owner-a".to_string(),
        status: ExperimentCampaignStatus::Running,
        baseline_commit: None,
        best_commit: None,
        best_metrics: serde_json::json!({}),
        experiment_branch: None,
        remote_ref: None,
        worktree_path: None,
        started_at: Some(now),
        ended_at: None,
        trial_count: 1,
        failure_count: 0,
        pause_reason: None,
        queue_state: ExperimentCampaignQueueState::Active,
        queue_position: 0,
        active_trial_id: Some(Uuid::new_v4()),
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
    };
    let mut trial = ExperimentTrial {
        id: Uuid::new_v4(),
        campaign_id: campaign.id,
        sequence: 1,
        candidate_commit: None,
        parent_best_commit: None,
        status: ExperimentTrialStatus::Running,
        runner_backend: ExperimentRunnerBackend::GenericRemoteRunner,
        exit_code: None,
        metrics_json: serde_json::json!({}),
        summary: None,
        decision_reason: None,
        artifact_manifest_json: serde_json::json!({}),
        log_preview_path: None,
        reviewer_decision: None,
        runtime_ms: None,
        attributed_cost_usd: None,
        llm_cost_usd: None,
        runner_cost_usd: None,
        hypothesis: None,
        mutation_summary: None,
        provider_job_id: None,
        provider_job_metadata: serde_json::json!({}),
        started_at: Some(now),
        completed_at: None,
        created_at: now,
        updated_at: now,
    };
    let mut lease = ExperimentLease {
        id: Uuid::new_v4(),
        campaign_id: campaign.id,
        trial_id: trial.id,
        runner_profile_id: campaign.runner_profile_id,
        status: ExperimentLeaseStatus::Completed,
        token_hash: "hash".to_string(),
        job_payload: serde_json::json!({}),
        credentials_payload: serde_json::json!({}),
        expires_at: now,
        claimed_at: Some(now),
        completed_at: Some(now),
        created_at: now,
        updated_at: now,
    };
    let completion = ExperimentRunnerCompletion {
        exit_code: Some(0),
        metrics_json: serde_json::json!({}),
        summary: Some("done".to_string()),
        runtime_ms: Some(1),
        attributed_cost_usd: None,
        log_preview_path: None,
        artifact_manifest_json: serde_json::json!({}),
    };

    let error = super::complete_trial_terminal(
        &store,
        &project,
        &mut campaign,
        &mut trial,
        Some(&mut lease),
        completion,
    )
    .await
    .expect_err("completed lease should reject repeated completion");

    match error {
        crate::api::error::ApiError::InvalidInput(message) => {
            assert!(message.contains("already recorded"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

fn git(repo: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(repo)
        .status()
        .expect("git command should start");
    assert!(status.success(), "git {:?} failed with {:?}", args, status);
}

/// Seed an `ExperimentProject` + `ExperimentRunnerProfile` so a campaign's
/// foreign keys (project_id, runner_profile_id) resolve, and return their ids.
async fn seed_reaper_project_and_runner(
    store: &std::sync::Arc<dyn crate::db::Database>,
    now: chrono::DateTime<Utc>,
) -> (Uuid, Uuid) {
    let project = ExperimentProject {
        id: Uuid::new_v4(),
        name: "reaper-test".to_string(),
        workspace_path: "/tmp/reaper-test".to_string(),
        git_remote_name: "origin".to_string(),
        base_branch: "main".to_string(),
        preset: Default::default(),
        strategy_prompt: "reaper test".to_string(),
        workdir: ".".to_string(),
        prepare_command: None,
        run_command: "true".to_string(),
        mutable_paths: Vec::new(),
        fixed_paths: Vec::new(),
        primary_metric: ExperimentMetricDefinition {
            name: "score".to_string(),
            regex: None,
            json_path: Some("score".to_string()),
            comparator: ExperimentMetricComparator::HigherIsBetter,
        },
        secondary_metrics: Vec::new(),
        comparison_policy: Default::default(),
        stop_policy: Default::default(),
        default_runner_profile_id: None,
        promotion_mode: "manual".to_string(),
        autonomy_mode: Default::default(),
        status: ExperimentProjectStatus::Ready,
        created_at: now,
        updated_at: now,
    };
    store
        .create_experiment_project(&project)
        .await
        .expect("seed project");

    let runner = ExperimentRunnerProfile {
        id: Uuid::new_v4(),
        name: "reaper-runner".to_string(),
        backend: ExperimentRunnerBackend::GenericRemoteRunner,
        backend_config: serde_json::json!({}),
        image_or_runtime: None,
        gpu_requirements: serde_json::json!({}),
        env_grants: serde_json::json!({}),
        secret_references: Vec::new(),
        cache_policy: serde_json::json!({}),
        status: ExperimentRunnerStatus::Validated,
        readiness_class: crate::experiments::ExperimentRunnerReadinessClass::LaunchReady,
        launch_eligible: true,
        created_at: now,
        updated_at: now,
    };
    store
        .create_experiment_runner_profile(&runner)
        .await
        .expect("seed runner");

    (project.id, runner.id)
}

fn reaper_test_trial(campaign_id: Uuid, now: chrono::DateTime<Utc>) -> ExperimentTrial {
    ExperimentTrial {
        id: Uuid::new_v4(),
        campaign_id,
        sequence: 1,
        candidate_commit: None,
        parent_best_commit: None,
        status: ExperimentTrialStatus::Accepted,
        runner_backend: ExperimentRunnerBackend::GenericRemoteRunner,
        exit_code: Some(0),
        metrics_json: serde_json::json!({}),
        summary: None,
        decision_reason: None,
        artifact_manifest_json: serde_json::json!({}),
        log_preview_path: None,
        reviewer_decision: None,
        runtime_ms: Some(1),
        attributed_cost_usd: None,
        llm_cost_usd: None,
        runner_cost_usd: None,
        hypothesis: None,
        mutation_summary: None,
        provider_job_id: None,
        provider_job_metadata: serde_json::json!({}),
        started_at: Some(now),
        completed_at: Some(now),
        created_at: now,
        updated_at: now,
    }
}

fn reaper_test_campaign(
    project_id: Uuid,
    runner_profile_id: Uuid,
    now: chrono::DateTime<Utc>,
) -> super::ExperimentCampaign {
    super::ExperimentCampaign {
        id: Uuid::new_v4(),
        project_id,
        runner_profile_id,
        owner_user_id: "default".to_string(),
        status: ExperimentCampaignStatus::Completed,
        baseline_commit: None,
        best_commit: None,
        best_metrics: serde_json::json!({}),
        experiment_branch: None,
        remote_ref: None,
        worktree_path: None,
        started_at: Some(now),
        ended_at: Some(now),
        trial_count: 1,
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

#[tokio::test]
async fn reap_expired_artifacts_removes_only_expired() {
    let (store, _guard) = crate::testing::test_db().await;
    let now = Utc::now();
    let (project_id, runner_id) = seed_reaper_project_and_runner(&store, now).await;
    let campaign = reaper_test_campaign(project_id, runner_id, now);
    store
        .create_experiment_campaign(&campaign)
        .await
        .expect("store campaign");
    let trial = reaper_test_trial(campaign.id, now);
    store
        .create_experiment_trial(&trial)
        .await
        .expect("store trial");

    let stale = crate::experiments::ExperimentArtifactRef {
        id: Uuid::new_v4(),
        trial_id: trial.id,
        kind: "run_log".to_string(),
        uri_or_local_path: "/pod/run.log".to_string(),
        size_bytes: Some(3),
        fetchable: false,
        metadata: serde_json::json!({}),
        created_at: now - chrono::Duration::days(40),
    };
    let fresh = crate::experiments::ExperimentArtifactRef {
        id: Uuid::new_v4(),
        trial_id: trial.id,
        kind: "summary_json".to_string(),
        uri_or_local_path: "/pod/summary.json".to_string(),
        size_bytes: Some(2),
        fetchable: false,
        metadata: serde_json::json!({}),
        created_at: now,
    };
    store
        .replace_experiment_artifacts(trial.id, &[stale.clone(), fresh.clone()])
        .await
        .expect("seed artifacts");

    let pruned = super::reap_expired_artifacts_once(&store, 30)
        .await
        .expect("reaper pass");
    assert_eq!(pruned, 1, "exactly the stale artifact should be pruned");

    let remaining = store
        .list_experiment_artifacts(trial.id)
        .await
        .expect("list artifacts");
    assert_eq!(remaining.len(), 1);
    assert_eq!(
        remaining[0].id, fresh.id,
        "only the fresh artifact survives"
    );
}

#[tokio::test]
async fn reap_expired_artifacts_disabled_when_retention_zero() {
    let (store, _guard) = crate::testing::test_db().await;
    let now = Utc::now();
    let (project_id, runner_id) = seed_reaper_project_and_runner(&store, now).await;
    let campaign = reaper_test_campaign(project_id, runner_id, now);
    store
        .create_experiment_campaign(&campaign)
        .await
        .expect("store campaign");
    let trial = reaper_test_trial(campaign.id, now);
    store
        .create_experiment_trial(&trial)
        .await
        .expect("store trial");
    let ancient = crate::experiments::ExperimentArtifactRef {
        id: Uuid::new_v4(),
        trial_id: trial.id,
        kind: "run_log".to_string(),
        uri_or_local_path: "/pod/run.log".to_string(),
        size_bytes: Some(3),
        fetchable: false,
        metadata: serde_json::json!({}),
        created_at: now - chrono::Duration::days(3650),
    };
    store
        .replace_experiment_artifacts(trial.id, &[ancient])
        .await
        .expect("seed artifact");

    let pruned = super::reap_expired_artifacts_once(&store, 0)
        .await
        .expect("reaper pass");
    assert_eq!(pruned, 0, "retention_days=0 disables reaping");
    let remaining = store
        .list_experiment_artifacts(trial.id)
        .await
        .expect("list artifacts");
    assert_eq!(remaining.len(), 1, "nothing pruned when disabled");
}

#[tokio::test]
async fn lease_artifact_with_inline_bytes_persists_durable_fetchable_ref() {
    let (store, _guard) = crate::testing::test_db().await;
    store
        .set_setting(
            "default",
            "experiments.enabled",
            &serde_json::Value::Bool(true),
        )
        .await
        .expect("enable experiments");

    let now = Utc::now();
    let (project_id, runner_id) = seed_reaper_project_and_runner(&store, now).await;
    let campaign = reaper_test_campaign(project_id, runner_id, now);
    store
        .create_experiment_campaign(&campaign)
        .await
        .expect("store campaign");
    let trial = reaper_test_trial(campaign.id, now);
    store
        .create_experiment_trial(&trial)
        .await
        .expect("store trial");

    let token = "durable-artifact-token";
    let lease = ExperimentLease {
        id: Uuid::new_v4(),
        campaign_id: campaign.id,
        trial_id: trial.id,
        runner_profile_id: campaign.runner_profile_id,
        status: ExperimentLeaseStatus::Claimed,
        token_hash: super::hash_lease_token(token),
        job_payload: serde_json::json!({}),
        credentials_payload: serde_json::json!({}),
        expires_at: now + chrono::Duration::hours(1),
        claimed_at: Some(now),
        completed_at: None,
        created_at: now,
        updated_at: now,
    };
    store
        .create_experiment_lease(&lease)
        .await
        .expect("store lease");

    let artifact_dir = TempDir::new().expect("tempdir");
    let artifact_store: Arc<dyn super::ArtifactStore> = Arc::new(
        crate::experiments::LocalArtifactStore::new(artifact_dir.path()),
    );
    let payload = b"benchmark-ok\nscore=1\n";
    let upload = crate::experiments::ExperimentRunnerArtifactUpload {
        kind: "run_log".to_string(),
        uri_or_local_path: "/pod/run.log".to_string(),
        size_bytes: Some(payload.len() as u64),
        fetchable: false,
        metadata: serde_json::json!({}),
        content_base64: Some(base64::engine::general_purpose::STANDARD.encode(payload)),
    };

    super::lease_artifact_with_store(&store, &artifact_store, "default", lease.id, token, upload)
        .await
        .expect("lease artifact");

    let artifacts = store
        .list_experiment_artifacts(trial.id)
        .await
        .expect("list artifacts");
    assert_eq!(artifacts.len(), 1);
    let recorded = &artifacts[0];
    assert!(recorded.fetchable, "durable artifact should be fetchable");
    let durable_path = Path::new(&recorded.uri_or_local_path);
    assert!(durable_path.exists(), "durable artifact path should exist");
    assert!(
        durable_path.starts_with(artifact_dir.path()),
        "artifact must live under the durable root"
    );
    let bytes = std::fs::read(durable_path).expect("read durable artifact");
    assert_eq!(bytes, payload);
}
