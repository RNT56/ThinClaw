use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rust_decimal::Decimal;
use tokio::sync::{Mutex, Notify, oneshot};
use uuid::Uuid;

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
pub(super) struct CapturedRequest {
    pub(super) messages: Vec<ChatMessage>,
    pub(super) context_documents: Vec<String>,
    pub(super) tool_names: Vec<String>,
    pub(super) max_tokens: Option<u32>,
    pub(super) thinking: ThinkingConfig,
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
pub(super) struct ScriptedResponse {
    result: ScriptedResult,
    finish_reason: FinishReason,
    thinking_content: Option<String>,
}

impl ScriptedResponse {
    pub(super) fn text(text: impl Into<String>, finish_reason: FinishReason) -> Self {
        Self {
            result: ScriptedResult::Text(text.into()),
            finish_reason,
            thinking_content: None,
        }
    }

    pub(super) fn text_with_thinking(
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

    pub(super) fn tool_calls(tool_calls: Vec<ToolCall>, finish_reason: FinishReason) -> Self {
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

pub(super) struct ScriptedLlm {
    model_name: String,
    responses: Mutex<VecDeque<ScriptedResponse>>,
    requests: Mutex<Vec<CapturedRequest>>,
    stream_support: StreamSupport,
}

struct DropSignal(Option<oneshot::Sender<()>>);

impl Drop for DropSignal {
    fn drop(&mut self) {
        if let Some(tx) = self.0.take() {
            let _ = tx.send(());
        }
    }
}

pub(super) struct BlockingLlm {
    model_name: String,
    started: Notify,
    release: Notify,
    dropped_tx: Mutex<Option<oneshot::Sender<()>>>,
}

impl BlockingLlm {
    pub(super) fn new(model_name: impl Into<String>, dropped_tx: oneshot::Sender<()>) -> Self {
        Self {
            model_name: model_name.into(),
            started: Notify::new(),
            release: Notify::new(),
            dropped_tx: Mutex::new(Some(dropped_tx)),
        }
    }

    pub(super) async fn wait_started(&self) {
        self.started.notified().await;
    }

    #[allow(dead_code)]
    pub(super) fn release(&self) {
        self.release.notify_waiters();
    }

    async fn wait_until_released(&self) {
        let _drop_signal = DropSignal(self.dropped_tx.lock().await.take());
        self.started.notify_waiters();
        self.release.notified().await;
    }
}

#[async_trait]
impl LlmProvider for BlockingLlm {
    fn model_name(&self) -> &str {
        &self.model_name
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    fn stream_support(&self) -> StreamSupport {
        StreamSupport::Unsupported
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.wait_until_released().await;
        Ok(CompletionResponse {
            content: "released".to_string(),
            provider_model: Some(self.model_name.clone()),
            cost_usd: Some(0.0),
            thinking_content: None,
            input_tokens: 1,
            output_tokens: 1,
            finish_reason: FinishReason::Stop,
            token_capture: None,
        })
    }

    async fn complete_with_tools(
        &self,
        _request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        self.wait_until_released().await;
        Ok(ToolCompletionResponse {
            content: Some("released".to_string()),
            provider_model: Some(self.model_name.clone()),
            cost_usd: Some(0.0),
            tool_calls: Vec::new(),
            thinking_content: None,
            input_tokens: 1,
            output_tokens: 1,
            finish_reason: FinishReason::Stop,
            token_capture: None,
        })
    }
}

impl ScriptedLlm {
    pub(super) fn new(model_name: impl Into<String>, responses: Vec<ScriptedResponse>) -> Self {
        Self::with_stream_support(model_name, responses, StreamSupport::Simulated)
    }

    pub(super) fn with_stream_support(
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

    pub(super) async fn requests(&self) -> Vec<CapturedRequest> {
        self.requests.lock().await.clone()
    }

    pub(super) async fn response_count(&self) -> usize {
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
pub(super) enum RecordedChannelEvent {
    Status(StatusUpdate),
    Draft(String),
    Deleted,
    Response,
}

#[derive(Clone)]
pub(super) struct RecordingChannel {
    name: String,
    stream_mode: StreamMode,
    formatting_hints: Option<String>,
    events: Arc<Mutex<Vec<RecordedChannelEvent>>>,
}

impl RecordingChannel {
    pub(super) fn new(name: impl Into<String>, stream_mode: StreamMode) -> Self {
        Self {
            name: name.into(),
            stream_mode,
            formatting_hints: None,
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn with_formatting_hints(mut self, hints: impl Into<String>) -> Self {
        self.formatting_hints = Some(hints.into());
        self
    }

    pub(super) async fn events(&self) -> Vec<RecordedChannelEvent> {
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

pub(super) struct TestTool {
    name: String,
    approval: ApprovalRequirement,
    result: String,
}

pub(super) struct BlockingTool {
    name: String,
    started: Notify,
    release: Notify,
    dropped_tx: Mutex<Option<oneshot::Sender<()>>>,
}

impl BlockingTool {
    pub(super) fn new(name: impl Into<String>, dropped_tx: oneshot::Sender<()>) -> Self {
        Self {
            name: name.into(),
            started: Notify::new(),
            release: Notify::new(),
            dropped_tx: Mutex::new(Some(dropped_tx)),
        }
    }

    pub(super) async fn wait_started(&self) {
        self.started.notified().await;
    }

    #[allow(dead_code)]
    pub(super) fn release(&self) {
        self.release.notify_waiters();
    }
}

#[async_trait]
impl Tool for BlockingTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        "Blocking test tool"
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
        let _drop_signal = DropSignal(self.dropped_tx.lock().await.take());
        self.started.notify_waiters();
        self.release.notified().await;
        Ok(ToolOutput::text("released", Duration::from_millis(1)))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }
}

impl TestTool {
    pub(super) fn new(
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

pub(super) fn runtime_status(
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

pub(super) async fn make_runtime_manager(
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

pub(super) async fn make_runtime_manager_for_mode(
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

pub(super) async fn make_test_agent(
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

pub(super) async fn make_test_agent_with_channel(
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

pub(super) async fn make_session_and_thread() -> (Arc<Mutex<Session>>, Uuid) {
    let session = Arc::new(Mutex::new(Session::new("user-1")));
    let thread_id = {
        let mut guard = session.lock().await;
        let thread = guard.create_thread();
        thread.start_turn("test request");
        thread.id
    };
    (session, thread_id)
}

pub(super) async fn register_tool(
    registry: &Arc<ToolRegistry>,
    name: &str,
    approval: ApprovalRequirement,
    result: &str,
) {
    registry
        .register(Arc::new(TestTool::new(name, approval, result)))
        .await;
}

pub(super) fn count_prompt(messages: &[ChatMessage], prompt: &str) -> usize {
    messages.iter().filter(|msg| msg.content == prompt).count()
}

pub(super) fn contains_prompt(messages: &[ChatMessage], prompt: &str) -> bool {
    count_prompt(messages, prompt) > 0
}

pub(super) fn tool_call(name: &str) -> ToolCall {
    ToolCall {
        id: format!("call_{}", name),
        name: name.to_string(),
        arguments: serde_json::json!({ "query": "demo" }),
    }
}
