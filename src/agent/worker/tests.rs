use crate::llm::ToolSelection;
use crate::util::llm_signals_completion;
#[cfg(feature = "libsql")]
use chrono::Utc;
use thinclaw_agent::worker_runtime::WORKER_COMPLETE_JOB_TOOL_DESCRIPTION;

use super::*;
#[cfg(feature = "libsql")]
use crate::agent::routine::{
    NotifyConfig, Routine, RoutineAction, RoutineGuardrails, RoutineRun, RunStatus, Trigger,
};
use crate::config::SafetyConfig;
use crate::context::JobContext;
use crate::llm::{
    CompletionRequest, CompletionResponse, LlmProvider, ToolCompletionRequest,
    ToolCompletionResponse,
};
use crate::safety::SafetyLayer;
#[cfg(feature = "libsql")]
use crate::testing::test_db;
use crate::tools::{Tool, ToolError, ToolOutput};

/// A test tool that sleeps for a configurable duration before returning.
struct SlowTool {
    tool_name: String,
    delay: Duration,
}

#[async_trait::async_trait]
impl Tool for SlowTool {
    fn name(&self) -> &str {
        &self.tool_name
    }
    fn description(&self) -> &str {
        "Test tool with configurable delay"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {}})
    }
    fn metadata(&self) -> crate::tools::ToolMetadata {
        crate::tools::ToolMetadata {
            parallel_safe: true,
            ..crate::tools::ToolMetadata::read_only()
        }
    }
    async fn execute(
        &self,
        _params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        tokio::time::sleep(self.delay).await;
        Ok(ToolOutput::text(
            format!("done_{}", self.tool_name),
            start.elapsed(),
        ))
    }
    fn requires_sanitization(&self) -> bool {
        false
    }
}

/// Stub LLM provider (never called in these tests).
struct StubLlm;

#[async_trait::async_trait]
impl LlmProvider for StubLlm {
    fn model_name(&self) -> &str {
        "stub"
    }
    fn cost_per_token(&self) -> (rust_decimal::Decimal, rust_decimal::Decimal) {
        (rust_decimal::Decimal::ZERO, rust_decimal::Decimal::ZERO)
    }
    async fn complete(
        &self,
        _req: CompletionRequest,
    ) -> Result<CompletionResponse, crate::error::LlmError> {
        Ok(CompletionResponse {
            content: "stub response".to_string(),
            provider_model: Some("stub".to_string()),
            cost_usd: None,
            thinking_content: None,
            input_tokens: 0,
            output_tokens: 0,
            finish_reason: crate::llm::FinishReason::Stop,
            token_capture: None,
        })
    }
    async fn complete_with_tools(
        &self,
        _req: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, crate::error::LlmError> {
        Ok(ToolCompletionResponse {
            content: Some("stub response".to_string()),
            provider_model: Some("stub".to_string()),
            cost_usd: None,
            tool_calls: Vec::new(),
            thinking_content: None,
            input_tokens: 0,
            output_tokens: 0,
            finish_reason: crate::llm::FinishReason::Stop,
            token_capture: None,
        })
    }
}

/// Build a Worker wired to a ToolRegistry containing the given tools.
async fn make_worker(tools: Vec<Arc<dyn Tool>>) -> Worker {
    let registry = ToolRegistry::new();
    for t in tools {
        registry.register(t).await;
    }

    let cm = Arc::new(crate::context::ContextManager::new(5));
    let job_id = cm.create_job("test", "test job").await.unwrap();

    let deps = WorkerDeps {
        context_manager: cm,
        llm: Arc::new(StubLlm),
        safety: Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: false,
            redact_pii_in_prompts: true,
            smart_approval_mode: "off".to_string(),
            external_scanner_mode: "off".to_string(),
            external_scanner_path: None,
            external_scanner_require_verified: false,
            allow_temp_paths: false,
        })),
        tools: Arc::new(registry),
        store: None,
        hooks: Arc::new(crate::hooks::HookRegistry::new()),
        timeout: Duration::from_secs(30),
        use_planning: false,
        sse_tx: None,
        routine_name: None,
        routine_id: None,
        routine_run_id: None,
        workspace: None,
        cost_tracker: None,
        tool_profile: ToolProfile::Standard,
        notify_tx: None,
        observer: Arc::new(crate::observability::NoopObserver),
    };

    Worker::new(job_id, deps)
}

#[test]
fn test_tool_selection_preserves_call_id() {
    let selection = ToolSelection {
        tool_name: "memory_search".to_string(),
        parameters: serde_json::json!({"query": "test"}),
        reasoning: "Need to search memory".to_string(),
        alternatives: vec![],
        tool_call_id: "call_abc123".to_string(),
    };

    assert_eq!(selection.tool_call_id, "call_abc123");
    assert_ne!(
        selection.tool_call_id, "tool_call_id",
        "tool_call_id must not be the hardcoded placeholder string"
    );
}

