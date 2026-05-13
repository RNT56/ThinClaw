use std::sync::Arc;

use super::{
    AdvisorAutoTrigger, AdvisorFailureContext, AdvisorTurnState, STUCK_LOOP_FINALIZATION_PROMPT,
    TOOL_PHASE_NO_TOOLS_SENTINEL, TOOL_PHASE_PLANNING_MAX_TOKENS, TOOL_PHASE_PLANNING_PROMPT,
    TOOL_PHASE_SYNTHESIS_PROMPT, classify_tool_phase_text, is_tool_phase_no_tools_signal,
    should_hold_complex_final_pass, tool_phase_synthesis_enabled,
};
use crate::channels::{IncomingMessage, StatusUpdate, StreamMode};
use crate::llm::{ChatMessage, FinishReason, StreamSupport, ThinkingConfig};
use crate::settings::RoutingMode;
use crate::tools::{ApprovalRequirement, ToolRegistry};

use super::test_support::*;

#[test]
fn tool_phase_requires_cheap_split_with_real_cheap_model() {
    let status = runtime_status(RoutingMode::CheapSplit, Some("openai/gpt-5.4-mini"), true);

    assert!(tool_phase_synthesis_enabled(
        Some(&status),
        true,
        false,
        true,
        false,
    ));
}

#[test]
fn tool_phase_is_disabled_without_cheap_model() {
    let status = runtime_status(RoutingMode::CheapSplit, None, true);

    assert!(!tool_phase_synthesis_enabled(
        Some(&status),
        true,
        false,
        true,
        false,
    ));
}

#[test]
fn tool_phase_is_disabled_outside_cheap_split() {
    let status = runtime_status(RoutingMode::Policy, Some("openai/gpt-5.4-mini"), true);

    assert!(!tool_phase_synthesis_enabled(
        Some(&status),
        true,
        false,
        true,
        false,
    ));
}

#[test]
fn complex_final_pass_only_holds_for_ready_advisor_complex_turns() {
    let status = runtime_status(
        RoutingMode::AdvisorExecutor,
        Some("openai/gpt-5.4-mini"),
        false,
    );
    let advisor_state = AdvisorTurnState::default();
    let messages = vec![ChatMessage::user(
        "Please design an architecture and implementation analysis for this migration.",
    )];

    assert!(should_hold_complex_final_pass(
        Some(&status),
        &messages,
        &advisor_state
    ));
    assert!(!should_hold_complex_final_pass(
        Some(&runtime_status(
            RoutingMode::CheapSplit,
            Some("openai/gpt-5.4-mini"),
            false
        )),
        &messages,
        &advisor_state
    ));
}

#[test]
fn complex_final_pass_uses_full_turn_context_not_only_last_user_message() {
    let status = runtime_status(
        RoutingMode::AdvisorExecutor,
        Some("openai/gpt-5.4-mini"),
        false,
    );
    let advisor_state = AdvisorTurnState::default();
    let messages = vec![
        ChatMessage::user(
            "Please design the migration architecture and review the implementation risks.",
        ),
        ChatMessage::assistant_with_tool_calls(
            Some(
                "I should inspect the current implementation before finalizing the design."
                    .to_string(),
            ),
            vec![tool_call("read_file"), tool_call("search_code")],
        ),
        ChatMessage::tool_result(
            "call_1",
            "read_file",
            "{\"status\":\"error\",\"message\":\"config missing\"}",
        ),
        ChatMessage::user("Continue."),
    ];

    assert!(should_hold_complex_final_pass(
        Some(&status),
        &messages,
        &advisor_state
    ));
}

