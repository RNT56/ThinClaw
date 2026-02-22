//! Request and response types for OpenClaw Tauri commands
//!
//! All typed structs used by the command layer, including
//! status responses, input types, session/message models, and diagnostics.

use super::super::config::{AgentProfile, CustomSecret};

/// OpenClaw status response
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct OpenClawStatus {
    pub gateway_running: bool,
    pub ws_connected: bool,
    pub slack_enabled: bool,
    pub telegram_enabled: bool,
    pub port: u16,
    pub gateway_mode: String,
    pub remote_url: Option<String>,
    pub remote_token: Option<String>,
    pub device_id: String,
    pub auth_token: String,
    pub state_dir: String,
    pub has_huggingface_token: bool,
    pub huggingface_granted: bool,
    pub has_anthropic_key: bool,
    pub anthropic_granted: bool,
    pub has_brave_key: bool,
    pub brave_granted: bool,
    pub has_openai_key: bool,
    pub openai_granted: bool,
    pub has_openrouter_key: bool,
    pub openrouter_granted: bool,
    pub has_gemini_key: bool,
    pub gemini_granted: bool,
    pub has_groq_key: bool,
    pub groq_granted: bool,
    pub custom_secrets: Vec<CustomSecret>,
    pub node_host_enabled: bool,
    pub local_inference_enabled: bool,
    pub selected_cloud_brain: Option<String>,
    pub selected_cloud_model: Option<String>,
    pub setup_completed: bool,
    pub auto_start_gateway: bool,
    pub dev_mode_wizard: bool,
    pub custom_llm_url: Option<String>,
    pub custom_llm_key: Option<String>,
    pub custom_llm_model: Option<String>,
    pub custom_llm_enabled: bool,
    pub enabled_cloud_providers: Vec<String>,
    pub enabled_cloud_models: std::collections::HashMap<String, Vec<String>>,
    pub profiles: Vec<AgentProfile>,
    // --- Implicit cloud provider status ---
    pub has_xai_key: bool,
    pub xai_granted: bool,
    pub has_venice_key: bool,
    pub venice_granted: bool,
    pub has_together_key: bool,
    pub together_granted: bool,
    pub has_moonshot_key: bool,
    pub moonshot_granted: bool,
    pub has_minimax_key: bool,
    pub minimax_granted: bool,
    pub has_nvidia_key: bool,
    pub nvidia_granted: bool,
    pub has_qianfan_key: bool,
    pub qianfan_granted: bool,
    pub has_mistral_key: bool,
    pub mistral_granted: bool,
    pub has_xiaomi_key: bool,
    pub xiaomi_granted: bool,
    pub has_bedrock_key: bool,
    pub bedrock_granted: bool,
}

/// Slack configuration input
#[derive(Debug, Clone, serde::Deserialize, specta::Type)]
pub struct SlackConfigInput {
    pub enabled: bool,
    pub bot_token: Option<String>,
    pub app_token: Option<String>,
}

/// Telegram configuration input
#[derive(Debug, Clone, serde::Deserialize, specta::Type)]
pub struct TelegramConfigInput {
    pub enabled: bool,
    pub bot_token: Option<String>,
    pub dm_policy: String,
    pub groups_enabled: bool,
}

/// Session info from gateway
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct OpenClawSession {
    #[serde(alias = "key")]
    pub session_key: String,
    #[serde(alias = "displayName")]
    pub title: Option<String>,
    #[serde(alias = "updatedAt")]
    pub updated_at_ms: Option<f64>,
    #[serde(alias = "lastChannel")]
    pub source: Option<String>,
}

/// Sessions list response
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct OpenClawSessionsResponse {
    pub sessions: Vec<OpenClawSession>,
}

/// Message in chat history
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct OpenClawMessage {
    #[serde(alias = "uuid")]
    pub id: String,
    pub role: String,
    #[serde(alias = "ts", alias = "timestamp", alias = "createdAt")]
    pub ts_ms: f64,
    #[serde(alias = "content", alias = "message")]
    pub text: String,
    #[serde(alias = "channel")]
    pub source: Option<String>,
    #[serde(default)]
    #[specta(skip)]
    pub metadata: Option<serde_json::Value>,
}

/// Chat history response
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct OpenClawHistoryResponse {
    pub messages: Vec<OpenClawMessage>,
    pub has_more: bool,
}

/// RPC result response
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct OpenClawRpcResponse {
    pub ok: bool,
    pub message: Option<String>,
}

/// Diagnostic info
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct OpenClawDiagnostics {
    pub timestamp: String,
    pub gateway_running: bool,
    pub ws_connected: bool,
    pub version: String,
    pub platform: String,
    pub port: Option<u16>,
    pub state_dir: Option<String>,
    pub slack_enabled: Option<bool>,
    pub telegram_enabled: Option<bool>,
}
