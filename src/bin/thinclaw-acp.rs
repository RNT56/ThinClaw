#![cfg(feature = "acp")]

use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use async_trait::async_trait;
use clap::Parser;
use rust_decimal::Decimal;
use thinclaw::agent::{Agent, AgentDeps, SessionManager};
use thinclaw::app::{AppBuilder, AppBuilderFlags};
use thinclaw::channels::ChannelManager;
use thinclaw::channels::acp;
use thinclaw::channels::web::log_layer::{LogBroadcaster, init_tracing};
use thinclaw::config::{AgentConfig, Config, LlmBackend, SafetyConfig, SkillsConfig};
use thinclaw::context::{ContextManager, JobContext};
use thinclaw::error::LlmError;
use thinclaw::hooks::HookRegistry;
use thinclaw::llm::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, ModelMetadata,
    Role, StreamSupport, ToolCall, ToolCompletionRequest, ToolCompletionResponse,
};
use thinclaw::safety::SafetyLayer;
use thinclaw::tools::{
    ApprovalRequirement, Tool, ToolError, ToolOutput, ToolProfile, ToolRegistry,
};

#[derive(Debug, Parser)]
#[command(name = "thinclaw-acp")]
#[command(about = "Run ThinClaw as an Agent Client Protocol stdio agent")]
struct Cli {
    /// Optional ThinClaw TOML config path.
    #[arg(long)]
    config: Option<PathBuf>,

    /// Optional workspace root override for this ACP session.
    #[arg(long)]
    workspace: Option<PathBuf>,

    /// Optional model override for the ACP process.
    #[arg(long)]
    model: Option<String>,

    /// Disable database-backed config and persistence.
    #[arg(long)]
    no_db: bool,

    /// Emit debug logs to stderr.
    #[arg(long)]
    debug: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if std::env::var_os("THINCLAW_ACP_STDIO_SMOKE").is_some() {
        let (acp_channel, _) = acp::channel_pair();
        return acp::run_stdio_without_agent(acp_channel.shared_state()).await;
    }

    if std::env::var_os("THINCLAW_ACP_AGENT_STDIO_SMOKE").is_some() {
        return run_agent_stdio_smoke().await;
    }

    let _ = dotenvy::dotenv();
    thinclaw::bootstrap::load_thinclaw_env();

    let log_broadcaster = Arc::new(LogBroadcaster::new());
    let _log_level_handle = init_tracing(Arc::clone(&log_broadcaster), cli.debug);

    let mut config = Config::from_env_with_toml_options(cli.config.as_deref(), !cli.no_db).await?;
    config.channels.acp_enabled = true;
    config.agent.main_tool_profile = thinclaw::tools::ToolProfile::Acp;
    if let Some(workspace) = cli.workspace {
        config.agent.workspace_mode = "project".to_string();
        config.agent.workspace_root = Some(workspace);
    }
    if let Some(model) = cli.model {
        apply_model_override(&mut config, model);
    }

    let components = AppBuilder::new(
        config.clone(),
        AppBuilderFlags { no_db: cli.no_db },
        cli.config.clone(),
        Arc::clone(&log_broadcaster),
    )
    .build_all()
    .await?;

    let channels = Arc::new(ChannelManager::new());
    let (acp_channel, acp_outbound_rx) = acp::channel_pair();
    let acp_state = acp_channel.shared_state();
    channels.add(Box::new(acp_channel)).await;

    let session_manager = Arc::new(SessionManager::new().with_hooks(components.hooks.clone()));
    let shared_context_manager = Arc::clone(&components.context_manager);
    let shared_db = components.db.clone();
    let shared_secrets_store = components.secrets_store.clone();

    components.tools.register_job_tools(
        Arc::clone(&shared_context_manager),
        None,
        shared_db.clone(),
        None,
        None,
        None,
        None,
        None,
        shared_secrets_store.clone(),
    );

    let model_override = thinclaw::tools::builtin::new_shared_model_override();
    components.tools.register_llm_tools(
        model_override.clone(),
        Arc::clone(&components.llm),
        components.cheap_llm.as_ref().map(Arc::clone),
    );

    let shared_agent_router = Arc::new(thinclaw::agent::AgentRouter::new());
    let agent_registry = Arc::new(thinclaw::agent::agent_registry::AgentRegistry::new(
        Arc::clone(&shared_agent_router),
        components.db.clone(),
    ));
    if components.db.is_some() {
        let _ = agent_registry.load_from_db().await;
    }
    components
        .tools
        .register_agent_management_tools(Arc::clone(&agent_registry));