#[tokio::test]
async fn auto_trigger_prefers_recorded_tool_failure() {
    let primary = Arc::new(ScriptedLlm::new(
        "primary-model",
        vec![ScriptedResponse::text("done", FinishReason::Stop)],
    ));
    let (agent, _) = make_test_agent(
        primary.clone(),
        Some(primary),
        Arc::new(ToolRegistry::new()),
        None,
        StreamMode::None,
        true,
        4,
    )
    .await;
    let status = runtime_status(
        RoutingMode::AdvisorExecutor,
        Some("openai/gpt-5.4-mini"),
        false,
    );
    let mut advisor_state = AdvisorTurnState::default();
    advisor_state.real_tool_result_count = 2;
    advisor_state.last_failure = Some(AdvisorFailureContext {
        tool_name: "shell".to_string(),
        message: "command failed".to_string(),
        signature: Some(42),
        checkpoint: 2,
    });

    let trigger = agent.next_auto_advisor_trigger(
        Some(&status),
        &[ChatMessage::user("Debug the deployment failure.")],
        &advisor_state,
        0,
        None,
    );

    assert!(matches!(
        trigger,
        Some((AdvisorAutoTrigger::ToolFailure, _, Some(42)))
    ));
}

#[test]
fn tool_phase_signal_requires_explicit_sentinel() {
    assert!(is_tool_phase_no_tools_signal("NO_TOOLS_NEEDED"));
    assert!(is_tool_phase_no_tools_signal("NO_TOOLS_NEEDED."));
    assert!(!is_tool_phase_no_tools_signal("No tools needed."));
    assert!(!is_tool_phase_no_tools_signal(
        "Here is the final answer for the user."
    ));
}

#[test]
fn tool_phase_text_classification_prefers_finish_reason() {
    assert_eq!(
        classify_tool_phase_text("NO_TOOLS_NEEDED", FinishReason::Stop),
        super::ToolPhaseTextOutcome::NoToolsSignal
    );
    assert_eq!(
        classify_tool_phase_text("Primary answer", FinishReason::Stop),
        super::ToolPhaseTextOutcome::PrimaryFinalText
    );
    assert_eq!(
        classify_tool_phase_text("Truncated answer", FinishReason::Length),
        super::ToolPhaseTextOutcome::PrimaryNeedsFinalization
    );
}

#[tokio::test]
async fn tool_phase_runs_cheap_synthesis_only_after_explicit_no_tools_signal() {
    let primary = Arc::new(ScriptedLlm::new(
        "primary-model",
        vec![
            ScriptedResponse::tool_calls(vec![tool_call("test_tool")], FinishReason::ToolUse),
            ScriptedResponse::text_with_thinking(
                TOOL_PHASE_NO_TOOLS_SENTINEL,
                FinishReason::Stop,
                "hidden planner thought",
            ),
        ],
    ));
    let cheap = Arc::new(ScriptedLlm::with_stream_support(
        "cheap-model",
        vec![ScriptedResponse::text_with_thinking(
            "Cheap final answer",
            FinishReason::Stop,
            "visible synthesis thought",
        )],
        StreamSupport::Native,
    ));
    let runtime = make_runtime_manager(true, true).await;
    let tools = Arc::new(ToolRegistry::new());
    register_tool(
        &tools,
        "test_tool",
        ApprovalRequirement::Never,
        "tool output",
    )
    .await;
    let (agent, channel) = make_test_agent(
        primary.clone(),
        Some(cheap.clone()),
        tools,
        Some(runtime),
        StreamMode::EditFirst,
        true,
        10,
    )
    .await;
    let (session, thread_id) = make_session_and_thread().await;
    let message = IncomingMessage::new("test", "user-1", "help");

    let result = agent
        .run_agentic_loop(
            &message,
            session,
            thread_id,
            vec![ChatMessage::user("help")],
        )
        .await
        .expect("agentic loop should succeed");

    match result {
        super::AgenticLoopResult::Streamed(text) => assert_eq!(text, "Cheap final answer"),
        other => panic!(
            "expected streamed result, got {:?}",
            std::mem::discriminant(&other)
        ),
    }

    assert_eq!(cheap.response_count().await, 1);

    let primary_requests = primary.requests().await;
    assert_eq!(primary_requests.len(), 2);
    assert_eq!(
        primary_requests
            .iter()
            .map(|req| req.max_tokens)
            .collect::<Vec<_>>(),
        vec![
            Some(TOOL_PHASE_PLANNING_MAX_TOKENS),
            Some(TOOL_PHASE_PLANNING_MAX_TOKENS)
        ]
    );
    assert!(
        primary_requests
            .iter()
            .all(|req| count_prompt(&req.messages, TOOL_PHASE_PLANNING_PROMPT) == 1)
    );

    let cheap_requests = cheap.requests().await;
    assert_eq!(cheap_requests.len(), 1);
    assert_eq!(cheap_requests[0].tool_names.len(), 0);
    assert_eq!(cheap_requests[0].max_tokens, Some(4096));
    assert!(contains_prompt(
        &cheap_requests[0].messages,
        TOOL_PHASE_SYNTHESIS_PROMPT
    ));
    assert!(!contains_prompt(
        &cheap_requests[0].messages,
        TOOL_PHASE_PLANNING_PROMPT
    ));

    let events = channel.events().await;
    assert!(events.iter().any(|event| matches!(
        event,
        RecordedChannelEvent::Draft(text) if text.contains("Cheap final answer")
    )));
    assert!(!events.iter().any(|event| matches!(
        event,
        RecordedChannelEvent::Draft(text) if text.contains(TOOL_PHASE_NO_TOOLS_SENTINEL)
    )));
    assert!(!events.iter().any(|event| matches!(
        event,
        RecordedChannelEvent::Status(StatusUpdate::Thinking(text))
            if text.contains("hidden planner thought")
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        RecordedChannelEvent::Status(StatusUpdate::Thinking(text))
            if text.contains("visible synthesis thought")
    )));
}

