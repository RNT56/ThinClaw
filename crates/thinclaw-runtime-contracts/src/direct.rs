use crate::asset::{AssetRecord, AssetRef};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DirectAttachedDocument {
    pub id: String,
    pub name: String,
    #[serde(default, alias = "asset_ref")]
    pub asset_ref: Option<AssetRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DirectChatMessage {
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub images: Option<Vec<String>>,
    #[serde(default)]
    pub assets: Option<Vec<AssetRef>>,
    #[serde(default, alias = "attached_docs")]
    pub attached_docs: Option<Vec<DirectAttachedDocument>>,
    #[serde(default, alias = "is_summary")]
    pub is_summary: Option<bool>,
    #[serde(default, alias = "original_messages")]
    pub original_messages: Option<Vec<DirectChatMessage>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DirectChatPayload {
    pub model: String,
    pub messages: Vec<DirectChatMessage>,
    pub temperature: f32,
    #[serde(alias = "top_p")]
    pub top_p: f32,
    #[serde(default, alias = "web_search_enabled")]
    pub web_search_enabled: bool,
    #[serde(default)]
    pub auto_mode: bool,
    #[serde(default, alias = "project_id")]
    pub project_id: Option<String>,
    #[serde(default, alias = "conversation_id")]
    pub conversation_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DirectTokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DirectStreamChunk {
    pub content: String,
    pub done: bool,
    #[serde(default)]
    pub usage: Option<DirectTokenUsage>,
    #[serde(default)]
    pub context_update: Option<Vec<DirectChatMessage>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DirectConversation {
    pub id: String,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DirectDocumentUploadResponse {
    pub path: String,
    pub asset: AssetRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DirectDocumentIngestResponse {
    pub document_id: String,
    pub asset: AssetRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DirectTtsResponse {
    /// Base64-encoded audio bytes for existing web audio playback flows.
    pub audio_bytes: String,
    pub asset: AssetRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DirectSttResponse {
    pub text: String,
    pub asset: AssetRecord,
}
