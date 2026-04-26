use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rust_decimal::Decimal;
use tokio::sync::Mutex;
use uuid::Uuid;

use super::{
    AdvisorAutoTrigger, AdvisorFailureContext, AdvisorTurnState, STUCK_LOOP_FINALIZATION_PROMPT,
    TOOL_PHASE_NO_TOOLS_SENTINEL, TOOL_PHASE_PLANNING_MAX_TOKENS, TOOL_PHASE_PLANNING_PROMPT,
    TOOL_PHASE_SYNTHESIS_PROMPT, classify_tool_phase_text, is_tool_phase_no_tools_signal,
    should_hold_complex_final_pass, tool_phase_synthesis_enabled,
};
use crate::agent::agent_loop::{Agent, AgentDeps};
use crate::agent::cost_guard::{CostGuard, CostGuardConfig};
use crate::agent::session::Session;
use crate::channels::{
    Channel, ChannelManager, DraftReplyState, IncomingMessage, MessageStream, OutgoingResponse,
    StatusUpdate, StreamMode,
};
use crate::config::{AgentConfig, Config, SafetyConfig, SkillsConfig};
use crate::context::ContextManager;
use crate::error::{ChannelError, LlmError};
use crate::hooks::HookRegistry;
use crate::llm::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, StreamSupport,
    ThinkingConfig, ToolCall, ToolCompletionRequest, ToolCompletionResponse,
};
use crate::safety::SafetyLayer;
use crate::settings::{
    AdvisorAutoEscalationMode, ProviderModelSlots, ProvidersSettings, RoutingMode,
    SecretsMasterKeySource, Settings,
};
use crate::tools::{ApprovalRequirement, Tool, ToolOutput, ToolRegistry};

#[derive(Debug, Clone)]
struct CapturedRequest {
    messages: Vec<ChatMessage>,
    context_documents: Vec<String>,
    tool_names: Vec<String>,
    max_tokens: Option<u32>,
    thinking: ThinkingConfig,
}

#[derive(Debug, Clone)]
enum ScriptedResult {
    Text(String),
    ToolCalls {
        content: Option<String>,
        tool_calls: Vec<ToolCall>,
    },
}

#[derive(Debug, Clone)]
struct ScriptedResponse {
    result: ScriptedResult,
    finish_reason: FinishReason,
    thinking_content: Option<String>,
}

impl ScriptedResponse {
    fn text(text: impl Into<String>, finish_reason: FinishReason) -> Self {
        Self {
            result: ScriptedResult::Text(text.into()),
            finish_reason,
            thinking_content: None,
        }
    }

    fn text_with_thinking(
        text: impl Into<String>,
        finish_reason: FinishReason,
        thinking: impl Into<String>,
    ) -> Self {
        Self {
            result: ScriptedResult::Text(text.into()),
            finish_reason,
            thinking_content: Some(thinking.into()),
        }
    }

    fn tool_calls(tool_calls: Vec<ToolCall>, finish_reason: FinishReason) -> Self {
        Self {
            result: ScriptedResult::ToolCalls {
                content: None,
                tool_calls,
            },
            finish_reason,
            thinking_content: None,
        }
    }
}

struct ScriptedLlm {
    model_name: String,
    responses: Mutex<VecDeque<ScriptedResponse>>,
    requests: Mutex<Vec<CapturedRequest>>,
    stream_support: StreamSupport,
}

impl ScriptedLlm {
    fn new(model_name: impl Into<String>, responses: Vec<ScriptedResponse>) -> Self {
        Self::with_stream_support(model_name, responses, StreamSupport::Simulated)
    }

    fn with_stream_support(
        model_name: impl Into<String>,
        responses: Vec<ScriptedResponse>,
        stream_support: StreamSupport,
    ) -> Self {
        Self {
            model_name: model_name.into(),
            responses: Mutex::new(VecDeque::from(responses)),
            requests: Mutex::new(Vec::new()),
            stream_support,
        }
    }

    async fn requests(&self) -> Vec<CapturedRequest> {
        self.requests.lock().await.clone()
    }

    async fn response_count(&self) -> usize {
        self.requests.lock().await.len()
    }

    async fn pop_response(&self) -> ScriptedResponse {
        self.responses
            .lock()
            .await
            .pop_front()
            .expect("scripted llm ran out of queued responses")
    }