#[tokio::test]
async fn tool_phase_direct_primary_text_skips_cheap_follow_up() {
    let primary = Arc::new(ScriptedLlm::new(
        "primary-model",
        vec![ScriptedResponse::text(
            "Primary final answer",
            FinishReason::Stop,
        )],
    ));
    let cheap = Arc::new(ScriptedLlm::new("cheap-model", vec![]));
    let runtime = make_runtime_manager(true, true).await;
    let tools = Arc::new(ToolRegistry::new());
    register_tool(
        &tools,
        "test_tool",
        ApprovalRequirement::Never,
        "tool output",
    )
    .await;
    let (agent, channel) = make_test_agent(
        primary.clone(),
        Some(cheap.clone()),
        tools,
        Some(runtime),
        StreamMode::None,
        true,
        10,
    )
    .await;
    let (session, thread_id) = make_session_and_thread().await;
    let message = IncomingMessage::new("test", "user-1", "help");

    let result = agent
        .run_agentic_loop(
            &message,
            session,
            thread_id,
            vec![ChatMessage::user("help")],
        )
        .await
        .expect("agentic loop should succeed");

    match result {
        super::AgenticLoopResult::Response(text) => assert_eq!(text, "Primary final answer"),
        other => panic!(
            "expected response result, got {:?}",
            std::mem::discriminant(&other)
        ),
    }

    assert_eq!(cheap.response_count().await, 0);
    let primary_requests = primary.requests().await;
    assert_eq!(primary_requests.len(), 1);
    assert_eq!(
        primary_requests[0].max_tokens,
        Some(TOOL_PHASE_PLANNING_MAX_TOKENS)
    );
    assert!(contains_prompt(
        &primary_requests[0].messages,
        TOOL_PHASE_PLANNING_PROMPT
    ));
    assert!(
        channel
            .events()
            .await
            .iter()
            .all(|event| !matches!(event, RecordedChannelEvent::Draft(_)))
    );
}

