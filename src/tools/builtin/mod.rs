//! Built-in tools that come with the agent.

pub mod advisor;
pub mod llm_tools;

mod agent_control;
pub mod agent_management;
mod apple_mail;
mod browser;
mod browser_a11y;
mod browser_cloud;
mod camera_capture;
mod canvas;
mod clarify;
mod desktop_autonomy;
mod device_info;
mod discord_actions;
mod echo;
mod execute_code;
pub mod extension_tools;
mod external_memory;
#[cfg(feature = "document-extraction")]
mod extract_document;
mod file;
mod homeassistant;
mod http;
mod job;
mod json;
mod learning_tools;
mod location;
mod memory;
mod moa;
mod nostr_actions;
pub(crate) mod process;
pub mod routine;
mod screen_capture;
mod search_files;
mod send_message;
pub(crate) mod shell;
pub mod shell_security;
pub mod skill_tools;
mod slack_actions;
pub mod subagent;
mod telegram_actions;
mod time;
pub(crate) mod todo;
mod tts;
mod vision;

pub use crate::sandbox_types::PromptQueue;
pub use agent_control::{AgentThinkTool, EmitUserMessageTool};
pub use agent_management::{
    CreateAgentTool, ListAgentsTool, MessageAgentTool, RemoveAgentTool, UpdateAgentTool,
};
pub use apple_mail::AppleMailTool;
pub use browser::BrowserTool;
pub use browser_a11y::AgentBrowserTool;
pub use camera_capture::CameraCaptureTool;
pub use canvas::{CanvasAction, CanvasTool, UiComponent};
pub use clarify::ClarifyTool;
pub use desktop_autonomy::DesktopAutonomyTool;
pub use device_info::DeviceInfoTool;
pub use discord_actions::DiscordActionsTool;
pub use echo::EchoTool;
pub use execute_code::ExecuteCodeTool;
pub use extension_tools::{
    ToolActivateTool, ToolAuthTool, ToolInstallTool, ToolListTool, ToolRemoveTool, ToolSearchTool,
};
pub use external_memory::{ExternalMemoryRecallTool, ExternalMemoryStatusTool};
#[cfg(feature = "document-extraction")]
pub use extract_document::ExtractDocumentTool;
pub use file::{ApplyPatchTool, GrepTool, ListDirTool, ReadFileTool, WriteFileTool};
pub use homeassistant::HomeAssistantTool;
pub use http::HttpTool;
pub use job::{
    CancelJobTool, CreateJobTool, JobEventsTool, JobPromptTool, JobStatusTool, ListJobsTool,
};
pub use json::JsonTool;
pub use learning_tools::{
    LearningFeedbackTool, LearningHistoryTool, LearningOutcomesTool, LearningProposalReviewTool,
    LearningStatusTool, PromptManageTool, SkillManageTool,
};
pub use llm_tools::{
    LlmListModelsTool, LlmSelectTool, SharedModelOverride, new_shared_model_override,
};
pub use location::LocationTool;
pub use memory::{
    MemoryDeleteTool, MemoryReadTool, MemorySearchTool, MemoryTreeTool, MemoryWriteTool,
    SessionSearchTool,
};
pub use moa::MoaTool;
pub use nostr_actions::NostrActionsTool;
pub use process::{ProcessTool, SharedProcessRegistry, start_reaper};
pub use routine::{
    RoutineCreateTool, RoutineDeleteTool, RoutineHistoryTool, RoutineListTool, RoutineUpdateTool,
};
pub use screen_capture::ScreenCaptureTool;
pub use search_files::SearchFilesTool;
pub use send_message::{SendMessageFn, SendMessageTool};
pub use shell::ShellTool;
pub use skill_tools::{
    SkillInstallTool, SkillListTool, SkillReadTool, SkillReloadTool, SkillRemoveTool,
    SkillSearchTool,
};
pub use slack_actions::SlackActionsTool;
pub use subagent::{CancelSubagentTool, ListSubagentsTool, SpawnSubagentTool};
pub use telegram_actions::TelegramActionsTool;
pub use time::TimeTool;
pub use todo::{SharedTodoStore, TodoTool, new_shared_todo_store};
pub use tts::TtsTool;
pub use vision::VisionAnalyzeTool;

mod html_converter;

pub use html_converter::convert_html_to_markdown;
