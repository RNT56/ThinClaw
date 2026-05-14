//! Root-independent built-in tools.

pub mod advisor;
pub mod agent_control;
pub mod agent_management;
pub mod apple_mail;
#[cfg(feature = "browser")]
pub mod browser;
pub mod browser_a11y;
pub mod browser_cloud;
pub mod camera_capture;
pub mod canvas;
pub mod clarify;
pub mod desktop_autonomy;
pub mod device_info;
pub mod discord_actions;
pub mod echo;
pub mod execute_code;
pub mod extension_tools;
pub mod external_memory;
#[cfg(feature = "document-extraction")]
pub mod extract_document;
pub mod file;
pub mod homeassistant;
pub mod html_converter;
pub mod http;
pub mod job;
pub mod json;
pub mod learning;
pub mod llm_tools;
pub mod location;
pub mod memory;
pub mod moa;
#[cfg(feature = "nostr")]
pub mod nostr_actions;
pub mod process;
pub mod screen_capture;
pub mod search_files;
pub mod send_message;
pub mod shell;
pub mod shell_security;
pub mod skill;
pub mod slack_actions;
pub mod subagent;
pub mod telegram_actions;
pub mod time;
pub mod todo;
pub mod tts;
pub mod vision;

pub use advisor::{
    ADVISOR_TOOL_NAME, AdvisorCallBudget, AdvisorConsultationEnvelope, AdvisorConsultationMode,
    AdvisorDecision, AdvisorEnvelopeStatus, AdvisorRecommendation, ConsultAdvisorTool,
    execute_advisor_consultation,
};
pub use agent_control::{AgentThinkTool, EmitUserMessageTool};
pub use agent_management::{
    AgentManagementPort, AgentToolRecord, AgentToolWorkspace, CreateAgentTool, ListAgentsTool,
    MessageAgentTool, RemoveAgentTool, UpdateAgentTool,
};
pub use apple_mail::AppleMailTool;
#[cfg(feature = "browser")]
pub use browser::{BrowserDockerRuntime, BrowserTool};
pub use browser_a11y::AgentBrowserTool;
pub use camera_capture::CameraCaptureTool;
pub use canvas::{CanvasAction, CanvasTool, UiComponent};
pub use clarify::ClarifyTool;
pub use desktop_autonomy::{DesktopAutonomyPort, DesktopAutonomyTool};
pub use device_info::DeviceInfoTool;
pub use discord_actions::DiscordActionsTool;
pub use echo::EchoTool;
pub use execute_code::{ExecuteCodeTool, ToolRpcHost};
pub use extension_tools::{
    ExtensionManagementPort, ToolActivateTool, ToolAuthRequestContext, ToolAuthTool,
    ToolExtensionKind, ToolInstallTool, ToolListTool, ToolRemoveTool, ToolSearchTool,
};
pub use external_memory::{
    ExternalMemoryExportTool, ExternalMemoryOffTool, ExternalMemoryPort,
    ExternalMemoryProviderConfig, ExternalMemoryProviderStatus, ExternalMemoryRecallTool,
    ExternalMemorySetupTool, ExternalMemoryStatusTool,
};
#[cfg(feature = "document-extraction")]
pub use extract_document::ExtractDocumentTool;
pub use file::{
    ApplyPatchTool, FileToolHost, GrepTool, ListDirTool, ReadFileTool, WriteFileTool,
    effective_base_dir, validate_path,
};
pub use homeassistant::HomeAssistantTool;
pub use html_converter::convert_html_to_markdown;
pub use http::HttpTool;
pub use job::{DANGEROUS_ENV_VARS, validate_env_var_name};
pub use json::JsonTool;
pub use learning::{
    PROMPT_TARGETS, SKILL_FILE_NAME, append_markdown_section, artifact_name_for_skill,
    find_section_byte_range, normalize_prompt_target, prompt_manage_user_target,
    remove_markdown_section, upsert_markdown_section, validate_agents_prompt_safety,
    validate_prompt_content, validate_prompt_manage_available, validate_relative_skill_path,
    validate_skill_admin_available,
};
pub use llm_tools::{
    LlmListModelsTool, LlmSelectTool, SharedModelOverride, new_shared_model_override,
};
pub use location::LocationTool;
pub use memory::{
    APPEND_ONLY_IDENTITY_FILES, DELETE_PROTECTED_FILES, FREELY_REWRITABLE_IDENTITY_FILES,
    MemoryConversationKind, MemoryScope, actor_scoped_path, memory_conversation_kind,
    resolve_memory_write_path, shared_root_path, split_scoped_target,
};
pub use moa::MoaTool;
#[cfg(feature = "nostr")]
pub use nostr_actions::NostrActionsTool;
pub use process::{ProcessTool, SharedProcessRegistry, start_reaper};
pub use screen_capture::ScreenCaptureTool;
pub use search_files::SearchFilesTool;
pub use send_message::{SendMessageFn, SendMessageTool};
pub use shell::{
    AcpTerminalExecution, AcpTerminalExecutor, ShellSafetyOptions, ShellSmartApprover, ShellTool,
};
pub use skill::{
    ensure_skill_admin_available, ensure_skill_allowed, is_skipped_package_name,
    normalize_tap_path, relative_path_is_safe, restricted_skill_names, validate_github_repo,
    validate_repo_path_component, validate_repo_relative_path,
};
pub use slack_actions::SlackActionsTool;
pub use subagent::{
    CancelSubagentTool, ListSubagentsTool, SpawnSubagentTool, SubagentSpawnRequest,
    SubagentToolPort,
};
pub use telegram_actions::TelegramActionsTool;
pub use time::TimeTool;
pub use todo::{SharedTodoStore, TodoItem, TodoStatus, TodoStore, TodoTool, new_shared_todo_store};
pub use tts::TtsTool;
pub use vision::VisionAnalyzeTool;