#[tokio::test]
async fn truncated_planner_text_runs_primary_finalization_without_cheap() {
    let primary = Arc::new(ScriptedLlm::new(
        "primary-model",
        vec![
            ScriptedResponse::text("Truncated planner answer", FinishReason::Length),
            ScriptedResponse::text("Primary finalized answer", FinishReason::Stop),
        ],
    ));
    let cheap = Arc::new(ScriptedLlm::new("cheap-model", vec![]));
    let runtime = make_runtime_manager(true, true).await;
    let tools = Arc::new(ToolRegistry::new());
    register_tool(
        &tools,
        "test_tool",
        ApprovalRequirement::Never,
        "tool output",
    )
    .await;
    let (agent, _) = make_test_agent(
        primary.clone(),
        Some(cheap.clone()),
        tools,
        Some(runtime),
        StreamMode::None,
        true,
        10,
    )
    .await;
    let (session, thread_id) = make_session_and_thread().await;
    let message = IncomingMessage::new("test", "user-1", "help");

    let result = agent
        .run_agentic_loop(
            &message,
            session,
            thread_id,
            vec![ChatMessage::user("help")],
        )
        .await
        .expect("agentic loop should succeed");

    match result {
        super::AgenticLoopResult::Response(text) => {
            assert_eq!(text, "Primary finalized answer")
        }
        other => panic!(
            "expected response result, got {:?}",
            std::mem::discriminant(&other)
        ),
    }

    assert_eq!(cheap.response_count().await, 0);
    let primary_requests = primary.requests().await;
    assert_eq!(primary_requests.len(), 2);
    assert_eq!(
        primary_requests[0].max_tokens,
        Some(TOOL_PHASE_PLANNING_MAX_TOKENS)
    );
    assert_eq!(primary_requests[1].max_tokens, Some(4096));
    assert!(!contains_prompt(
        &primary_requests[1].messages,
        TOOL_PHASE_PLANNING_PROMPT
    ));
    assert!(primary_requests[1].tool_names.is_empty());
}

#[tokio::test]
async fn force_text_iteration_does_not_run_tool_phase_synthesis() {
    let primary = Arc::new(ScriptedLlm::new(
        "primary-model",
        vec![ScriptedResponse::text(
            "Forced final answer",
            FinishReason::Stop,
        )],
    ));
    let cheap = Arc::new(ScriptedLlm::new("cheap-model", vec![]));
    let runtime = make_runtime_manager(true, true).await;
    let tools = Arc::new(ToolRegistry::new());
    register_tool(
        &tools,
        "test_tool",
        ApprovalRequirement::Never,
        "tool output",
    )
    .await;
    let (agent, _) = make_test_agent(
        primary.clone(),
        Some(cheap.clone()),
        tools,
        Some(runtime),
        StreamMode::None,
        true,
        1,
    )
    .await;
    let (session, thread_id) = make_session_and_thread().await;
    let message = IncomingMessage::new("test", "user-1", "help");

    let result = agent
        .run_agentic_loop(
            &message,
            session,
            thread_id,
            vec![ChatMessage::user("help")],
        )
        .await
        .expect("agentic loop should succeed");

    match result {
        super::AgenticLoopResult::Response(text) => assert_eq!(text, "Forced final answer"),
        other => panic!(
            "expected response result, got {:?}",
            std::mem::discriminant(&other)
        ),
    }

    assert_eq!(cheap.response_count().await, 0);
    let primary_requests = primary.requests().await;
    assert_eq!(primary_requests.len(), 1);
    assert!(primary_requests[0].tool_names.is_empty());
    assert!(!contains_prompt(
        &primary_requests[0].messages,
        TOOL_PHASE_PLANNING_PROMPT
    ));
    assert!(!contains_prompt(
        &primary_requests[0].messages,
        TOOL_PHASE_SYNTHESIS_PROMPT
    ));
    assert_eq!(primary_requests[0].max_tokens, Some(4096));
}