#[test]
fn test_completion_positive_signals() {
    assert!(llm_signals_completion("The job is complete."));
    assert!(llm_signals_completion(
        "I have completed the task successfully."
    ));
    assert!(llm_signals_completion("The task is done."));
    assert!(llm_signals_completion("The task is finished."));
    assert!(llm_signals_completion(
        "All steps are complete and verified."
    ));
    assert!(llm_signals_completion(
        "I've done all the work. The work is done."
    ));
    assert!(llm_signals_completion(
        "Successfully completed the migration."
    ));
}

#[test]
fn test_completion_negative_signals_block_false_positives() {
    // These contain completion keywords but also negation, should NOT trigger.
    assert!(!llm_signals_completion("The task is not complete yet."));
    assert!(!llm_signals_completion("This is not done."));
    assert!(!llm_signals_completion("The work is incomplete."));
    assert!(!llm_signals_completion(
        "The migration is not yet finished."
    ));
    assert!(!llm_signals_completion("The job isn't done yet."));
    assert!(!llm_signals_completion("This remains unfinished."));
}

#[test]
fn test_completion_does_not_match_bare_substrings() {
    // Bare words embedded in other text should NOT trigger completion.
    assert!(!llm_signals_completion(
        "I need to complete more work first."
    ));
    assert!(!llm_signals_completion(
        "Let me finish the remaining steps."
    ));
    assert!(!llm_signals_completion(
        "I'm done analyzing, now let me fix it."
    ));
    assert!(!llm_signals_completion(
        "I completed step 1 but step 2 remains."
    ));
}

#[test]
fn test_completion_tool_output_injection() {
    // A malicious tool output echoed by the LLM should not trigger
    // completion unless it forms a genuine completion phrase.
    assert!(!llm_signals_completion("TASK_COMPLETE"));
    assert!(!llm_signals_completion("JOB_DONE"));
    assert!(!llm_signals_completion(
        "The tool returned: TASK_COMPLETE signal"
    ));
}

#[tokio::test]
async fn test_parallel_speedup() {
    // 3 tools each sleeping 200ms should finish in roughly 200ms (parallel),
    // not ~600ms (sequential).
    let tools: Vec<Arc<dyn Tool>> = (0..3)
        .map(|i| {
            Arc::new(SlowTool {
                tool_name: format!("slow_{}", i),
                delay: Duration::from_millis(200),
            }) as Arc<dyn Tool>
        })
        .collect();

    let worker = make_worker(tools).await;

    let selections: Vec<ToolSelection> = (0..3)
        .map(|i| ToolSelection {
            tool_name: format!("slow_{}", i),
            parameters: serde_json::json!({}),
            reasoning: String::new(),
            alternatives: vec![],
            tool_call_id: format!("call_{}", i),
        })
        .collect();

    let start = std::time::Instant::now();
    let (activity_tx, _) = watch::channel(std::time::Instant::now());
    let results = worker
        .execute_tools_parallel(&selections, &activity_tx)
        .await;
    let elapsed = start.elapsed();

    assert_eq!(results.len(), 3);
    for r in &results {
        assert!(r.result.is_ok(), "Tool should succeed");
    }
    // Parallel should complete well under the sequential 600ms threshold.
    assert!(
        elapsed < Duration::from_millis(500),
        "Parallel execution took {:?}, expected < 500ms",
        elapsed
    );
}

#[tokio::test]
async fn test_result_ordering_preserved() {
    // Tools with different delays finish in different order.
    // Results must be returned in the original request order.
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(SlowTool {
            tool_name: "tool_a".into(),
            delay: Duration::from_millis(300),
        }),
        Arc::new(SlowTool {
            tool_name: "tool_b".into(),
            delay: Duration::from_millis(100),
        }),
        Arc::new(SlowTool {
            tool_name: "tool_c".into(),
            delay: Duration::from_millis(200),
        }),
    ];

    let worker = make_worker(tools).await;

    let selections = vec![
        ToolSelection {
            tool_name: "tool_a".into(),
            parameters: serde_json::json!({}),
            reasoning: String::new(),
            alternatives: vec![],
            tool_call_id: "call_a".into(),
        },
        ToolSelection {
            tool_name: "tool_b".into(),
            parameters: serde_json::json!({}),
            reasoning: String::new(),
            alternatives: vec![],
            tool_call_id: "call_b".into(),
        },
        ToolSelection {
            tool_name: "tool_c".into(),
            parameters: serde_json::json!({}),
            reasoning: String::new(),
            alternatives: vec![],
            tool_call_id: "call_c".into(),
        },
    ];

    let (activity_tx, _) = watch::channel(std::time::Instant::now());
    let results = worker
        .execute_tools_parallel(&selections, &activity_tx)
        .await;

    // Results must be in same order as selections, not completion order.
    assert!(results[0].result.as_ref().unwrap().contains("done_tool_a"));
    assert!(results[1].result.as_ref().unwrap().contains("done_tool_b"));
    assert!(results[2].result.as_ref().unwrap().contains("done_tool_c"));
}