    let restart_requested = Arc::new(AtomicBool::new(false));
    let deps = AgentDeps {
        store: components.db,
        llm: components.llm,
        cheap_llm: components.cheap_llm,
        safety: components.safety,
        tools: components.tools,
        workspace: components.workspace,
        extension_manager: components.extension_manager,
        skill_registry: components.skill_registry,
        skill_catalog: components.skill_catalog,
        skills_config: config.skills.clone(),
        hooks: components.hooks,
        cost_guard: components.cost_guard,
        sse_sender: None,
        agent_router: Some(shared_agent_router),
        agent_registry: Some(agent_registry),
        canvas_store: None,
        subagent_executor: None,
        cost_tracker: Some(components.cost_tracker),
        response_cache: Some(components.response_cache),
        llm_runtime: Some(components.llm_runtime),
        routing_policy: Some(components.routing_policy),
        model_override: Some(model_override),
        restart_requested: Arc::clone(&restart_requested),
        sandbox_children: None,
    };

    let agent = Arc::new(Agent::new(
        config.agent.clone(),
        deps,
        channels,
        Some(config.heartbeat.clone()),
        Some(config.hygiene.clone()),
        Some(config.routines.clone()),
        Some(shared_context_manager),
        Some(session_manager),
    ));

    agent.scheduler().tools().register_job_tools(
        Arc::clone(agent.context_manager()),
        None,
        shared_db,
        Some(Arc::clone(agent.scheduler())),
        None,
        None,
        None,
        None,
        shared_secrets_store,
    );

    acp::run_stdio(agent, acp_outbound_rx, acp_state).await?;

    if restart_requested.load(Ordering::SeqCst) {
        eprintln!("ThinClaw ACP restart was requested; exiting for supervisor restart.");
    }
    Ok(())
}

async fn run_agent_stdio_smoke() -> anyhow::Result<()> {
    let channels = Arc::new(ChannelManager::new());
    let (acp_channel, acp_outbound_rx) = acp::channel_pair();
    let acp_state = acp_channel.shared_state();
    channels.add(Box::new(acp_channel)).await;

    let tools = Arc::new(ToolRegistry::new());
    tools.register_builtin(Arc::new(SmokeApprovalTool)).await;

    let llm: Arc<dyn LlmProvider> = Arc::new(SmokeLlm);
    let restart_requested = Arc::new(AtomicBool::new(false));
    let deps = AgentDeps {
        store: None,
        llm: Arc::clone(&llm),
        cheap_llm: Some(llm),
        safety: Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: false,
            redact_pii_in_prompts: false,
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
        cost_guard: Arc::new(thinclaw::agent::cost_guard::CostGuard::new(
            thinclaw::agent::cost_guard::CostGuardConfig::default(),
        )),
        sse_sender: None,
        agent_router: None,
        agent_registry: None,
        canvas_store: None,
        subagent_executor: None,
        cost_tracker: None,
        response_cache: None,
        llm_runtime: None,
        routing_policy: None,
        model_override: None,
        restart_requested: Arc::clone(&restart_requested),
        sandbox_children: None,
    };

    let agent = Arc::new(Agent::new(
        AgentConfig {
            name: "acp-smoke-agent".to_string(),
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
            max_tool_iterations: 4,
            max_context_messages: 200,
            thinking_enabled: false,
            thinking_budget_tokens: 128,
            auto_approve_tools: false,
            subagent_transparency_level: "balanced".to_string(),
            main_tool_profile: ToolProfile::Acp,
            worker_tool_profile: ToolProfile::Restricted,
            subagent_tool_profile: ToolProfile::ExplicitOnly,
            model_thinking_overrides: std::collections::HashMap::new(),
            workspace_mode: "unrestricted".to_string(),
            workspace_root: None,
            notify_channel: None,
            model_guidance_enabled: false,
            cli_skin: "cockpit".to_string(),
            personality_pack: "balanced".to_string(),
            persona_seed: "default".to_string(),
            checkpoints_enabled: false,
            max_checkpoints: 0,
            browser_backend: "chromium".to_string(),
            cloud_browser_provider: None,
        },
        deps,
        channels,
        None,
        None,
        None,
        Some(Arc::new(ContextManager::new(1))),
        Some(Arc::new(SessionManager::new())),
    ));

    acp::run_stdio(agent, acp_outbound_rx, acp_state).await
}