#[tokio::test]
async fn stuck_loop_recovery_uses_primary_finalization_only() {
    let mut responses = Vec::new();
    for _ in 0..5 {
        responses.push(ScriptedResponse::tool_calls(
            vec![tool_call("loop_tool")],
            FinishReason::ToolUse,
        ));
    }
    responses.push(ScriptedResponse::text(
        "Recovered on primary",
        FinishReason::Stop,
    ));

    let primary = Arc::new(ScriptedLlm::new("primary-model", responses));
    let cheap = Arc::new(ScriptedLlm::new("cheap-model", vec![]));
    let runtime = make_runtime_manager(true, true).await;
    let tools = Arc::new(ToolRegistry::new());
    register_tool(
        &tools,
        "loop_tool",
        ApprovalRequirement::Never,
        "loop result",
    )
    .await;
    let (agent, _) = make_test_agent(
        primary.clone(),
        Some(cheap.clone()),
        tools,
        Some(runtime),
        StreamMode::None,
        true,
        20,
    )
    .await;
    let (session, thread_id) = make_session_and_thread().await;
    let message = IncomingMessage::new("test", "user-1", "help");

    let result = agent
        .run_agentic_loop(
            &message,
            session,
            thread_id,
            vec![ChatMessage::user("help")],
        )
        .await
        .expect("agentic loop should succeed");

    match result {
        super::AgenticLoopResult::Response(text) => assert_eq!(text, "Recovered on primary"),
        other => panic!(
            "expected response result, got {:?}",
            std::mem::discriminant(&other)
        ),
    }

    assert_eq!(cheap.response_count().await, 0);
    let primary_requests = primary.requests().await;
    assert_eq!(primary_requests.len(), 6);
    let final_request = primary_requests.last().expect("final request should exist");
    assert!(contains_prompt(
        &final_request.messages,
        STUCK_LOOP_FINALIZATION_PROMPT
    ));
    assert!(final_request.tool_names.is_empty());
    assert!(!contains_prompt(
        &final_request.messages,
        TOOL_PHASE_SYNTHESIS_PROMPT
    ));
}

#[tokio::test]
async fn planner_thinking_toggle_only_changes_hidden_primary_phase() {
    async fn run_case(
        primary_planning_thinking_enabled: bool,
    ) -> (Vec<CapturedRequest>, Vec<CapturedRequest>) {
        let primary = Arc::new(ScriptedLlm::new(
            "primary-model",
            vec![ScriptedResponse::text(
                TOOL_PHASE_NO_TOOLS_SENTINEL,
                FinishReason::Stop,
            )],
        ));
        let cheap = Arc::new(ScriptedLlm::new(
            "cheap-model",
            vec![ScriptedResponse::text("Cheap reply", FinishReason::Stop)],
        ));
        let runtime = make_runtime_manager(true, primary_planning_thinking_enabled).await;
        let tools = Arc::new(ToolRegistry::new());
        register_tool(
            &tools,
            "test_tool",
            ApprovalRequirement::Never,
            "tool output",
        )
        .await;
        let (agent, _) = make_test_agent(
            primary.clone(),
            Some(cheap.clone()),
            tools,
            Some(runtime),
            StreamMode::None,
            true,
            10,
        )
        .await;
        let (session, thread_id) = make_session_and_thread().await;
        let message = IncomingMessage::new("test", "user-1", "help");

        let _ = agent
            .run_agentic_loop(
                &message,
                session,
                thread_id,
                vec![ChatMessage::user("help")],
            )
            .await
            .expect("agentic loop should succeed");

        (primary.requests().await, cheap.requests().await)
    }

    let (primary_enabled, cheap_enabled) = run_case(true).await;
    let (primary_disabled, cheap_disabled) = run_case(false).await;

    assert!(matches!(
        primary_enabled[0].thinking,
        ThinkingConfig::Enabled { .. }
    ));
    assert!(matches!(
        primary_disabled[0].thinking,
        ThinkingConfig::Disabled
    ));
    assert!(matches!(
        cheap_enabled[0].thinking,
        ThinkingConfig::Enabled { .. }
    ));
    assert!(matches!(
        cheap_disabled[0].thinking,
        ThinkingConfig::Enabled { .. }
    ));
}

