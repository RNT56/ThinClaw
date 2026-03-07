//! Built-in tools that come with the agent.

mod agent_control;
mod browser;
mod camera_capture;
mod canvas;
mod device_info;
mod discord_actions;
mod echo;
pub mod extension_tools;
mod file;
mod http;
mod job;
mod json;
mod location;
mod memory;
pub mod routine;
mod screen_capture;
pub(crate) mod shell;
pub mod skill_tools;
mod slack_actions;
pub mod subagent;
mod telegram_actions;
mod time;
mod tts;

pub use agent_control::{AgentThinkTool, EmitUserMessageTool};
pub use browser::BrowserTool;
pub use camera_capture::CameraCaptureTool;
pub use canvas::{CanvasAction, CanvasTool, UiComponent};
pub use device_info::DeviceInfoTool;
pub use discord_actions::DiscordActionsTool;
pub use echo::EchoTool;
pub use extension_tools::{
    ToolActivateTool, ToolAuthTool, ToolInstallTool, ToolListTool, ToolRemoveTool, ToolSearchTool,
};
pub use file::{ApplyPatchTool, GrepTool, ListDirTool, ReadFileTool, WriteFileTool};
pub use http::HttpTool;
pub use job::{
    CancelJobTool, CreateJobTool, JobEventsTool, JobPromptTool, JobStatusTool, ListJobsTool,
    PromptQueue,
};
pub use json::JsonTool;
pub use location::LocationTool;
pub use memory::{MemoryReadTool, MemorySearchTool, MemoryTreeTool, MemoryWriteTool};
pub use routine::{
    RoutineCreateTool, RoutineDeleteTool, RoutineHistoryTool, RoutineListTool, RoutineUpdateTool,
};
pub use screen_capture::ScreenCaptureTool;
pub use shell::ShellTool;
pub use skill_tools::{SkillInstallTool, SkillListTool, SkillRemoveTool, SkillSearchTool};
pub use slack_actions::SlackActionsTool;
pub use subagent::{CancelSubagentTool, ListSubagentsTool, SpawnSubagentTool};
pub use telegram_actions::TelegramActionsTool;
pub use time::TimeTool;
pub use tts::TtsTool;

mod html_converter;

pub use html_converter::convert_html_to_markdown;