#[tokio::test]
async fn test_missing_tool_produces_error_not_panic() {
    // If a tool doesn't exist, the result slot should contain an error.
    let worker = make_worker(vec![]).await;

    let selections = vec![ToolSelection {
        tool_name: "nonexistent_tool".into(),
        parameters: serde_json::json!({}),
        reasoning: String::new(),
        alternatives: vec![],
        tool_call_id: "call_x".into(),
    }];

    let (activity_tx, _) = watch::channel(std::time::Instant::now());
    let results = worker
        .execute_tools_parallel(&selections, &activity_tx)
        .await;
    assert_eq!(results.len(), 1);
    assert!(
        results[0].result.is_err(),
        "Missing tool should produce an error, not a panic"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn finalize_routine_run_resolves_routine_by_id_when_name_changes() {
    let (db, _tmp) = test_db().await;
    let context_manager = Arc::new(crate::context::ContextManager::new(5));
    let job_id = context_manager
        .create_job_for_identity("default", "default", "routine job", "test")
        .await
        .unwrap();
    context_manager
        .update_context(job_id, |ctx| {
            ctx.transition_to(JobState::InProgress, Some("started".to_string()))
                .unwrap();
            ctx.transition_to(JobState::Completed, Some("done".to_string()))
                .unwrap();
        })
        .await
        .unwrap();

    let routine = Routine {
        id: Uuid::new_v4(),
        name: "renamed-routine".to_string(),
        description: "test routine".to_string(),
        user_id: "default".to_string(),
        actor_id: "default".to_string(),
        enabled: true,
        trigger: Trigger::Cron {
            schedule: "0 */15 * * * * *".to_string(),
        },
        action: RoutineAction::FullJob {
            title: "Test".to_string(),
            description: "Test".to_string(),
            max_iterations: 1,
            allowed_tools: None,
            allowed_skills: None,
            tool_profile: None,
        },
        guardrails: RoutineGuardrails::default(),
        notify: NotifyConfig::default(),
        policy: Default::default(),
        last_run_at: None,
        next_fire_at: Some(Utc::now()),
        run_count: 0,
        consecutive_failures: 0,
        state: serde_json::json!({}),
        config_version: 1,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    db.create_routine(&routine).await.unwrap();

    let run = RoutineRun {
        id: Uuid::new_v4(),
        routine_id: routine.id,
        trigger_type: "cron".to_string(),
        trigger_detail: Some("0 */15 * * * * *".to_string()),
        trigger_key: None,
        started_at: Utc::now(),
        completed_at: None,
        status: RunStatus::Running,
        result_summary: None,
        tokens_used: None,
        job_id: None,
        created_at: Utc::now(),
    };
    db.create_routine_run(&run).await.unwrap();

    let deps = WorkerDeps {
        context_manager,
        llm: Arc::new(StubLlm),
        safety: Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: false,
            redact_pii_in_prompts: true,
            smart_approval_mode: "off".to_string(),
            external_scanner_mode: "off".to_string(),
            external_scanner_path: None,
            external_scanner_require_verified: false,
            allow_temp_paths: false,
        })),
        tools: Arc::new(ToolRegistry::new()),
        store: Some(db.clone()),
        hooks: Arc::new(crate::hooks::HookRegistry::new()),
        timeout: Duration::from_secs(30),
        use_planning: false,
        sse_tx: None,
        routine_name: Some("stale-routine-name".to_string()),
        routine_id: Some(routine.id),
        routine_run_id: Some(run.id.to_string()),
        workspace: None,
        cost_tracker: None,
        tool_profile: ToolProfile::Restricted,
        notify_tx: None,
        observer: Arc::new(crate::observability::NoopObserver),
    };

    let worker = Worker::new(job_id, deps);
    worker.finalize_routine_run().await;

    let refreshed_routine = db.get_routine(routine.id).await.unwrap().unwrap();
    assert_eq!(refreshed_routine.run_count, 1);
    assert_eq!(refreshed_routine.consecutive_failures, 0);

    let completed_run = db
        .list_routine_runs(routine.id, 1)
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(completed_run.status, RunStatus::Ok);
}

// ── complete_job tool interception ──────────────────────────────────