#[tokio::test]
async fn advisor_interception_runs_in_parallel_path_and_enforces_budget() {
    let primary = Arc::new(ScriptedLlm::new(
        "primary-model",
        vec![
            ScriptedResponse::tool_calls(
                vec![tool_call("consult_advisor"), tool_call("test_tool")],
                FinishReason::ToolUse,
            ),
            ScriptedResponse::text("Final answer", FinishReason::Stop),
            ScriptedResponse::text("Final answer", FinishReason::Stop),
            ScriptedResponse::text("Final answer", FinishReason::Stop),
        ],
    ));
    let runtime = make_runtime_manager_for_mode(false, true, RoutingMode::AdvisorExecutor, 0).await;
    let tools = Arc::new(ToolRegistry::new());
    register_tool(
        &tools,
        "test_tool",
        ApprovalRequirement::Never,
        "tool output",
    )
    .await;
    let (agent, channel) = make_test_agent(
        primary.clone(),
        Some(primary.clone()),
        tools,
        Some(runtime),
        StreamMode::None,
        true,
        10,
    )
    .await;
    let (session, thread_id) = make_session_and_thread().await;
    let message = IncomingMessage::new("test", "user-1", "help");

    let result = agent
        .run_agentic_loop(
            &message,
            session,
            thread_id,
            vec![ChatMessage::user("help")],
        )
        .await
        .expect("agentic loop should succeed");

    match result {
        super::AgenticLoopResult::Response(text) => assert_eq!(text, "Final answer"),
        other => panic!(
            "expected response result, got {:?}",
            std::mem::discriminant(&other)
        ),
    }

    let events = channel.events().await;
    assert!(events.iter().any(|event| matches!(
        event,
        RecordedChannelEvent::Status(StatusUpdate::ToolCompleted { name, success, .. })
            if name == "consult_advisor" && *success
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        RecordedChannelEvent::Status(StatusUpdate::ToolResult { name, preview, .. })
            if name == "consult_advisor" && preview.contains("advisor_call_limit_reached")
    )));
}

#[tokio::test]
async fn pending_approval_context_does_not_persist_planning_prompt() {
    let primary = Arc::new(ScriptedLlm::new(
        "primary-model",
        vec![ScriptedResponse::tool_calls(
            vec![tool_call("approval_tool")],
            FinishReason::ToolUse,
        )],
    ));
    let cheap = Arc::new(ScriptedLlm::new("cheap-model", vec![]));
    let runtime = make_runtime_manager(true, true).await;
    let tools = Arc::new(ToolRegistry::new());
    register_tool(
        &tools,
        "approval_tool",
        ApprovalRequirement::Always,
        "approval tool output",
    )
    .await;
    let (agent, _) = make_test_agent(
        primary,
        Some(cheap),
        tools,
        Some(runtime),
        StreamMode::None,
        true,
        10,
    )
    .await;
    let (session, thread_id) = make_session_and_thread().await;
    let message = IncomingMessage::new("test", "user-1", "help");

    let result = agent
        .run_agentic_loop(
            &message,
            session,
            thread_id,
            vec![ChatMessage::user("help")],
        )
        .await
        .expect("agentic loop should succeed");

    match result {
        super::AgenticLoopResult::NeedApproval { pending } => {
            assert!(!contains_prompt(
                &pending.context_messages,
                TOOL_PHASE_PLANNING_PROMPT
            ));
            assert!(!contains_prompt(
                &pending.context_messages,
                TOOL_PHASE_SYNTHESIS_PROMPT
            ));
        }
        other => panic!(
            "expected approval result, got {:?}",
            std::mem::discriminant(&other)
        ),
    }
}

#[tokio::test]
async fn run_agentic_loop_uses_channel_formatting_hints_from_channel_manager() {
    let primary = Arc::new(ScriptedLlm::new(
        "primary-model",
        vec![ScriptedResponse::text(
            "Plain text response",
            FinishReason::Stop,
        )],
    ));
    let tools = Arc::new(ToolRegistry::new());
    let recording_channel = RecordingChannel::new("test", StreamMode::None)
        .with_formatting_hints("- Test channel prefers plain text only.");
    let (agent, _) = make_test_agent_with_channel(
        primary.clone(),
        None,
        tools,
        None,
        recording_channel,
        false,
        10,
    )
    .await;
    let (session, thread_id) = make_session_and_thread().await;
    let message = IncomingMessage::new("test", "user-1", "hello");

    let result = agent
        .run_agentic_loop(
            &message,
            session,
            thread_id,
            vec![ChatMessage::user("hello")],
        )
        .await
        .expect("agentic loop should succeed");

    match result {
        super::AgenticLoopResult::Response(text) => assert_eq!(text, "Plain text response"),
        other => panic!(
            "expected text response, got {:?}",
            std::mem::discriminant(&other)
        ),
    }

    let requests = primary.requests().await;
    assert!(requests.iter().any(|req| {
        req.context_documents
            .iter()
            .any(|doc| doc.contains("Test channel prefers plain text only."))
    }));
}