    async fn record_request(
        &self,
        messages: Vec<ChatMessage>,
        context_documents: Vec<String>,
        tool_names: Vec<String>,
        max_tokens: Option<u32>,
        thinking: ThinkingConfig,
    ) {
        self.requests.lock().await.push(CapturedRequest {
            messages,
            context_documents,
            tool_names,
            max_tokens,
            thinking,
        });
    }
}

#[async_trait]
impl LlmProvider for ScriptedLlm {
    fn model_name(&self) -> &str {
        &self.model_name
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    fn stream_support(&self) -> StreamSupport {
        self.stream_support
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.record_request(
            request.messages,
            request.context_documents,
            Vec::new(),
            request.max_tokens,
            request.thinking,
        )
        .await;

        let response = self.pop_response().await;
        match response.result {
            ScriptedResult::Text(content) => Ok(CompletionResponse {
                content,
                provider_model: Some(self.model_name.clone()),
                cost_usd: Some(0.0),
                thinking_content: response.thinking_content,
                input_tokens: 10,
                output_tokens: 5,
                finish_reason: response.finish_reason,
                token_capture: None,
            }),
            ScriptedResult::ToolCalls { .. } => {
                panic!("complete() received a tool-call scripted response");
            }
        }
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        self.record_request(
            request.messages,
            request.context_documents,
            request.tools.iter().map(|tool| tool.name.clone()).collect(),
            request.max_tokens,
            request.thinking,
        )
        .await;

        let response = self.pop_response().await;
        match response.result {
            ScriptedResult::Text(content) => Ok(ToolCompletionResponse {
                content: Some(content),
                provider_model: Some(self.model_name.clone()),
                cost_usd: Some(0.0),
                tool_calls: Vec::new(),
                thinking_content: response.thinking_content,
                input_tokens: 10,
                output_tokens: 5,
                finish_reason: response.finish_reason,
                token_capture: None,
            }),
            ScriptedResult::ToolCalls {
                content,
                tool_calls,
            } => Ok(ToolCompletionResponse {
                content,
                provider_model: Some(self.model_name.clone()),
                cost_usd: Some(0.0),
                tool_calls,
                thinking_content: response.thinking_content,
                input_tokens: 10,
                output_tokens: 5,
                finish_reason: response.finish_reason,
                token_capture: None,
            }),
        }
    }
}

#[derive(Debug, Clone)]
enum RecordedChannelEvent {
    Status(StatusUpdate),
    Draft(String),
    Deleted,
    Response,
}

#[derive(Clone)]
struct RecordingChannel {
    name: String,
    stream_mode: StreamMode,
    formatting_hints: Option<String>,
    events: Arc<Mutex<Vec<RecordedChannelEvent>>>,
}

impl RecordingChannel {
    fn new(name: impl Into<String>, stream_mode: StreamMode) -> Self {
        Self {
            name: name.into(),
            stream_mode,
            formatting_hints: None,
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn with_formatting_hints(mut self, hints: impl Into<String>) -> Self {
        self.formatting_hints = Some(hints.into());
        self
    }

    async fn events(&self) -> Vec<RecordedChannelEvent> {
        self.events.lock().await.clone()
    }
}

#[async_trait]
impl Channel for RecordingChannel {
    fn name(&self) -> &str {
        &self.name
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        Ok(Box::pin(futures::stream::empty()))
    }

    async fn respond(
        &self,
        _msg: &IncomingMessage,
        _response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        self.events
            .lock()
            .await
            .push(RecordedChannelEvent::Response);
        Ok(())
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        _metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        self.events
            .lock()
            .await
            .push(RecordedChannelEvent::Status(status));
        Ok(())
    }

    async fn send_draft(
        &self,
        draft: &DraftReplyState,
        _metadata: &serde_json::Value,
    ) -> Result<Option<String>, ChannelError> {
        self.events
            .lock()
            .await
            .push(RecordedChannelEvent::Draft(draft.accumulated.clone()));
        Ok(Some("draft-id".to_string()))
    }

    async fn delete_message(
        &self,
        _message_id: &str,
        _metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        self.events.lock().await.push(RecordedChannelEvent::Deleted);
        Ok(())
    }

    fn stream_mode(&self) -> StreamMode {
        self.stream_mode
    }

    fn formatting_hints(&self) -> Option<String> {
        self.formatting_hints.clone()
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        Ok(())
    }
}

struct TestTool {
    name: String,
    approval: ApprovalRequirement,
    result: String,
}

impl TestTool {
    fn new(
        name: impl Into<String>,
        approval: ApprovalRequirement,
        result: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            approval,
            result: result.into(),
        }
    }
}

#[async_trait]
impl Tool for TestTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        "Test tool"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" }
            }
        })
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        _ctx: &crate::context::JobContext,
    ) -> Result<ToolOutput, crate::tools::ToolError> {
        Ok(ToolOutput::text(
            self.result.clone(),
            Duration::from_millis(1),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        self.approval
    }
}

fn runtime_status(
    routing_mode: RoutingMode,
    cheap_model: Option<&str>,
    enabled: bool,
) -> crate::llm::runtime_manager::RuntimeStatus {
    crate::llm::runtime_manager::RuntimeStatus {
        revision: 1,
        last_error: None,
        primary_model: "openai_compatible/primary-model".to_string(),
        cheap_model: cheap_model.map(str::to_string),
        routing_enabled: true,
        routing_mode,
        tool_phase_synthesis_enabled: enabled,
        tool_phase_primary_thinking_enabled: true,
        primary_provider: Some("openai_compatible".to_string()),
        fallback_chain: Vec::new(),
        advisor_max_calls: 4,
        advisor_auto_escalation_mode: AdvisorAutoEscalationMode::RiskAndComplexFinal,
        advisor_escalation_prompt: None,
        advisor_ready: routing_mode == RoutingMode::AdvisorExecutor,
        advisor_disabled_reason: None,
        executor_target: (routing_mode == RoutingMode::AdvisorExecutor)
            .then_some("cheap".to_string()),
        advisor_target: (routing_mode == RoutingMode::AdvisorExecutor)
            .then_some("primary".to_string()),
    }
}

async fn make_runtime_manager(
    tool_phase_synthesis_enabled: bool,
    tool_phase_primary_thinking_enabled: bool,
) -> Arc<crate::llm::runtime_manager::LlmRuntimeManager> {
    make_runtime_manager_for_mode(
        tool_phase_synthesis_enabled,
        tool_phase_primary_thinking_enabled,
        RoutingMode::CheapSplit,
        3,
    )
    .await
}

async fn make_runtime_manager_for_mode(
    tool_phase_synthesis_enabled: bool,
    tool_phase_primary_thinking_enabled: bool,
    routing_mode: RoutingMode,
    advisor_max_calls: u32,
) -> Arc<crate::llm::runtime_manager::LlmRuntimeManager> {
    let mut settings = Settings {
        llm_backend: Some("openai_compatible".to_string()),
        openai_compatible_base_url: Some("http://localhost:12345/v1".to_string()),
        selected_model: Some("gpt-5.4".to_string()),
        ..Settings::default()
    };
    settings.secrets.master_key_source = SecretsMasterKeySource::None;
    let config = Config::from_test_settings(&settings)
        .await
        .expect("config should load");

    let mut providers = ProvidersSettings {
        enabled: vec!["openai_compatible".to_string()],
        primary: Some("openai_compatible".to_string()),
        primary_model: Some("gpt-5.4".to_string()),
        cheap_model: Some("openai_compatible/gpt-5.4-mini".to_string()),
        smart_routing_enabled: true,
        routing_mode,
        tool_phase_synthesis_enabled,
        tool_phase_primary_thinking_enabled,
        advisor_max_calls,
        advisor_auto_escalation_mode: AdvisorAutoEscalationMode::RiskAndComplexFinal,
        ..ProvidersSettings::default()
    };
    providers.provider_models.insert(
        "openai_compatible".to_string(),
        ProviderModelSlots {
            primary: Some("gpt-5.4".to_string()),
            cheap: Some("gpt-5.4-mini".to_string()),
        },
    );

    crate::llm::runtime_manager::LlmRuntimeManager::new(
        config,
        providers,
        None,
        None,
        "test-user",
        None,
    )
    .expect("runtime manager should build")
}

async fn make_test_agent(
    primary_llm: Arc<dyn LlmProvider>,
    cheap_llm: Option<Arc<dyn LlmProvider>>,
    tools: Arc<ToolRegistry>,
    llm_runtime: Option<Arc<crate::llm::runtime_manager::LlmRuntimeManager>>,
    stream_mode: StreamMode,
    thinking_enabled: bool,
    max_tool_iterations: usize,
) -> (Agent, RecordingChannel) {
    let recording_channel = RecordingChannel::new("test", stream_mode);
    make_test_agent_with_channel(
        primary_llm,
        cheap_llm,
        tools,
        llm_runtime,
        recording_channel,
        thinking_enabled,
        max_tool_iterations,
    )
    .await
}

async fn make_test_agent_with_channel(
    primary_llm: Arc<dyn LlmProvider>,
    cheap_llm: Option<Arc<dyn LlmProvider>>,
    tools: Arc<ToolRegistry>,
    llm_runtime: Option<Arc<crate::llm::runtime_manager::LlmRuntimeManager>>,
    recording_channel: RecordingChannel,
    thinking_enabled: bool,
    max_tool_iterations: usize,
) -> (Agent, RecordingChannel) {
    let channels = Arc::new(ChannelManager::new());
    channels.add(Box::new(recording_channel.clone())).await;

    let deps = AgentDeps {
        store: None,
        llm: primary_llm,
        cheap_llm,
        safety: Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: false,
            redact_pii_in_prompts: true,
            smart_approval_mode: "off".to_string(),
            external_scanner_mode: "off".to_string(),
            external_scanner_path: None,
        })),
        tools,
        workspace: None,
        extension_manager: None,
        skill_registry: None,
        skill_catalog: None,
        skills_config: SkillsConfig::default(),
        hooks: Arc::new(HookRegistry::new()),
        cost_guard: Arc::new(CostGuard::new(CostGuardConfig::default())),
        sse_sender: None,
        agent_router: None,
        agent_registry: None,
        canvas_store: None,
        subagent_executor: None,
        cost_tracker: None,
        response_cache: None,
        llm_runtime,
        routing_policy: None,
        model_override: None,
        restart_requested: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        sandbox_children: None,
    };

    let agent = Agent::new(
        AgentConfig {
            name: "test-agent".to_string(),
            max_parallel_jobs: 1,
            job_timeout: Duration::from_secs(60),
            stuck_threshold: Duration::from_secs(60),
            repair_check_interval: Duration::from_secs(30),
            max_repair_attempts: 1,
            use_planning: false,
            session_idle_timeout: Duration::from_secs(300),
            allow_local_tools: false,
            max_cost_per_day_cents: None,
            max_actions_per_hour: None,
            max_tool_iterations,
            max_context_messages: 200,
            thinking_enabled,
            thinking_budget_tokens: 128,
            auto_approve_tools: false,
            main_tool_profile: crate::tools::ToolProfile::Standard,
            worker_tool_profile: crate::tools::ToolProfile::Restricted,
            subagent_tool_profile: crate::tools::ToolProfile::ExplicitOnly,
            subagent_transparency_level: "balanced".to_string(),
            model_thinking_overrides: HashMap::new(),
            workspace_mode: "unrestricted".to_string(),
            workspace_root: None,
            notify_channel: None,
            model_guidance_enabled: true,
            cli_skin: "cockpit".to_string(),
            personality_pack: "balanced".to_string(),
            persona_seed: "default".to_string(),
            checkpoints_enabled: true,
            max_checkpoints: 50,
            browser_backend: "chromium".to_string(),
            cloud_browser_provider: None,
        },
        deps,
        channels,
        None,
        None,
        None,
        Some(Arc::new(ContextManager::new(1))),
        None,
    );

    (agent, recording_channel)
}

async fn make_session_and_thread() -> (Arc<Mutex<Session>>, Uuid) {
    let session = Arc::new(Mutex::new(Session::new("user-1")));
    let thread_id = {
        let mut guard = session.lock().await;
        let thread = guard.create_thread();
        thread.start_turn("test request");
        thread.id
    };
    (session, thread_id)
}

async fn register_tool(
    registry: &Arc<ToolRegistry>,
    name: &str,
    approval: ApprovalRequirement,
    result: &str,
) {
    registry
        .register(Arc::new(TestTool::new(name, approval, result)))
        .await;
}

fn count_prompt(messages: &[ChatMessage], prompt: &str) -> usize {
    messages.iter().filter(|msg| msg.content == prompt).count()
}

fn contains_prompt(messages: &[ChatMessage], prompt: &str) -> bool {
    count_prompt(messages, prompt) > 0
}

fn tool_call(name: &str) -> ToolCall {
    ToolCall {
        id: format!("call_{}", name),
        name: name.to_string(),
        arguments: serde_json::json!({ "query": "demo" }),
    }
}

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
        RecordedChannelEvent::Status(StatusUpdate::ToolResult { name, preview })
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
