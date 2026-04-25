#![cfg(feature = "acp")]

use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use clap::Parser;
use thinclaw::agent::{Agent, AgentDeps, SessionManager};
use thinclaw::app::{AppBuilder, AppBuilderFlags};
use thinclaw::channels::ChannelManager;
use thinclaw::channels::acp;
use thinclaw::channels::web::log_layer::{LogBroadcaster, init_tracing};
use thinclaw::config::{Config, LlmBackend};

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