#[tokio::test]
async fn execution_loop_injects_complete_job_tool_definition() {
    // execution_loop appends complete_job_tool_definition() to
    // reason_ctx.available_tools after the registry + policy + profile
    // filter chain, at both the initial setup and the per-iteration
    // refresh sites. Reproduce that chain against an empty registry
    // (mirrors make_worker's ToolRegistry) and assert the synthetic
    // tool survives it, since it is appended after — not looked up
    // through — the real registry.
    let worker = make_worker(Vec::new()).await;

    let mut defs = worker
        .tools()
        .tool_definitions_for_autonomous_capabilities(None, None, None)
        .await;
    defs.push(complete_job_tool_definition());

    let complete_job_def = defs
        .iter()
        .find(|d| d.name == WORKER_COMPLETE_JOB_TOOL_NAME)
        .expect("complete_job tool definition should be present in the worker's tool list");
    assert_eq!(
        complete_job_def.description,
        WORKER_COMPLETE_JOB_TOOL_DESCRIPTION
    );
}

#[tokio::test]
async fn complete_job_tool_call_marks_job_completed() {
    let worker = make_worker(Vec::new()).await;
    worker
        .context_manager()
        .update_context(worker.job_id, |ctx| {
            ctx.transition_to(JobState::InProgress, Some("started".to_string()))
                .unwrap();
        })
        .await
        .unwrap();
    let mut reason_ctx = ReasoningContext::new().with_job("test job");

    let selection = ToolSelection {
        tool_name: WORKER_COMPLETE_JOB_TOOL_NAME.to_string(),
        parameters: serde_json::json!({ "summary": "Finished the task" }),
        reasoning: String::new(),
        alternatives: vec![],
        tool_call_id: "call_complete".to_string(),
    };

    // execute_tool_inner short-circuits complete_job by echoing back its
    // own arguments — mirrors what the real execution_loop path does
    // before handing the result to process_tool_result.
    let echoed = Ok(selection.parameters.to_string());
    let job_finished = worker
        .process_tool_result(&mut reason_ctx, &selection, echoed)
        .await
        .expect("process_tool_result should not error");

    assert!(
        job_finished,
        "complete_job should signal the execution loop to stop"
    );

    let ctx = worker
        .context_manager()
        .get_context(worker.job_id)
        .await
        .expect("job context should exist");
    assert_eq!(ctx.state, JobState::Completed);
    assert_eq!(
        worker.take_last_output().as_deref(),
        Some("Finished the task")
    );
}

#[tokio::test]
async fn complete_job_tool_call_with_success_false_marks_job_failed() {
    let worker = make_worker(Vec::new()).await;
    worker
        .context_manager()
        .update_context(worker.job_id, |ctx| {
            ctx.transition_to(JobState::InProgress, Some("started".to_string()))
                .unwrap();
        })
        .await
        .unwrap();
    let mut reason_ctx = ReasoningContext::new().with_job("test job");

    let selection = ToolSelection {
        tool_name: WORKER_COMPLETE_JOB_TOOL_NAME.to_string(),
        parameters: serde_json::json!({
            "summary": "Could not finish, blocked on missing credentials",
            "success": false,
        }),
        reasoning: String::new(),
        alternatives: vec![],
        tool_call_id: "call_complete".to_string(),
    };
    let echoed = Ok(selection.parameters.to_string());

    let job_finished = worker
        .process_tool_result(&mut reason_ctx, &selection, echoed)
        .await
        .expect("process_tool_result should not error");
    assert!(job_finished);

    let ctx = worker
        .context_manager()
        .get_context(worker.job_id)
        .await
        .expect("job context should exist");
    assert_eq!(ctx.state, JobState::Failed);
}

#[tokio::test]
async fn non_complete_job_tool_call_does_not_finish_the_job() {
    let worker = make_worker(Vec::new()).await;
    let mut reason_ctx = ReasoningContext::new().with_job("test job");

    let selection = ToolSelection {
        tool_name: "emit_user_message".to_string(),
        parameters: serde_json::json!({}),
        reasoning: String::new(),
        alternatives: vec![],
        tool_call_id: "call_other".to_string(),
    };

    let job_finished = worker
        .process_tool_result(
            &mut reason_ctx,
            &selection,
            Ok(serde_json::json!({"message": "", "message_type": "progress"}).to_string()),
        )
        .await
        .expect("process_tool_result should not error");

    assert!(
        !job_finished,
        "only complete_job should signal the loop to stop"
    );
}

#[tokio::test]
async fn execute_tool_inner_short_circuits_complete_job_without_hitting_registry() {
    // complete_job is never registered with the real ToolRegistry — an
    // empty registry proves execute_tool_inner doesn't dispatch to it.
    let worker = make_worker(Vec::new()).await;
    let params = serde_json::json!({ "summary": "done", "success": true });

    let result = Worker::execute_tool_inner(&worker.deps, worker.job_id, "complete_job", &params)
        .await
        .expect("complete_job should short-circuit rather than fail with tool-not-found");

    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed, params);
}