#[tokio::test]
async fn run_agentic_loop_cancels_in_flight_provider_call() {
    let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
    let blocking_llm = Arc::new(BlockingLlm::new("blocking-model", dropped_tx));
    let tools = Arc::new(ToolRegistry::new());
    let (agent, _) = make_test_agent(
        blocking_llm.clone(),
        None,
        tools,
        None,
        StreamMode::None,
        false,
        10,
    )
    .await;
    let agent = Arc::new(agent);
    let (session, thread_id) = make_session_and_thread().await;
    agent.begin_turn_cancellation(thread_id).await;
    let message = IncomingMessage::new("test", "user-1", "hello");
    let run_agent = Arc::clone(&agent);
    let run_session = Arc::clone(&session);
    let run_message = message.clone();

    let run = tokio::spawn(async move {
        run_agent
            .run_agentic_loop(
                &run_message,
                run_session,
                thread_id,
                vec![ChatMessage::user("hello")],
            )
            .await
    });

    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        blocking_llm.wait_started(),
    )
    .await
    .expect("provider call should start");
    agent.signal_turn_cancellation(thread_id).await;

    tokio::time::timeout(std::time::Duration::from_secs(1), dropped_rx)
        .await
        .expect("provider future should be dropped on cancellation")
        .expect("drop signal should be sent");
    let result = tokio::time::timeout(std::time::Duration::from_secs(1), run)
        .await
        .expect("agent loop should return after cancellation")
        .expect("join should succeed");
    let err = match result {
        Err(err) => err,
        Ok(_) => panic!("cancelled turn should return an interrupted error"),
    };
    assert!(err.to_string().contains("Interrupted"));
    agent.finish_turn_cancellation(thread_id).await;
}

#[tokio::test]
async fn run_agentic_loop_cancels_in_flight_tool_call() {
    let primary = Arc::new(ScriptedLlm::new(
        "primary-model",
        vec![ScriptedResponse::tool_calls(
            vec![tool_call("blocking_tool")],
            FinishReason::ToolUse,
        )],
    ));
    let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
    let blocking_tool = Arc::new(BlockingTool::new("blocking_tool", dropped_tx));
    let tools = Arc::new(ToolRegistry::new());
    tools.register(blocking_tool.clone()).await;
    let (agent, _) = make_test_agent(primary, None, tools, None, StreamMode::None, false, 10).await;
    let agent = Arc::new(agent);
    let (session, thread_id) = make_session_and_thread().await;
    agent.begin_turn_cancellation(thread_id).await;
    let message = IncomingMessage::new("test", "user-1", "hello");
    let run_agent = Arc::clone(&agent);
    let run_session = Arc::clone(&session);
    let run_message = message.clone();

    let run = tokio::spawn(async move {
        run_agent
            .run_agentic_loop(
                &run_message,
                run_session,
                thread_id,
                vec![ChatMessage::user("hello")],
            )
            .await
    });

    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        blocking_tool.wait_started(),
    )
    .await
    .expect("tool call should start");
    agent.signal_turn_cancellation(thread_id).await;

    tokio::time::timeout(std::time::Duration::from_secs(1), dropped_rx)
        .await
        .expect("tool future should be dropped on cancellation")
        .expect("drop signal should be sent");
    let result = tokio::time::timeout(std::time::Duration::from_secs(1), run)
        .await
        .expect("agent loop should return after cancellation")
        .expect("join should succeed");
    let err = match result {
        Err(err) => err,
        Ok(_) => panic!("cancelled turn should return an interrupted error"),
    };
    assert!(err.to_string().contains("Interrupted"));
    agent.finish_turn_cancellation(thread_id).await;
}