struct SmokeLlm;

impl SmokeLlm {
    fn last_user_text(messages: &[ChatMessage]) -> String {
        messages
            .iter()
            .rev()
            .find(|message| matches!(message.role, Role::User))
            .map(|message| message.content.clone())
            .unwrap_or_default()
    }

    fn saw_tool_result(messages: &[ChatMessage]) -> bool {
        messages
            .iter()
            .any(|message| matches!(message.role, Role::Tool))
    }

    async fn maybe_wait_for_slow_prompt(messages: &[ChatMessage]) {
        if Self::last_user_text(messages).contains("slow") {
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    }
}

#[async_trait]
impl LlmProvider for SmokeLlm {
    fn model_name(&self) -> &str {
        "acp-smoke-llm"
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Self::maybe_wait_for_slow_prompt(&request.messages).await;
        Ok(CompletionResponse {
            content: "smoke streamed alpha beta gamma".to_string(),
            provider_model: Some(self.model_name().to_string()),
            cost_usd: Some(0.0),
            thinking_content: None,
            input_tokens: 3,
            output_tokens: 5,
            finish_reason: FinishReason::Stop,
            token_capture: None,
        })
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        Self::maybe_wait_for_slow_prompt(&request.messages).await;
        if Self::saw_tool_result(&request.messages) {
            return Ok(ToolCompletionResponse {
                content: Some("approval tool result observed".to_string()),
                provider_model: Some(self.model_name().to_string()),
                cost_usd: Some(0.0),
                tool_calls: Vec::new(),
                thinking_content: None,
                input_tokens: 6,
                output_tokens: 4,
                finish_reason: FinishReason::Stop,
                token_capture: None,
            });
        }

        let wants_approval = Self::last_user_text(&request.messages).contains("approval");
        Ok(ToolCompletionResponse {
            content: if wants_approval {
                None
            } else {
                Some("smoke streamed alpha beta gamma".to_string())
            },
            provider_model: Some(self.model_name().to_string()),
            cost_usd: Some(0.0),
            tool_calls: if wants_approval {
                vec![ToolCall {
                    id: "smoke_call_write_file".to_string(),
                    name: "write_file".to_string(),
                    arguments: serde_json::json!({ "message": "approval requested" }),
                }]
            } else {
                Vec::new()
            },
            thinking_content: None,
            input_tokens: 5,
            output_tokens: 5,
            finish_reason: if wants_approval {
                FinishReason::ToolUse
            } else {
                FinishReason::Stop
            },
            token_capture: None,
        })
    }

    fn stream_support(&self) -> StreamSupport {
        StreamSupport::Native
    }

    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
        Ok(ModelMetadata {
            id: self.model_name().to_string(),
            context_length: Some(4096),
        })
    }
}

struct SmokeApprovalTool;

#[async_trait]
impl Tool for SmokeApprovalTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Deterministic ACP smoke tool that always requires approval."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "message": { "type": "string" }
            },
            "additionalProperties": false
        })
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput::text(
            "approval tool output",
            Duration::from_millis(1),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Always
    }
}

fn apply_model_override(config: &mut Config, model: String) {
    match config.llm.backend {
        LlmBackend::OpenAi => {
            if let Some(ref mut provider) = config.llm.openai {
                provider.model = model;
            }
        }
        LlmBackend::Anthropic => {
            if let Some(ref mut provider) = config.llm.anthropic {
                provider.model = model;
            }
        }
        LlmBackend::Ollama => {
            if let Some(ref mut provider) = config.llm.ollama {
                provider.model = model;
            }
        }
        LlmBackend::OpenAiCompatible => {
            if let Some(ref mut provider) = config.llm.openai_compatible {
                provider.model = model;
            }
        }
        LlmBackend::Tinfoil => {
            if let Some(ref mut provider) = config.llm.tinfoil {
                provider.model = model;
            }
        }
        LlmBackend::Gemini => {
            if let Some(ref mut provider) = config.llm.gemini {
                provider.model = model;
            }
        }
        LlmBackend::Bedrock => {
            if let Some(ref mut provider) = config.llm.bedrock {
                provider.model_id = model;
            }
        }
        LlmBackend::LlamaCpp => {
            if let Some(ref mut provider) = config.llm.llama_cpp {
                provider.model = model;
            }
        }
    }
}
